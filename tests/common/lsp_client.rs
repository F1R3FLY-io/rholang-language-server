use std::collections::HashMap;
use std::io::{self, BufReader, Read, Write, stdout};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Once, RwLock, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use tracing::{info, warn, error, debug, trace};

use tracing_subscriber::{self, fmt, prelude::*};

use time::macros::format_description;
use time::UtcOffset;

use tower_lsp::lsp_types::{
    ClientCapabilities, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, InitializeParams, InitializeResult,
    InitializedParams, LogMessageParams, MessageType, Position,
    PublishDiagnosticsParams, Range, ServerCapabilities,
    TextDocumentClientCapabilities, TextDocumentContentChangeEvent,
    TextDocumentIdentifier, TextDocumentItem, TextDocumentSyncCapability,
    TextDocumentSyncClientCapabilities, TextDocumentSyncKind, Url,
    VersionedTextDocumentIdentifier
};

use serde_json::{self, json, Value};

use crate::common::lsp_document::LspDocument;
use crate::common::lsp_event::LspEvent;
use crate::common::lsp_message_stream::LspMessageStream;

type RequestHandler = fn(&LspClient, &Value) -> Result<Option<Value>, String>;
type NotificationHandler = fn(&LspClient, &Value) -> Result<(), String>;
type ResponseHandler = fn(&LspClient, &Value) -> Result<(), String>;

#[allow(dead_code)]
pub struct LspClient {
    server: Mutex<Child>,
    sender: Mutex<Option<Sender<String>>>,
    receiver: Mutex<Receiver<String>>,
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
    stdin_thread: Mutex<Option<JoinHandle<()>>>,
    stdout_thread: Mutex<Option<JoinHandle<()>>>,
    stderr_thread: Mutex<Option<JoinHandle<()>>>,
    event_sender: Sender<LspEvent>,
}

#[allow(dead_code)]
impl LspClient {
    pub fn start(
        language_id: String,
        server_path: String,
        server_args: Vec<String>,
        event_sender: Sender<LspEvent>,
    ) -> std::io::Result<Self> {
        let mut server = Command::new(server_path)
            .args(&server_args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let server_stdin = server.stdin.take().expect("Failed to open server stdin");
        let server_stdout = server.stdout.take().expect("Failed to open server stdout");
        let server_stderr = server.stderr.take().expect("Failed to open server stderr");

        let (sender, rx) = channel::<String>();
        let (tx, receiver) = channel::<String>();

        let stdin_thread = thread::spawn(move || {
            let mut server_stdin = server_stdin;
            loop {
                match rx.recv() {
                    Ok(body) => {
                        let content_length = body.len();
                        let headers = format!("Content-Length: {}\r\n\r\n", content_length);
                        trace!("Sending:\r\n{}{}", headers, body);
                        server_stdin.write_all(headers.as_bytes())
                            .expect("Failed to write header");
                        server_stdin.write_all(body.as_bytes())
                            .expect("Failed to write message");
                        server_stdin.flush().expect("Failed to flush stdin");
                    }
                    Err(e) => {
                        if e.to_string() == "receiving on a closed channel" {
                            info!("Server stdin channel closed.");
                        } else {
                            error!("Failed to receive message for the server: {}", e);
                        }
                        return;
                    }
                }
            }
        });

        let stdout_thread = thread::spawn(move || {
            let reader = BufReader::with_capacity(4096, server_stdout);
            let mut message_stream = LspMessageStream::new(reader);
            loop {
                match message_stream.next_payload() {
                    Ok(payload) => {
                        trace!("Receiving:\r\n{}", message_stream.message());
                        tx.send(payload).expect("Failed to send message.");
                    }
                    Err(e) => {
                        if e.as_str() == "Input stream closed" {
                            info!("Server stdout closed.");
                        } else {
                            error!("Failed to read next message: {}", e);
                        }
                        return;
                    }
                }
            }
        });

        let stderr_thread = thread::spawn(move || {
            let mut client_stdout = stdout();
            let mut server_stderr = server_stderr;
            let mut read_buffer = vec![0u8; 4096];
            loop {
                match server_stderr.read(&mut read_buffer) {
                    Ok(0) => {
                        info!("Server stderr closed.");
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
                            return;
                        }
                    }
                    Err(e) => {
                        error!("Error reading from server stderr: {}", e);
                        if let Err(e) = client_stdout.flush() {
                            error!("Error flushing client stdout: {}", e);
                        }
                        return;
                    }
                }
            }
        });

        // Register request handlers by their LSP method ids, here:
        let request_handlers = HashMap::new();

        // Register notification handlers by their LSP method ids, here:
        let mut notification_handlers = HashMap::new();
        notification_handlers.insert(
            "textDocument/publishDiagnostics".to_string(),
            LspClient::receive_text_document_publish_diagnostics as NotificationHandler
        );
        notification_handlers.insert(
            "window/logMessage".to_string(),
            LspClient::receive_window_log_message as NotificationHandler
        );

        // Register response handlers by their LSP method ids, here:
        let mut response_handlers = HashMap::new();
        response_handlers.insert(
            "initialize".to_string(),
            LspClient::receive_initialize as ResponseHandler
        );
        response_handlers.insert(
            "shutdown".to_string(),
            LspClient::receive_shutdown as ResponseHandler
        );

        let client = LspClient {
            server: Mutex::new(server),
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
            stdin_thread: Mutex::new(Some(stdin_thread)),
            stdout_thread: Mutex::new(Some(stdout_thread)),
            stderr_thread: Mutex::new(Some(stderr_thread)),
            event_sender,
        };

        Ok(client)
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
        {
            let mut documents_by_uri = self.documents_by_uri.write()
                .expect("Failed to acquire write lock on documents_by_uri");
            documents_by_uri.insert(document.uri(), document.clone());
        }
        document.open()?;
        Ok(document)
    }

