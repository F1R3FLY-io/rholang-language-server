use std::sync::Arc;

use serde_json::{json, Value};

use tower_lsp::lsp_types::{
    ClientCapabilities, DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DocumentHighlight, DocumentHighlightParams, DocumentSymbol, DocumentSymbolParams, GotoDefinitionParams,
    InitializeParams, InitializeResult, Location, LogMessageParams, MessageType, Position, PublishDiagnosticsParams, Range,
    ReferenceContext, ReferenceParams, RenameParams, SemanticTokens, SemanticTokensDeltaParams, SemanticTokensFullDeltaResult,
    SemanticTokensParams, SemanticTokensResult, SymbolInformation, TextDocumentClientCapabilities,
    TextDocumentContentChangeEvent, TextDocumentIdentifier, TextDocumentItem, TextDocumentSyncClientCapabilities,
    TextDocumentSyncKind, Url, VersionedTextDocumentIdentifier, WorkspaceEdit, WorkspaceSymbol, WorkspaceSymbolParams,
};
use tower_lsp::lsp_types::request::GotoDeclarationParams;

use tracing::{debug, error, info, warn};

use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

use crate::lsp::client::LspClient;
use crate::lsp::document::LspDocument;
use crate::lsp::events::LspEvent;

pub type RequestHandler = fn(&LspClient, &Value) -> Result<Option<Arc<Value>>, String>;
pub type NotificationHandler = fn(&LspClient, &Value) -> Result<(), String>;
pub type ResponseHandler = fn(&LspClient, Arc<Value>) -> Result<(), String>;

