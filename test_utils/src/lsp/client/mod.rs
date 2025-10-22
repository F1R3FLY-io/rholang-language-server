use std::collections::HashMap;
use std::io::{self, BufReader, Read, Write};
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, RwLock};
use std::sync::atomic::AtomicU64;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::fs;

#[cfg(unix)]
use nix::sys::signal::{self, Signal};
#[cfg(unix)]
use nix::unistd::Pid;

#[cfg(windows)]
use tokio::net::windows::named_pipe::NamedPipeClient;

use tokio::io::{AsyncWriteExt, split};
use tokio::net::{TcpStream, UnixStream};
use tokio::runtime::Handle;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};
use futures_util::{SinkExt, StreamExt};

use uuid::Uuid;

use tracing::{debug, error, info};
use tracing_subscriber::{self, fmt, prelude::*};

use time::macros::format_description;
use time::UtcOffset;

use tower_lsp::lsp_types::ServerCapabilities;

use serde_json::Value;

pub mod handlers;
pub mod macros;

use crate::lsp::client::handlers::{NotificationHandler, RequestHandler, ResponseHandler};
use crate::lsp::streams::{AsyncLspReadStream, AsyncLspWriteStream, LspStream, WebSocketStreamAdapter};
use crate::lsp::document::LspDocument;
use crate::lsp::events::LspEvent;
use crate::lsp::message_stream::LspMessageStream;

/// Enum representing different communication types with the LSP server.
#[derive(Clone)]
pub enum CommType {
    Stdio,
    Tcp { port: Option<u16> },
    Pipe { path: Option<String> },
    WebSocket { port: Option<u16> },
}

/// Extension trait for joining threads with a timeout.
trait JoinHandleExt {
    fn join_timeout(self, timeout: Duration) -> Result<(), Box<dyn std::any::Any + Send>>;
}

impl JoinHandleExt for JoinHandle<()> {
    fn join_timeout(self, timeout: Duration) -> Result<(), Box<dyn std::any::Any + Send>> {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if self.is_finished() {
                return self.join();
            }
            thread::sleep(Duration::from_millis(100));
        }
        Err(Box::new("Thread join timeout"))
    }
}

/// The LSP client for testing, managing connection, threads, and state.
#[allow(dead_code)]
pub struct LspClient {
    pub server: Mutex<Option<Child>>,
    pub runtime_handle: Handle,
    pub sender: Mutex<Option<Sender<String>>>,
    pub receiver: Mutex<Receiver<String>>,
    pub language_id: String,
    pub server_capabilities: RwLock<Option<ServerCapabilities>>,
    pub request_handlers: HashMap<String, RequestHandler>,
    pub notification_handlers: HashMap<String, NotificationHandler>,
    pub response_handlers: HashMap<String, ResponseHandler>,
    pub requests_by_id: RwLock<HashMap<u64, Arc<Value>>>,
    pub responses_by_id: RwLock<HashMap<u64, Arc<Value>>>,
    pub diagnostics_by_id: RwLock<HashMap<u64, Arc<tower_lsp::lsp_types::PublishDiagnosticsParams>>>,
    pub semantic_tokens_by_uri: RwLock<HashMap<String, Arc<Option<tower_lsp::lsp_types::SemanticTokensResult>>>>,
    pub serial_request_id: AtomicU64,
    pub serial_document_id: AtomicU64,
    pub documents_by_uri: RwLock<HashMap<String, Arc<LspDocument>>>,
    pub output_thread: Mutex<Option<JoinHandle<()>>>,
    pub input_thread: Mutex<Option<JoinHandle<()>>>,
    pub logger_thread: Mutex<Option<JoinHandle<()>>>,
    pub event_sender: Sender<LspEvent>,
    pub tcp_write_stream: Mutex<Option<Arc<Mutex<tokio::io::WriteHalf<TcpStream>>>>>,
    #[cfg(windows)] pub pipe_write_stream: Mutex<Option<Arc<Mutex<tokio::io::WriteHalf<NamedPipeClient>>>>>,
    #[cfg(unix)] pub unix_write_stream: Mutex<Option<Arc<Mutex<tokio::io::WriteHalf<UnixStream>>>>>,
    pub websocket_stream: Mutex<Option<Arc<Mutex<futures_util::stream::SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>>>>>,
    pub generated_pipe_path: Mutex<Option<String>>,
    pub comm_type: CommType,
}