    fn dispatch(&self, message: String) -> Result<(), String> {
        match serde_json::from_str::<Value>(&message) {
            Ok(json) => {
                if json.get("method").is_some() {
                    if json.get("id").is_some() {
                        if let Err(e) = self.dispatch_request(json) {
                            return Err(format!("Failed to dispatch request: {}", e));
                        }
                    } else {
                        if let Err(e) = self.dispatch_notification(json) {
                            return Err(format!("Failed to dispatch notification: {}", e));
                        }
                    }
                } else {
                    if let Err(e) = self.dispatch_response(json) {
                        return Err(format!("Failed to dispatch response: {}", e));
                    }
                }
            }
            Err(e) => {
                return Err(format!("Failed to parse message as JSON: {}\n{}", e, message));
            }
        }
        Ok(())
    }

    fn dispatch_request(&self, json: Value) -> Result<(), String> {
        let method = json["method"].as_str().expect("Missing required attribute: method");
        let id = json["id"].as_u64().expect("Missing required attribute: id");
        if let Some(handler) = self.request_handlers.get(method) {
            match handler(self, &json) {
                Ok(result) => {
                    if let Err(e) = self.send_response(result, Some(id)) {
                        return Err(format!("Failed to send response: {}", e));
                    }
                }
                Err(e) => {
                    return Err(format!(
                        "Failed to dispatch response for method=\"{}\": {}",
                        method, e
                    ));
                }
            }
        } else if !method.starts_with("$/") {
            self.send_method_not_found(method, Some(id));
        } else {
            error!(
                "No request handler exists for optional method: {}",
                method
            );
        }
        Ok(())
    }

    fn dispatch_notification(&self, json: Value) -> Result<(), String> {
        let method = json["method"].as_str().expect("Missing required attribute: method");
        if let Some(handler) = self.notification_handlers.get(method) {
            match handler(self, &json) {
                Ok(_) => {},
                Err(e) => return Err(format!(
                    "Failed to dispatch notification with method=\"{}\": {}",
                    method, e
                )),
            }
        } else if !method.starts_with("$/") {
            self.send_method_not_found(method, None);
        } else {
            error!(
                "No notification handler exists for optional method: {}",
                method
            );
        }
        Ok(())
    }

