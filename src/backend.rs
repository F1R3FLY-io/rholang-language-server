use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU32, Ordering};

use tokio::sync::{Mutex as AsyncMutex, RwLock};

use tonic::Request;
use tonic::transport::Channel;

use tower_lsp::{Client, LanguageServer, jsonrpc};
use tower_lsp::lsp_types::{
    Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DidSaveTextDocumentParams, InitializedParams, InitializeParams,
    InitializeResult, Position, Range, ServerCapabilities,
    TextDocumentSyncCapability, TextDocumentSyncKind, Url,
};

use tracing::{debug, error, info, warn};

use ropey::Rope;

use crate::parser::Parser;  // Import the new parser module
use crate::rnode_apis::lsp::{
    lsp_client::LspClient,
    DiagnosticSeverity as LspDiagnosticSeverity,
    ValidateRequest,
    validate_response,
};
use crate::models::{LspDocument, LspDocumentHistory, LspDocumentState};

#[derive(Debug, Clone)]
pub struct RholangBackend {
    client: Client,
    documents_by_uri: Arc<RwLock<HashMap<Url, Arc<LspDocument>>>>,
    documents_by_id: Arc<RwLock<HashMap<u32, Arc<LspDocument>>>>,
    serial_document_id: Arc<AtomicU32>,
    rnode_client: Arc<AsyncMutex<LspClient<Channel>>>,
    client_process_id: Arc<Mutex<Option<u32>>>,
    parser: Arc<AsyncMutex<Parser>>,
}

impl RholangBackend {
    pub fn new(client: Client, rnode_client: LspClient<Channel>, client_process_id: Option<u32>) -> Self {
        RholangBackend {
            client,
            documents_by_uri: Arc::new(RwLock::new(HashMap::new())),
            documents_by_id: Arc::new(RwLock::new(HashMap::new())),
            serial_document_id: Arc::new(AtomicU32::new(0)),
            rnode_client: Arc::new(AsyncMutex::new(rnode_client)),
            client_process_id: Arc::new(Mutex::new(client_process_id)),
            parser: Arc::new(AsyncMutex::new(Parser::new().expect("Failed to create parser"))),
        }
    }

    fn next_document_id(&self) -> u32 {
        self.serial_document_id.fetch_add(1, Ordering::SeqCst)
    }

