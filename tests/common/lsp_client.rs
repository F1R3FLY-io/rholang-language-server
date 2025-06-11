use std::collections::HashMap;
use std::io::{PipeReader, Read, Write};
use std::net::TcpListener;
use std::process::{Child, Command, ChildStdin, ChildStdout, ChildStderr, Stdio};
use std::sync::{Arc, Once, Mutex, RwLock};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::fs;

use tokio::io::{AsyncWriteExt, WriteHalf};
use tokio::net::TcpStream;
use tokio::runtime::{Handle, Runtime};

#[cfg(unix)]
use tokio::net::UnixStream;

#[cfg(windows)]
use tokio::net::windows::named_pipe::NamedPipeClient;

use tokio_tungstenite::{tungstenite::Message, MaybeTlsStream, WebSocketStream};

use futures_util::sink::SinkExt;
use futures_util::stream::StreamExt;

use uuid::Uuid;

#[cfg(unix)]
use nix::sys::signal::{self, Signal};
#[cfg(unix)]
use nix::unistd::Pid;

use tracing::{info, warn, error, debug, trace};

use tracing_subscriber::{self, fmt, prelude::*};

use time::macros::format_description;
use time::UtcOffset;

use tower_lsp::lsp_types::{
    ClientCapabilities, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, InitializedParams, InitializeParams,
    InitializeResult, LogMessageParams, MessageType, Position,
    PublishDiagnosticsParams, Range, ServerCapabilities,
    TextDocumentClientCapabilities, TextDocumentContentChangeEvent,
    TextDocumentIdentifier, TextDocumentItem, TextDocumentSyncCapability,
    TextDocumentSyncClientCapabilities, TextDocumentSyncKind, Url,
    VersionedTextDocumentIdentifier,
};

use serde_json::{self, json, Value};

use crate::common::lsp_document::LspDocument;
use crate::common::lsp_event::LspEvent;
use crate::common::lsp_message_stream::LspMessageStream;

// WebSocket Stream Adapter for LspStream
struct WebSocketStreamAdapter {
    sink: Arc<Mutex<futures_util::stream::SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>>>,
    stream: Arc<Mutex<futures_util::stream::SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>>>,
    runtime_handle: Handle,
    read_buffer: Vec<u8>,
}

impl WebSocketStreamAdapter {
    fn new(
        sink: Arc<Mutex<futures_util::stream::SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>>>,
        stream: Arc<Mutex<futures_util::stream::SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>>>,
        runtime_handle: Handle,
    ) -> Self {
        WebSocketStreamAdapter {
            sink,
            stream,
            runtime_handle,
            read_buffer: Vec::new(),
        }
    }
}

impl LspStream for WebSocketStreamAdapter {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if !self.read_buffer.is_empty() {
            let to_copy = std::cmp::min(buf.len(), self.read_buffer.len());
            buf[..to_copy].copy_from_slice(&self.read_buffer[..to_copy]);
            self.read_buffer.drain(..to_copy);
            trace!("Read {} bytes from buffer", to_copy);
            return Ok(to_copy);
        }

        let result = self.runtime_handle.block_on(async {
            let mut stream = self.stream.lock().map_err(|e| {
                std::io::Error::new(std::io::ErrorKind::Other, format!("Mutex lock failed: {}", e))
            })?;
            match stream.next().await {
                Some(Ok(Message::Text(text))) => {
                    trace!("Client received WebSocket text message: {}", text);
                    Ok(text.into_bytes())
                }
                Some(Ok(Message::Binary(data))) => {
                    trace!("Client received WebSocket binary message: {:?}", data);
                    Ok(data)
                }
                Some(Ok(Message::Ping(_))) => {
                    trace!("Received Ping");
                    Ok(Vec::new())
                }
                Some(Ok(Message::Pong(_))) => {
                    trace!("Received Pong");
                    Ok(Vec::new())
                }
                Some(Ok(Message::Frame(_))) => {
                    trace!("Received Frame");
                    Ok(Vec::new())
                }
                Some(Ok(Message::Close(_))) => {
                    info!("WebSocket connection closed");
                    Ok(Vec::new())
                }
                Some(Err(e)) => {
                    error!("WebSocket error: {}", e);
                    Err(std::io::Error::new(std::io::ErrorKind::Other, e))
                }
                None => {
                    info!("WebSocket stream ended");
                    Ok(Vec::new())
                }
            }
        })?;

        if result.is_empty() {
            return Ok(0);
        }

        self.read_buffer.extend_from_slice(&result);
        let to_copy = std::cmp::min(buf.len(), self.read_buffer.len());
        buf[..to_copy].copy_from_slice(&self.read_buffer[..to_copy]);
        self.read_buffer.drain(..to_copy);
        trace!("Read {} bytes from stream", to_copy);
        Ok(to_copy)
    }

    fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        trace!("Client preparing to send WebSocket binary message: {:?}", buf);
        self.runtime_handle.block_on(async {
            let mut sink = self.sink.lock().map_err(|e| {
                std::io::Error::new(std::io::ErrorKind::Other, format!("Mutex lock failed: {}", e))
            })?;
            sink.send(Message::Binary(buf.to_vec()))
                .await
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
            sink.flush()
                .await
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
            Ok(())
        })
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.runtime_handle.block_on(async {
            let mut sink = self.sink.lock().map_err(|e| {
                std::io::Error::new(std::io::ErrorKind::Other, format!("Mutex lock failed: {}", e))
            })?;
            sink.flush()
                .await
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
            Ok(())
        })
    }
}