    fn dispatch_response(&self, response: Value) -> Result<(), String> {
        if response.get("result").is_some() {
            if let Some(request_id) = response["id"].as_u64() {
                let response = Arc::new(response);
                {
                    let mut responses_by_id = self.responses_by_id.write()
                        .expect("Failed to acquire write lock on responses_by_id");
                    responses_by_id.insert(request_id, response.clone());
                }
                let requests_by_id = self.requests_by_id.read()
                    .expect("Failed to acquire read lock on requests_by_id");
                if let Some(request) = requests_by_id.get(&request_id) {
                    let method = request["method"].as_str()
                        .expect("Missing required attribute: method");
                    if let Some(handler) = self.response_handlers.get(method) {
                        if let Err(e) = handler(self, &response) {
                            return Err(format!(
                                "Failed to handle response for method=\"{}\": {}",
                                method, e
                            ));
                        }
                    } else {
                        return Err(format!(
                            "No handler exists for method: \"{}\"", method
                        ));
                    }
                }
            }
        } else if response.get("error").is_some() {
            if let Some(id) = response["id"].as_u64() {
                self.responses_by_id.write()
                    .expect("Failed to acquire write lock on responses_by_id")
                    .insert(id, Arc::new(response));
            }
        } else {
            return Err(format!(
                "Cannot dispatch response without a result or error: {:?}",
                serde_json::to_string(&response)
            ));
        }
        Ok(())
    }

    fn send_invalid_request(&self, method: &str, message: &str, id: Option<u64>) {
        let invalid_request = -32600;
        self.send_error(
            id,
            invalid_request,
            format!("Invalid request for method=\"{}\": {}", method, message),
            None
        );
    }

    fn send_method_not_found(&self, method: &str, id: Option<u64>) {
        let method_not_found = -32601;
        self.send_error(
            id,
            method_not_found,
            format!("Unsupported method: {}", method),
            None
        );
    }

    fn send_error(&self, id: Option<u64>, code: i32, message: String, data: Option<Value>) {
        let error = json!({
            "code": code,
            "message": message,
            "data": data,
        });
        let message = json!({
            "jsonrpc": "2.0", //<- invariant
            "id": id,
            "error": error,
        });
        let message_str = serde_json::to_string(&message).expect("Failed to serialize message");
        self.sender
            .lock()
            .expect("Failed to acquire lock on sender")
            .as_ref()
            .expect("Sender dropped")
            .send(message_str)
            .expect("Failed to send message");
    }

    fn send_response(&self, result: Option<Value>, id: Option<u64>) -> Result<(), String> {
        let message = json!({
            "jsonrpc": "2.0", //<- invariant
            "id": id,
            "result": result,
        });
        let message_str = serde_json::to_string(&message)
            .expect("Failed to serialize message");
        self.sender
            .lock()
            .expect("Failed to acquire lock on sender")
            .as_ref()
            .expect("Sender dropped")
            .send(message_str)
            .expect("Failed to send message");
        Ok(())
    }

    fn send_request(&self, request_id: u64, method: &str, params: Option<Value>) {
        let mut message = json!({
            "jsonrpc": "2.0", //<- invariant
            "id": request_id,
            "method": method
        });
        if params.is_some() {
            message["params"] = params.expect("Failed to unwrap params");
        }
        let message_str = serde_json::to_string(&message)
            .expect("Failed to serialize message");
        {
            let mut requests_by_id = self.requests_by_id.write()
                .expect("Failed to acquire write lock on requests_by_id");
            requests_by_id.insert(request_id, Arc::new(message));
        }
        self.sender
            .lock()
            .expect("Failed to acquire lock on sender")
            .as_ref()
            .expect("Sender dropped")
            .send(message_str)
            .expect("Failed to send message");
    }

