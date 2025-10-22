#![recursion_limit = "1024"]
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[cfg(unix)]
use std::fs;

use futures_util::sink::SinkExt;
use futures_util::stream::TryStreamExt;
#[allow(unused_imports)]
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, DuplexStream, Stdout};
use tokio::net::{TcpListener, UnixListener};
use tokio::sync::{Notify, oneshot};
use tokio::task::JoinHandle;

#[cfg(windows)]
use tokio::net::windows::named_pipe::NamedPipeServer;

use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{accept_async, WebSocketStream};

use tower_lsp::{LspService, Server};

use tracing::{debug, error, info, trace, warn};

use clap::Parser;

use rholang_language_server::lsp::backend::RholangBackend;
use rholang_language_server::logging::init_logger;
use rholang_language_server::rnode_apis::lsp::lsp_client::LspClient;

// Define communication mode enum for ServerConfig
#[derive(Debug, Clone, PartialEq)]
enum CommMode {
    Stdio,
    Socket(u16),
    Pipe(String),
    WebSocket(u16),
}

// Server configuration struct
#[derive(Debug)]
struct ServerConfig {
    log_level: String,
    no_color: bool,
    comm_mode: CommMode,
    rnode_address: String,
    rnode_port: u16,
    client_process_id: Option<u32>,
    no_rnode: bool,
}

impl ServerConfig {
    fn from_args() -> io::Result<Self> {
        #[derive(Parser, Debug)]
        #[command(
            version = "1.0",
            about = "Rholang Language Server",
            long_about = "LSP-based language server for Rholang."
        )]
        struct Args {
            #[arg(
                long,
                default_value = "debug",
                help = "Set the logging level for the server",
                value_parser = ["error", "warn", "info", "debug", "trace"]
            )]
            log_level: String,
            #[arg(long, help = "Disable ANSI color output")]
            no_color: bool,
            #[arg(
                long,
                help = "Use stdin/stdout for communication (mutually exclusive with --socket, --pipe, --websocket)",
                conflicts_with_all = ["socket", "websocket", "pipe"]
            )]
            stdio: bool,
            #[arg(
                long,
                requires = "port",
                help = "Use TCP socket for communication (requires --port; mutually exclusive with --stdio, --pipe, --websocket)",
                conflicts_with_all = ["stdio", "pipe", "websocket"]
            )]
            socket: bool,
            #[arg(
                long,
                requires = "port",
                help = "Use WebSocket for communication (requires --port; mutually exclusive with --stdio, --socket, --pipe)",
                conflicts_with_all = ["stdio", "socket", "pipe"]
            )]
            websocket: bool,
            #[arg(long, help = "Port number for socket or WebSocket communication")]
            port: Option<u16>,
            #[arg(
                long,
                help = "Address of the RNode server (e.g., '127.0.0.1'). Can be set via RHOLANG_ADDRESS_NODE env variable.",
                default_value = "localhost"
            )]
            rnode_address: String,
            #[arg(
                long,
                help = "Port of the RNode server. Can be set via RHOLANG_PORT_NODE env variable.",
                default_value_t = 40402
            )]
            rnode_port: u16,
            #[arg(
                long,
                alias = "clientProcessId",
                help = "Process ID of the client for monitoring (optional)"
            )]
            client_process_id: Option<u32>,
            #[arg(
                long,
                help = "Path to named pipe or Unix socket (e.g., '\\\\.\\pipe\\rholang-lsp' on Windows or '/tmp/rholang.socket' on Unix; mutually exclusive with --stdio, --socket, --websocket)",
                conflicts_with_all = ["stdio", "socket", "websocket"]
            )]
            pipe: Option<String>,
            #[arg(long, help = "Disable RNode integration for semantic analysis (rely on parser only)")]
            no_rnode: bool,
        }

        let args = Args::parse();

        let rnode_address = std::env::var("RHOLANG_ADDRESS_NODE").unwrap_or(args.rnode_address);
        let rnode_port = match std::env::var("RHOLANG_PORT_NODE") {
            Ok(port_str) => port_str.parse::<u16>().map_err(|e| {
                error!("Invalid RHOLANG_PORT_NODE environment variable: {}", e);
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("Invalid port in RHOLANG_PORT_NODE: {}", e),
                )
            })?,
            Err(_) => args.rnode_port,
        };

        let comm_mode = match (args.stdio, args.socket, args.websocket, args.pipe) {
            (true, false, false, None) => CommMode::Stdio,
            (false, true, false, None) => {
                let port = args.port.ok_or_else(|| {
                    error!("The --port option is required when --socket is used.");
                    io::Error::new(io::ErrorKind::InvalidInput, "Port required for socket mode")
                })?;
                CommMode::Socket(port)
            }
            (false, false, true, None) => {
                let port = args.port.ok_or_else(|| {
                    error!("The --port option is required when --websocket is used.");
                    io::Error::new(io::ErrorKind::InvalidInput, "Port required for websocket mode")
                })?;
                CommMode::WebSocket(port)
            }
            (false, false, false, Some(pipe)) => {
                #[cfg(windows)]
                if !pipe.starts_with(r"\\.\pipe\") {
                    error!("Invalid named pipe path: {}. Must start with '\\\\.\\pipe\\'.", pipe);
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("Invalid named pipe path: {}", pipe),
                    ));
                }
                CommMode::Pipe(pipe)
            }
            _ => {
                error!("Exactly one of --stdio, --socket, --websocket, --pipe must be specified.");
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Invalid communication mode",
                ));
            }
        };

        Ok(ServerConfig {
            log_level: args.log_level,
            no_color: args.no_color,
            comm_mode,
            rnode_address,
            rnode_port,
            client_process_id: args.client_process_id,
            no_rnode: args.no_rnode,
        })
    }
}

