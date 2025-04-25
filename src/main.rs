use std::collections::{BinaryHeap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use lazy_static::lazy_static;

use regex::Regex;

use tokio::sync::{Mutex, RwLock};

use tonic::Request;
use tonic::transport::Channel;

use tower_lsp::{Client, LanguageServer, LspService, Server};
use tower_lsp::jsonrpc::Result;
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

use repl::ReplResponse;

pub mod repl {
    tonic::include_proto!("repl");
}

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

fn parse_u32(capture: &str) -> u32 {
    capture.parse().expect("Failed to parse u32")
}

lazy_static! {
    static ref RE_SUCCESS: Regex = Regex::new(
        r"^Deployment cost: .+"
    ).expect("Invalid regex");
    static ref RE_ERROR_RANGE: Regex = Regex::new(
        r"^.+ at (\d+):(\d+)-(\d+):(\d+)\.?$"
    ).expect("Invalid regex");
    static ref RE_ERROR_CONTEXT: Regex = Regex::new(
        r"^.+ (\w+) at \d+:\d+ used in \w+ context at (\d+):(\d+)$"
    ).expect("Invalid regex");
    static ref RE_ERROR_UNBOUND: Regex = Regex::new(
        r"^Variable reference: =(\S+) at (\d+):(\d+) is unbound\.$"
    ).expect("Invalid regex");
    static ref RE_ERROR_TWICE_BOUND: Regex = Regex::new(
        r"^Free variable (\S+) is used twice as a binder \(at (\d+):(\d+) and (\d+):(\d+)\) in \S+ context\.$"
    ).expect("Invalid regex");
    static ref RE_ERROR_POSITION_AT_END: Regex = Regex::new(
        r"^.+ at (\d+):(\d+)\.?$"
    ).expect("Invalid regex");
    static ref RE_ERROR_POSITION_IN_MIDDLE: Regex = Regex::new(
        r"^.+ at (\d+):(\d+) .+$"
    ).expect("Invalid regex");
}

#[derive(Debug)]
struct RholangBackend {
    client: Client,
    documents_by_uri: RwLock<HashMap<Url, Arc<LspDocument>>>,
    documents_by_id: RwLock<HashMap<u32, Arc<LspDocument>>>,
    serial_document_id: AtomicU32,
    rnode_client: Mutex<repl::repl_client::ReplClient<Channel>>,
}

impl RholangBackend {
    fn next_document_id(&self) -> u32 {
        self.serial_document_id.fetch_add(1, Ordering::SeqCst)
    }

    async fn validate(&self, document: Arc<LspDocument>, text: &str, version: i32) -> Option<Vec<Diagnostic>> {
        let mut client = self.rnode_client.lock().await.clone();
        if document.version().await == version {
            let request = Request::new(repl::EvalRequest {
                program: text.to_string(),
                print_unmatched_sends_only: false,
            });
            let response: ReplResponse = match client.eval(request).await {
                Ok(response) => response.into_inner(),
                Err(e) => {
                    eprintln!("Failed to evaluate Rholang: {}", e);
                    return None;
                }
            };
            let message = &response.output;
            if let Some(_captures) = RE_SUCCESS.captures(message) {
                return Some(vec![]);
            } else if let Some(captures) = RE_ERROR_RANGE.captures(message) {
                let start_line: u32 = parse_u32(&captures[1]);
                let start_column: u32 = parse_u32(&captures[2]);
                let end_line: u32 = parse_u32(&captures[3]);
                let end_column: u32 = parse_u32(&captures[4]);
                return Some(vec![Diagnostic {
                    range: Range {
                        start: Position {
                            line: start_line - 1,
                            character: start_column - 1,
                        },
                        end: Position {
                            line: end_line - 1,
                            character: end_column - 1,
                        },
                    },
                    severity: Some(DiagnosticSeverity::ERROR),
                    message: captures[0].to_string(),
                    source: Some("rholang".to_string()),
                    ..Default::default()
                }]);
            } else if let Some(captures) = RE_ERROR_CONTEXT.captures(message) {
                let variable_name = &captures[1];
                let start_line: u32 = parse_u32(&captures[2]);
                let start_column: u32 = parse_u32(&captures[3]);
                let end_line: u32 = start_line;
                let end_column: u32 = start_column + variable_name.chars().count() as u32;
                return Some(vec![Diagnostic {
                    range: Range {
                        start: Position {
                            line: start_line - 1,
                            character: start_column - 1,
                        },
                        end: Position {
                            line: end_line - 1,
                            character: end_column - 1,
                        },
                    },
                    severity: Some(DiagnosticSeverity::ERROR),
                    message: captures[0].to_string(),
                    source: Some("rholang".to_string()),
                    ..Default::default()
                }]);
            } else if let Some(captures) = RE_ERROR_UNBOUND.captures(message) {
                let variable_name = &captures[1];
                let start_line: u32 = parse_u32(&captures[2]);
                let start_column: u32 = parse_u32(&captures[3]);
                let end_line = start_line;
                let end_column = start_column + variable_name.chars().count() as u32;
                return Some(vec![Diagnostic {
                    range: Range {
                        start: Position {
                            line: start_line - 1,
                            character: start_column - 1,
                        },
                        end: Position {
                            line: end_line - 1,
                            character: end_column - 1,
                        },
                    },
                    severity: Some(DiagnosticSeverity::ERROR),
                    message: captures[0].to_string(),
                    source: Some("rholang".to_string()),
                    ..Default::default()
                }]);
            } else if let Some(captures) = RE_ERROR_TWICE_BOUND.captures(message) {
                let variable_name = &captures[1];
                let len: u32 = variable_name.chars().count() as u32;

                let start_line_1: u32 = parse_u32(&captures[2]);
                let start_column_1: u32 = parse_u32(&captures[3]);
                let end_line_1 = start_line_1;
                let end_column_1 = start_column_1 + len;

                let start_line_2: u32 = parse_u32(&captures[4]);
                let start_column_2: u32 = parse_u32(&captures[5]);
                let end_line_2 = start_line_2;
                let end_column_2 = start_column_2 + len;

                return Some(vec![
                    Diagnostic {
                        range: Range {
                            start: Position {
                                line: start_line_1 - 1,
                                character: start_column_1 - 1,
                            },
                            end: Position {
                                line: end_line_1 - 1,
                                character: end_column_1 - 1,
                            },
                        },
                        severity: Some(DiagnosticSeverity::ERROR),
                        message: captures[0].to_string(),
                        source: Some("rholang".to_string()),
                        ..Default::default()
                    },
                    Diagnostic {
                        range: Range {
                            start: Position {
                                line: start_line_2 - 1,
                                character: start_column_2 - 1,
                            },
                            end: Position {
                                line: end_line_2 - 1,
                                character: end_column_2 - 1,
                            },
                        },
                        severity: Some(DiagnosticSeverity::ERROR),
                        message: captures[0].to_string(),
                        source: Some("rholang".to_string()),
                        ..Default::default()
                    },
                ]);
            } else if let Some(captures) = RE_ERROR_POSITION_AT_END.captures(message) {
                let start_line: u32 = parse_u32(&captures[1]);
                let start_column: u32 = parse_u32(&captures[2]);
                let end_line = start_line;
                let end_column = start_column;
                return Some(vec![Diagnostic {
                    range: Range {
                        start: Position {
                            line: start_line - 1,
                            character: start_column - 1,
                        },
                        end: Position {
                            line: end_line - 1,
                            character: end_column - 1,
                        },
                    },
                    severity: Some(DiagnosticSeverity::ERROR),
                    message: captures[0].to_string(),
                    source: Some("rholang".to_string()),
                    ..Default::default()
                }]);
            } else if let Some(captures) = RE_ERROR_POSITION_IN_MIDDLE.captures(message) {
                let start_line: u32 = parse_u32(&captures[1]);
                let start_column: u32 = parse_u32(&captures[2]);
                let end_line = start_line;
                let end_column = start_column;
                return Some(vec![Diagnostic {
                    range: Range {
                        start: Position {
                            line: start_line - 1,
                            character: start_column - 1,
                        },
                        end: Position {
                            line: end_line - 1,
                            character: end_column - 1,
                        },
                    },
                    severity: Some(DiagnosticSeverity::ERROR),
                    message: captures[0].to_string(),
                    source: Some("rholang".to_string()),
                    ..Default::default()
                }]);
            } else {
                let start_line: u32 = 0;
                let start_column: u32 = 0;
                let (end_line, end_column) = document.last_linecol().await;
                return Some(vec![Diagnostic {
                    range: Range {
                        start: Position {
                            line: start_line,
                            character: start_column,
                        },
                        end: Position {
                            line: end_line as u32,
                            character: end_column as u32,
                        },
                    },
                    severity: Some(DiagnosticSeverity::ERROR),
                    message: message.to_string(),
                    source: Some("rholang".to_string()),
                    ..Default::default()
                }]);
            }
        }
        None
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for RholangBackend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
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

    async fn shutdown(&self) -> Result<()> {
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
        if let Some(diagnostics) = self.validate(Arc::clone(&document), &text, version).await {
            if document.state.read().await.version == version {
                self.client.publish_diagnostics(
                    uri,
                    diagnostics,
                    Some(version)
                ).await;
            }
        }
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let version = params.text_document.version;
        if let Some(document) = self.documents_by_uri.read().await.get(&uri) {
            if let Some(text) = document.apply(params.content_changes, version).await {
                if let Some(diagnostics) = self.validate(Arc::clone(&document), &text, version).await {
                    if document.version().await == version {
                        self.client.publish_diagnostics(uri, diagnostics, Some(version)).await;
                    }
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

#[tokio::main]
async fn main() -> () {
    // Connect to rnode gRPC service
    let rnode_client = match repl::repl_client::ReplClient::connect("http://localhost:40402").await {
        Ok(client) => client,
        Err(e) => {
            eprintln!("Failed to connect to rnode: {}", e);
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