    fn send_notification(&self, method: &str, params: Value) {
        let message = json!({
            "jsonrpc": "2.0", //<- invariant
            "method": method,
            "params": params
        });
        let message_str = serde_json::to_string(&message)
            .expect("Failed to serialize message");
        self.sender
            .lock()
            .expect("Failed to acquire lock on sender")
            .as_ref()
            .expect("Sender dropped")
            .send(message_str)
            .expect("Failed to send message");
    }

    fn await_response(&self, request_id: u64) -> Result<Arc<Value>, String> {
        {
            let responses_by_id = self.responses_by_id.read()
                .expect("Failed to acquire read lock on responses_by_id");
            if let Some(response) = responses_by_id.get(&request_id) {
                return Ok(response.clone());
            }
        }

        let timeout = Duration::from_secs(5);
        let start = Instant::now();

        while start.elapsed() < timeout {
            if let Ok(message) = self.receiver.lock()
                .expect("Failed to acquire lock on receiver")
                .recv_timeout(Duration::from_millis(100)) {
                match self.dispatch(message) {
                    Ok(_) => {
                        let responses_by_id = self.responses_by_id.read()
                            .expect("Failed to acquire read lock on responses_by_id");
                        if let Some(response) = responses_by_id.get(&request_id) {
                            return Ok(response.clone());
                        }
                    }
                    Err(e) => {
                        return Err(format!("Failed to dispatch await response: {}", e));
                    }
                }
            }
        }

        return Err(format!("Timeout waiting for response with id {}", request_id));
    }

    pub fn await_diagnostics(&self, doc: &LspDocument) -> Result<Arc<PublishDiagnosticsParams>, String> {
        {
            let diagnostics_by_id = self.diagnostics_by_id.read()
                .expect("Failed to acquire read lock on diagnostics_by_id");
            if let Some(diagnostics) = diagnostics_by_id.get(&doc.id) {
                return Ok(diagnostics.clone());
            }
        }

        let timeout = Duration::from_secs(5);
        let start = Instant::now();

        while start.elapsed() < timeout {
            if let Ok(message) = self.receiver.lock()
                .expect("Failed to acquire lock on receiver")
                .recv_timeout(Duration::from_millis(100)) {
                match self.dispatch(message) {
                    Ok(_) => {
                        let diagnostics_by_id = self.diagnostics_by_id.read()
                            .expect("Failed to acquire read lock on diagnostics_by_id");
                        if let Some(diagnostics) = diagnostics_by_id.get(&doc.id) {
                            return Ok(diagnostics.clone());
                        }
                    }
                    Err(e) => {
                        return Err(format!("Failed to dispatch await response: {}", e));
                    }
                }
            }
        }

        return Err(format!(
            "Timeout waiting for diagnostics for document with URI: {}",
            doc.uri()
        ));
    }

    pub fn initialize(&self) -> Result<Arc<Value>, String> {
        let request_id = self.send_initialize();
        return self.await_response(request_id);
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
                        ..Default::default()
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
        self.send_request(
            request_id, "initialize",
            Some(serde_json::to_value(params)
                 .expect("Failed to serialize params to JSON"))
        );
        return request_id;
    }

    fn receive_initialize(&self, json: &Value) -> Result<(), String> {
        if let Some(result) = json.get("result") {
            let init_result: InitializeResult =
                serde_json::from_value(result.clone()).expect("Failed to parse InitializeResult");
            {
                let mut server_capabilities = self.server_capabilities.write()
                    .expect("Failed to acquire write lock on server_capabilities");
                *server_capabilities = Some(init_result.capabilities);
            }
        } else {
            self.send_invalid_request(
                "initialize",
                "Missing required attribute: result",
                json["id"].as_u64()
            );
        }
        Ok(())
    }

    fn send_initialized(&self) {
        let params = InitializedParams {};
        self.send_notification("initialized", serde_json::to_value(params)
                               .expect("Failed to serialize params to JSON"));
    }

    pub fn initialized(&self) -> Result<(), String> {
        self.send_initialized();
        Ok(())
    }

