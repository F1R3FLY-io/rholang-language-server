use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use lazy_static::lazy_static;

use regex::{Regex, Captures};

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

use repl::{EvalRequest, ReplResponse};
use repl::repl_client::ReplClient;

pub mod repl {
    tonic::include_proto!("repl");
}

struct PatternMatcher {
    regex: Regex,
    description: &'static str,
    callback: Box<dyn Fn(&Captures) -> Vec<Diagnostic> + Send + Sync + 'static>,
}

impl PatternMatcher {
    fn new(
        pattern: &str,
        description: &'static str,
        callback: impl Fn(&Captures) -> Vec<Diagnostic> + Send + Sync + 'static,
    ) -> Option<Self> {
        Regex::new(pattern).ok().map(|regex| PatternMatcher {
            regex,
            description,
            callback: Box::new(callback),
        })
    }

    fn matches(&self, text: &str) -> Option<Vec<Diagnostic>> {
        self.regex.captures(text).map(|captures| (self.callback)(&captures))
    }
}

lazy_static! {
    static ref MATCHERS: Vec<PatternMatcher> = vec![
        PatternMatcher::new(
            r"^Deployment cost: .+",
            "success",
            |_captures: &Captures| {
                vec![]  //-> no errors!
            },
        ).expect("failed to create PatternMatcher"),
        PatternMatcher::new(
            r"^.+ at (\d+):(\d+)-(\d+):(\d+)\.?$",
            "error at position range",
            |captures: &Captures| {
                let start_line: u32 =
                    captures[1].parse().expect("Failed to parse start_line");
                let start_column: u32 =
                    captures[2].parse().expect("Failed to parse start_column");
                let end_line: u32 =
                    captures[3].parse().expect("Failed to parse end_line");
                let end_column: u32 =
                    captures[4].parse().expect("Failed to parse end_column");

                vec![Diagnostic {
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
                }]
            },
        ).expect("failed to create PatternMatcher"),
        PatternMatcher::new(
            r"^.+ (\d+):(\d+) used in \w+ context at (\d+):(\d+)$",
            "position used in context at position",
            |captures: &Captures| {
                let start_line: u32 =
                    captures[1].parse().expect("Failed to parse start_line");
                let start_column: u32 =
                    captures[2].parse().expect("Failed to parse start_column");
                let end_line: u32 =
                    captures[3].parse().expect("Failed to parse end_line");
                let end_column: u32 =
                    captures[4].parse().expect("Failed to parse end_column");

                vec![Diagnostic {
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
                }]
            },
        ).expect("failed to create PatternMatcher"),
        PatternMatcher::new(
            r"^Variable reference: =(\S+) at (\d+):(\d+) is unbound\.$",
            "unbound variable at position",
            |captures: &Captures| {
                let variable_name = &captures[1];
                let start_line: u32 =
                    captures[2].parse().expect("Failed to parse start_line");
                let start_column: u32 =
                    captures[3].parse().expect("Failed to parse start_column");
                let end_line = start_line;
                let end_column = start_column + variable_name.chars().count() as u32;

                vec![Diagnostic {
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
                }]
            },
        ).expect("failed to create PatternMatcher"),
        PatternMatcher::new(
            r"^Free variable (\S+) is used twice as a binder \(at (\d+):(\d+) and (\d+):(\d+)\) in \S+ context\.$",
            "free variable used twice",
            |captures: &Captures| {
                let variable_name = &captures[1];
                let start_line_1: u32 =
                    captures[2].parse().expect("Failed to parse start_line");
                let start_column_1: u32 =
                    captures[3].parse().expect("Failed to parse start_column");
                let end_line_1 = start_line_1;
                let end_column_1 = start_column_1 + variable_name.chars().count() as u32;
                let start_line_2: u32 =
                    captures[4].parse().expect("Failed to parse start_line");
                let start_column_2: u32 =
                    captures[5].parse().expect("Failed to parse start_column");
                let end_line_2 = start_line_2;
                let end_column_2 = start_column_2 + variable_name.chars().count() as u32;

                vec![
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
                ]
            },
        ).expect("failed to create PatternMatcher"),
        PatternMatcher::new(
            r"^.+ at (\d+):(\d+)\.?$",
            "single position at end",
            |captures: &Captures| {
                let start_line: u32 =
                    captures[1].parse().expect("Failed to parse start_line");
                let start_column: u32 =
                    captures[2].parse().expect("Failed to parse start_column");
                let end_line = start_line;
                let end_column = start_column;

                vec![Diagnostic {
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
                }]
            },
        ).expect("failed to create PatternMatcher"),
        PatternMatcher::new(
            r"^.+ at (\d+):(\d+) .+$",
            "single position in middle",
            |captures: &Captures| {
                let start_line: u32 =
                    captures[1].parse().expect("Failed to parse start_line");
                let start_column: u32 =
                    captures[2].parse().expect("Failed to parse start_column");
                let end_line = start_line;
                let end_column = start_column;

                vec![Diagnostic {
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
                }]
            },
        ).expect("failed to create PatternMatcher"),
        PatternMatcher::new(
            r".*",
            "anything else",
            |captures: &Captures| {
                let start_line: u32 = 1;
                let start_column: u32 = 1;
                // TODO: Use the last line and column, here
                let end_line: u32 = 1;
                let end_column = 1;

                vec![Diagnostic {
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
                }]
            },
        ).expect("failed to create PatternMatcher"),
    ];
}

// Struct to hold document text, version, and ID
#[derive(Debug, Clone)]
struct LspDocument {
    // Unique identifier for the document, immutable after construction
    id: u32,
    text: String,
    version: i32,
}