// WebSocketStreamAdapter
struct WebSocketStreamAdapter<S> {
    inner: WebSocketStream<S>,
    read_buffer: Vec<u8>,
}

impl<S> WebSocketStreamAdapter<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn new(inner: WebSocketStream<S>) -> Self {
        WebSocketStreamAdapter {
            inner,
            read_buffer: Vec::new(),
        }
    }

    #[allow(dead_code)]
    async fn close(&mut self) -> io::Result<()> {
        self.inner
            .send(Message::Close(None))
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        self.inner
            .flush()
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        Ok(())
    }
}

impl<S> AsyncRead for WebSocketStreamAdapter<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        let this = self.get_mut();
        if !this.read_buffer.is_empty() {
            info!("Using buffered data: {} bytes", this.read_buffer.len());
            let to_copy = std::cmp::min(buf.remaining(), this.read_buffer.len());
            buf.put_slice(&this.read_buffer[..to_copy]);
            this.read_buffer.drain(..to_copy);
            return std::task::Poll::Ready(Ok(()));
        }

        match this.inner.try_poll_next_unpin(cx) {
            std::task::Poll::Ready(Some(Ok(Message::Text(text)))) => {
                trace!("Received WebSocket text message: {}", text);
                this.read_buffer.extend_from_slice(text.as_bytes());
                let to_copy = std::cmp::min(buf.remaining(), this.read_buffer.len());
                buf.put_slice(&this.read_buffer[..to_copy]);
                this.read_buffer.drain(..to_copy);
                std::task::Poll::Ready(Ok(()))
            }
            std::task::Poll::Ready(Some(Ok(Message::Binary(data)))) => {
                trace!("Received WebSocket binary message: {:?}", data);
                this.read_buffer.extend_from_slice(&data);
                let to_copy = std::cmp::min(buf.remaining(), this.read_buffer.len());
                buf.put_slice(&this.read_buffer[..to_copy]);
                this.read_buffer.drain(..to_copy);
                std::task::Poll::Ready(Ok(()))
            }
            std::task::Poll::Ready(Some(Ok(Message::Ping(_)))) => {
                trace!("Received WebSocket ping message");
                std::task::Poll::Ready(Ok(()))
            }
            std::task::Poll::Ready(Some(Ok(Message::Pong(_)))) => {
                trace!("Received WebSocket pong message");
                std::task::Poll::Ready(Ok(()))
            }
            std::task::Poll::Ready(Some(Ok(Message::Frame(_)))) => {
                trace!("Received WebSocket frame message");
                std::task::Poll::Ready(Ok(()))
            }
            std::task::Poll::Ready(Some(Ok(Message::Close(_)))) => {
                trace!("Received WebSocket close message");
                std::task::Poll::Ready(Ok(()))
            }
            std::task::Poll::Ready(Some(Err(e))) => {
                trace!("WebSocket error: {}", e);
                std::task::Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, e)))
            }
            std::task::Poll::Ready(None) => {
                trace!("WebSocket stream closed");
                std::task::Poll::Ready(Ok(()))
            }
            std::task::Poll::Pending => {
                trace!("WebSocket poll pending");
                std::task::Poll::Pending
            }
        }
    }
}