    async fn validate(
        &self,
        document: Arc<LspDocument>,
        text: &str,
        version: i32,
    ) -> Result<Vec<Diagnostic>, String> {
        let state = document.state.read().await;
        if state.version != version {
            debug!("Skipping validation for outdated version {} (current: {})", version, state.version);
            return Ok(Vec::new());
        }

        // Local syntax validation using rholang-parser
        let mut parser = self.parser.lock().await;
        let diagnostics = match parser.validate(text) {
            Ok(()) => Vec::new(),
            Err(parse_error) => {
                let position = parse_error.position; // May be None if parser can't locate error
                let (line, character) = if let Some(pos) = position {
                    // Ensure position is valid (1-based, so > 0)
                    if pos.line > 0 && pos.column > 0 {
                        // Convert to 0-based indexing safely
                        (pos.line as u32 - 1, pos.column as u32 - 1)
                    } else {
                        // Log invalid position and default to end
                        debug!("Invalid error position from parser: {:?}", pos);
                        let (line, character) = document.last_linecol().await; // Use document's last position
                        (line as u32, character as u32)
                    }
                } else {
                    // No position provided, default to end of document
                    debug!("No error position provided by parser, using end of document");
                    let (line, character) = document.last_linecol().await; // Use document's last position
                    (line as u32, character as u32)
                };
                // Create diagnostic at the calculated position
                vec![Diagnostic {
                    range: Range {
                        start: Position { line, character },
                        end: Position { line, character },
                    },
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("rholang-parser".to_string()),
                    message: parse_error.message,
                    ..Default::default()
                }]
            }
        };

        if !diagnostics.is_empty() {
            info!("Local syntax errors found for URI={} (version={}): {:?}", state.uri, version, diagnostics);
            return Ok(diagnostics);
        }

        // RNode validation if syntax is valid
        let mut client = self.rnode_client.lock().await.clone();
        let request = Request::new(ValidateRequest {
            text: text.to_string(),
        });
        match client.validate(request).await {
            Ok(response) => match response.into_inner().result {
                Some(result) => match result {
                    validate_response::Result::Success(diagnostic_list) => {
                        let diagnostics = diagnostic_list
                            .diagnostics
                            .into_iter()
                            .map(|diagnostic| {
                                let range = diagnostic.range.expect("Missing range in diagnostic");
                                let start = range.start.expect("Missing start position");
                                let end = range.end.expect("Missing end position");
                                let severity = match LspDiagnosticSeverity::try_from(diagnostic.severity) {
                                    Ok(severity) => match severity {
                                        LspDiagnosticSeverity::Error => DiagnosticSeverity::ERROR,
                                        LspDiagnosticSeverity::Warning => DiagnosticSeverity::WARNING,
                                        LspDiagnosticSeverity::Information => DiagnosticSeverity::INFORMATION,
                                        LspDiagnosticSeverity::Hint => DiagnosticSeverity::HINT,
                                    },
                                    Err(e) => {
                                        error!("Invalid DiagnosticSeverity: {}", e);
                                        DiagnosticSeverity::ERROR
                                    }
                                };
                                Diagnostic {
                                    range: Range {
                                        start: Position {
                                            line: start.line as u32,
                                            character: start.column as u32,
                                        },
                                        end: Position {
                                            line: end.line as u32,
                                            character: end.column as u32,
                                        },
                                    },
                                    severity: Some(severity),
                                    source: Some(diagnostic.source),
                                    message: diagnostic.message,
                                    ..Default::default()
                                }
                            })
                            .collect();
                        info!("RNode validation diagnostics for URI={} (version={}): {:?}", state.uri, version, diagnostics);
                        Ok(diagnostics)
                    }
                    validate_response::Result::Error(message) => Err(format!(
                        "RNode validation failed for URI={}: {}",
                        state.uri, message
                    )),
                },
                None => Err("RNode returned no response".to_string()),
            },
            Err(e) => Err(format!(
                "Failed to communicate with RNode for URI={}: {}",
                state.uri, e
            )),
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for RholangBackend {
    async fn initialize(&self, params: InitializeParams) -> jsonrpc::Result<InitializeResult> {
        info!("Received initialize: {:?}", params);

        // Update client process ID if provided
        if let Some(client_pid) = params.process_id {
            let mut locked_pid = self.client_process_id.lock().unwrap();
            if let Some(cmdline_pid) = *locked_pid {
                if cmdline_pid != client_pid {
                    warn!(
                        "Client process ID from command line ({}) differs from LSP initialize process ID ({})",
                        cmdline_pid, client_pid
                    );
                }
            }
            *locked_pid = Some(client_pid);
        }

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::INCREMENTAL,
                )),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, params: InitializedParams) {
        info!("initialized: {:?}", params);
    }

    async fn shutdown(&self) -> jsonrpc::Result<()> {
        info!("Received shutdown request");
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        info!("Opening document: URI={}, version={}", params.text_document.uri, params.text_document.version);
        let uri = params.text_document.uri.clone();
        let text = params.text_document.text;
        let version = params.text_document.version;
        let document_id = self.next_document_id();
        let document = Arc::new(LspDocument {
            id: document_id,
            state: RwLock::new(LspDocumentState {
                uri: uri.clone(),
                text: Rope::from_str(&text),
                version,
                history: LspDocumentHistory {
                    text: text.clone(),
                    changes: Vec::new(),
                },
            }),
        });
        self.documents_by_uri
            .write()
            .await
            .insert(uri.clone(), Arc::clone(&document));
        self.documents_by_id
            .write()
            .await
            .insert(document_id, Arc::clone(&document));
        info!("Opened document: URI={}, id={}, version={}", uri, document_id, version);

        if tracing::level_enabled!(tracing::Level::DEBUG) {
            let mut parser = self.parser.lock().await;
            if let Ok(pretty_tree) = parser.get_pretty_tree(&text) {
                debug!("Parse tree for URI={}:\n{}", uri, pretty_tree);
            }
        }

        match self.validate(Arc::clone(&document), &text, version).await {
            Ok(diagnostics) => {
                if document.version().await == version {
                    self.client
                        .publish_diagnostics(uri, diagnostics, Some(version))
                        .await;
                }
            }
            Err(e) => error!("Validation failed for URI={}: {}", uri, e),
        }
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        info!("textDocument/didChange: {:?}", params);
        let uri = params.text_document.uri.clone();
        let version = params.text_document.version;
        if let Some(document) = self.documents_by_uri.read().await.get(&uri) {
            if let Some(text) = document.apply(params.content_changes, version).await {
                match self.validate(Arc::clone(&document), &text, version).await {
                    Ok(diagnostics) => {
                        if document.version().await == version {
                            self.client
                                .publish_diagnostics(uri, diagnostics, Some(version))
                                .await;
                        }
                    }
                    Err(e) => error!(
                        "Failed to validate document with ID={}, URI={}: {}",
                        document.id,
                        document.uri().await,
                        e
                    ),
                }
            } else {
                warn!("Failed to apply changes to document with URI={}", uri);
            }
        } else {
            warn!("Failed to find document with URI={}", uri);
        }
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        info!("textDocument/didSave: {:?}", params);
        // NOTE: The document was validated on textDocument/didOpen and on each
        // textDocument/didChange. There is nothing more to validate.
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        info!("textDocument/didClose: {:?}", params);
        let uri = params.text_document.uri;
        if let Some(document) = self.documents_by_uri.write().await.remove(&uri) {
            self.documents_by_id.write().await.remove(&document.id);
            info!("Closed document: {}, id: {}", uri, document.id);
        } else {
            warn!("Failed to find document with URI={}", uri);
        }
        self.client
            .publish_diagnostics(uri, Vec::new(), None)
            .await;
    }
}