impl LspDocument {
    // Updates the document's text based on content changes and returns a new LspDocument
    fn update_text(&self, content_changes: Vec<TextDocumentContentChangeEvent>, new_version: i32) -> LspDocument {
        let mut text = self.text.clone();

        for change in content_changes {
            if let Some(range) = change.range {
                let start_pos = position_to_offset(&text, range.start);
                let end_pos = position_to_offset(&text, range.end);
                if start_pos <= end_pos && end_pos <= text.len() {
                    text.replace_range(start_pos..end_pos, &change.text);
                }
            } else {
                // Full update if no range is provided
                text = change.text;
            }
        }

        LspDocument {
            id: self.id,
            text,
            version: new_version,
        }
    }
}

#[derive(Debug)]
struct Backend {
    client: Client,
    documents_by_uri: RwLock<HashMap<Url, Arc<LspDocument>>>,
    documents_by_id: RwLock<HashMap<u32, Arc<LspDocument>>>,
    serial_document_id: AtomicU32,
    rnode_client: Mutex<repl::repl_client::ReplClient<Channel>>,
}

impl Backend {
    fn next_document_id(&self) -> u32 {
        self.serial_document_id.fetch_add(1, Ordering::SeqCst)
    }

    async fn validate(&self, uri: &Url, text: &str, version: i32) -> Option<Vec<Diagnostic>> {
        // Send EvalRequest to rnode
        let request = Request::new(repl::EvalRequest {
            program: text.to_string(),
            print_unmatched_sends_only: false,
        });

        let mut client = self.rnode_client.lock().await.clone();

        if self
            .documents_by_uri
            .read()
            .await
            .get(&uri)
            .map(|doc| doc.version == version)
            .unwrap_or(false)
        {
            let response: ReplResponse = match client.eval(request).await {
                Ok(response) => response.into_inner(),
                Err(e) => {
                    eprintln!("Failed to evaluate Rholang: {}", e);
                    return None;
                }
            };

            for matcher in MATCHERS.iter() {
                let message = &response.output;
                if let Some(diagnostics) = matcher.matches(&message) {
                    return Some(diagnostics);
                }
            }
        }

        None
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
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
        self.client
            .log_message(MessageType::INFO, "Server initialized!")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let text = params.text_document.text;
        let version = params.text_document.version;
        let id = self.next_document_id();

        // Create and store LspDocument
        let document = Arc::new(LspDocument {
            id,
            text: text.clone(),
            version,
        });
        {
            let mut documents_by_uri = self.documents_by_uri.write().await;
            documents_by_uri.insert(uri.clone(), Arc::clone(&document));
        }
        {
            let mut documents_by_id = self.documents_by_id.write().await;
            documents_by_id.insert(id, Arc::clone(&document));
        }

        self.client
            .log_message(
                MessageType::INFO,
                format!("Opened document: {}, id: {}, version: {}", uri, id, version),
            )
            .await;

        // Validate Rholang and publish diagnostics if version matches
        if let Some(diagnostics) = self.validate(&uri, &text, version).await {
            if self
                .documents_by_uri
                .read()
                .await
                .get(&uri)
                .map(|doc| doc.version == version)
                .unwrap_or(false)
            {
                self.client
                    .publish_diagnostics(uri, diagnostics, Some(version))
                    .await;
            }
        }
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let version = params.text_document.version;

        // Get existing document or create new one
        let document = self.documents_by_uri
            .read()
            .await
            .get(&uri)
            .cloned()
            .unwrap_or(Arc::new(LspDocument {
                id: self.next_document_id(),
                text: String::new(),
                version: 0,
            }));

        // Update document text using instance method
        let updated_document = Arc::new(document.update_text(params.content_changes, version));

        // Store updated document in both maps
        let id = updated_document.id;
        {
            let mut documents_by_uri = self.documents_by_uri.write().await;
            documents_by_uri.insert(uri.clone(), Arc::clone(&updated_document));
        }
        {
            let mut documents_by_id = self.documents_by_id.write().await;
            documents_by_id.insert(id, Arc::clone(&updated_document));
        }

        self.client
            .log_message(
                MessageType::INFO,
                format!("Updated document: {}, id: {}, version: {}", uri, id, version),
            )
            .await;

        // Validate Rholang and publish diagnostics if version matches
        if let Some(diagnostics) = self.validate(&uri, &updated_document.text, version).await {
            if self
                .documents_by_uri
                .read()
                .await
                .get(&uri)
                .map(|doc| doc.version == version)
                .unwrap_or(false)
            {
                self.client
                    .publish_diagnostics(uri, diagnostics, Some(version))
                    .await;
            }
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;

        // Remove document from documents_by_uri map and get its id
        let document = {
            let mut documents_by_uri = self.documents_by_uri.write().await;
            documents_by_uri.remove(&uri)
        };

        if let Some(document) = document {
            let id = document.id;
            {
                let mut documents_by_id = self.documents_by_id.write().await;
                documents_by_id.remove(&id);
            }

            self.client
                .log_message(
                    MessageType::INFO,
                    format!("Closed document: {}, id: {}", uri, id),
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

        // Clear diagnostics for the closed document
        self.client
            .publish_diagnostics(uri, Vec::new(), None)
            .await;
    }
}

// Helper function to convert LSP Position to byte offset in the text
fn position_to_offset(text: &str, position: Position) -> usize {
    let mut offset = 0;
    let mut line = 0;
    let mut char_pos = 0;

    for c in text.chars() {
        if line == position.line {
            if char_pos == position.character {
                return offset;
            }
            char_pos += 1;
        }
        if c == '\n' {
            line += 1;
            char_pos = 0;
        }
        offset += c.len_utf8();
    }

    // If position is beyond the text, return the end
    offset
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

    let (service, socket) = LspService::new(|client| Backend {
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
