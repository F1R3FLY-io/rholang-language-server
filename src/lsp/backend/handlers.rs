//! LSP protocol handler implementations
//!
//! This module contains all `tower_lsp::LanguageServer` trait implementations
//! for the Rholang backend, including:
//! - Lifecycle handlers (initialize, initialized, shutdown)
//! - Document lifecycle (did_open, did_change, did_save, did_close)
//! - Navigation handlers (goto_definition, goto_declaration, references)
//! - Symbol operations (rename, document_symbol, symbol, document_highlight)
//! - Information providers (hover, semantic_tokens_full)

use std::collections::HashMap;
use std::sync::Arc;

use tower_lsp::{LanguageServer, jsonrpc};
use tower_lsp::lsp_types::{
    DeclarationCapability, Diagnostic, DiagnosticSeverity,
    DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DidSaveTextDocumentParams, DocumentHighlight,
    DocumentHighlightKind, DocumentHighlightParams, GotoDefinitionParams,
    GotoDefinitionResponse, InitializedParams, InitializeParams,
    InitializeResult, Location, Position as LspPosition, Range, ReferenceParams,
    RenameParams, ServerCapabilities, TextDocumentSyncCapability,
    TextDocumentSyncKind, TextEdit, Url, WorkspaceEdit, DocumentSymbolParams,
    DocumentSymbolResponse, WorkspaceSymbolParams, WorkspaceSymbol,
    SymbolInformation, Hover, HoverContents, HoverParams, MarkupContent, MarkupKind,
    SemanticTokensParams, SemanticTokensResult, SemanticTokensLegend,
    SemanticTokenType, SemanticTokensFullOptions, SemanticTokensServerCapabilities,
    SemanticTokensOptions, SignatureHelp, SignatureHelpParams, SignatureInformation,
    ParameterInformation, ParameterLabel, SignatureHelpOptions, CompletionParams,
    CompletionResponse, CompletionItem, CompletionItemKind, CompletionOptions,
    CompletionOptionsCompletionItem,
};
use tower_lsp::lsp_types::request::{GotoDeclarationParams, GotoDeclarationResponse};
use tower_lsp::jsonrpc::Result as LspResult;

use tracing::{debug, error, info, trace, warn};

use ropey::Rope;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use walkdir::WalkDir;

use crate::ir::rholang_node::{RholangNode, Position as IrPosition, find_node_at_position_with_path, find_node_at_position, compute_absolute_positions};
use crate::ir::symbol_table::SymbolType;
use crate::ir::transforms::document_symbol_visitor::collect_document_symbols;

use super::state::RholangBackend;
use super::state::{DocumentChangeEvent, IndexingTask};
use super::utils::SemanticTokensBuilder;
use crate::lsp::models::{LspDocument, LspDocumentHistory, LspDocumentState};

