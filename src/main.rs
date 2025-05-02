use std::collections::{BinaryHeap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use tokio::sync::{Mutex, RwLock};

use tracing::error;

use tracing_subscriber::{self, fmt, prelude::*};

use time::macros::format_description;
use time::UtcOffset;

use tonic::Request;
use tonic::transport::Channel;

use tower_lsp::{Client, LanguageServer, LspService, Server};
use tower_lsp::jsonrpc;
use tower_lsp::lsp_types::{
    Diagnostic,
    DiagnosticSeverity,
    DidChangeTextDocumentParams,
    DidCloseTextDocumentParams,
    DidOpenTextDocumentParams,
    InitializedParams,
    InitializeParams,
    InitializeResult,
    MessageType,
    Position,
    Range,
    ServerCapabilities,
    TextDocumentContentChangeEvent,
    TextDocumentSyncCapability,
    TextDocumentSyncKind,
    Url,
};

use ropey::Rope;

pub mod lsp {
    tonic::include_proto!("lsp");
}

use lsp as rnode_lsp;

#[derive(Debug)]
struct PendingChanges {
    version: i32,
    changes: Vec<TextDocumentContentChangeEvent>,
}

impl PartialEq for PendingChanges {
    fn eq(&self, other: &Self) -> bool {
        self.version == other.version
    }
}

impl Eq for PendingChanges {}

impl PartialOrd for PendingChanges {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(other.version.cmp(&self.version))
    }
}

impl Ord for PendingChanges {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other.version.cmp(&self.version)
    }
}

#[derive(Debug)]
struct LspDocumentState {
    uri: Url,
    text: Rope,
    version: i32,
    pending: BinaryHeap<PendingChanges>,
}

#[derive(Debug)]
struct LspDocument {
    id: u32,
    state: RwLock<LspDocumentState>,
}

fn lsp_range_to_offset(position: &Position, text: &Rope) -> usize {
    let line = position.line as usize;
    let char = position.character as usize;
    text.line_to_char(line) + char
}

impl LspDocumentState {
    fn apply(
        &mut self,
        mut changes: Vec<TextDocumentContentChangeEvent>,
        version: i32
    ) -> bool {
        // Re-order the changes so they will be applied in reverse in case there
        // are more than one:
        changes.sort_by(|a, b| {
            match (a.range, b.range) {
                (Some(range_a), Some(range_b)) => {
                    let pos_a = range_a.start;
                    let pos_b = range_b.start;
                    pos_b.line.cmp(&pos_a.line).then(pos_b.character.cmp(&pos_a.character))
                }
                // Handle cases where range is None
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, None) => std::cmp::Ordering::Equal,
            }
        });
        self.pending.push(PendingChanges { version, changes });
        while let Some(pending) = self.pending.peek() {
            if pending.version == self.version + 1 {
                for change in &pending.changes {
                    if let Some(range) = change.range {
                        let start = lsp_range_to_offset(&range.start, &self.text);
                        let end = lsp_range_to_offset(&range.end, &self.text);
                        self.text.remove(start..end);
                        self.text.insert(start, &change.text);
                    } else {
                        self.text = Rope::from_str(&change.text);
                    }
                }
                self.version = pending.version;
                self.pending.pop(); // Remove the processed change
            } else {
                // Next change is not sequential, wait for the correct version
                break;
            }
        }
        self.version == version
    }
}

#[allow(dead_code)]
impl LspDocument {
    async fn uri(&self) -> Url {
        self.state.read().await.uri.clone()
    }

    async fn text(&self) -> String {
        self.state.read().await.text.to_string()
    }

    async fn version(&self) -> i32 {
        self.state.read().await.version
    }

    async fn num_lines(&self) -> usize {
        self.state.read().await.text.len_lines()
    }

    async fn last_line(&self) -> usize {
        self.num_lines().await - 1
    }

    async fn num_columns(&self, line: usize) -> usize {
        self.state.read().await.text.line(line).len_chars()
    }

    async fn last_column(&self, line: usize) -> usize {
        self.state.read().await.text.line(line).len_chars() - 1
    }

    async fn last_linecol(&self) -> (usize, usize) {
        let state = self.state.read().await;
        let text = &state.text;
        let last_line = text.len_lines() - 1;
        let last_column = text.line(last_line).len_chars() - 1;
        (last_line, last_column)
    }

    async fn apply(
        &self,
        changes: Vec<TextDocumentContentChangeEvent>,
        version: i32
    ) -> Option<String> {
        let mut state = self.state.write().await;
        if state.apply(changes, version) {
            Some(state.text.to_string())
        } else {
            None
        }
    }
}

#[derive(Debug)]
struct RholangBackend {
    client: Client,
    documents_by_uri: RwLock<HashMap<Url, Arc<LspDocument>>>,
    documents_by_id: RwLock<HashMap<u32, Arc<LspDocument>>>,
    serial_document_id: AtomicU32,
    rnode_client: Mutex<lsp::lsp_client::LspClient<Channel>>,
}

impl RholangBackend {
    fn next_document_id(&self) -> u32 {
        self.serial_document_id.fetch_add(1, Ordering::SeqCst)
    }