    fn receive_window_log_message(&self, json: &Value) -> Result<(), String> {
        let params: LogMessageParams =
                serde_json::from_value(json["params"].clone())
            .expect("Failed to parse LogMessageParams");
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
        self.server_capabilities.read()
            .expect("Failed to acquire read lock on server_capabilities")
            .as_ref()
            .and_then(|caps| {
                caps.text_document_sync.as_ref().and_then(|sync| match sync {
                    TextDocumentSyncCapability::Kind(kind) => Some(*kind),
                    TextDocumentSyncCapability::Options(options) => options.change,
                })
            })
    }

    fn send_text_document_did_open(&self, uri: String, text: String) {
        if !self.supports_text_document_sync() {
            warn!("Server does not support text document synchronization. Skipping didOpen.");
            return;
        }
        let params = DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: Url::parse(uri.as_str()).expect("Invalid URI"),
                language_id: "plaintext".to_string(),
                version: 1,
                text: text.to_string(),
            },
        };
        self.send_notification(
            "textDocument/didOpen",
            serde_json::to_value(params)
                .expect("Failed to serialize params to JSON")
        );
    }

    fn send_text_document_did_change(
        &self,
        uri: &str,
        version: i32,
        changes: Vec<TextDocumentContentChangeEvent>
    ) {
        match self.get_text_document_sync_kind() {
            Some(TextDocumentSyncKind::FULL) | Some(TextDocumentSyncKind::INCREMENTAL) => {
                let params = DidChangeTextDocumentParams {
                    text_document: VersionedTextDocumentIdentifier {
                        uri: Url::parse(uri).expect("Invalid URI"),
                        version,
                    },
                    content_changes: changes,
                };
                self.send_notification(
                    "textDocument/didChange",
                    serde_json::to_value(params)
                        .expect("Failed to serialize params to JSON")
                );
            }
            _ => {
                println!("Server does not support text document changes. Skipping didChange.");
            }
        }
    }

    fn send_text_document_did_close(&self, uri: &str) -> Result<(), String> {
        if !self.supports_text_document_sync() {
            return Err(
                "Server does not support text document synchronization. Skipping didClose.".to_string()
            );
        }
        let params = DidCloseTextDocumentParams {
            text_document: TextDocumentIdentifier {
                uri: Url::parse(uri).expect("Invalid URI"),
            },
        };
        self.send_notification(
            "textDocument/didClose",
            serde_json::to_value(params)
                .expect("Failed to serialize params to JSON")
        );
        Ok(())
    }

    fn receive_text_document_publish_diagnostics(
        &self,
        json: &Value
    ) -> Result<(), String> {
        let params: PublishDiagnosticsParams =
                serde_json::from_value(json["params"].clone())
            .expect("Failed to parse PublishDiagnosticsParams");
        let uri = params.uri.to_string();
        if let Some(version) = params.version {
            let documents_by_uri = self.documents_by_uri.read()
                .expect("Failed to acquire read lock on documents_by_uri");
            if let Some(document) = documents_by_uri.get(&uri) {
                let latest_version = document.version.load(Ordering::Relaxed);
                if latest_version == version {
                    let params = Arc::new(params.clone());
                    let mut diagnostics_by_id = self.diagnostics_by_id.write()
                        .expect("Failed to acquire write lock on diagnostics_by_id");
                    diagnostics_by_id.insert(document.id, params);
                } else {
                    warn!(
                        "Diagnostics were for an older version of document with URI=\"{}\": {} < {}",
                        uri, version, latest_version
                    )
                }
            } else {
                return Err(format!("No document exists for URI: {}", uri));
            }
        }
        Ok(())
    }

    fn emit(&self, event: LspEvent) -> Result<(), String> {
        self.event_sender.send(event)
            .map_err(|e| format!("Failed to emit event: {}", e))
    }

    pub fn shutdown(&self) -> Result<Arc<Value>, String> {
        let request_id = self.send_shutdown();
        let result = self.await_response(request_id);
        self.emit(LspEvent::Shutdown)?;
        result
    }

    fn send_shutdown(&self) -> u64 {
        let request_id = self.next_request_id();
        self.send_request(request_id, "shutdown", None);
        return request_id;
    }

    fn receive_shutdown(&self, _json: &Value) -> Result<(), String> {
        info!("Server was successfully shut down.");
        Ok(())
    }

    fn send_exit(&self) {
        self.send_notification("exit", json!({}));
    }

    pub fn exit(&self) -> Result<(), String> {
        self.send_exit();
        self.emit(LspEvent::Exit)
    }

    pub fn stop(&self) -> io::Result<()> {
        // Drop sender to close the rx channel and terminate stdin thread
        {
            let mut sender = self.sender.lock()
                .expect("Failed to acquire lock on sender");
            *sender = None;
        }
        // Join stderr thread first to ensure all stderr output is written
        if let Some(stderr_thread) = self.stderr_thread.lock()
            .expect("Failed to acquire lock on stderr_thread")
            .take() {
            if let Err(e) = stderr_thread.join() {
                error!("Error joining stderr thread: {:?}", e);
            }
        }
        // Join stdout and stdin threads
        if let Some(stdout_thread) = self.stdout_thread.lock()
            .expect("Failed to acquire lock on stdout_thread")
            .take() {
            if let Err(e) = stdout_thread.join() {
                error!("Error joining stdout thread: {:?}", e);
            }
        }
        if let Some(stdin_thread) = self.stdin_thread.lock()
            .expect("Failed to acquire lock on stdin_thread")
            .take() {
            if let Err(e) = stdin_thread.join() {
                error!("Error joining stdin thread: {:?}", e);
            }
        }
        // Kill and wait for the server process
        {
            let mut server = self.server.lock()
                .expect("Failed to acquire lock on server");
            server.kill()?;
            server.wait()?;
        }
        Ok(())
    }

    pub fn handle_lsp_document_event(&self, event: LspEvent) {
        match event {
            LspEvent::FileOpened {
                document_id: _,
                uri,
                text,
            } => {
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
                let changes = vec![
                    TextDocumentContentChangeEvent {
                        range: Some(Range{
                            start: Position{
                                line: from_line as u32,
                                character: from_column as u32,
                            },
                            end: Position{
                                line: to_line as u32,
                                character: to_column as u32,
                            },
                        }),
                        range_length: Some(text.chars().count() as u32),
                        text: text.to_string(),
                    },
                ];
                self.send_text_document_did_change(uri.as_str(), version, changes);
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
            format_description!("[[[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z]"),
        );
        tracing_subscriber::registry()
            .with(
                fmt::layer()
                    .with_timer(timer)
            )
            .with(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("trace"))
            )
            .init();
    });
}