impl<S> AsyncWrite for WebSocketStreamAdapter<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        let this = self.get_mut();
        match this.inner.poll_ready_unpin(cx) {
            std::task::Poll::Ready(Ok(())) => match this.inner.start_send_unpin(Message::Binary(buf.to_vec())) {
                Ok(()) => std::task::Poll::Ready(Ok(buf.len())),
                Err(e) => std::task::Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, e))),
            },
            std::task::Poll::Ready(Err(e)) => std::task::Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, e))),
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        let this = self.get_mut();
        match this.inner.poll_flush_unpin(cx) {
            std::task::Poll::Ready(Ok(())) => std::task::Poll::Ready(Ok(())),
            std::task::Poll::Ready(Err(e)) => std::task::Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, e))),
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        let this = self.get_mut();
        match this.inner.poll_close_unpin(cx) {
            std::task::Poll::Ready(Ok(())) => std::task::Poll::Ready(Ok(())),
            std::task::Poll::Ready(Err(e)) => std::task::Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, e))),
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}

// ConnectionManager
#[derive(Clone)]
struct ConnectionManager {
    shutdown_notify: Arc<Notify>,
    connections: Arc<Mutex<Vec<oneshot::Sender<()>>>>,
    tasks: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl ConnectionManager {
    fn new() -> Self {
        ConnectionManager {
            shutdown_notify: Arc::new(Notify::new()),
            connections: Arc::new(Mutex::new(Vec::new())),
            tasks: Arc::new(Mutex::new(Vec::new())),
        }
    }

    async fn add_connection(&self, tx: oneshot::Sender<()>) {
        let mut conns = self.connections.lock().unwrap();
        conns.push(tx);
        info!("Added connection, total: {}", conns.len());
    }

    fn add_task(&self, task: JoinHandle<()>) {
        let mut tasks = self.tasks.lock().unwrap();
        tasks.push(task);
        info!("Added task, total: {}", tasks.len());
    }

    async fn remove_closed_connections(&self) {
        let mut conns = self.connections.lock().unwrap();
        conns.retain(|tx| !tx.is_closed());
        info!("Remaining connections: {}", conns.len());
    }

    async fn shutdown_all(&self) {
        info!("Initiating shutdown of all connections and tasks");
        // Remove closed connections first
        self.remove_closed_connections().await;
        // Signal remaining connections
        let mut conns = self.connections.lock().unwrap();
        for tx in conns.drain(..) {
            if tx.send(()).is_err() {
                debug!("Failed to send shutdown signal to a connection; likely already closed");
            }
        }
        self.shutdown_notify.notify_waiters();

        let mut tasks = self.tasks.lock().unwrap();
        for task in tasks.drain(..) {
            task.abort();
        }
        info!("All tasks canceled");
    }

    async fn wait_for_tasks(&self) {
        let tasks: Vec<JoinHandle<()>> = {
            let mut tasks = self.tasks.lock().unwrap();
            tasks.drain(..).collect()
        };
        for task in tasks {
            if let Err(e) = tokio::time::timeout(Duration::from_secs(5), task).await {
                error!("Task did not complete in time: {:?}", e);
            }
        }
        info!("All tasks completed or timed out");
    }
}

async fn serve_connection<R, W>(
    read: R,
    write: W,
    addr: impl std::fmt::Display + Send + 'static,
    rnode_client: Option<LspClient<tonic::transport::Channel>>,
    conn_manager: &ConnectionManager,
    client_process_id: Option<u32>,
    pid_channel: Option<tokio::sync::mpsc::Sender<u32>>,
) where
    R: tokio::io::AsyncRead + Send + Unpin + 'static,
    W: tokio::io::AsyncWrite + Send + Unpin + 'static,
{
    info!("Accepted connection from {}", addr);
    let (service, socket) = LspService::new(|client| {
        Arc::new(RholangBackend::new(client, rnode_client, client_process_id, pid_channel.clone()))
    });
    let (conn_tx, conn_rx) = oneshot::channel::<()>();
    conn_manager.add_connection(conn_tx).await;

    let shutdown_notify = conn_manager.shutdown_notify.clone();
    let task = tokio::spawn(async move {
        let server = Server::new(read, write, socket);
        tokio::select! {
            _ = server.serve(service) => {
                info!("Connection from {} closed normally", addr);
            }
            _ = conn_rx => {
                info!("Shutdown signal received for connection from {}", addr);
                shutdown_notify.notified().await;
                info!("Exit processed for connection from {}", addr);
            }
        }
    });
    conn_manager.add_task(task);
}

#[cfg(unix)]
async fn monitor_client_process(client_pid: u32, conn_manager: ConnectionManager) {
    use nix::unistd::Pid;
    use tokio::time::{sleep, Duration};

    let pid = Pid::from_raw(client_pid as i32);
    loop {
        match nix::sys::signal::kill(pid, None) {
            Ok(()) => {
                sleep(Duration::from_secs(1)).await;
            }
            Err(nix::Error::ESRCH) => {
                info!("Client process (PID: {}) no longer exists, shutting down server", client_pid);
                conn_manager.shutdown_notify.notify_waiters(); // Signal main to shut down
                break;
            }
            Err(e) => {
                error!("Error checking client process (PID: {}): {}", client_pid, e);
                sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

#[cfg(windows)]
async fn monitor_client_process(client_pid: u32, conn_manager: ConnectionManager) {
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, WaitForSingleObject};
    use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
    use std::ptr;

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_INFORMATION, false, client_pid);
        if handle == HANDLE(ptr::null_mut()) {
            error!("Failed to open client process (PID: {})", client_pid);
            return;
        }
        let result = WaitForSingleObject(handle, 0xFFFFFFFF);
        if result == WAIT_OBJECT_0 {
            info!("Client process (PID: {}) terminated, shutting down server", client_pid);
            conn_manager.shutdown_notify.notify_waiters(); // Signal main to shut down
        } else {
            error!("Error waiting for client process (PID: {}): {:?}", client_pid, result);
        }
    }
}

async fn run_stdio_server(
    rnode_client: Option<LspClient<tonic::transport::Channel>>,
    config: ServerConfig,
    conn_manager: ConnectionManager
) -> io::Result<()> {
    info!("Starting server with stdin/stdout communication.");

    // Create reactive channel for PID events
    let (pid_tx, mut pid_rx) = tokio::sync::mpsc::channel::<u32>(1);

    let (service, socket) = LspService::build(|client| {
        RholangBackend::new(client, rnode_client.clone(), config.client_process_id, Some(pid_tx.clone()))
    }).finish();
    let stdin = BufReader::new(tokio::io::stdin()); // Wrap stdin in BufReader
    let stdout = tokio::io::stdout();

    // Spawn reactive listener for PID events
    let conn_manager_clone = conn_manager.clone();
    let conn_manager_clone2 = conn_manager.clone();
    tokio::spawn(async move {
        if let Some(pid) = pid_rx.recv().await {
            info!("Received client PID from LSP initialization: {}", pid);
            let monitor_task = tokio::spawn(async move {
                monitor_client_process(pid, conn_manager_clone).await;
            });
            conn_manager_clone2.add_task(monitor_task);
        }
    });

    let shutdown_notify = conn_manager.shutdown_notify.clone();
    let server_task = tokio::spawn(async move {
        let server = Server::new(stdin, stdout, socket);
        tokio::select! {
            _ = server.serve(service) => {
                info!("Stdio server terminated normally");
            }
            _ = shutdown_notify.notified() => {
                info!("Shutdown signal received, stopping stdio server");
            }
        }
    });
    conn_manager.add_task(server_task);

    // Wait for shutdown signal (from client monitor or signal)
    conn_manager.shutdown_notify.notified().await;
    conn_manager.shutdown_all().await;
    conn_manager.wait_for_tasks().await;
    Ok(())
}

async fn run_socket_server(
    rnode_client: Option<LspClient<tonic::transport::Channel>>,
    config: ServerConfig,
    conn_manager: ConnectionManager,
    port: u16
) -> io::Result<()> {
    info!("Starting server with TCP socket communication on port {}.", port);
    let listener = TcpListener::bind(format!("127.0.0.1:{}", port)).await?;
    info!("TCP server listening on 127.0.0.1:{}", port);

    loop {
        tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok((stream, addr)) => {
                        let (read, write) = tokio::io::split(stream);
                        serve_connection(read, write, addr, rnode_client.clone(), &conn_manager, config.client_process_id, None).await;
                        conn_manager.remove_closed_connections().await;
                    }
                    Err(e) => {
                        error!("Failed to accept TCP connection: {}", e);
                    }
                }
            }
            _ = conn_manager.shutdown_notify.notified() => {
                info!("Main shutdown signal received, closing TCP server");
                break;
            }
        }
    }
    conn_manager.shutdown_all().await;
    conn_manager.wait_for_tasks().await;
    Ok(())
}