#[tower_lsp::async_trait]
impl LanguageServer for RholangBackend {
    /// Handles the LSP initialize request, setting up capabilities and indexing workspace files.
    async fn initialize(&self, params: InitializeParams) -> jsonrpc::Result<InitializeResult> {
        info!("Received initialize request");
        debug!("Initialize params: {:?}", params);

        if let Some(client_pid) = params.process_id {
            {
                let mut locked_pid = self.client_process_id.lock().await;
                if let Some(cmdline_pid) = *locked_pid {
                    if cmdline_pid != client_pid {
                        warn!("Client PID mismatch: command line ({}) vs LSP ({})", cmdline_pid, client_pid);
                    }
                }
                *locked_pid = Some(client_pid);
            } // Drop the lock here before next await

            // Send PID through reactive channel to trigger monitoring
            if let Some(ref tx) = self.pid_channel {
                if let Err(e) = tx.send(client_pid).await {
                    warn!("Failed to send client PID through channel: {}", e);
                } else {
                    info!("Sent client PID {} for monitoring", client_pid);
                }
            }
        }

        let mut root_guard = self.root_dir.write().await;
        if let Some(root_uri) = params.root_uri {
            if let Ok(root_path) = root_uri.to_file_path() {
                *root_guard = Some(root_path.clone());
                drop(root_guard);

                // Phase 2 optimization: Count files first, then set indexing state before queuing
                let file_paths: Vec<_> = WalkDir::new(&root_path)
                    .into_iter()
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().extension().map_or(false, |ext| ext == "rho"))
                    .collect();

                let file_count = file_paths.len();

                if file_count > 0 {
                    // Set indexing state to InProgress before queuing tasks
                    {
                        let mut state = self.workspace.indexing_state.write().await;
                        *state = crate::lsp::models::IndexingState::InProgress {
                            total: file_count,
                            completed: 0,
                        };
                    }

                    // Send initial progress notification
                    self.client.send_notification::<tower_lsp::lsp_types::notification::Progress>(
                        tower_lsp::lsp_types::ProgressParams {
                            token: tower_lsp::lsp_types::NumberOrString::String("workspace-indexing".to_string()),
                            value: tower_lsp::lsp_types::ProgressParamsValue::WorkDone(
                                tower_lsp::lsp_types::WorkDoneProgress::Begin(
                                    tower_lsp::lsp_types::WorkDoneProgressBegin {
                                        title: "Indexing workspace".to_string(),
                                        message: Some(format!("Found {} files", file_count)),
                                        percentage: Some(0),
                                        cancellable: Some(false),
                                    }
                                )
                            ),
                        }
                    ).await;

                    // Queue all .rho files for progressive indexing
                    let mut queued_count = 0;
                    for entry in file_paths {
                        let uri = Url::from_file_path(entry.path()).unwrap();
                        let text = std::fs::read_to_string(entry.path()).unwrap_or_default();

                        // All files get priority 1 during initialization
                        // Files will be prioritized to 0 when opened via did_open
                        let task = IndexingTask {
                            uri: uri.clone(),
                            text,
                            priority: 1,
                        };

                        if let Err(e) = self.indexing_tx.send(task).await {
                            error!("Failed to queue indexing task for {}: {}", uri, e);
                        } else {
                            queued_count += 1;
                        }
                    }
                    info!("Queued {} .rho files for progressive indexing", queued_count);
                } else {
                    info!("No .rho files found in workspace");
                }

                let tx = self.file_sender.lock().unwrap().clone();
                let mut watcher = RecommendedWatcher::new(
                    move |res| { let _ = tx.send(res); },
                    notify::Config::default()
                ).map_err(|_| jsonrpc::Error::internal_error())?;
                watcher.watch(&root_path, RecursiveMode::Recursive).map_err(|_| jsonrpc::Error::internal_error())?;
                *self.file_watcher.lock().unwrap() = Some(watcher);

                // Spawn reactive file watcher event batcher
                Self::spawn_reactive_file_watcher(self.clone(), self.file_events.clone());
            } else {
                warn!("Failed to convert root_uri to path: {}. Skipping workspace indexing and file watching.", root_uri);
            }
        }

