use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use tokio::sync::{Mutex, RwLock};

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

use tracing::{error, info, warn};

use ropey::Rope;

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
    rnode_client: Arc<Mutex<LspClient<Channel>>>,
    client_process_id: Arc<Mutex<Option<u32>>>,
}

impl RholangBackend {
    pub fn new(client: Client, rnode_client: LspClient<Channel>, client_process_id: Option<u32>) -> Self {
        RholangBackend {
            client,
            documents_by_uri: Arc::new(RwLock::new(HashMap::new())),
            documents_by_id: Arc::new(RwLock::new(HashMap::new())),
            serial_document_id: Arc::new(AtomicU32::new(0)),
            rnode_client: Arc::new(Mutex::new(rnode_client)),
            client_process_id: Arc::new(Mutex::new(client_process_id)),
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
        let mut client = self.rnode_client.lock().await.clone();
        let state = document.state.read().await;
        if state.version == version {
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
                                    let range = diagnostic
                                        .range
                                        .expect("Missing required field for lsp::Diagnostic: range");
                                    let start = range
                                        .start
                                        .expect("Missing required field for lsp::Range: start");
                                    let end = range
                                        .end
                                        .expect("Missing required field for lsp::Range: end");
                                    let severity = match LspDiagnosticSeverity::try_from(
                                        diagnostic.severity,
                                    ) {
                                        Ok(severity) => match severity {
                                            LspDiagnosticSeverity::Error => DiagnosticSeverity::ERROR,
                                            LspDiagnosticSeverity::Warning => DiagnosticSeverity::WARNING,
                                            LspDiagnosticSeverity::Information => {
                                                DiagnosticSeverity::INFORMATION
                                            }
                                            LspDiagnosticSeverity::Hint => DiagnosticSeverity::HINT,
                                        },
                                        Err(e) => {
                                            error!("Invalid lsp::DiagnosticSeverity: {}", e);
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
                            info!("Publishing diagnostics for document with URI={}, version={}: {:?}",
                                  state.uri, version, diagnostics);
                            Ok(diagnostics)
                        }
                        validate_response::Result::Error(message) => Err(format!(
                            "Failed to validate document with ID={}, URI={}: {}",
                            document.id, state.uri, message
                        )),
                    },
                    None => Err("RNode did not return a response".to_string()),
                },
                Err(e) => Err(format!(
                    "Failed to validate document with ID={}, URI={}: {}",
                    document.id, state.uri, e
                )),
            }
        } else {
            Ok(Vec::new())
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for RholangBackend {
    async fn initialize(&self, params: InitializeParams) -> jsonrpc::Result<InitializeResult> {
        info!("Received initialize: {:?}", params);

        // Store client process ID from InitializeParams if provided
        if let Some(cmdline_pid) = *self.client_process_id.lock().await {
            if let Some(lsp_pid) = params.process_id {
                if cmdline_pid != lsp_pid {
                    warn!("Client process ID from command line ({}) differs from LSP initialize process ID ({})",
                          cmdline_pid, lsp_pid);
                }
            }
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
        info!("textDocument/didOpen: {:?}", params);
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
        info!("Opened document: {}, id: {}, version: {}",
              uri, document_id, version);
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