async fn run_websocket_server(
    rnode_client: Option<LspClient<tonic::transport::Channel>>,
    config: ServerConfig,
    conn_manager: ConnectionManager,
    port: u16
) -> io::Result<()> {
    info!("Starting server with WebSocket communication on port {}.", port);
    let listener = TcpListener::bind(format!("127.0.0.1:{}", port)).await?;
    info!("WebSocket server listening on 127.0.0.1:{}", port);

    loop {
        tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok((stream, addr)) => {
                        match accept_async(stream).await {
                            Ok(ws_stream) => {
                                let ws_adapter = WebSocketStreamAdapter::new(ws_stream);
                                let (read, write) = tokio::io::split(ws_adapter);
                                serve_connection(read, write, addr, rnode_client.clone(), &conn_manager, config.client_process_id, None).await;
                                conn_manager.remove_closed_connections().await;
                            }
                            Err(e) => {
                                error!("Failed to accept WebSocket connection from {}: {}", addr, e);
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to accept TCP connection: {}", e);
                    }
                }
            }
            _ = conn_manager.shutdown_notify.notified() => {
                info!("Main shutdown signal received, closing WebSocket server");
                break;
            }
        }
    }
    conn_manager.shutdown_all().await;
    conn_manager.wait_for_tasks().await;
    Ok(())
}