// Ensure JoinHandleExt trait is included
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

// Trait to abstract reading and writing for streams
pub trait LspStream: Send + 'static {
    fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
        Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "read not supported"))
    }
    fn write_all(&mut self, _buf: &[u8]) -> std::io::Result<()> {
        Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "write not supported"))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// Implement std::io::Read for Box<dyn LspStream>
impl std::io::Read for Box<dyn LspStream> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.as_mut().read(buf)
    }
}

// Implement LspStream for stdio types
impl LspStream for ChildStdin {
    fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        std::io::Write::write_all(self, buf)?;
        Ok(())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        std::io::Write::flush(self)?;
        Ok(())
    }
}

impl LspStream for ChildStdout {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        std::io::Read::read(self, buf)
    }
}

impl LspStream for ChildStderr {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        std::io::Read::read(self, buf)
    }
}

impl LspStream for PipeReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        std::io::Read::read(self, buf)
    }
}

impl LspStream for std::io::BufReader<std::io::Empty> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        std::io::Read::read(self, buf)
    }
}

// Struct for async read streams
struct AsyncLspReadStream<T: tokio::io::AsyncRead + Unpin + Send + 'static> {
    stream: T,
    runtime_handle: Handle,
}

impl<T: tokio::io::AsyncRead + Unpin + Send + 'static> AsyncLspReadStream<T> {
    fn new(stream: T, runtime_handle: Handle) -> Self {
        AsyncLspReadStream { stream, runtime_handle }
    }
}

impl<T: tokio::io::AsyncRead + Unpin + Send + 'static> LspStream for AsyncLspReadStream<T> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.runtime_handle
            .block_on(tokio::io::AsyncReadExt::read(&mut self.stream, buf))
    }
}

// Struct for async write streams
struct AsyncLspWriteStream<T: tokio::io::AsyncWrite + Unpin + Send + 'static> {
    stream: Arc<Mutex<T>>,
    runtime_handle: Handle,
}

impl<T: tokio::io::AsyncWrite + Unpin + Send + 'static> AsyncLspWriteStream<T> {
    fn new(stream: Arc<Mutex<T>>, runtime_handle: Handle) -> Self {
        AsyncLspWriteStream {
            stream,
            runtime_handle,
        }
    }
}

impl<T: tokio::io::AsyncWrite + Unpin + Send + 'static> LspStream for AsyncLspWriteStream<T> {
    fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        let mut stream = self.stream.lock().map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::Other, format!("Mutex lock failed: {}", e))
        })?;
        self.runtime_handle
            .block_on(tokio::io::AsyncWriteExt::write_all(&mut *stream, buf))?;
        Ok(())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        let mut stream = self.stream.lock().map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::Other, format!("Mutex lock failed: {}", e))
        })?;
        self.runtime_handle
            .block_on(tokio::io::AsyncWriteExt::flush(&mut *stream))?;
        Ok(())
    }
}

type RequestHandler = fn(&LspClient, &Value) -> Result<Option<Arc<Value>>, String>;
type NotificationHandler = fn(&LspClient, &Value) -> Result<(), String>;
type ResponseHandler = fn(&LspClient, Arc<Value>) -> Result<(), String>;

#[allow(dead_code)]
pub struct LspClient {
    pub server: Mutex<Option<Child>>,
    runtime: Runtime,
    sender: Mutex<Option<Sender<String>>>,
    pub receiver: Mutex<Receiver<String>>,
    language_id: String,
    server_capabilities: RwLock<Option<ServerCapabilities>>,
    request_handlers: HashMap<String, RequestHandler>,
    notification_handlers: HashMap<String, NotificationHandler>,
    response_handlers: HashMap<String, ResponseHandler>,
    requests_by_id: RwLock<HashMap<u64, Arc<Value>>>,
    responses_by_id: RwLock<HashMap<u64, Arc<Value>>>,
    diagnostics_by_id: RwLock<HashMap<u64, Arc<PublishDiagnosticsParams>>>,
    serial_request_id: AtomicU64,
    serial_document_id: AtomicU64,
    documents_by_uri: RwLock<HashMap<String, Arc<LspDocument>>>,
    output_thread: Mutex<Option<JoinHandle<()>>>,
    input_thread: Mutex<Option<JoinHandle<()>>>,
    logger_thread: Mutex<Option<JoinHandle<()>>>,
    event_sender: Sender<LspEvent>,
    tcp_write_stream: Mutex<Option<Arc<Mutex<WriteHalf<TcpStream>>>>>,
    #[cfg(windows)]
    pipe_write_stream: Mutex<Option<Arc<Mutex<WriteHalf<NamedPipeClient>>>>>,
    #[cfg(unix)]
    unix_write_stream: Mutex<Option<Arc<Mutex<WriteHalf<UnixStream>>>>>,
    websocket_stream: Mutex<Option<Arc<Mutex<futures_util::stream::SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>>>>>,
    generated_pipe_path: Mutex<Option<String>>,
    comm_type: CommType,
}