        // Define semantic token legend
        let token_types = vec![
            SemanticTokenType::COMMENT,
            SemanticTokenType::STRING,
            SemanticTokenType::NUMBER,
            SemanticTokenType::KEYWORD,
            SemanticTokenType::OPERATOR,
            SemanticTokenType::VARIABLE,
            SemanticTokenType::FUNCTION,
            SemanticTokenType::TYPE,
        ];

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::INCREMENTAL)),
                rename_provider: Some(tower_lsp::lsp_types::OneOf::Left(true)),
                declaration_provider: Some(DeclarationCapability::Simple(true)),
                definition_provider: Some(tower_lsp::lsp_types::OneOf::Left(true)),
                references_provider: Some(tower_lsp::lsp_types::OneOf::Left(true)),
                document_symbol_provider: Some(tower_lsp::lsp_types::OneOf::Left(true)),
                workspace_symbol_provider: Some(tower_lsp::lsp_types::OneOf::Left(true)),
                document_highlight_provider: Some(tower_lsp::lsp_types::OneOf::Left(true)),
                hover_provider: Some(tower_lsp::lsp_types::HoverProviderCapability::Simple(true)),
                signature_help_provider: Some(SignatureHelpOptions {
                    trigger_characters: Some(vec!["!".to_string(), "(".to_string(), ",".to_string()]),
                    retrigger_characters: None,
                    work_done_progress_options: Default::default(),
                }),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![".".to_string(), "@".to_string()]),
                    all_commit_characters: None,
                    resolve_provider: Some(false),
                    completion_item: Some(CompletionOptionsCompletionItem {
                        label_details_support: Some(true),
                    }),
                    work_done_progress_options: Default::default(),
                }),
                semantic_tokens_provider: Some(SemanticTokensServerCapabilities::SemanticTokensOptions(
                    SemanticTokensOptions {
                        legend: SemanticTokensLegend {
                            token_types,
                            token_modifiers: vec![],
                        },
                        full: Some(SemanticTokensFullOptions::Bool(true)),
                        range: None,
                        ..Default::default()
                    }
                )),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    /// Handles the LSP initialized notification.
    async fn initialized(&self, params: InitializedParams) {
        info!("Initialized");
        debug!("Initialized params: {:?}", params);
    }

    /// Handles the LSP shutdown request.
    async fn shutdown(&self) -> jsonrpc::Result<()> {
        info!("Received shutdown request");

        // Signal all background tasks to shut down gracefully
        let _ = self.shutdown_tx.send(());
        info!("Shutdown signal sent to all background tasks");

        Ok(())
    }

    /// Handles opening a text document, indexing it, and validating.
    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        info!("Opening document: URI={}, version={}", params.text_document.uri, params.text_document.version);
        let uri = params.text_document.uri.clone();
        let text = params.text_document.text;
        let version = params.text_document.version;

        let mut root_guard = self.root_dir.write().await;
        if root_guard.is_none() {
            if let Ok(path) = uri.to_file_path() {
                if let Some(parent) = path.parent() {
                    *root_guard = Some(parent.to_owned());
                    drop(root_guard);

                    let dir = parent.to_owned();
                    // Use parallel indexing for initial workspace scan (4-8x faster)
                    self.index_directory_parallel(&dir).await;

                    let tx = self.file_sender.lock().unwrap().clone();
                    let mut watcher = RecommendedWatcher::new(
                        move |res| { let _ = tx.send(res); },
                        notify::Config::default()
                    ).map_err(|_| jsonrpc::Error::internal_error()).expect("Failed to initialize watcher");
                    if let Err(e) = watcher.watch(parent, RecursiveMode::NonRecursive) {
                        warn!("Failed to watch directory {:?}: {}", parent, e);
                    }
                    *self.file_watcher.lock().unwrap() = Some(watcher);

                    // Spawn reactive file watcher event batcher
                    Self::spawn_reactive_file_watcher(self.clone(), self.file_events.clone());
                }
            }
        } else {
            drop(root_guard);
        }

        let document_id = self.next_document_id();
        let document = std::sync::Arc::new(LspDocument {
            id: document_id,
            state: tokio::sync::RwLock::new(LspDocumentState {
                uri: uri.clone(),
                text: {
                    let rope = Rope::from_str(&text);
                    debug!("Created rope from text with {} lines for URI {}", rope.len_lines(), uri);
                    debug!("Text: {:?}", &text);
                    rope
                },
                version,
                history: LspDocumentHistory {
                    text: text.clone(),
                    changes: Vec::new(),
                },
            }),
        });
        // DashMap provides lock-free concurrent access (Phase 3 optimization)
        self.documents_by_uri.insert(uri.clone(), document.clone());
        self.documents_by_id.insert(document_id, document.clone());

        // Index file and update workspace in a single batched write lock
        match self.index_file(&uri, &text, version, None).await {
            Ok(cached_doc) => {
                self.update_workspace_document(&uri, std::sync::Arc::new(cached_doc)).await;
                self.link_symbols().await;
            }
            Err(e) => error!("Failed to index file: {}", e),
        }

        // Spawn async validation task
        let backend = self.clone();
        let uri_clone = uri.clone();
        let document_clone = document.clone();
        let text_clone = text.clone();
        tokio::spawn(async move {
            match backend.validate(document_clone.clone(), &text_clone, version).await {
                Ok(diagnostics) => {
                    if document_clone.version().await == version {
                        backend.client.publish_diagnostics(uri_clone, diagnostics, Some(version)).await;
                    }
                }
                Err(e) => error!("Validation failed for URI={}: {}", uri_clone, e),
            }
        });
    }

    /// Handles changes to a text document, applying incremental updates and re-validating.
    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let version = params.text_document.version;
        info!("textDocument/didChange: uri={}, version={}", uri, version);
        debug!("didChange params: {:?}", params);
        // DashMap::get returns a guard that dereferences to the value
        if let Some(document) = self.documents_by_uri.get(&uri).map(|r| r.value().clone()) {
            if let Some((text, tree)) = document.apply(params.content_changes, version).await {
                match self.index_file(&uri, &text, version, Some(tree)).await {
                    Ok(cached_doc) => {
                        self.update_workspace_document(&uri, std::sync::Arc::new(cached_doc)).await;
                        self.link_symbols().await;
                    }
                    Err(e) => warn!("Failed to update {}: {}", uri, e),
                }

                // Send change event to debouncer instead of immediate validation
                let text_arc = std::sync::Arc::new(text.to_string());
                let event = DocumentChangeEvent {
                    uri: uri.clone(),
                    version,
                    document: document.clone(),
                    text: text_arc,
                };

                if let Err(e) = self.doc_change_tx.send(event).await {
                    error!("Failed to send document change event: {}", e);
                }
            } else {
                warn!("Failed to apply changes to document with URI={}", uri);
            }
        } else {
            warn!("Failed to find document with URI={}", uri);
        }
    }

    /// Handles saving a text document (no-op since validation is on change).
    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        info!("textDocument/didSave: uri={}", params.text_document.uri);
        debug!("didSave params: {:?}", params);
        // Validation occurs on open and change; no additional action needed here
    }

    /// Handles closing a text document, removing it from state and clearing diagnostics.
    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        info!("textDocument/didClose: uri={}", uri);
        debug!("didClose params: {:?}", params);
        // DashMap::remove returns Option<(K, V)>
        if let Some((_key, document)) = self.documents_by_uri.remove(&uri) {
            self.documents_by_id.remove(&document.id);
            info!("Closed document: {}, id: {}", uri, document.id);

            // Unregister any virtual documents associated with this parent
            let mut virtual_docs = self.virtual_docs.write().await;
            virtual_docs.unregister_parent(&uri);
        } else {
            warn!("Failed to find document with URI={}", uri);
        }
        self.client.publish_diagnostics(uri, Vec::new(), None).await;
    }

    /// Handles renaming a symbol, updating all references across the workspace.
    async fn rename(&self, params: RenameParams) -> LspResult<Option<WorkspaceEdit>> {
        debug!("rename request for {:?}", params);

        // Eagerly ensure symbols are linked before rename operation
        if self.needs_symbol_linking().await {
            debug!("Eagerly linking symbols for rename operation");
            self.link_symbols().await;
        }

        // Use unified handler (Phase 4c: replaces 70+ lines of language-specific logic)
        Ok(self.unified_rename(params).await)
    }
    async fn goto_definition(&self, params: GotoDefinitionParams) -> LspResult<Option<GotoDefinitionResponse>> {
        let start = std::time::Instant::now();
        let uri = &params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        debug!("goto_definition request for {} at {:?}", uri, position);

        // Use unified handler (Phase 4c: replaces 300+ lines of language-specific logic)
        let goto_result = self.unified_goto_definition(uri, position).await;

        // Log the result for debugging
        match &goto_result {
            Some(GotoDefinitionResponse::Scalar(loc)) => {
                debug!("goto_definition -> Location {{ uri: {}, range: {:?} }}", loc.uri, loc.range);
            }
            Some(GotoDefinitionResponse::Array(locs)) => {
                debug!("goto_definition -> {} locations", locs.len());
                for loc in locs {
                    debug!("  - Location {{ uri: {}, range: {:?} }}", loc.uri, loc.range);
                }
            }
            Some(GotoDefinitionResponse::Link(_)) => {
                debug!("goto_definition -> LocationLink (omitted from log)");
            }
            None => {
                debug!("goto_definition -> None");
            }
        }

        info!("goto_definition completed in {:.3}ms", start.elapsed().as_secs_f64() * 1000.0);
        Ok(goto_result)
    }

    /// Handles going to a symbol's declaration.
    async fn goto_declaration(&self, params: GotoDeclarationParams) -> LspResult<Option<GotoDeclarationResponse>> {
        let uri = params.text_document_position_params.text_document.uri.clone();
        let position = params.text_document_position_params.position;

        debug!("goto_declaration request for {} at {:?}", uri, position);

        // Eagerly ensure symbols are linked before goto-declaration operation
        if self.needs_symbol_linking().await {
            debug!("Eagerly linking symbols for goto-declaration operation");
            self.link_symbols().await;
        }

        if let Some(symbol) = self.get_symbol_at_position(&uri, position).await {
            let range = Self::position_to_range(symbol.declaration_location, symbol.name.len());
            let loc = Location { uri: symbol.declaration_uri.clone(), range };
            Ok(Some(GotoDeclarationResponse::Scalar(loc)))
        } else {
            Ok(None)
        }
    }

    /// Handles finding all references to a symbol.
    async fn references(&self, params: ReferenceParams) -> LspResult<Option<Vec<Location>>> {
        debug!("references request for {:?}", params);

        // Eagerly ensure symbols are linked before references operation
        if self.needs_symbol_linking().await {
            debug!("Eagerly linking symbols for references operation");
            self.link_symbols().await;
        }

        // Use unified handler (Phase 4c: replaces 180+ lines of language-specific logic)
        Ok(self.unified_references(params).await)
    }
    async fn document_symbol(&self, params: DocumentSymbolParams) -> LspResult<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri;
        debug!("Handling documentSymbol request for {}", uri);
        if let Some(doc) = self.workspace.documents.get(&uri) {
            use crate::lsp::models::DocumentLanguage;

            let symbols = match doc.language {
                DocumentLanguage::Metta => {
                    // Collect symbols from MeTTa IR
                    if let Some(metta_ir) = &doc.metta_ir {
                        use crate::ir::transforms::metta_symbol_collector::collect_metta_document_symbols;
                        collect_metta_document_symbols(metta_ir)
                    } else {
                        debug!("MeTTa document has no metta_ir: {}", uri);
                        vec![]
                    }
                }
                DocumentLanguage::Rholang | DocumentLanguage::Unknown => {
                    // Collect symbols from Rholang IR
                    collect_document_symbols(&doc.ir, &*doc.positions)
                }
            };

            debug!("Found {} symbols in document {}", symbols.len(), uri);
            Ok(Some(DocumentSymbolResponse::Nested(symbols)))
        } else {
            debug!("Document not found: {}", uri);
            Ok(None)
        }
    }

    /// Searches for workspace symbols matching the query.
    async fn symbol(&self, params: WorkspaceSymbolParams) -> LspResult<Option<Vec<SymbolInformation>>> {
        let query = params.query;
        debug!("Handling workspace symbol request with query '{}'", query);

        // Ultra-fast path: Use suffix array for O(m log n + k) substring search
        // This is significantly faster than O(documents × symbols × name_length) filtering
        let symbols: Vec<SymbolInformation> = self.workspace.documents
            .iter()
            .flat_map(|entry| entry.value().symbol_index.search(&query))
            .collect();

        debug!("Found {} matching workspace symbols via suffix array", symbols.len());
        Ok(Some(symbols))
    }

    /// Resolves additional information for a workspace symbol (no-op as all info is initial).
    async fn symbol_resolve(&self, params: WorkspaceSymbol) -> LspResult<WorkspaceSymbol> {
        debug!("Resolving workspace symbol: {}", params.name);
        Ok(params) // Return as-is since all info is provided initially
    }

    /// Provides highlights for occurrences of the symbol at the position in the document.
    async fn document_highlight(&self, params: DocumentHighlightParams) -> LspResult<Option<Vec<DocumentHighlight>>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        debug!("documentHighlight at {}:{:?}", uri, position);

        // Eagerly ensure symbols are linked before document highlight operation
        if self.needs_symbol_linking().await {
            debug!("Eagerly linking symbols for document highlight operation");
            self.link_symbols().await;
        }

        // Check if position is within a virtual document (embedded language)
        {
            let virtual_docs = self.virtual_docs.read().await;
            if let Some((virtual_uri, virtual_position, virtual_doc)) =
                virtual_docs.find_virtual_document_at_position(&uri, position)
            {
                debug!(
                    "Position {:?} is in virtual document {} at virtual position {:?}",
                    position, virtual_uri, virtual_position
                );
                drop(virtual_docs);

                // Get highlights from virtual document (MeTTa)
                if virtual_doc.language == "metta" {
                    return self.document_highlight_metta(&virtual_doc, virtual_position, position).await;
                }
            }
        }

        // Rholang document highlighting
        let symbol = match self.get_symbol_at_position(&uri, position).await {
            Some(s) => s,
            None => {
                debug!("No symbol at position");
                return Ok(None);
            }
        };

        let references = self.get_symbol_references(&symbol, true).await;

        let highlights: Vec<DocumentHighlight> = references
            .into_iter()
            .filter(|(ref_uri, _)| ref_uri == &uri)
            .map(|(_, range)| DocumentHighlight {
                range,
                kind: Some(DocumentHighlightKind::READ),
            })
            .collect();

        debug!("Found {} highlights", highlights.len());

        Ok(Some(highlights))
    }

    async fn hover(&self, params: HoverParams) -> LspResult<Option<Hover>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        debug!("Hover request at {}:{:?}", uri, position);

        // Use unified handler (Phase 4c: replaces 200+ lines of language-specific logic)
        Ok(self.unified_hover(uri, position).await)
    }

    /// Provides signature help for contract calls
    async fn signature_help(&self, params: SignatureHelpParams) -> LspResult<Option<SignatureHelp>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        debug!("Signature help request at {}:{:?}", uri, position);

        // Get the document
        let doc = match self.workspace.documents.get(&uri) {
            Some(doc) => doc,
            None => {
                debug!("Document not found: {}", uri);
                return Ok(None);
            }
        };

        // Convert LSP position to byte offset
        let byte_offset = match Self::byte_offset_from_position(
            &doc.text,
            position.line as usize,
            position.character as usize,
        ) {
            Some(offset) => offset,
            None => {
                debug!("Invalid position");
                return Ok(None);
            }
        };

        let ir_pos = IrPosition {
            row: position.line as usize,
            column: position.character as usize,
            byte: byte_offset,
        };

        // Find the node at cursor position with path for context
        let (node, path) = match find_node_at_position_with_path(&doc.ir, &*doc.positions, ir_pos) {
            Some(result) => result,
            None => {
                debug!("No node found at position");
                return Ok(None);
            }
        };

        // Look for Send/SendSync nodes in the path (contract calls)
        for ancestor in path.iter().rev() {
            match &**ancestor {
                RholangNode::Send { channel, inputs, .. } | RholangNode::SendSync { channel, inputs, .. } => {
                    // Extract contract name
                    let contract_name = match Self::extract_contract_name(channel) {
                        Some(name) => name,
                        None => continue,
                    };

                    debug!("Found contract call '{}' with {} arguments", contract_name, inputs.len());

                    // Get all matching overloads using pattern-based lookup
                    let global_table = self.workspace.global_table.read().await;
                    let arg_count = inputs.len();

                    // Get matching overloads for this call
                    let overloads = global_table.get_matching_overloads(&contract_name, arg_count);

                    if overloads.is_empty() {
                        // Fallback: try to get all overloads regardless of arity
                        let all_overloads = global_table.lookup_all_contract_overloads(&contract_name);
                        if all_overloads.is_empty() {
                            debug!("No contract overloads found for '{}'", contract_name);
                            return Ok(None);
                        }

                        // Build signatures from all overloads
                        let signatures = all_overloads.iter().map(|symbol| {
                            let arity = symbol.arity().unwrap_or(0);
                            let variadic_suffix = if symbol.is_variadic() { "..." } else { "" };

                            // Build parameter list
                            let parameters: Vec<ParameterInformation> = (0..arity)
                                .map(|i| ParameterInformation {
                                    label: ParameterLabel::Simple(format!("param{}", i + 1)),
                                    documentation: None,
                                })
                                .collect();

                            SignatureInformation {
                                label: format!("{}({}){}", contract_name, arity, variadic_suffix),
                                documentation: None,
                                parameters: Some(parameters),
                                active_parameter: None,
                            }
                        }).collect();

                        return Ok(Some(SignatureHelp {
                            signatures,
                            active_signature: None,
                            active_parameter: None,
                        }));
                    }

                    // Build signatures from matching overloads
                    let signatures: Vec<SignatureInformation> = overloads.iter().map(|symbol| {
                        let arity = symbol.arity().unwrap_or(0);
                        let variadic_suffix = if symbol.is_variadic() { "..." } else { "" };

                        // Build parameter list
                        let parameters: Vec<ParameterInformation> = (0..arity)
                            .map(|i| ParameterInformation {
                                label: ParameterLabel::Simple(format!("param{}", i + 1)),
                                documentation: None,
                            })
                            .collect();

                        SignatureInformation {
                            label: format!("{}({}){}", contract_name, arity, variadic_suffix),
                            documentation: Some(tower_lsp::lsp_types::Documentation::String(
                                format!("Contract with {} parameter{}", arity, if arity == 1 { "" } else { "s" })
                            )),
                            parameters: Some(parameters),
                            active_parameter: None,
                        }
                    }).collect();

                    // Determine active signature (prefer exact match, then variadic)
                    let active_signature = if let Some(exact_idx) = overloads.iter().position(|s| {
                        s.arity() == Some(arg_count) && !s.is_variadic()
                    }) {
                        Some(exact_idx as u32)
                    } else if let Some(variadic_idx) = overloads.iter().position(|s| s.is_variadic()) {
                        Some(variadic_idx as u32)
                    } else {
                        Some(0)
                    };

                    // Determine active parameter (current argument being typed)
                    let active_parameter = if arg_count > 0 {
                        Some((arg_count - 1).min(9) as u32) // Cap at 9 for safety
                    } else {
                        Some(0)
                    };

                    debug!(
                        "Returning {} signatures for '{}', active: {:?}, param: {:?}",
                        signatures.len(),
                        contract_name,
                        active_signature,
                        active_parameter
                    );

                    return Ok(Some(SignatureHelp {
                        signatures,
                        active_signature,
                        active_parameter,
                    }));
                }
                _ => continue,
            }
        }

        debug!("Not in a contract call context");
        Ok(None)
    }

    /// Provides code completion suggestions
    async fn completion(&self, params: CompletionParams) -> LspResult<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;

        debug!("Completion request at {}:{:?}", uri, position);

        // Get document
        let doc = match self.workspace.documents.get(&uri) {
            Some(doc) => doc,
            None => {
                debug!("Document not found: {}", uri);
                return Ok(None);
            }
        };

        let mut completions = Vec::new();

        // Get all contract symbols from global table using pattern-based lookup
        // This is O(1) for accessing the entire contract index
        let global_table = self.workspace.global_table.read().await;

        // Collect all unique contract names from the pattern index
        // This gives us O(1) access to all contracts
        let all_symbols = global_table.collect_all_symbols();

        let mut contract_names_seen = std::collections::HashSet::new();

        for symbol in all_symbols {
            if matches!(symbol.symbol_type, SymbolType::Contract) {
                // Only add each contract name once, even if it has multiple overloads
                if contract_names_seen.insert(symbol.name.clone()) {
                    // Get all overloads for this contract name
                    let overloads = global_table.lookup_all_contract_overloads(&symbol.name);

                    // Create detail string showing all arities
                    let arities: Vec<String> = overloads.iter()
                        .map(|s| {
                            let arity = s.arity().unwrap_or(0);
                            let variadic = if s.is_variadic() { "..." } else { "" };
                            format!("({}){}", arity, variadic)
                        })
                        .collect();

                    let detail = if arities.len() > 1 {
                        format!("contract - overloads: {}", arities.join(", "))
                    } else {
                        format!("contract {}", arities.first().unwrap_or(&"".to_string()))
                    };

                    completions.push(CompletionItem {
                        label: symbol.name.clone(),
                        kind: Some(CompletionItemKind::FUNCTION),
                        detail: Some(detail),
                        documentation: Some(tower_lsp::lsp_types::Documentation::String(
                            format!("Contract with {} overload{}",
                                overloads.len(),
                                if overloads.len() == 1 { "" } else { "s" }
                            )
                        )),
                        ..Default::default()
                    });
                }
            }
        }

        // Also add symbols from local scope (variables, parameters)
        let symbol_table = doc.symbol_table.clone();
        let local_symbols = symbol_table.current_symbols();

        for symbol in local_symbols {
            let kind = match symbol.symbol_type {
                SymbolType::Variable => CompletionItemKind::VARIABLE,
                SymbolType::Contract => CompletionItemKind::FUNCTION,
                SymbolType::Parameter => CompletionItemKind::VARIABLE,
            };

            let type_str = match symbol.symbol_type {
                SymbolType::Variable => "variable",
                SymbolType::Contract => "contract",
                SymbolType::Parameter => "parameter",
            };

            // Skip contracts we've already added from global scope
            if matches!(symbol.symbol_type, SymbolType::Contract) && contract_names_seen.contains(&symbol.name) {
                continue;
            }

            completions.push(CompletionItem {
                label: symbol.name.clone(),
                kind: Some(kind),
                detail: Some(type_str.to_string()),
                documentation: None,
                ..Default::default()
            });
        }

        // Add Rholang keywords
        let keywords = vec![
            ("new", "Declare new channels"),
            ("contract", "Define a contract"),
            ("for", "Input guarded process"),
            ("match", "Pattern matching"),
            ("Nil", "Empty process"),
            ("bundle", "Bundle channels"),
            ("true", "Boolean true"),
            ("false", "Boolean false"),
        ];

        for (keyword, doc) in keywords {
            completions.push(CompletionItem {
                label: keyword.to_string(),
                kind: Some(CompletionItemKind::KEYWORD),
                detail: Some("keyword".to_string()),
                documentation: Some(tower_lsp::lsp_types::Documentation::String(doc.to_string())),
                ..Default::default()
            });
        }

        debug!("Returning {} completion items", completions.len());

        if completions.is_empty() {
            Ok(None)
        } else {
            Ok(Some(CompletionResponse::Array(completions)))
        }
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> LspResult<Option<SemanticTokensResult>> {
        let uri = params.text_document.uri;
        debug!("Semantic tokens request for: {}", uri);

        // Get virtual documents for this file
        let virtual_docs_guard = self.virtual_docs.read().await;
        let virtual_docs_list = virtual_docs_guard.get_by_parent(&uri);

        if virtual_docs_list.is_empty() {
            debug!("No virtual documents (embedded languages) found for {}", uri);
            return Ok(None);
        }

        // Build semantic tokens for all embedded language regions
        let mut tokens_builder = SemanticTokensBuilder::new();

        for virtual_doc in virtual_docs_list {
            debug!(
                "Processing {} virtual document at line {} (bytes {})",
                virtual_doc.language, virtual_doc.parent_start.line, virtual_doc.byte_offset
            );

            // Only process MeTTa regions for now
            if virtual_doc.language == "metta" {
                // Use VirtualDocument directly - it now caches parsed trees
                self.add_metta_semantic_tokens(&mut tokens_builder, &virtual_doc).await;
            }
        }
        drop(virtual_docs_guard);

        let tokens_data = tokens_builder.build();

        debug!("Generated {} semantic tokens", tokens_data.len());

        Ok(Some(SemanticTokensResult::Tokens(
            tower_lsp::lsp_types::SemanticTokens {
                result_id: None,
                data: tokens_data,
            }
        )))
    }
}