async fn run_named_pipe_server(
    rnode_client: Option<LspClient<tonic::transport::Channel>>,
    config: &ServerConfig,
    conn_manager: ConnectionManager,
    pipe_path: &String
) -> io::Result<()> {
    #[cfg(windows)]
    {
        info!("Starting server with named pipe communication at {}.", pipe_path);
        loop {
            let server = NamedPipeServer::new(&pipe_path).await?;
            tokio::select! {
                _ = server.connect() => {
                    let addr = format!("named_pipe:{}", pipe_path);
                    let (read, write) = tokio::io::split(server);
                    serve_connection(read, write, addr, rnode_client.clone(), &conn_manager, config.client_process_id, None).await;
                    conn_manager.remove_closed_connections().await;
                }
                _ = conn_manager.shutdown_notify.notified() => {
                    info!("Main shutdown signal received, closing named pipe server");
                    break;
                }
            }
        }
        conn_manager.shutdown_all().await;
        conn_manager.wait_for_tasks().await;
    }
    #[cfg(unix)]
    {
        info!("Starting server with Unix domain socket communication at {}.", pipe_path);
        if std::path::Path::new(&pipe_path).exists() {
            fs::remove_file(&pipe_path)?;
        }
        let listener = UnixListener::bind(&pipe_path)?;
        let cleanup = scopeguard::guard(pipe_path.clone(), |path| {
            if let Err(e) = fs::remove_file(&path) {
                error!("Failed to clean up Unix socket file {}: {}", path, e);
            } else {
                info!("Cleaned up Unix socket file {}.", path);
            }
        });
        loop {
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((stream, addr)) => {
                            let addr = format!("unix_socket:{:?}", addr);
                            let (read, write) = tokio::io::split(stream);
                            serve_connection(read, write, addr, rnode_client.clone(), &conn_manager, config.client_process_id, None).await;
                            conn_manager.remove_closed_connections().await;
                        }
                        Err(e) => {
                            error!("Failed to accept Unix socket connection: {}", e);
                        }
                    }
                }
                _ = conn_manager.shutdown_notify.notified() => {
                    info!("Main shutdown signal received, closing Unix socket server");
                    break;
                }
            }
        }
        drop(cleanup);
        conn_manager.shutdown_all().await;
        conn_manager.wait_for_tasks().await;
    }
    #[cfg(not(any(windows, unix)))]
    {
        error!("Named pipe/Unix domain socket communication is not supported on this platform.");
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "Named pipe/Unix domain socket communication is not supported on this platform.",
        ));
    }
    Ok(())
}