impl LspClient {
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
            .insert(document.uri(), document.clone());
        document.open()?;
        Ok(document)
    }

    pub fn close_document(&self, document: &Arc<LspDocument>) -> Result<(), String> {
        document.close()?;
        self.documents_by_uri
            .write()
            .expect("Failed to acquire write lock on documents_by_uri")
            .remove(&document.uri());
        Ok(())
    }

    fn dispatch(&self, message: String) -> Result<(), String> {
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
            .map_err(|e| format!("Failed to send response: {}", e))
    }

    fn send_request(&self, request_id: u64, method: &str, params: Option<Value>) {
        let mut message = json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": method,
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
            "params": params,
        });
        let message_str = serde_json::to_string(&message).expect("Failed to serialize message");
        if let Err(e) = self.sender.lock().expect("Failed to lock sender").as_ref().expect("Sender dropped").send(message_str) {
            error!("Failed to send notification: {}", e);
        }
    }

    fn await_response(&self, request_id: u64) -> Result<Arc<Value>, String> {
        // Check if response already available
        {
            let responses_by_id = self.responses_by_id.read().expect("Failed to acquire read lock on responses_by_id");
            if let Some(response) = responses_by_id.get(&request_id) {
                return Ok(response.clone());
            }
        }

        let timeout = Duration::from_secs(30);
        let start = Instant::now();

        // Process messages until we find the response or timeout
        loop {
            // Check timeout before waiting
            if start.elapsed() >= timeout {
                return Err(format!("Timeout waiting for response with id {}", request_id));
            }

            // Calculate remaining time
            let remaining = timeout.saturating_sub(start.elapsed());
            let wait_duration = remaining.min(Duration::from_millis(50)); // Check every 50ms max

            // Try to receive a message with a short timeout
            match self.receiver.lock().expect("Failed to lock receiver").recv_timeout(wait_duration) {
                Ok(message) => {
                    debug!("Processing message: {:?}", message);
                    // Process the message
                    if let Err(e) = self.dispatch(message) {
                        return Err(format!("Failed to dispatch message: {}", e));
                    }

                    // Check if we got the response we're waiting for
                    let responses_by_id = self.responses_by_id.read().expect("Failed to acquire read lock on responses_by_id");
                    if let Some(response) = responses_by_id.get(&request_id) {
                        return Ok(response.clone());
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    // Timeout on recv - loop back to check overall timeout
                    continue;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("LSP server disconnected while waiting for response".to_string());
                }
            }
        }
    }

    pub fn await_diagnostics(&self, doc: &LspDocument) -> Result<Arc<PublishDiagnosticsParams>, String> {
        // Check if diagnostics already available
        {
            let diagnostics_by_id = self.diagnostics_by_id.read().expect("Failed to acquire read lock on diagnostics_by_id");
            if let Some(diagnostics) = diagnostics_by_id.get(&doc.id) {
                if let Some(version) = diagnostics.version {
                    if version == doc.version.load(Ordering::Relaxed) {
                        return Ok(diagnostics.clone());
                    }
                }
            }
        }

        let timeout = Duration::from_secs(20);  // Increased for large files like robot_planning.rho
        let start = Instant::now();

        // Process messages until we find the diagnostic or timeout
        // Use shorter recv_timeout to check for timeout periodically while remaining responsive
        loop {
            // Check timeout before waiting
            if start.elapsed() >= timeout {
                return Err(format!("Timeout waiting for diagnostics for document with URI: {}", doc.uri()));
            }

            // Calculate remaining time
            let remaining = timeout.saturating_sub(start.elapsed());
            let wait_duration = remaining.min(Duration::from_millis(50)); // Check every 50ms max

            // Try to receive a message with a short timeout
            match self.receiver.lock().expect("Failed to lock receiver").recv_timeout(wait_duration) {
                Ok(message) => {
                    // Process the message
                    if let Err(e) = self.dispatch(message) {
                        return Err(format!("Failed to dispatch message: {}", e));
                    }

                    // Check if we got the diagnostics we're waiting for
                    let diagnostics_by_id = self.diagnostics_by_id.read().expect("Failed to acquire read lock on diagnostics_by_id");
                    if let Some(diagnostics) = diagnostics_by_id.get(&doc.id) {
                        if let Some(version) = diagnostics.version {
                            if version == doc.version.load(Ordering::Relaxed) {
                                return Ok(diagnostics.clone());
                            }
                        }
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    // Timeout on recv - loop back to check overall timeout
                    continue;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("LSP server disconnected while waiting for diagnostics".to_string());
                }
            }
        }
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

    pub fn receive_initialize(&self, json: Arc<Value>) -> Result<(), String> {
        if let Some(result) = json.get("result") {
            let init_result: InitializeResult = serde_json::from_value(result.clone())
                .map_err(|e| format!("Failed to parse InitializeResult: {}", e))?;
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
        let params = tower_lsp::lsp_types::InitializedParams {};
        self.send_notification("initialized", serde_json::to_value(params).expect("Failed to serialize params"));
    }

    pub fn initialized(&self) -> Result<(), String> {
        self.send_initialized();
        Ok(())
    }

    pub fn receive_window_log_message(&self, json: &Value) -> Result<(), String> {
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

    pub fn receive_semantic_tokens_full(&self, json: Arc<Value>) -> Result<(), String> {
        let uri = {
            let requests_by_id = self.requests_by_id.read().expect("Failed to acquire read lock on requests_by_id");
            let request_id = json["id"].as_u64().ok_or("Missing id in response")?;
            let request = requests_by_id.get(&request_id).ok_or("No request found for response id")?;
            request["params"]["textDocument"]["uri"]
                .as_str()
                .ok_or("Missing URI in request params")?
                .to_string()
        };
        if json.get("result").is_some() {
            let result: Option<SemanticTokensResult> = serde_json::from_value(json["result"].clone())
                .map_err(|e| format!("Failed to parse SemanticTokensResult: {}", e))?;
            debug!("Received semanticTokens/full response for URI {}: {:?}", uri, result);
            self.semantic_tokens_by_uri
                .write()
                .expect("Failed to acquire write lock on semantic_tokens_by_uri")
                .insert(uri, Arc::new(result));
            Ok(())
        } else {
            let id = json["id"].as_u64();
            self.send_invalid_request("textDocument/semanticTokens/full", "Missing result", id);
            Err("Missing result in semanticTokens/full response".to_string())
        }
    }

    pub fn receive_semantic_tokens_full_delta(&self, json: Arc<Value>) -> Result<(), String> {
        let uri = {
            let requests_by_id = self.requests_by_id.read().expect("Failed to acquire read lock on requests_by_id");
            let request_id = json["id"].as_u64().ok_or("Missing id in response")?;
            let request = requests_by_id.get(&request_id).ok_or("No request found for response id")?;
            request["params"]["textDocument"]["uri"]
                .as_str()
                .ok_or("Missing URI in request params")?
                .to_string()
        };
        if let Some(result) = json.get("result") {
            let result: Option<SemanticTokensFullDeltaResult> = serde_json::from_value(result.clone())
                .map_err(|e| format!("Failed to parse SemanticTokensFullDeltaResult: {}", e))?;
            let semantic_tokens_result = result.map(|r| match r {
                SemanticTokensFullDeltaResult::Tokens(tokens) => {
                    debug!("Received full tokens for URI {}", uri);
                    SemanticTokensResult::Tokens(tokens)
                }
                SemanticTokensFullDeltaResult::TokensDelta(delta) => {
                    debug!("Received delta edits for URI {}, count: {}", uri, delta.edits.len());
                    // Get previous tokens from cache
                    let previous_tokens = self
                        .semantic_tokens_by_uri
                        .read()
                        .expect("Failed to acquire read lock on semantic_tokens_by_uri")
                        .get(&uri)
                        .and_then(|r| r.as_ref().as_ref())
                        .map(|r| match r {
                            SemanticTokensResult::Tokens(t) => t.data.clone(),
                            _ => vec![],
                        })
                        .unwrap_or_default();
                    let mut current_tokens = previous_tokens;
                    // Apply delta edits
                    for edit in delta.edits {
                        let start = edit.start as usize;
                        let delete_count = edit.delete_count as usize;
                        if start <= current_tokens.len() {
                            current_tokens.drain(start..start.saturating_add(delete_count));
                            if let Some(data) = edit.data {
                                current_tokens.splice(start..start, data);
                            }
                        } else {
                            warn!(
                                "Invalid edit for URI {}: start={}, delete_count={}, token_count={}",
                                uri, start, delete_count, current_tokens.len()
                            );
                        }
                    }
                    debug!("Reconstructed {} tokens for URI {}", current_tokens.len(), uri);
                    SemanticTokensResult::Tokens(SemanticTokens {
                        result_id: delta.result_id,
                        data: current_tokens,
                    })
                }
                SemanticTokensFullDeltaResult::PartialTokensDelta { edits } => {
                    debug!("Received partial delta edits for URI {}, count: {}", uri, edits.len());
                    // Get previous tokens from cache
                    let previous_tokens = self
                        .semantic_tokens_by_uri
                        .read()
                        .expect("Failed to acquire read lock on semantic_tokens_by_uri")
                        .get(&uri)
                        .and_then(|r| r.as_ref().as_ref())
                        .map(|r| match r {
                            SemanticTokensResult::Tokens(t) => t.data.clone(),
                            _ => vec![],
                        })
                        .unwrap_or_default();
                    let mut current_tokens = previous_tokens;
                    // Apply delta edits
                    for edit in edits {
                        let start = edit.start as usize;
                        let delete_count = edit.delete_count as usize;
                        if start <= current_tokens.len() {
                            current_tokens.drain(start..start.saturating_add(delete_count));
                            if let Some(data) = edit.data {
                                current_tokens.splice(start..start, data);
                            }
                        } else {
                            warn!(
                                "Invalid edit for URI {}: start={}, delete_count={}, token_count={}",
                                uri, start, delete_count, current_tokens.len()
                            );
                        }
                    }
                    debug!("Reconstructed {} tokens for URI {}", current_tokens.len(), uri);
                    SemanticTokensResult::Tokens(SemanticTokens {
                        result_id: None,
                        data: current_tokens,
                    })
                }
            });
            self.semantic_tokens_by_uri
                .write()
                .expect("Failed to acquire write lock on semantic_tokens_by_uri")
                .insert(uri, Arc::new(semantic_tokens_result));
            Ok(())
        } else {
            let id = json["id"].as_u64();
            self.send_invalid_request("textDocument/semanticTokens/full/delta", "Missing result", id);
            Err("Missing result in semanticTokens/full/delta response".to_string())
        }
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
                    tower_lsp::lsp_types::TextDocumentSyncCapability::Kind(kind) => Some(*kind),
                    tower_lsp::lsp_types::TextDocumentSyncCapability::Options(options) => options.change,
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
        debug!("Sending textDocument/didOpen notification for URI: {}", uri);
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

    pub fn send_text_document_did_close(&self, uri: &str) {
        if self.supports_text_document_sync() {
            let params = DidCloseTextDocumentParams {
                text_document: TextDocumentIdentifier {
                    uri: Url::parse(uri).expect("Invalid URI"),
                },
            };
            self.send_notification("textDocument/didClose", serde_json::to_value(params).expect("Failed to serialize params"));
        }
    }

    pub fn receive_text_document_publish_diagnostics(&self, json: &Value) -> Result<(), String> {
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

    pub fn receive_rename(&self, response: Arc<Value>) -> Result<(), String> {
        if let Some(result) = response.get("result") {
            let workspace_edit: WorkspaceEdit = serde_json::from_value(result.clone())
                .map_err(|e| format!("Failed to parse WorkspaceEdit: {}", e))?;
            if let Some(changes) = workspace_edit.changes {
                for (uri, edits) in changes {
                    self.documents_by_uri
                        .read()
                        .expect("Failed to read self.documents_by_uri")
                        .get(&uri.to_string())
                        .expect(&format!("Failed to find document for URI: {}", uri))
                        .apply(edits);
                }
            }
            Ok(())
        } else {
            Err("No result in rename response".to_string())
        }
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

    pub fn receive_shutdown(&self, _json: Arc<Value>) -> Result<(), String> {
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
            LspEvent::FileClosed { document_id: _, uri } => {
                self.send_text_document_did_close(&uri);
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
                debug!(
                    "Sending textDocument/didChange for URI={} with version={} and text='{}'",
                    uri, version, changes[0].text
                );
                self.send_text_document_did_change(&uri, version, changes);
            }
            _ => (),
        }
    }

    pub fn semantic_tokens_full(&self, params: SemanticTokensParams) -> Result<Option<SemanticTokensResult>, String> {
        let request_id = self.next_request_id();
        self.send_request(
            request_id,
            "textDocument/semanticTokens/full",
            Some(serde_json::to_value(params).map_err(|e| format!("Failed to serialize params: {}", e))?),
        );
        let response = self.await_response(request_id)?;
        if response.get("result").is_some() {
            let result = serde_json::from_value(response["result"].clone())
                .map_err(|e| format!("Failed to parse SemanticTokensResult: {}", e))?;
            Ok(result)
        } else {
            Err("No result in semanticTokens/full response".to_string())
        }
    }

    pub fn semantic_tokens_full_delta(
        &self,
        params: SemanticTokensDeltaParams,
    ) -> Result<Option<SemanticTokensFullDeltaResult>, String> {
        let request_id = self.next_request_id();
        self.send_request(
            request_id,
            "textDocument/semanticTokens/full/delta",
            Some(serde_json::to_value(params).map_err(|e| format!("Failed to serialize params: {}", e))?),
        );
        let response = self.await_response(request_id)?;
        if response.get("result").is_some() {
            let result = serde_json::from_value(response["result"].clone())
                .map_err(|e| format!("Failed to parse SemanticTokensFullDeltaResult: {}", e))?;
            Ok(result)
        } else {
            Err("No result in semanticTokens/full/delta response".to_string())
        }
    }

    pub fn rename(&self, uri: &str, position: Position, new_name: &str) -> Result<WorkspaceEdit, String> {
        let params = RenameParams {
            text_document_position: tower_lsp::lsp_types::TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: Url::parse(uri).map_err(|e| format!("Invalid URI: {}", e))?,
                },
                position,
            },
            new_name: new_name.to_string(),
            work_done_progress_params: Default::default(),
        };

        let request_id = self.next_request_id();
        self.send_request(
            request_id,
            "textDocument/rename",
            Some(serde_json::to_value(params).map_err(|e| format!("Failed to serialize params: {}", e))?),
        );

        let response = self.await_response(request_id)?;
        if let Some(result) = response.get("result") {
            serde_json::from_value(result.clone()).map_err(|e| format!("Failed to parse WorkspaceEdit: {}", e))
        } else {
            Err("No result in rename response".to_string())
        }
    }

    pub fn declaration(&self, uri: &str, position: Position) -> Result<Option<Location>, String> {
        let params = GotoDeclarationParams {
            text_document_position_params: tower_lsp::lsp_types::TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: Url::parse(uri).unwrap(),
                },
                position,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let request_id = self.next_request_id();
        self.send_request(request_id, "textDocument/declaration", Some(serde_json::to_value(params).unwrap()));
        let response = self.await_response(request_id)?;
        if let Some(result) = response.get("result") {
            if result.is_array() {
                let locations: Vec<Location> = serde_json::from_value(result.clone()).unwrap();
                Ok(locations.into_iter().next())
            } else {
                let location: Location = serde_json::from_value(result.clone()).unwrap();
                Ok(Some(location))
            }
        } else {
            Ok(None)
        }
    }

    pub fn receive_declaration(&self, response: Arc<Value>) -> Result<(), String> {
        if response.get("result").is_some() {
            Ok(())
        } else {
            Err("No result in declaration response".to_string())
        }
    }

    pub fn definition(&self, uri: &str, position: Position) -> Result<Option<Location>, String> {
        let params = GotoDefinitionParams {
            text_document_position_params: tower_lsp::lsp_types::TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: Url::parse(uri).unwrap(),
                },
                position,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let request_id = self.next_request_id();
        self.send_request(request_id, "textDocument/definition", Some(serde_json::to_value(params).unwrap()));
        let response = self.await_response(request_id)?;
        if let Some(result) = response.get("result") {
            if result.is_null() {
                // Server returned null, meaning no definition found
                Ok(None)
            } else if result.is_array() {
                let locations: Vec<Location> = serde_json::from_value(result.clone()).unwrap();
                Ok(locations.into_iter().next())
            } else {
                let location: Location = serde_json::from_value(result.clone())
                    .map_err(|e| format!("Failed to deserialize location: {}", e))?;
                Ok(Some(location))
            }
        } else {
            Ok(None)
        }
    }

    pub fn receive_definition(&self, response: Arc<Value>) -> Result<(), String> {
        if response.get("result").is_some() {
            Ok(())
        } else {
            Err("No result in definition response".to_string())
        }
    }

    pub fn references(
        &self,
        uri: &str,
        position: Position,
        include_declaration: bool,
    ) -> Result<Vec<Location>, String> {
        debug!("Sending references request for URI: {}, position: {:?}", uri, position);

        let params = ReferenceParams {
            text_document_position: tower_lsp::lsp_types::TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: Url::parse(uri).map_err(|e| format!("Invalid URI: {}", e))?,
                },
                position,
            },
            context: ReferenceContext {
                include_declaration,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let request_id = self.next_request_id();
        self.send_request(
            request_id,
            "textDocument/references",
            Some(serde_json::to_value(params).map_err(|e| format!("Failed to serialize params: {}", e))?),
        );

        let response = self.await_response(request_id)?;
        if let Some(result) = response.get("result") {
            if result.is_array() {
                let locations: Vec<Location> = serde_json::from_value(result.clone())
                    .map_err(|e| format!("Failed to parse locations: {}", e))?;
                debug!("Received {} references for URI: {}, position: {:?}", locations.len(), uri, position);
                Ok(locations)
            } else {
                debug!("Received empty or invalid references response for URI: {}, position: {:?}", uri, position);
                Ok(vec![])
            }
        } else {
            debug!("No result in references response for URI: {}, position: {:?}", uri, position);
            Ok(vec![])
        }
    }

    pub fn receive_references(&self, _response: Arc<Value>) -> Result<(), String> {
        debug!("Received references response");
        Ok(())
    }

    pub fn document_symbols(&self, uri: &str) -> Result<Vec<DocumentSymbol>, String> {
        debug!("Sending documentSymbol request for URI: {}", uri);

        let params = DocumentSymbolParams {
            text_document: TextDocumentIdentifier {
                uri: Url::parse(uri).map_err(|e| format!("Invalid URI: {}", e))?,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let request_id = self.next_request_id();
        self.send_request(
            request_id,
            "textDocument/documentSymbol",
            Some(serde_json::to_value(params).map_err(|e| format!("Failed to serialize params: {}", e))?),
        );

        let response = self.await_response(request_id)?;
        if let Some(result) = response.get("result") {
            if result.is_array() {
                let symbols: Vec<DocumentSymbol> = serde_json::from_value(result.clone())
                    .map_err(|e| format!("Failed to parse document symbols: {}", e))?;
                debug!("Received {} document symbols for URI: {}", symbols.len(), uri);
                Ok(symbols)
            } else {
                debug!("Received empty or invalid document symbols response for URI: {}", uri);
                Ok(vec![])
            }
        } else {
            debug!("No result in documentSymbol response for URI: {}", uri);
            Ok(vec![])
        }
    }

    pub fn receive_document_symbol(&self, _response: Arc<Value>) -> Result<(), String> {
        debug!("Received documentSymbol response");
        Ok(())
    }

    pub fn workspace_symbols(&self, query: &str) -> Result<Vec<SymbolInformation>, String> {
        debug!("Sending workspaceSymbol request with query: {}", query);

        let params = WorkspaceSymbolParams {
            query: query.to_string(),
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let request_id = self.next_request_id();
        self.send_request(
            request_id,
            "workspace/symbol",
            Some(serde_json::to_value(params).map_err(|e| format!("Failed to serialize params: {}", e))?),
        );

        let response = self.await_response(request_id)?;
        if let Some(result) = response.get("result") {
            if result.is_array() {
                let symbols: Vec<SymbolInformation> = serde_json::from_value(result.clone())
                    .map_err(|e| format!("Failed to parse symbol information: {}", e))?;
                debug!("Received {} workspace symbols for query: {}", symbols.len(), query);
                Ok(symbols)
            } else {
                debug!("Received empty or invalid workspace symbols response for query: {}", query);
                Ok(vec![])
            }
        } else {
            debug!("No result in workspaceSymbol response for query: {}", query);
            Ok(vec![])
        }
    }

    pub fn receive_workspace_symbol(&self, _response: Arc<Value>) -> Result<(), String> {
        debug!("Received workspaceSymbol response");
        Ok(())
    }

    pub fn workspace_symbol_resolve(&self, symbol: WorkspaceSymbol) -> Result<WorkspaceSymbol, String> {
        debug!("Sending workspaceSymbol/resolve request for symbol: {}", symbol.name);

        let request_id = self.next_request_id();
        self.send_request(
            request_id,
            "workspaceSymbol/resolve",
            Some(serde_json::to_value(symbol).map_err(|e| format!("Failed to serialize params: {}", e))?),
        );

        let response = self.await_response(request_id)?;
        if let Some(result) = response.get("result") {
            let resolved_symbol: WorkspaceSymbol = serde_json::from_value(result.clone())
                .map_err(|e| format!("Failed to parse resolved symbol: {}", e))?;
            debug!("Received resolved symbol for: {}", resolved_symbol.name);
            Ok(resolved_symbol)
        } else {
            Err("No result in workspaceSymbol/resolve response".to_string())
        }
    }

    pub fn receive_workspace_symbol_resolve(&self, _response: Arc<Value>) -> Result<(), String> {
        debug!("Received workspaceSymbol/resolve response");
        Ok(())
    }

    pub fn document_highlight(&self, uri: &str, position: Position) -> Result<Vec<DocumentHighlight>, String> {
        debug!("Sending documentHighlight request for URI: {}, position: {:?}", uri, position);

        let params = DocumentHighlightParams {
            text_document_position_params: tower_lsp::lsp_types::TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: Url::parse(uri).map_err(|e| format!("Invalid URI: {}", e))?,
                },
                position,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let request_id = self.next_request_id();
        self.send_request(
            request_id,
            "textDocument/documentHighlight",
            Some(serde_json::to_value(params).map_err(|e| format!("Failed to serialize params: {}", e))?),
        );

        let response = self.await_response(request_id)?;
        if let Some(result) = response.get("result") {
            if result.is_array() {
                let highlights: Vec<DocumentHighlight> = serde_json::from_value(result.clone())
                    .map_err(|e| format!("Failed to parse document highlights: {}", e))?;
                debug!("Received {} highlights for URI: {}, position: {:?}", highlights.len(), uri, position);
                Ok(highlights)
            } else {
                debug!("Received empty or null highlights response for URI: {}, position: {:?}", uri, position);
                Ok(vec![])
            }
        } else {
            debug!("No result in documentHighlight response for URI: {}, position: {:?}", uri, position);
            Ok(vec![])
        }
    }

    pub fn receive_document_highlight(&self, _response: Arc<Value>) -> Result<(), String> {
        debug!("Received documentHighlight response");
        Ok(())
    }
}