impl LspClient {
    /// Starts the LSP client with the given configuration.
    pub async fn start(
        language_id: String,
        server_path: String,
        comm_type: CommType,
        event_sender: Sender<LspEvent>,
    ) -> io::Result<Self> {
        let runtime_handle = Handle::current();
        let (sender, rx) = channel::<String>();
        let (tx, receiver) = channel::<String>();

        // Get the client's process ID
        let client_pid = std::process::id();

        // Get RNode address from environment variable or default
        let rnode_address = std::env::var("RHOLANG_RNODE_ADDRESS").unwrap_or_else(|_| "localhost".to_string());

        // Get RNode port from environment variable or default
        let rnode_port = std::env::var("RHOLANG_RNODE_PORT")
            .unwrap_or_else(|_| "40402".to_string())
            .parse::<u16>()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, format!("Invalid port: {}", e)))?;

        let log_level = std::env::var("RUST_LOG").unwrap_or("debug".to_string());

        let (output, input, logger, server, tcp_write_stream, pipe_or_unix_write_stream, websocket_stream, generated_pipe_path) =
            match comm_type.clone() {
                CommType::Stdio => {
                    let server_args = &[
                        "--stdio",
                        "--client-process-id", &client_pid.to_string(),
                        "--log-level", &log_level,
                        "--rnode-address", &rnode_address,
                        "--rnode-port", &rnode_port.to_string(),
                        "--no-rnode",  // Tests use parser-only validation (no RNode dependency)
                    ];
                    let mut server = Command::new(&server_path)
                        .args(server_args)
                        .envs(std::env::vars())
                        .stdin(Stdio::piped())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .spawn()?;
                    let output = Box::new(server.stdin.take().expect("Failed to open server stdin")) as Box<dyn LspStream>;
                    let input = Box::new(server.stdout.take().expect("Failed to open server stdout")) as Box<dyn LspStream>;
                    let logger = Box::new(server.stderr.take().expect("Failed to open server stderr")) as Box<dyn LspStream>;
                    (output, input, logger, Some(server), None, None, None, None)
                }
                CommType::Tcp { port } => {
                    let port = port.unwrap_or_else(find_free_port);
                    let server_args = &[
                        "--socket",
                        "--port", &port.to_string(),
                        "--client-process-id", &client_pid.to_string(),
                        "--log-level", &log_level,
                        "--rnode-address", &rnode_address,
                        "--rnode-port", &rnode_port.to_string(),
                        "--no-rnode",  // Tests use parser-only validation (no RNode dependency)
                    ];
                    let mut server = Command::new(&server_path)
                        .args(server_args)
                        .envs(std::env::vars())
                        .stdin(Stdio::null())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .spawn()?;
                    let logger = Box::new(server.stderr.take().expect("Failed to open server stderr")) as Box<dyn LspStream>;
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    let stream = TcpStream::connect(format!("127.0.0.1:{}", port)).await?;
                    stream.set_nodelay(true)?;
                    let (read_half, write_half) = split(stream);
                    let write_stream = Arc::new(Mutex::new(write_half));
                    let output = Box::new(AsyncLspWriteStream::new(
                        Arc::clone(&write_stream),
                        runtime_handle.clone(),
                    )) as Box<dyn LspStream>;
                    let input = Box::new(AsyncLspReadStream::new(read_half, runtime_handle.clone())) as Box<dyn LspStream>;
                    (
                        output,
                        input,
                        logger,
                        Some(server),
                        Some(write_stream),
                        None,
                        None,
                        None,
                    )
                }
                CommType::Pipe { path } => {
                    #[cfg(not(any(windows, unix)))]
                    return Err(io::Error::new(
                        io::ErrorKind::Other,
                        "Named pipe/Unix domain socket communication is not supported on this platform.",
                    ));
                    let path_is_generated = path.is_none();
                    let path = path.unwrap_or_else(|| {
                        let uuid = Uuid::new_v4().to_string();
                        if cfg!(windows) {
                            format!("\\\\.\\pipe\\rholang-lsp-{}", uuid)
                        } else {
                            format!("/tmp/rholang-lsp-{}.sock", uuid)
                        }
                    });
                    let generated_pipe_path = if path_is_generated { Some(path.clone()) } else { None };
                    let server_args = &[
                        "--pipe", &path.clone(),
                        "--client-process-id", &client_pid.to_string(),
                        "--log-level", &log_level,
                        "--rnode-address", &rnode_address,
                        "--rnode-port", &rnode_port.to_string(),
                        "--no-rnode",  // Tests use parser-only validation (no RNode dependency)
                    ];
                    let mut server = Command::new(&server_path)
                        .args(server_args)
                        .envs(std::env::vars())
                        .stdin(Stdio::null())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .spawn()?;
                    let logger = Box::new(server.stderr.take().expect("Failed to open server stderr")) as Box<dyn LspStream>;
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    #[cfg(windows)]
                    let (read_half, write_half) = {
                        let client = NamedPipeClient::connect(&path).await?;
                        split(client)
                    };
                    #[cfg(unix)]
                    let (read_half, write_half) = {
                        let stream = UnixStream::connect(&path).await?;
                        split(stream)
                    };
                    let write_stream = Arc::new(Mutex::new(write_half));
                    let output = Box::new(AsyncLspWriteStream::new(
                        Arc::clone(&write_stream),
                        runtime_handle.clone(),
                    )) as Box<dyn LspStream>;
                    let input = Box::new(AsyncLspReadStream::new(read_half, runtime_handle.clone())) as Box<dyn LspStream>;
                    (
                        output,
                        input,
                        logger,
                        Some(server),
                        None,
                        Some(write_stream),
                        None,
                        generated_pipe_path,
                    )
                }
                CommType::WebSocket { port } => {
                    let port = port.unwrap_or_else(find_free_port);
                    info!("Starting WebSocket server on port {}", port);
                    let server_args = &[
                        "--websocket",
                        "--port", &port.to_string(),
                        "--client-process-id", &client_pid.to_string(),
                        "--log-level", &log_level,
                        "--rnode-address", &rnode_address,
                        "--rnode-port", &rnode_port.to_string(),
                        "--no-rnode",  // Tests use parser-only validation (no RNode dependency)
                    ];
                    debug!("Server command: {} {:?}", server_path, server_args);
                    let mut server = Command::new(&server_path)
                        .args(server_args)
                        .envs(std::env::vars())
                        .stdin(Stdio::null())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .spawn()
                        .map_err(|e| {
                            error!("Failed to spawn server: {}", e);
                            io::Error::new(io::ErrorKind::Other, format!("Failed to spawn server: {}", e))
                        })?;
                    let logger = Box::new(server.stderr.take().expect("Failed to open server stderr")) as Box<dyn LspStream>;
                    info!("Waiting 500ms for server to start");
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    info!("Connecting to ws://127.0.0.1:{}", port);
                    let ws_stream = connect_async(format!("ws://127.0.0.1:{}", port))
                        .await
                        .map_err(|e| {
                            error!("Failed to connect to WebSocket server: {}", e);
                            io::Error::new(
                                io::ErrorKind::ConnectionRefused,
                                format!("Failed to connect to WebSocket server: {}", e),
                            )
                        })?;
                    info!("WebSocket connection established");
                    let (sink, stream) = ws_stream.0.split();
                    let ws_sink = Arc::new(Mutex::new(sink));
                    let ws_stream = Arc::new(Mutex::new(stream));
                    let output_adapter = Box::new(WebSocketStreamAdapter::new(
                        Arc::clone(&ws_sink),
                        Arc::clone(&ws_stream),
                        runtime_handle.clone(),
                    )) as Box<dyn LspStream>;
                    let input_adapter = Box::new(WebSocketStreamAdapter::new(
                        Arc::clone(&ws_sink),
                        Arc::clone(&ws_stream),
                        runtime_handle.clone(),
                    )) as Box<dyn LspStream>;
                    (
                        output_adapter,
                        input_adapter,
                        logger,
                        Some(server),
                        None,
                        None,
                        Some(ws_sink),
                        None,
                    )
                }
            };

        let output_thread = thread::spawn(move || {
            let mut output = output;
            loop {
                match rx.recv() {
                    Ok(message) => {
                        let content_length = message.len();
                        let headers = format!("Content-Length: {}\r\n\r\n", content_length);
                        debug!("Sending headers: {:?}", headers);
                        if let Err(e) = output.write_all(headers.as_bytes()) {
                            error!("Failed to write header: {}", e);
                            return;
                        }
                        debug!("Sending message: {:?}", message);
                        if let Err(e) = output.write_all(message.as_bytes()) {
                            error!("Failed to write message: {}", e);
                            return;
                        }
                        if let Err(e) = output.flush() {
                            error!("Failed to flush output: {}", e);
                            return;
                        }
                    }
                    Err(e) => {
                        match e.to_string().as_str() {
                            "channel is empty and sending is closed" | "receiving on a closed channel" => {
                                info!("Output channel closed.");
                            }
                            _ => {
                                error!("Failed to receive message: {}", e);
                            }
                        };
                        return;
                    }
                }
            }
        });

        let input_thread = thread::spawn(move || {
            let reader = BufReader::with_capacity(4096, input);
            let mut message_stream = LspMessageStream::new(reader);
            loop {
                match message_stream.next_payload() {
                    Ok(payload) => {
                        debug!("Received payload: {:?}", payload);
                        if let Err(e) = tx.send(payload) {
                            error!("Failed to send payload to receiver: {}", e);
                            return;
                        }
                    }
                    Err(e) => {
                        match e.as_str() {
                            "Input stream closed"
                            | "Error reading byte: A Tokio 1.x context was found, but it is being shutdown." => {
                                info!("Input stream closed.");
                            }
                            _ => {
                                error!("Failed to read from input: {}", e);
                            }
                        }
                        return;
                    }
                }
            }
        });

        let logger_thread = Some(thread::spawn(move || {
            let mut client_stdout = std::io::stdout();
            let mut logger = logger;
            let mut read_buffer = vec![0u8; 4096];
            loop {
                match logger.read(&mut read_buffer) {
                    Ok(0) => {
                        info!("Server logger closed.");
                        if let Err(e) = client_stdout.flush() {
                            error!("Error flushing client stdout: {}", e);
                        }
                        return;
                    }
                    Ok(n) => {
                        if let Err(e) = client_stdout.write_all(&read_buffer[..n]) {
                            error!("Error writing to client stdout: {}", e);
                            return;
                        }
                        if let Err(e) = client_stdout.flush() {
                            error!("Error flushing client stdout: {}", e);
                        }
                    }
                    Err(e) => {
                        if e.kind() == io::ErrorKind::BrokenPipe {
                            info!("Server logger pipe broken, exiting.");
                        } else {
                            error!("Error reading from server logger: {}", e);
                        }
                        if let Err(e) = client_stdout.flush() {
                            error!("Error flushing client stdout: {}", e);
                        }
                        return;
                    }
                }
            }
        }));

        let request_handlers = HashMap::new();

        let mut notification_handlers = HashMap::new();
        notification_handlers.insert(
            "textDocument/publishDiagnostics".to_string(),
            Self::receive_text_document_publish_diagnostics as NotificationHandler,
        );
        notification_handlers.insert(
            "window/logMessage".to_string(),
            Self::receive_window_log_message as NotificationHandler,
        );

        let mut response_handlers = HashMap::new();
        response_handlers.insert(
            "initialize".to_string(),
            Self::receive_initialize as ResponseHandler,
        );
        response_handlers.insert("shutdown".to_string(), Self::receive_shutdown as ResponseHandler);
        response_handlers.insert(
            "textDocument/semanticTokens/full".to_string(),
            Self::receive_semantic_tokens_full as ResponseHandler,
        );
        response_handlers.insert(
            "textDocument/semanticTokens/full/delta".to_string(),
            Self::receive_semantic_tokens_full_delta as ResponseHandler,
        );
        response_handlers.insert(
            "textDocument/rename".to_string(),
            Self::receive_rename as ResponseHandler,
        );
        response_handlers.insert(
            "textDocument/declaration".to_string(),
            Self::receive_declaration as ResponseHandler,
        );
        response_handlers.insert(
            "textDocument/definition".to_string(),
            Self::receive_definition as ResponseHandler,
        );
        response_handlers.insert(
            "textDocument/references".to_string(),
            Self::receive_references as ResponseHandler,
        );
        response_handlers.insert(
            "textDocument/documentSymbol".to_string(),
            Self::receive_document_symbol as ResponseHandler,
        );
        response_handlers.insert(
            "workspace/symbol".to_string(),
            Self::receive_workspace_symbol as ResponseHandler,
        );
        response_handlers.insert(
            "workspaceSymbol/resolve".to_string(),
            Self::receive_workspace_symbol_resolve as ResponseHandler,
        );
        response_handlers.insert(
            "textDocument/documentHighlight".to_string(),
            Self::receive_document_highlight as ResponseHandler,
        );

        let client = LspClient {
            server: Mutex::new(server),
            runtime_handle,
            sender: Mutex::new(Some(sender)),
            receiver: Mutex::new(receiver),
            language_id,
            server_capabilities: RwLock::new(None),
            request_handlers,
            notification_handlers,
            response_handlers,
            requests_by_id: RwLock::new(HashMap::new()),
            responses_by_id: RwLock::new(HashMap::new()),
            diagnostics_by_id: RwLock::new(HashMap::new()),
            semantic_tokens_by_uri: RwLock::new(HashMap::new()),
            serial_request_id: AtomicU64::new(0),
            serial_document_id: AtomicU64::new(0),
            documents_by_uri: RwLock::new(HashMap::new()),
            output_thread: Mutex::new(Some(output_thread)),
            input_thread: Mutex::new(Some(input_thread)),
            logger_thread: Mutex::new(logger_thread),
            event_sender,
            tcp_write_stream: Mutex::new(tcp_write_stream),
            #[cfg(windows)]
            pipe_write_stream: Mutex::new(pipe_or_unix_write_stream),
            #[cfg(unix)]
            unix_write_stream: Mutex::new(pipe_or_unix_write_stream),
            websocket_stream: Mutex::new(websocket_stream),
            generated_pipe_path: Mutex::new(generated_pipe_path),
            comm_type,
        };

        Ok(client)
    }

    /// Stops the LSP client, closing connections and joining threads.
    pub async fn stop(&self) -> io::Result<()> {
        // Drop sender to close output channel
        {
            let mut sender = self.sender.lock().expect("Failed to lock sender");
            *sender = None;
        }

        self.close_connections().await?;
        self.terminate_server()?;
        self.join_threads()?;
        self.async_shutdown_streams().await?;
        self.clear_streams();
        self.cleanup_files()?;

        Ok(())
    }

    async fn close_connections(&self) -> io::Result<()> {
        if let CommType::WebSocket { .. } = self.comm_type {
            if let Some(ws_stream) = self.websocket_stream.lock().expect("Failed to lock websocket_stream").as_mut() {
                let mut stream = ws_stream.lock().expect("Failed to lock WebSocket stream");
                if let Err(e) = stream.send(Message::Close(None)).await {
                    debug!("Failed to send WebSocket close: {}", e);
                }
                if let Err(e) = stream.flush().await {
                    debug!("Failed to flush WebSocket stream: {}", e);
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        Ok(())
    }

    fn terminate_server(&self) -> io::Result<()> {
        let mut server = self.server.lock().expect("Failed to lock server");
        if let Some(ref mut server) = *server {
            #[cfg(unix)]
            {
                let pid = server.id() as i32;
                if pid > 0 {
                    match signal::kill(Pid::from_raw(pid), Signal::SIGTERM) {
                        Ok(()) => debug!("Sent SIGTERM to server process (PID: {})", pid),
                        Err(e) => error!("Failed to send SIGTERM to server process (PID: {}): {}", pid, e),
                    }
                }
            }
            #[cfg(windows)]
            {
                if let Err(e) = server.kill() {
                    debug!("Failed to terminate server process: {}", e);
                } else {
                    debug!("Terminated server process successfully");
                }
            }
            // Wait briefly to allow server to start terminating
            thread::sleep(Duration::from_millis(200));
        }

        // Ensure server is terminated
        if let Some(ref mut server) = *server {
            if server.try_wait()?.is_none() {
                debug!("Server process still running, attempting to kill");
                server.kill()?;
                // Poll for server to exit with timeout
                let start = Instant::now();
                let timeout = Duration::from_secs(2);
                while start.elapsed() < timeout {
                    if server.try_wait()?.is_some() {
                        debug!("Server process terminated successfully");
                        break;
                    }
                    thread::sleep(Duration::from_millis(100));
                }
                if server.try_wait()?.is_none() {
                    error!("Server did not terminate after kill within 2 seconds");
                }
            }
        }
        Ok(())
    }

    fn join_threads(&self) -> io::Result<()> {
        let join_timeout = Duration::from_secs(5);

        if let Some(logger_thread) = self.logger_thread.lock().expect("Failed to lock logger_thread").take() {
            debug!("Attempting to join logger thread");
            if let Err(e) = logger_thread.join_timeout(join_timeout) {
                error!("Failed to join logger thread: {:?}", e);
            } else {
                info!("Logger thread joined successfully");
            }
        }
        if let Some(input_thread) = self.input_thread.lock().expect("Failed to lock input_thread").take() {
            debug!("Attempting to join input thread");
            if let Err(e) = input_thread.join_timeout(join_timeout) {
                error!("Failed to join input thread: {:?}", e);
            } else {
                info!("Input thread joined successfully");
            }
        }
        if let Some(output_thread) = self.output_thread.lock().expect("Failed to lock output_thread").take() {
            debug!("Attempting to join output thread");
            if let Err(e) = output_thread.join_timeout(join_timeout) {
                error!("Failed to join output thread: {:?}", e);
            } else {
                debug!("Output thread joined successfully");
            }
        }
        Ok(())
    }

    async fn async_shutdown_streams(&self) -> io::Result<()> {
        // Take the streams to drop them after shutdown
        let mut tcp_opt = self.tcp_write_stream.lock().expect("Failed to lock tcp_write_stream").take();
        if let Some(tcp) = tcp_opt.as_mut() {
            let mut stream = tcp.lock().expect("Failed to lock TCP stream");
            if let Err(e) = stream.shutdown().await {
                if e.kind() != io::ErrorKind::NotConnected {
                    error!("Failed to shut down TCP write stream: {}", e);
                }
            }
        }

        if cfg!(windows) {
            #[cfg(windows)]
            {
                let mut pipe_opt = self.pipe_write_stream.lock().expect("Failed to lock pipe_write_stream").take();
                if let Some(pipe) = pipe_opt.as_mut() {
                    let mut stream = pipe.lock().expect("Failed to lock named pipe stream");
                    if let Err(e) = stream.shutdown().await {
                        if e.kind() != io::ErrorKind::NotConnected {
                            error!("Failed to shut down named pipe write stream: {}", e);
                        }
                    }
                }
            }
        }

        if cfg!(unix) {
            #[cfg(unix)]
            {
                let mut unix_opt = self.unix_write_stream.lock().expect("Failed to lock unix_write_stream").take();
                if let Some(unix) = unix_opt.as_mut() {
                    let mut stream = unix.lock().expect("Failed to lock Unix socket stream");
                    if let Err(e) = stream.shutdown().await {
                        if e.kind() != io::ErrorKind::NotConnected {
                            error!("Failed to shut down Unix socket stream: {}", e);
                        }
                    }
                }
            }
        }

        let mut ws_opt = self.websocket_stream.lock().expect("Failed to lock websocket_stream").take();
        if let Some(ws) = ws_opt.as_mut() {
            let mut stream = ws.lock().expect("Failed to lock WebSocket stream");
            if let Err(e) = stream.send(Message::Close(None)).await {
                debug!("Failed to send WebSocket close: {}", e);
            }
            if let Err(e) = stream.flush().await {
                debug!("Failed to flush WebSocket stream: {}", e);
            }
            if let Err(e) = stream.close().await {
                debug!("Failed to close WebSocket stream: {}", e);
            }
        }

        Ok(())
    }

    fn clear_streams(&self) {
        *self.tcp_write_stream.lock().expect("Failed to lock tcp_write_stream") = None;

        #[cfg(windows)] {
            *self.pipe_write_stream.lock().expect("Failed to lock pipe_write_stream") = None;
        }

        #[cfg(unix)] {
            *self.unix_write_stream.lock().expect("Failed to lock unix_write_stream") = None;
        }

        *self.websocket_stream.lock().expect("Failed to lock websocket_stream") = None;
    }

    fn cleanup_files(&self) -> io::Result<()> {
        #[cfg(unix)]
        if let Some(path) = self.generated_pipe_path.lock().expect("Failed to lock generated_pipe_path").as_ref() {
            if let Err(e) = fs::remove_file(path) {
                debug!("Failed to remove Unix socket file {}: {}", path, e);
            } else {
                info!("Cleaned up Unix socket file {}", path);
            }
        }
        Ok(())
    }
}

/// Finds a free TCP port on localhost.
fn find_free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind to a free port");
    listener.local_addr().expect("Failed to get local address").port()
}

pub fn init_logger() -> io::Result<()> {
    let timer = fmt::time::OffsetTime::new(
        UtcOffset::UTC,
        format_description!("[[[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z]"),
    );

    // Log to stderr
    let stderr_layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_timer(timer)
        .with_ansi(true);

    // Configure the log level based on whether --log-level was provided
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("trace"));

    // Combine the layers using a registry
    let result = tracing_subscriber::registry()
        .with(env_filter)
        .with(stderr_layer)
        .try_init();

    match result {
        Ok(()) => Ok(()),
        Err(e) => {
            // Ignore errors due to the subscriber or logger already being set
            if e.to_string().contains("already been set") || e.to_string().contains("SetLoggerError") {
                Ok(())
            } else {
                // Propagate unexpected errors
                Err(io::Error::new(io::ErrorKind::Other, e))
            }
        }
    }
}