#[derive(Clone)]
pub enum CommType {
    Stdio,
    Tcp { port: Option<u16> },
    Pipe { path: Option<String> },
    WebSocket { port: Option<u16> },
}

fn find_free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind to a free port");
    listener.local_addr().expect("Failed to get local address").port()
}

#[allow(dead_code)]
impl LspClient {
    pub fn start(
        language_id: String,
        server_path: String,
        comm_type: CommType,
        event_sender: Sender<LspEvent>,
    ) -> std::io::Result<Self> {
        let runtime = Runtime::new()?;
        let runtime_handle = runtime.handle().clone();
        let (sender, rx) = channel::<String>();
        let (tx, receiver) = channel::<String>();

        // Get the client's process ID
        let client_pid = std::process::id();

        // Get RNode address from environment variable or default
        let rnode_address = std::env::var("RHOLANG_RNODE_ADDRESS").unwrap_or_else(|_| "localhost".to_string());

        // Get RNode port from environment variable or default
        let rnode_port = match std::env::var("RHOLANG_RNODE_PORT") {
            Ok(port_str) => port_str.parse::<u16>().map_err(|e| {
                error!("Invalid RHOLANG_RNODE_PORT environment variable: {}", e);
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("Invalid port in RHOLANG_RNODE_PORT: {}", e),
                )
            })?,
            Err(_) => 40402,
        };