    async fn validate(&self, document: Arc<LspDocument>, text: &str, version: i32) -> Result<Vec<Diagnostic>, String> {
        let mut client = self.rnode_client.lock().await.clone();
        if document.version().await == version {
            let request = Request::new(rnode_lsp::ValidateRequest {
                text: text.to_string(),
            });
            match client.validate(request).await {
                Ok(response) => match response.into_inner().result {
                    Some(result) => match result {
                        rnode_lsp::validate_response::Result::Success(diagnostic_list) => {
                            Ok(diagnostic_list.diagnostics.into_iter().map(|diagnostic| {
                                let range = diagnostic.range.expect("Missing required field for lsp::Diagnostic: range");
                                let start = range.start.expect("Missing required field for lsp::Range: start");
                                let end = range.end.expect("Missing required field for lsp::Range: end");
                                let severity = match rnode_lsp::DiagnosticSeverity::try_from(diagnostic.severity) {
                                    Ok(severity) => match severity {
                                        rnode_lsp::DiagnosticSeverity::Error => DiagnosticSeverity::ERROR,
                                        rnode_lsp::DiagnosticSeverity::Warning => DiagnosticSeverity::WARNING,
                                        rnode_lsp::DiagnosticSeverity::Information => DiagnosticSeverity::INFORMATION,
                                        rnode_lsp::DiagnosticSeverity::Hint => DiagnosticSeverity::HINT,
                                    }
                                    Err(e) => {
                                        error!("Invalid lsp::DiagnosticSeverity: {}", e);
                                        DiagnosticSeverity::ERROR
                                    }
                                };
                                Diagnostic {
                                    range: Range {
                                        start: Position {
                                            line: start.line as u32,
                                            character: start.column as u32
                                        },
                                        end: Position {
                                            line: end.line as u32,
                                            character: end.column as u32
                                        }
                                    },
                                    severity: Some(severity),
                                    source: Some(diagnostic.source),
                                    message: diagnostic.message,
                                    ..Default::default()
                                }
                            }).collect())
                        }
                        rnode_lsp::validate_response::Result::Error(message) => Err(format!(
                            "Failed to validate document with ID={}, URI={}: {}",
                            document.id, document.uri().await, message
                        ))
                    }
                    None => Err("RNode did not return a response".to_string()),
                }
                Err(e) => Err(format!(
                    "Failed to validate document with ID={}, URI={}: {}",
                    document.id, document.uri().await, e
                ))
            }
        } else {
            Ok(Vec::new())
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for RholangBackend {
    async fn initialize(&self, _: InitializeParams) -> jsonrpc::Result<InitializeResult> {
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

    async fn initialized(&self, _: InitializedParams) {
        self.client.log_message(
            MessageType::INFO,
            "Server initialized!"
        ).await;
    }

    async fn shutdown(&self) -> jsonrpc::Result<()> {
        self.client.log_message(
            MessageType::INFO,
            "Shutting server down."
        ).await;
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let text = &params.text_document.text;
        let version = params.text_document.version;
        let document_id = self.next_document_id();
        let document = Arc::new(LspDocument {
            id: document_id,
            state: RwLock::new(LspDocumentState {
                uri: uri.clone(),
                text: Rope::from_str(text),
                version,
                pending: BinaryHeap::new()
            })
        });
        self.documents_by_uri.write().await.insert(
            uri.clone(),
            Arc::clone(&document)
        );
        self.documents_by_id.write().await.insert(
            document_id,
            Arc::clone(&document)
        );
        self.client.log_message(
            MessageType::INFO,
            format!(
                "Opened document: {}, id: {}, version: {}",
                uri, document_id, version
            ),
        ).await;
        match self.validate(Arc::clone(&document), &text, version).await {
            Ok(diagnostics) => if document.state.read().await.version == version {
                self.client.publish_diagnostics(uri, diagnostics, Some(version)).await;
            }
            Err(e) => error!(
                "Failed to validate document with ID={}, URI={}: {}",
                document.id, document.uri().await, e
            )
        }
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let version = params.text_document.version;
        if let Some(document) = self.documents_by_uri.read().await.get(&uri) {
            if let Some(text) = document.apply(params.content_changes, version).await {
                match self.validate(Arc::clone(&document), &text, version).await {
                    Ok(diagnostics) => if document.state.read().await.version == version {
                        self.client.publish_diagnostics(uri, diagnostics, Some(version)).await;
                    }
                    Err(e) => error!(
                        "Failed to validate document with ID={}, URI={}: {}",
                        document.id, document.uri().await, e
                    )
                }
            }
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        if let Some(document) = self.documents_by_uri.write().await.remove(&uri) {
            self.documents_by_id.write().await.remove(&document.id);
            self.client
                .log_message(
                    MessageType::INFO,
                    format!("Closed document: {}, id: {}", uri, document.id),
                )
                .await;
        } else {
            self.client
                .log_message(
                    MessageType::WARNING,
                    format!("Closed document not found: {}", uri),
                )
                .await;
        }
        self.client.publish_diagnostics(uri, Vec::new(), None).await;
    }
}

fn init_logger() {
    let timer = fmt::time::OffsetTime::new(
        UtcOffset::UTC,
        format_description!("[[[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z]"),
    );
    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_writer(std::io::stderr)
                .with_timer(timer)
        )
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("debug"))
        )
        .init();
}

#[tokio::main]
async fn main() -> () {
    init_logger();

    // Connect to rnode gRPC service
    let rnode_client = match lsp::lsp_client::LspClient::connect("http://localhost:40402").await {
        Ok(client) => client,
        Err(e) => {
            error!("Failed to connect to rnode: {}", e);
            return;
        }
    };

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| RholangBackend {
        client,
        documents_by_uri: RwLock::new(HashMap::new()),
        documents_by_id: RwLock::new(HashMap::new()),
        serial_document_id: AtomicU32::new(0),
        rnode_client: Mutex::new(rnode_client),
    });

    Server::new(stdin, stdout, socket)
        .serve(service)
        .await;
}