// ========================================================================
// Pattern-Based Lookup Helper Functions
// ========================================================================

impl RholangBackend {
    /// Extracts contract name from a channel node (Var or Quote)
    fn extract_contract_name(channel: &RholangNode) -> Option<String> {
        match channel {
            RholangNode::Var { name, .. } => Some(name.clone()),
            RholangNode::Quote { quotable, .. } => {
                if let RholangNode::Var { name, .. } = &**quotable {
                    Some(name.clone())
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Filters global contracts using pattern-based lookup for better performance.
    /// Returns (Url, Arc<RholangNode>) tuples for contracts matching the pattern.
    ///
    /// This provides O(1) lookup by (name, arity) instead of O(n) iteration.
    /// Falls back to full scan if pattern lookup yields no results for safety.
    fn filter_contracts_by_pattern<'a>(
        global_contracts: &'a [(Url, std::sync::Arc<RholangNode>)],
        global_table: &crate::ir::symbol_table::SymbolTable,
        contract_name: &str,
        arg_count: usize,
    ) -> Vec<&'a (Url, std::sync::Arc<RholangNode>)> {
        use crate::ir::symbol_table::SymbolType;

        // Try pattern-based O(1) lookup first
        let pattern_matches = global_table.lookup_contracts_by_pattern(contract_name, arg_count);

        if !pattern_matches.is_empty() {
            debug!(
                "Pattern index found {} candidate contracts for '{}' with arity {}",
                pattern_matches.len(),
                contract_name,
                arg_count
            );

            // Filter global_contracts to only those matching the pattern index results
            let candidate_uris: std::collections::HashSet<_> = pattern_matches
                .iter()
                .map(|s| s.declaration_uri.clone())
                .collect();

            let filtered: Vec<_> = global_contracts
                .iter()
                .filter(|(uri, contract)| {
                    // Check if URI matches and contract name matches
                    if !candidate_uris.contains(uri) {
                        return false;
                    }

                    // Verify contract name matches
                    if let RholangNode::Contract { name, .. } = &**contract {
                        if let Some(name_str) = Self::extract_contract_name(name) {
                            return name_str == contract_name;
                        }
                    }
                    false
                })
                .collect();

            if !filtered.is_empty() {
                debug!(
                    "Pattern-based filtering reduced search space from {} to {} contracts",
                    global_contracts.len(),
                    filtered.len()
                );
                return filtered;
            }
        }

        // Fallback: filter by name only if pattern lookup found nothing
        debug!(
            "Pattern index returned no results for '{}', falling back to name-based filtering",
            contract_name
        );

        global_contracts
            .iter()
            .filter(|(_, contract)| {
                if let RholangNode::Contract { name, .. } = &**contract {
                    if let Some(name_str) = Self::extract_contract_name(name) {
                        return name_str == contract_name;
                    }
                }
                false
            })
            .collect()
    }
}