#[macro_export]
macro_rules! with_lsp_client {
    ($test_name:ident, $callback:expr) => {
        #[test]
        fn $test_name() {
            crate::common::lsp_client::setup();
            let (event_sender, event_receiver) =
                std::sync::mpsc::channel::<
                    crate::common::lsp_event::LspEvent
                >();
            match crate::common::lsp_client::LspClient::start(
                "rholang".to_string(),
                "target/debug/rholang-language-server".to_string(),
                Vec::new(),
                event_sender,
            ) {
                Ok(client) => {
                    let client = std::sync::Arc::new(client);
                    let _event_thread = {
                        let client = client.clone();
                        std::thread::spawn(move || {
                            for event in event_receiver {
                                match event {
                                    crate::common::lsp_event::LspEvent::FileOpened{..} =>
                                        client.handle_lsp_document_event(event),
                                    crate::common::lsp_event::LspEvent::TextChanged{..} =>
                                        client.handle_lsp_document_event(event),
                                    crate::common::lsp_event::LspEvent::Shutdown => break,
                                    crate::common::lsp_event::LspEvent::Exit => break,
                                }
                            }
                        })
                    };
                    assert!(client.initialize().is_ok());
                    assert!(client.initialized().is_ok());
                    $callback(&client);
                    assert!(client.shutdown().is_ok());
                    assert!(client.exit().is_ok());
                    assert!(client.stop().is_ok());
                }
                Err(e) => {
                    tracing::error!("Failed to start client: {}", e);
                }
            }
        }
    };
}