        let (output, input, logger, server, tcp_write_stream,
             pipe_or_unix_write_stream, websocket_stream, generated_pipe_path)
            = match comm_type.clone() {
                CommType::Stdio => {
                    let server_args = &[
                        "--stdio",
                        "--client-process-id", &client_pid.to_string(),
                        "--log-level", "debug",
                        "--rnode-address", &rnode_address,
                        "--rnode-port", &rnode_port.to_string(),
                    ];
                    let mut server = Command::new(&server_path)
                        .args(server_args)
                        .stdin(Stdio::piped())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .spawn()?;
                    let output = server.stdin.take().expect("Failed to open server stdin");
                    let input = server.stdout.take().expect("Failed to open server stdout");
                    let logger = server.stderr.take().expect("Failed to open server stderr");
                    (
                        Box::new(output) as Box<dyn LspStream>,
                        Box::new(input) as Box<dyn LspStream>,
                        Box::new(logger) as Box<dyn LspStream>,
                        Some(server),
                        None,
                        None,
                        None,
                        None,
                    )
                }
                CommType::Tcp { port } => {
                    let port = port.unwrap_or_else(|| find_free_port());
                    let server_args = &[
                        "--socket",
                        "--port", &port.to_string(),
                        "--client-process-id", &client_pid.to_string(),
                        "--log-level", "debug",
                        "--rnode-address", &rnode_address,
                        "--rnode-port", &rnode_port.to_string(),
                    ];
                    let mut server = Command::new(&server_path)
                        .args(server_args)
                        .stdin(Stdio::null())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .spawn()?;
                    let logger = server.stderr.take().expect("Failed to open server stderr");
                    thread::sleep(Duration::from_millis(100));
                    let stream = runtime_handle
                        .block_on(TcpStream::connect(format!("127.0.0.1:{}", port)))?;
                    stream.set_nodelay(true)?;
                    let (read_half, write_half) = tokio::io::split(stream);
                    let write_stream = Arc::new(Mutex::new(write_half));
                    let output = Box::new(AsyncLspWriteStream::new(
                        Arc::clone(&write_stream),
                        runtime_handle.clone(),
                    )) as Box<dyn LspStream>;
                    let input = Box::new(AsyncLspReadStream::new(read_half, runtime_handle.clone()))
                        as Box<dyn LspStream>;
                    (
                        output,
                        input,
                        Box::new(logger) as Box<dyn LspStream>,
                        Some(server),
                        Some(write_stream),
                        None,
                        None,
                        None,
                    )
                }
                CommType::Pipe { path } => {
                    #[cfg(not(any(windows, unix)))]
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
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
                        "--log-level", "debug",
                        "--rnode-address", &rnode_address,
                        "--rnode-port", &rnode_port.to_string(),
                    ];
                    let mut server = Command::new(&server_path)
                        .args(server_args)
                        .stdin(Stdio::null())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .spawn()?;
                    let logger = server.stderr.take().expect("Failed to open server stderr");
                    thread::sleep(Duration::from_millis(100));
                    #[cfg(windows)]
                    let (read_half, write_half) = {
                        let client = runtime_handle.block_on(NamedPipeClient::connect(&path))?;
                        tokio::io::split(client)
                    };
                    #[cfg(unix)]
                    let (read_half, write_half) = {
                        let stream = runtime_handle.block_on(UnixStream::connect(&path))?;
                        tokio::io::split(stream)
                    };
                    let write_stream = Arc::new(Mutex::new(write_half));
                    let output = Box::new(AsyncLspWriteStream::new(
                        Arc::clone(&write_stream),
                        runtime_handle.clone(),
                    )) as Box<dyn LspStream>;
                    let input = Box::new(AsyncLspReadStream::new(read_half, runtime_handle.clone()))
                        as Box<dyn LspStream>;
                    (
                        output,
                        input,
                        Box::new(logger) as Box<dyn LspStream>,
                        Some(server),
                        None,
                        Some(write_stream),
                        None,
                        generated_pipe_path,
                    )
                }
                CommType::WebSocket { port } => {
                    let port = port.unwrap_or_else(|| find_free_port());
                    info!("Starting WebSocket server on port {}", port);
                    let server_args = &[
                        "--websocket",
                        "--port", &port.to_string(),
                        "--client-process-id", &client_pid.to_string(),
                        "--log-level", "debug",
                        "--rnode-address", &rnode_address,
                        "--rnode-port", &rnode_port.to_string(),
                    ];
                    debug!("Server command: {} {:?}", server_path, server_args);
                    let mut server = Command::new(&server_path)
                        .args(server_args)
                        .stdin(Stdio::null())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .spawn()
                        .map_err(|e| {
                            error!("Failed to spawn server: {}", e);
                            std::io::Error::new(std::io::ErrorKind::Other, format!("Failed to spawn server: {}", e))
                        })?;
                    let logger = server.stderr.take().expect("Failed to open server stderr");
                    info!("Waiting 500ms for server to start");
                    thread::sleep(Duration::from_millis(500));
                    info!("Connecting to ws://127.0.0.1:{}", port);
                    let ws_stream = runtime_handle
                        .block_on(tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{}", port)))
                        .map_err(|e| {
                            error!("Failed to connect to WebSocket server: {}", e);
                            std::io::Error::new(
                                std::io::ErrorKind::ConnectionRefused,
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
                        Box::new(logger) as Box<dyn LspStream>,
                        Some(server),
                        None,
                        None,
                        Some(ws_sink), // Store sink instead of stream
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
                            "channel is empty and sending is closed" |
                            "receiving on a closed channel" => {
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
            let reader = std::io::BufReader::with_capacity(4096, input);
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
                            "Input stream closed" |
                            "Error reading byte: A Tokio 1.x context was found, but it is being shutdown." => {
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
                        if e.kind() == std::io::ErrorKind::BrokenPipe {
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
            LspClient::receive_text_document_publish_diagnostics as NotificationHandler,
        );
        notification_handlers.insert(
            "window/logMessage".to_string(),
            LspClient::receive_window_log_message as NotificationHandler,
        );

        let mut response_handlers = HashMap::new();
        response_handlers.insert(
            "initialize".to_string(),
            LspClient::receive_initialize as ResponseHandler,
        );
        response_handlers.insert(
            "shutdown".to_string(),
            LspClient::receive_shutdown as ResponseHandler,
        );

        let client = LspClient {
            server: Mutex::new(server),
            runtime,
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

    pub fn stop(&self) -> std::io::Result<()> {
        // Drop sender to close output channel
        {
            let mut sender = self.sender.lock().expect("Failed to lock sender");
            *sender = None;
        }

        // Close WebSocket stream for WebSocket communication
        if let CommType::WebSocket { .. } = self.comm_type {
            if let Some(ws_stream) = self.websocket_stream.lock().expect("Failed to lock websocket_stream").as_mut() {
                let mut stream = ws_stream.lock().expect("Failed to lock WebSocket stream");
                if let Err(e) = self.runtime.block_on(stream.send(Message::Close(None))) {
                    debug!("Failed to send WebSocket close: {}", e);
                }
                if let Err(e) = self.runtime.block_on(stream.flush()) {
                    debug!("Failed to flush WebSocket stream: {}", e);
                }
                thread::sleep(Duration::from_millis(100));
            }
        }

        // Send SIGTERM to the server process for all communication types
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

        let join_timeout = Duration::from_secs(5); // Increased timeout

        // Join threads
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
                error!("Failed to join input thread: {:?}", e); // Use {:?} for debugging
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

        // Ensure server is terminated
        if let Some(ref mut server) = *server {
            if server.try_wait()?.is_none() {
                debug!("Server process still running, attempting to kill");
                server.kill()?;
                // Poll for server to exit with timeout
                let start = std::time::Instant::now();
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

        // Clean up streams
        if let Some(tcp_write_stream) = self.tcp_write_stream.lock().expect("Failed to lock tcp_write_stream").as_mut() {
            let mut stream = tcp_write_stream.lock().expect("Failed to lock TCP stream");
            if let Err(e) = self.runtime.block_on(stream.shutdown()) {
                if e.kind() != std::io::ErrorKind::NotConnected {
                    error!("Failed to shut down TCP write stream: {}", e);
                }
            }
        }
        #[cfg(windows)]
        if let Some(pipe_write_stream) = self.pipe_write_stream.lock().expect("Failed to lock pipe_write_stream").as_mut() {
            let mut stream = pipe_write_stream.lock().expect("Failed to lock named pipe stream");
            if let Err(e) = self.runtime.block_on(stream.shutdown()) {
                if e.kind() != std::io::ErrorKind::NotConnected {
                    error!("Failed to shut down named pipe write stream: {}", e);
                }
            }
        }
        #[cfg(unix)]
        if let Some(unix_write_stream) = self.unix_write_stream.lock().expect("Failed to lock unix_write_stream").as_mut() {
            let mut stream = unix_write_stream.lock().expect("Failed to lock Unix socket stream");
            if let Err(e) = self.runtime.block_on(stream.shutdown()) {
                if e.kind() != std::io::ErrorKind::NotConnected {
                    error!("Failed to shut down Unix socket stream: {}", e);
                }
            }
        }
        if let Some(ws_stream) = self.websocket_stream.lock().expect("Failed to lock websocket_stream").as_mut() {
            let mut stream = ws_stream.lock().expect("Failed to lock WebSocket stream");
            if let Err(e) = self.runtime.block_on(stream.close()) {
                debug!("Failed to close WebSocket stream: {}", e);
            }
        }

        // Clear streams
        {
            let mut tcp_write_stream = self.tcp_write_stream.lock().expect("Failed to lock tcp_write_stream");
            *tcp_write_stream = None;
        }
        #[cfg(windows)]
        {
            let mut pipe_write_stream = self.pipe_write_stream.lock().expect("Failed to lock pipe_write_stream");
            *pipe_write_stream = None;
        }
        #[cfg(unix)]
        {
            let mut unix_write_stream = self.unix_write_stream.lock().expect("Failed to lock unix_write_stream");
            *unix_write_stream = None;
        }
        {
            let mut websocket_stream = self.websocket_stream.lock().expect("Failed to lock websocket_stream");
            *websocket_stream = None;
        }

        // Clean up generated Unix socket file
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

    fn next_request_id(&self) -> u64 {
        self.serial_request_id.fetch_add(1, Ordering::SeqCst)
    }

    fn next_document_id(&self) -> u64 {
        self.serial_document_id.fetch_add(1, Ordering::SeqCst)
    }

    pub fn open_document(&self, path: &str, text: &str) -> Result<Arc<LspDocument>, String> {
        let document_id = self.next_document_id();
        let document = LspDocument::from_path_and_text(
            document_id,
            self.language_id.clone(),
            path.to_string(),
            text.to_string(),
            self.event_sender.clone(),
        );
        let document = Arc::new(document);
        self.documents_by_uri
            .write()
            .expect("Failed to acquire write lock on documents_by_uri")
            .insert(document.uri().to_string(), document.clone());
        document.open()?;
        Ok(document)
    }

    pub fn dispatch(&self, message: String) -> Result<(), String> {
        match serde_json::from_str::<Value>(&message) {
            Ok(json) => {
                if json.get("method").is_some() {
                    if json.get("id").is_some() {
                        if let Err(e) = self.dispatch_request(json.clone()) {
                            return Err(format!("Failed to dispatch request: {}", e));
                        }
                    } else {
                        if let Err(e) = self.dispatch_notification(json.clone()) {
                            return Err(format!("Failed to dispatch notification: {}", e));
                        }
                    }
                } else if json.get("id").is_some() {
                    if let Err(e) = self.dispatch_response(json.clone()) {
                        return Err(format!("Failed to dispatch response: {}", e));
                    }
                } else {
                    return Err(format!("Invalid JSON-RPC message: {}", message));
                }
                Ok(())
            }
            Err(e) => Err(format!("Failed to parse message as JSON: {}\n{}", e, message)),
        }
    }

    fn dispatch_request(&self, json: Value) -> Result<(), String> {
        let method = json["method"].as_str().ok_or("Missing method")?;
        let id = json["id"].as_u64().ok_or("Missing or invalid id")?;
        if let Some(handler) = self.request_handlers.get(method) {
            match handler(self, &json) {
                Ok(result) => {
                    if let Err(e) = self.send_response(result, Some(id)) {
                        return Err(format!("Failed to send response: {}", e));
                    }
                }
                Err(e) => {
                    return Err(format!("Failed to handle request '{}': {}", method, e));
                }
            }
        } else if !method.starts_with("$/") {
            self.send_method_not_found(method, Some(id));
        } else {
            debug!("Ignoring optional method: {}", method);
        }
        Ok(())
    }

    fn dispatch_notification(&self, json: Value) -> Result<(), String> {
        let method = json["method"].as_str().ok_or("Missing method")?;
        if let Some(handler) = self.notification_handlers.get(method) {
            handler(self, &json).map_err(|e| format!("Failed to handle notification '{}': {}", method, e))
        } else if !method.starts_with("$/") {
            self.send_method_not_found(method, None);
            Ok(())
        } else {
            debug!("Ignoring optional notification: {}", method);
            Ok(())
        }
    }

    fn dispatch_response(&self, response: Value) -> Result<(), String> {
        let id = response["id"].as_u64().ok_or("Missing or invalid id")?;
        let response = Arc::new(response);
        self.responses_by_id
            .write()
            .expect("Failed to acquire write lock on responses_by_id")
            .insert(id, response.clone());
        let requests_by_id = self.requests_by_id.read().expect("Failed to acquire read lock on requests_by_id");
        if let Some(request) = requests_by_id.get(&id) {
            let method = request["method"].as_str().ok_or("Missing method in request")?;
            if let Some(handler) = self.response_handlers.get(method) {
                handler(self, response).map_err(|e| format!("Failed to handle response for '{}': {}", method, e))
            } else {
                Err(format!("No handler for method '{}'", method))
            }
        } else {
            debug!("No request found for response id {}", id);
            Ok(())
        }
    }

    fn send_invalid_request(&self, method: &str, message: &str, id: Option<u64>) {
        let invalid_request = -32600;
        self.send_error(id, invalid_request, format!("Invalid request for method '{}': {}", method, message), None);
    }

    fn send_method_not_found(&self, method: &str, id: Option<u64>) {
        let method_not_found = -32601;
        self.send_error(id, method_not_found, format!("Unsupported method: {}", method), None);
    }

    fn send_error(&self, id: Option<u64>, code: i32, message: String, data: Option<Value>) {
        let error = json!({
            "code": code,
            "message": message,
            "data": data,
        });
        let message = json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": error,
        });
        let message_str = serde_json::to_string(&message).expect("Failed to serialize message");
        if let Err(e) = self.sender.lock().expect("Failed to lock sender").as_ref().expect("Sender dropped").send(message_str) {
            error!("Failed to send error message: {}", e);
        }
    }

    fn send_response(&self, result: Option<Arc<Value>>, id: Option<u64>) -> Result<(), String> {
        let message = json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result.map(|r| r.as_ref().clone()).unwrap_or(Value::Null),
        });
        let message_str = serde_json::to_string(&message).map_err(|e| format!("Failed to serialize response: {}", e))?;
        self.sender
            .lock()
            .expect("Failed to lock sender")
            .as_ref()
            .expect("Sender dropped")
            .send(message_str)
            .map_err(|e| format!("Failed to send response: {}", e))?;
        Ok(())
    }

    fn send_request(&self, request_id: u64, method: &str, params: Option<Value>) {
        let mut message = json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": method
        });
        if let Some(params) = params {
            message["params"] = params;
        }
        let message_str = serde_json::to_string(&message).expect("Failed to serialize message");
        self.requests_by_id
            .write()
            .expect("Failed to acquire write lock on requests_by_id")
            .insert(request_id, Arc::new(message.clone()));
        if let Err(e) = self.sender.lock().expect("Failed to lock sender").as_ref().expect("Sender dropped").send(message_str) {
            error!("Failed to send request: {}", e);
        }
    }

    fn send_notification(&self, method: &str, params: Value) {
        let message = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });
        let message_str = serde_json::to_string(&message).expect("Failed to serialize message");
        if let Err(e) = self.sender.lock().expect("Failed to lock sender").as_ref().expect("Sender dropped").send(message_str) {
            error!("Failed to send notification: {}", e);
        }
    }

    fn await_response(&self, request_id: u64) -> Result<Arc<Value>, String> {
        {
            let responses_by_id = self.responses_by_id.read().expect("Failed to acquire read lock on responses_by_id");
            if let Some(response) = responses_by_id.get(&request_id) {
                return Ok(response.clone());
            }
        }

        let timeout = Duration::from_secs(30);
        let start = Instant::now();

        while start.elapsed() < timeout {
            if let Ok(message) = self.receiver.lock().expect("Failed to lock receiver").recv_timeout(Duration::from_millis(100)) {
                debug!("Processing message: {:?}", message);
                if let Err(e) = self.dispatch(message) {
                    return Err(format!("Failed to dispatch message: {}", e));
                }
                let responses_by_id = self.responses_by_id.read().expect("Failed to acquire read lock on responses_by_id");
                if let Some(response) = responses_by_id.get(&request_id) {
                    return Ok(response.clone());
                }
            }
        }

        Err(format!("Timeout waiting for response with id {}", request_id))
    }

    pub fn await_diagnostics(&self, doc: &LspDocument) -> Result<Arc<PublishDiagnosticsParams>, String> {
        {
            let diagnostics_by_id = self.diagnostics_by_id.read().expect("Failed to acquire read lock on diagnostics_by_id");
            if let Some(diagnostics) = diagnostics_by_id.get(&doc.id) {
                return Ok(diagnostics.clone());
            }
        }

        let timeout = Duration::from_secs(10);
        let start = Instant::now();

        while start.elapsed() < timeout {
            if let Ok(message) = self.receiver.lock().expect("Failed to lock receiver").recv_timeout(Duration::from_millis(100)) {
                if let Err(e) = self.dispatch(message) {
                    return Err(format!("Failed to dispatch message: {}", e));
                }
                let diagnostics_by_id = self.diagnostics_by_id.read().expect("Failed to acquire read lock on diagnostics_by_id");
                if let Some(diagnostics) = diagnostics_by_id.get(&doc.id) {
                    return Ok(diagnostics.clone());
                }
            }
        }

        Err(format!("Timeout waiting for diagnostics for document with URI: {}", doc.uri()))
    }

    pub fn initialize(&self) -> Result<Arc<Value>, String> {
        let request_id = self.send_initialize();
        self.await_response(request_id)
    }

    fn send_initialize(&self) -> u64 {
        #[allow(deprecated)]
        let params = InitializeParams {
            root_path: None,
            process_id: Some(std::process::id()),
            root_uri: None,
            initialization_options: None,
            capabilities: ClientCapabilities {
                text_document: Some(TextDocumentClientCapabilities {
                    synchronization: Some(TextDocumentSyncClientCapabilities {
                        dynamic_registration: Some(false),
                        will_save: None,
                        will_save_wait_until: None,
                        did_save: None,
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
            trace: None,
            workspace_folders: None,
            client_info: None,
            locale: None,
        };
        let request_id = self.next_request_id();
        self.send_request(request_id, "initialize", Some(serde_json::to_value(params).expect("Failed to serialize params")));
        request_id
    }

    fn receive_initialize(&self, json: Arc<Value>) -> Result<(), String> {
        if let Some(result) = json.get("result") {
            let init_result: InitializeResult =
                serde_json::from_value(result.clone()).map_err(|e| format!("Failed to parse InitializeResult: {}", e))?;
            debug!("Received initialize response: {:?}", init_result);
            self.server_capabilities
                .write()
                .expect("Failed to acquire write lock on server_capabilities")
                .replace(init_result.capabilities);
            Ok(())
        } else {
            let id = json["id"].as_u64();
            self.send_invalid_request("initialize", "Missing result", id);
            Err("Missing result in initialize response".to_string())
        }
    }

    fn send_initialized(&self) {
        let params = InitializedParams {};
        self.send_notification("initialized", serde_json::to_value(params).expect("Failed to serialize params"));
    }

    pub fn initialized(&self) -> Result<(), String> {
        self.send_initialized();
        Ok(())
    }

    fn receive_window_log_message(&self, json: &Value) -> Result<(), String> {
        let params: LogMessageParams = serde_json::from_value(json["params"].clone())
            .map_err(|e| format!("Failed to parse LogMessageParams: {}", e))?;
        match params.typ {
            MessageType::ERROR => error!("[Server] {}", params.message),
            MessageType::WARNING => warn!("[Server] {}", params.message),
            MessageType::INFO => info!("[Server] {}", params.message),
            MessageType::LOG => debug!("[Server] {}", params.message),
            _ => info!("[Server] {}", params.message),
        }
        Ok(())
    }

    fn supports_text_document_sync(&self) -> bool {
        self.server_capabilities
            .read()
            .expect("Failed to acquire read lock on server_capabilities")
            .as_ref()
            .map(|caps| caps.text_document_sync.is_some())
            .unwrap_or(false)
    }

    fn get_text_document_sync_kind(&self) -> Option<TextDocumentSyncKind> {
        self.server_capabilities
            .read()
            .expect("Failed to acquire read lock on server_capabilities")
            .as_ref()
            .and_then(|caps| {
                caps.text_document_sync.as_ref().and_then(|sync| match sync {
                    TextDocumentSyncCapability::Kind(kind) => Some(*kind),
                    TextDocumentSyncCapability::Options(options) => options.change,
                })
            })
    }

    pub fn send_text_document_did_open(&self, uri: String, text: String) {
        if !self.supports_text_document_sync() {
            warn!("Server does not support text document synchronization. Skipping didOpen.");
            return;
        }
        let params = DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: Url::parse(&uri).expect("Invalid URI"),
                language_id: self.language_id.clone(),
                version: 1,
                text,
            },
        };
        self.send_notification("textDocument/didOpen", serde_json::to_value(params).expect("Failed to serialize params"));
    }

    pub fn send_text_document_did_change(&self, uri: &str, version: i32, changes: Vec<TextDocumentContentChangeEvent>) {
        if let Some(sync_kind) = self.get_text_document_sync_kind() {
            if sync_kind == TextDocumentSyncKind::FULL || sync_kind == TextDocumentSyncKind::INCREMENTAL {
                let params = DidChangeTextDocumentParams {
                    text_document: VersionedTextDocumentIdentifier {
                        uri: Url::parse(uri).expect("Invalid URI"),
                        version,
                    },
                    content_changes: changes,
                };
                self.send_notification("textDocument/didChange", serde_json::to_value(params).expect("Failed to serialize params"));
            }
        } else {
            debug!("Server does not support text document changes. Skipping didChange.");
        }
    }

    pub fn send_text_document_did_close(&self, uri: &str) -> Result<(), String> {
        if !self.supports_text_document_sync() {
            return Err("Server does not support text document synchronization.".to_string());
        }
        let params = DidCloseTextDocumentParams {
            text_document: TextDocumentIdentifier {
                uri: Url::parse(uri).expect("Invalid URI"),
            },
        };
        self.send_notification("textDocument/didClose", serde_json::to_value(params).expect("Failed to serialize params"));
        Ok(())
    }

    fn receive_text_document_publish_diagnostics(&self, json: &Value) -> Result<(), String> {
        let params: PublishDiagnosticsParams = serde_json::from_value(json["params"].clone())
            .map_err(|e| format!("Failed to parse PublishDiagnosticsParams: {}", e))?;
        let uri = params.uri.to_string();
        if let Some(version) = params.version {
            let documents_by_uri = self.documents_by_uri.read().expect("Failed to acquire read lock on documents_by_uri");
            if let Some(document) = documents_by_uri.get(&uri) {
                let latest_version = document.version.load(Ordering::Relaxed);
                if latest_version == version {
                    self.diagnostics_by_id
                        .write()
                        .expect("Failed to acquire write lock on diagnostics_by_id")
                        .insert(document.id, Arc::new(params));
                } else {
                    warn!(
                        "Diagnostics for older version of document '{}': {} < {}",
                        uri, version, latest_version
                    );
                }
            } else {
                return Err(format!("No document found for URI: {}", uri));
            }
        }
        Ok(())
    }

    fn emit(&self, event: LspEvent) -> Result<(), String> {
        self.event_sender.send(event).map_err(|e| format!("Failed to emit event: {}", e))
    }

    pub fn shutdown(&self) -> Result<Arc<Value>, String> {
        let request_id = self.send_shutdown();
        let result = self.await_response(request_id)?;
        self.emit(LspEvent::Shutdown)?;
        Ok(result)
    }

    fn send_shutdown(&self) -> u64 {
        let request_id = self.next_request_id();
        self.send_request(request_id, "shutdown", None);
        request_id
    }

    fn receive_shutdown(&self, _json: Arc<Value>) -> Result<(), String> {
        info!("Server shutdown successfully.");
        Ok(())
    }

    fn send_exit(&self) {
        self.send_notification("exit", json!({}));
    }

    pub fn exit(&self) -> Result<(), String> {
        self.send_exit();
        thread::sleep(Duration::from_millis(5));
        self.emit(LspEvent::Exit)
    }

    pub fn handle_lsp_document_event(&self, event: LspEvent) {
        match event {
            LspEvent::FileOpened { document_id: _, uri, text } => {
                self.send_text_document_did_open(uri, text);
            }
            LspEvent::TextChanged {
                document_id: _,
                uri,
                version,
                from_line,
                from_column,
                to_line,
                to_column,
                text,
            } => {
                let changes = vec![TextDocumentContentChangeEvent {
                    range: Some(Range {
                        start: Position {
                            line: from_line as u32,
                            character: from_column as u32,
                        },
                        end: Position {
                            line: to_line as u32,
                            character: to_column as u32,
                        },
                    }),
                    range_length: Some(text.chars().count() as u32),
                    text,
                }];
                self.send_text_document_did_change(&uri, version, changes);
            }
            _ => (),
        }
    }
}

static INIT: Once = Once::new();
pub fn setup() {
    INIT.call_once(|| {
        let timer = fmt::time::OffsetTime::new(
            UtcOffset::UTC,
            format_description!("[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z"),
        );
        tracing_subscriber::registry()
            .with(fmt::layer().with_timer(timer))
            .with(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "debug".into()))
            .init();
    });
}

#[macro_export]
macro_rules! with_lsp_client {
    ($test_name:ident, $comm_type:expr, $callback:expr) => {
        #[test]
        fn $test_name() {
            crate::common::lsp_client::setup();
            let (event_sender, event_receiver) = std::sync::mpsc::channel::<crate::common::lsp_event::LspEvent>();

            // Determine communication type
            let comm_type = match $comm_type {
                crate::common::lsp_client::CommType::Stdio => {
                    crate::common::lsp_client::CommType::Stdio
                }
                crate::common::lsp_client::CommType::Tcp { port } => {
                    crate::common::lsp_client::CommType::Tcp { port }
                }
                crate::common::lsp_client::CommType::Pipe { path } => {
                    crate::common::lsp_client::CommType::Pipe { path }
                }
                crate::common::lsp_client::CommType::WebSocket { port } => {
                    crate::common::lsp_client::CommType::WebSocket { port }
                }
            };

            match crate::common::lsp_client::LspClient::start(
                String::from("rholang"),
                env!("CARGO_BIN_EXE_rholang-language-server").to_string(),
                comm_type,
                event_sender,
            ) {
                Ok(client) => {
                    let client = std::sync::Arc::new(client);
                    let event_thread = {
                        let client = std::sync::Arc::clone(&client);
                        std::thread::spawn(move || {
                            for event in event_receiver {
                                match event {
                                    crate::common::lsp_event::LspEvent::FileOpened { .. } => {
                                        client.handle_lsp_document_event(event)
                                    }
                                    crate::common::lsp_event::LspEvent::TextChanged { .. } => {
                                        client.handle_lsp_document_event(event)
                                    }
                                    crate::common::lsp_event::LspEvent::Exit => break,
                                    _ => {},
                                }
                            }
                        })
                    };

                    let result = client.initialize();
                    assert!(result.is_ok(), "Initialize failed: {}", result.unwrap_err());
                    let result = client.initialized();
                    assert!(result.is_ok(), "Initialized failed: {}", result.unwrap_err());
                    $callback(&client);
                    let result = client.shutdown();
                    assert!(result.is_ok(), "Shutdown failed: {}", result.unwrap_err());
                    let result = client.exit();
                    assert!(result.is_ok(), "Exit failed: {}", result.unwrap_err());
                    let result = client.stop();
                    assert!(result.is_ok(), "Stop failed: {}", result.unwrap_err());
                    event_thread.join().expect("Failed to join event thread");
                }
                Err(e) => {
                    panic!("Failed to start client: {}", e);
                }
            }
        }
    };
}