async fn run_server(config: ServerConfig, conn_manager: ConnectionManager) -> io::Result<()> {
    init_logger(config.no_color, Some(&config.log_level))?;
    info!("Initializing rholang-language-server with log level {} ...", config.log_level);

    let rnode_client_opt: Option<LspClient<tonic::transport::Channel>> = if !config.no_rnode {
        let rnode_endpoint = format!("http://{}:{}", config.rnode_address, config.rnode_port);
        match tonic::transport::Uri::try_from(&rnode_endpoint) {
            Ok(rnode_uri) => {
                match LspClient::connect(tonic::transport::Endpoint::from(rnode_uri)).await {
                    Ok(client) => {
                        info!("Successfully connected to RNode at {}", rnode_endpoint);
                        Some(client)
                    }
                    Err(e) => {
                        warn!("Failed to connect to RNode at {}: {}. Continuing with parser-only validation.", rnode_endpoint, e);
                        None
                    }
                }
            }
            Err(e) => {
                warn!("Invalid RNode endpoint {}: {}. Continuing with parser-only validation.", rnode_endpoint, e);
                None
            }
        }
    } else {
        info!("RNode integration disabled via --no-rnode flag; relying on parser for analysis.");
        None
    };

    if let Some(client_pid) = config.client_process_id {
        let conn_manager_clone = conn_manager.clone();
        let monitor_task = tokio::spawn(async move {
            monitor_client_process(client_pid, conn_manager_clone).await;
        });
        conn_manager.add_task(monitor_task);
    }

    match config.comm_mode {
        CommMode::Stdio => run_stdio_server(rnode_client_opt, config, conn_manager).await?,
        CommMode::Socket(port) => run_socket_server(rnode_client_opt, config, conn_manager, port).await?,
        CommMode::WebSocket(port) => run_websocket_server(rnode_client_opt, config, conn_manager, port).await?,
        CommMode::Pipe(ref pipe_path) => run_named_pipe_server(rnode_client_opt, &config, conn_manager, pipe_path).await?,
    }

    info!("Server terminated.");
    Ok(())
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let config = ServerConfig::from_args()?;
    let conn_manager = ConnectionManager::new();

    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigint = signal(SignalKind::interrupt())?;
        let mut sigterm = signal(SignalKind::terminate())?;

        tokio::select! {
            result = run_server(config, conn_manager.clone()) => {
                conn_manager.shutdown_all().await;
                conn_manager.wait_for_tasks().await;
                result
            }
            _ = sigint.recv() => {
                info!("Received SIGINT, initiating shutdown");
                conn_manager.shutdown_all().await;
                conn_manager.wait_for_tasks().await;
                Ok(())
            }
            _ = sigterm.recv() => {
                info!("Received SIGTERM, initiating shutdown");
                conn_manager.shutdown_all().await;
                conn_manager.wait_for_tasks().await;
                Ok(())
            }
        }
    }

    #[cfg(windows)]
    {
        use tokio::signal::ctrl_c;
        tokio::select! {
            result = run_server(config, conn_manager.clone()) => {
                conn_manager.shutdown_all().await;
                conn_manager.wait_for_tasks().await;
                result
            }
            _ = ctrl_c() => {
                info!("Received Ctrl+C, initiating shutdown");
                conn_manager.shutdown_all().await;
                conn_manager.wait_for_tasks().await;
                Ok(())
            }
        }
    }

    #[cfg(not(any(unix, windows)))]
    {
        run_server(config, conn_manager.clone()).await?;
        conn_manager.shutdown_all().await;
        conn_manager.wait_for_tasks().await;
        Ok(())
    }
}
