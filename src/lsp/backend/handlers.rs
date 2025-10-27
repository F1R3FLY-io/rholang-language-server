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

use crate::ir::rholang_node::{RholangNode, Position as IrPosition, find_node_at_position_with_path, find_node_at_position};
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
        info!("Received initialize: {:?}", params);

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

                // Queue all .rho files for progressive indexing
                let mut file_count = 0;
                for entry in WalkDir::new(&root_path).into_iter().filter_map(|e| e.ok()) {
                    if entry.path().extension().map_or(false, |ext| ext == "rho") {
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
                            file_count += 1;
                        }
                    }
                }
                info!("Queued {} .rho files for progressive indexing", file_count);

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
        info!("Initialized: {:?}", params);
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
        info!("textDocument/didChange: {:?}", params);
        let uri = params.text_document.uri.clone();
        let version = params.text_document.version;
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
        info!("textDocument/didSave: {:?}", params);
        // Validation occurs on open and change; no additional action needed here
    }

    /// Handles closing a text document, removing it from state and clearing diagnostics.
    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        info!("textDocument/didClose: {:?}", params);
        let uri = params.text_document.uri;
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
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let new_name = params.new_name;

        debug!("Starting rename for {} at {:?} to '{}'", uri, position, new_name);

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

                // Get rename from virtual document (MeTTa)
                if virtual_doc.language == "metta" {
                    return self.rename_metta(&virtual_doc, virtual_position, &new_name).await;
                }
            }
        }

        let symbol = match self.get_symbol_at_position(&uri, position).await {
            Some(s) => s,
            None => {
                debug!("No renameable symbol at {}:{:?}", uri, position);
                return Ok(None);
            }
        };

        // Step 2: Collect all reference locations
        let references = self.get_symbol_references(&symbol, true).await;
        if references.is_empty() {
            debug!("No references to rename for '{}'", symbol.name);
            return Ok(None);
        }

        // Step 3: Group references by URI and create TextEdits
        let mut changes = HashMap::new();
        for (ref_uri, range) in references {
            let edit = TextEdit {
                range,
                new_text: new_name.clone(),
            };
            changes.entry(ref_uri).or_insert_with(Vec::new).push(edit);
        }

        debug!("Prepared {} edits across {} files for '{}'",
            changes.values().map(|v| v.len()).sum::<usize>(),
            changes.len(),
            symbol.name
        );

        // Step 4: Construct and return the WorkspaceEdit
        Ok(Some(WorkspaceEdit {
            changes: Some(changes),
            document_changes: None,
            change_annotations: None,
        }))
    }

    /// Handles going to a symbol's definition.
    async fn goto_definition(&self, params: GotoDefinitionParams) -> LspResult<Option<GotoDefinitionResponse>> {
        let start = std::time::Instant::now();
        let uri = params.text_document_position_params.text_document.uri.clone();
        let lsp_pos = params.text_document_position_params.position;

        debug!("goto_definition request for {} at {:?}", uri, lsp_pos);

        // Check if position is within a virtual document (embedded language)
        {
            let virtual_docs = self.virtual_docs.read().await;
            if let Some((virtual_uri, virtual_position, virtual_doc)) =
                virtual_docs.find_virtual_document_at_position(&uri, lsp_pos)
            {
                debug!(
                    "Position {:?} is in virtual document {} at virtual position {:?}",
                    lsp_pos, virtual_uri, virtual_position
                );
                drop(virtual_docs);

                // Get goto-definition from virtual document (MeTTa)
                if virtual_doc.language == "metta" {
                    let result = self.goto_definition_metta(&virtual_doc, virtual_position).await;
                    info!("goto_definition completed in {:.3}ms (MeTTa virtual document)", start.elapsed().as_secs_f64() * 1000.0);
                    return result;
                }
            }
        }

        let byte = {
            let workspace = self.workspace.read().await;
            if let Some(doc) = workspace.documents.get(&uri) {
                let text = &doc.text;
                Self::byte_offset_from_position(text, lsp_pos.line as usize, lsp_pos.character as usize)
            } else {
                info!("goto_definition completed in {:.3}ms (document not found)", start.elapsed().as_secs_f64() * 1000.0);
                debug!("Document not found in workspace: {}", uri);
                return Ok(None);
            }
        };

        let ir_pos = IrPosition {
            row: lsp_pos.line as usize,
            column: lsp_pos.character as usize,
            byte: byte.unwrap_or(0),
        };

        debug!("Computed IR position: {:?}", ir_pos);

        let workspace = self.workspace.read().await;
        if let Some(doc) = workspace.documents.get(&uri) {
            debug!("Document found in workspace: {}", uri);
            let root = &doc.ir;
            if let Some((node, path)) = find_node_at_position_with_path(root, &*doc.positions, ir_pos) {
                debug!("Found node at position: '{}', type: {:?}, path length: {}",
                    node.text(&doc.text, root).to_string(),
                    match node.as_ref() {
                        RholangNode::StringLiteral { .. } => "StringLiteral",
                        RholangNode::Quote { .. } => "Quote",
                        RholangNode::Var { .. } => "Var",
                        RholangNode::Send { .. } => "Send",
                        _ => "Other"
                    },
                    path.len()
                );
                if path.len() >= 2 {
                    let parent = path[path.len() - 2].clone();

                    // Check if this node is directly a channel in Send/SendSync
                    let is_direct_channel = match &*parent {
                        RholangNode::Send { channel, .. } | RholangNode::SendSync { channel, .. } => std::sync::Arc::ptr_eq(channel, &node),
                        _ => false,
                    };

                    // Check if this node is inside a Quote that's the channel of Send/SendSync
                    // For quoted contracts like @"myContract", the path is: [..., Send, Quote, StringLiteral]
                    let is_quoted_channel = if path.len() >= 3 {
                        match (&*parent, &*path[path.len() - 3]) {
                            (RholangNode::Quote { quotable: _, .. }, RholangNode::Send { channel, .. }) |
                            (RholangNode::Quote { quotable: _, .. }, RholangNode::SendSync { channel, .. }) => {
                                // Check that the parent Quote is the channel
                                std::sync::Arc::ptr_eq(channel, &parent)
                            }
                            _ => false
                        }
                    } else {
                        false
                    };

                    let is_channel = is_direct_channel || is_quoted_channel;

                    debug!("Parent type: {:?}, is_channel: {} (direct: {}, quoted: {})",
                        match parent.as_ref() {
                            RholangNode::StringLiteral { .. } => "StringLiteral",
                            RholangNode::Quote { .. } => "Quote",
                            RholangNode::Var { .. } => "Var",
                            RholangNode::Send { .. } => "Send",
                            _ => "Other"
                        },
                        is_channel, is_direct_channel, is_quoted_channel
                    );
                    if is_channel {
                        // Fast path: Try GlobalSymbolIndex pattern matching for O(k) lookup
                        let contract_name_opt = match node.as_ref() {
                            RholangNode::Var { name, .. } => Some(name.clone()),
                            RholangNode::StringLiteral { value, .. } => Some(value.clone()),
                            _ => None
                        };

                        if let Some(contract_name) = contract_name_opt {
                            debug!("Attempting fast-path contract lookup via GlobalSymbolIndex for: {}", contract_name);
                            let global_index = workspace.global_index.clone();
                            drop(workspace); // Release lock before potentially async work

                            if let Ok(global_index_guard) = global_index.read() {
                                if let Ok(Some(symbol_loc)) = global_index_guard.find_contract_definition(&contract_name) {
                                    info!("goto_definition completed in {:.3}ms (fast path via GlobalSymbolIndex)", start.elapsed().as_secs_f64() * 1000.0);
                                    debug!("Found contract '{}' via GlobalSymbolIndex at {}", contract_name, symbol_loc.uri);
                                    return Ok(Some(GotoDefinitionResponse::Scalar(symbol_loc.to_lsp_location())));
                                } else {
                                    debug!("Contract '{}' not found in GlobalSymbolIndex, falling back to iteration", contract_name);
                                }
                            }
                        }

                        // Reacquire workspace lock for fallback (or use existing if not dropped)
                        let workspace = self.workspace.read().await;

                        // For quoted contracts, the parent is Quote and grandparent is Send
                        // For non-quoted, parent is Send
                        let send_node = if is_quoted_channel && path.len() >= 3 {
                            &path[path.len() - 3]
                        } else {
                            &parent
                        };

                        if let RholangNode::Send { channel, inputs, .. } | RholangNode::SendSync { channel, inputs, .. } = &**send_node {
                            debug!("Checking {} global contracts for match with channel: {:?}", workspace.global_contracts.len(), channel);

                            // Extract contract name and arity for pattern-based lookup
                            let contract_name = Self::extract_contract_name(channel);
                            let arg_count = inputs.len();

                            // Use pattern-based filtering for O(1) lookup
                            let candidates = if let Some(name) = &contract_name {
                                Self::filter_contracts_by_pattern(
                                    &workspace.global_contracts,
                                    &workspace.global_table,
                                    name,
                                    arg_count
                                )
                            } else {
                                // Fallback: no name extraction possible, check all contracts
                                debug!("Could not extract contract name from channel, checking all contracts");
                                workspace.global_contracts.iter().collect()
                            };

                            debug!("Pattern-based filtering returned {} candidate contracts", candidates.len());

                            let matching = candidates.iter().filter(|(_, contract)| {
                                use crate::ir::rholang_node::match_contract;
                                let result = match_contract(channel, inputs, contract);
                                debug!("match_contract(channel, inputs, contract) = {} for contract: {:?}", result, contract);
                                result
                            }).map(|(u, c)| {
                                let cached_doc = workspace.documents.get(u).expect("Document not found");
                                let positions = cached_doc.positions.clone();
                                debug!("Matched contract in {}: '{}'", u, c.text(&cached_doc.text, &cached_doc.ir).to_string());
                                let name = if let RholangNode::Contract { name, .. } = &**c {
                                    debug!("Contact name: {:?}", name);
                                    name
                                } else {
                                    debug!("Unreachable!");
                                    unreachable!()
                                };
                                debug!("Found contract name");
                                let key = &**name as *const RholangNode as usize;
                                let (start, _) = (*positions).get(&key).unwrap();
                                Location {
                                    uri: u.clone(),
                                    range: Self::position_to_range(*start, name.text(&cached_doc.text, &cached_doc.ir).len_chars()),
                                }
                            }).collect::<Vec<_>>();
                            debug!("Found {} matching contracts", matching.len());
                            if matching.is_empty() {
                                drop(workspace);
                                debug!("No matching contracts; falling back to symbol lookup");
                                let result = if let Some(symbol) = self.get_symbol_at_position(&uri, lsp_pos).await {
                                    let pos = symbol.definition_location.unwrap_or(symbol.declaration_location);
                                    let range = Self::position_to_range(pos, symbol.name.len());
                                    let loc = Location { uri: symbol.declaration_uri.clone(), range };
                                    Ok(Some(GotoDefinitionResponse::Scalar(loc)))
                                } else {
                                    Ok(None)
                                };
                                info!("goto_definition completed in {:.3}ms (no matching contracts, symbol lookup fallback)", start.elapsed().as_secs_f64() * 1000.0);
                                result
                            } else if matching.len() == 1 {
                                info!("goto_definition completed in {:.3}ms (found 1 matching contract)", start.elapsed().as_secs_f64() * 1000.0);
                                Ok(Some(GotoDefinitionResponse::Scalar(matching[0].clone())))
                            } else {
                                info!("goto_definition completed in {:.3}ms (found {} matching contracts)", start.elapsed().as_secs_f64() * 1000.0, matching.len());
                                Ok(Some(GotoDefinitionResponse::Array(matching)))
                            }
                        } else {
                            unreachable!()
                        }
                    } else {
                        debug!("Not a channel; checking if contract definition click");
                        // Check if clicking on a contract name in its definition
                        if let RholangNode::Contract { name, .. } = &*parent {
                            debug!("Parent is Contract node");
                            if std::sync::Arc::ptr_eq(name, &node) {
                                debug!("Node is contract name - returning contract location");
                                // Clicking on contract name - return the contract's location
                                let contract_doc = workspace.documents.get(&uri).expect("Document not found");
                                let positions = contract_doc.positions.clone();
                                let key = &**name as *const RholangNode as usize;
                                if let Some((start_pos, _)) = (*positions).get(&key) {
                                    let range = Self::position_to_range(*start_pos, name.text(&contract_doc.text, &contract_doc.ir).len_chars());
                                    let loc = Location { uri: uri.clone(), range };
                                    info!("goto_definition completed in {:.3}ms (contract definition)", start.elapsed().as_secs_f64() * 1000.0);
                                    return Ok(Some(GotoDefinitionResponse::Scalar(loc)));
                                } else {
                                    debug!("Position not found in positions map for contract name");
                                }
                            } else {
                                debug!("Node is not the contract name (ptr_eq failed)");
                            }
                        } else {
                            debug!("Parent is not Contract node, parent type: {:?}", std::mem::discriminant(&*parent));
                        }

                        drop(workspace);
                        debug!("Falling back to symbol lookup");
                        let result = if let Some(symbol) = self.get_symbol_at_position(&uri, lsp_pos).await {
                            let pos = symbol.definition_location.unwrap_or(symbol.declaration_location);
                            let range = Self::position_to_range(pos, symbol.name.len());
                            let loc = Location { uri: symbol.declaration_uri.clone(), range };
                            Ok(Some(GotoDefinitionResponse::Scalar(loc)))
                        } else {
                            debug!("No symbol found at position");
                            Ok(None)
                        };
                        info!("goto_definition completed in {:.3}ms (symbol lookup fallback)", start.elapsed().as_secs_f64() * 1000.0);
                        result
                    }
                } else {
                    drop(workspace);
                    debug!("Path too short; falling back to symbol lookup");
                    let result = if let Some(symbol) = self.get_symbol_at_position(&uri, lsp_pos).await {
                        let pos = symbol.definition_location.unwrap_or(symbol.declaration_location);
                        let range = Self::position_to_range(pos, symbol.name.len());
                        let loc = Location { uri: symbol.declaration_uri.clone(), range };
                        Ok(Some(GotoDefinitionResponse::Scalar(loc)))
                    } else {
                        Ok(None)
                    };
                    info!("goto_definition completed in {:.3}ms (symbol lookup fallback)", start.elapsed().as_secs_f64() * 1000.0);
                    result
                }
            } else {
                debug!("No node found at position {:?} in find_node_at_position", ir_pos);
                // Try symbol lookup as fallback
                drop(workspace);
                let result = if let Some(symbol) = self.get_symbol_at_position(&uri, lsp_pos).await {
                    let pos = symbol.definition_location.unwrap_or(symbol.declaration_location);
                    let range = Self::position_to_range(pos, symbol.name.len());
                    let loc = Location { uri: symbol.declaration_uri.clone(), range };
                    Ok(Some(GotoDefinitionResponse::Scalar(loc)))
                } else {
                    debug!("No symbol found at position");
                    Ok(None)
                };
                info!("goto_definition completed in {:.3}ms (symbol lookup for missing node)", start.elapsed().as_secs_f64() * 1000.0);
                result
            }
        } else {
            info!("goto_definition completed in {:.3}ms (document not found in workspace)", start.elapsed().as_secs_f64() * 1000.0);
            debug!("Document {} not found in workspace for goto_definition", uri);
            Ok(None)
        }
    }

    /// Handles going to a symbol's declaration.
    async fn goto_declaration(&self, params: GotoDeclarationParams) -> LspResult<Option<GotoDeclarationResponse>> {
        let uri = params.text_document_position_params.text_document.uri.clone();
        let position = params.text_document_position_params.position;

        debug!("goto_declaration request for {} at {:?}", uri, position);

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
        let uri = params.text_document_position.text_document.uri.clone();
        let lsp_pos = params.text_document_position.position;
        let include_decl = params.context.include_declaration;

        debug!("references request for {} at {:?} (include_decl: {})", uri, lsp_pos, include_decl);

        let byte = {
            let workspace = self.workspace.read().await;
            if let Some(doc) = workspace.documents.get(&uri) {
                let text = &doc.text;
                Self::byte_offset_from_position(text, lsp_pos.line as usize, lsp_pos.character as usize)
            } else {
                debug!("Document {} not found in workspace", uri);
                return Ok(None);
            }
        };

        let ir_pos = IrPosition {
            row: lsp_pos.line as usize,
            column: lsp_pos.character as usize,
            byte: byte.unwrap_or(0),
        };

        debug!("Computed IR position: {:?}", ir_pos);

        let workspace = self.workspace.read().await;
        if let Some(doc) = workspace.documents.get(&uri) {
            debug!("Document found in workspace: {}", uri);
            let root = &doc.ir;
            if let Some((node, path)) = find_node_at_position_with_path(root, &*doc.positions, ir_pos) {
                debug!("Found node at position: '{}'", node.text(&doc.text, root).to_string());
                if path.len() >= 2 {
                    let parent = path[path.len() - 2].clone();
                    let is_name = match &*parent {
                        RholangNode::Contract { name, .. } => std::sync::Arc::ptr_eq(name, &node),
                        _ => false,
                    };
                    debug!("Is name in Contract: {}", is_name);
                    if is_name {
                        // Fast path: Try GlobalSymbolIndex for O(k) reference lookup
                        if let RholangNode::Var { name: contract_name, .. } = node.as_ref() {
                            debug!("Attempting fast-path reference lookup via GlobalSymbolIndex for: {}", contract_name);
                            let global_index = workspace.global_index.clone();

                            if let Ok(global_index_guard) = global_index.read() {
                                if let Ok(ref_locs) = global_index_guard.find_contract_references(contract_name) {
                                    if !ref_locs.is_empty() {
                                        debug!("Found {} references via GlobalSymbolIndex", ref_locs.len());
                                        let mut locations: Vec<Location> = ref_locs.into_iter()
                                            .map(|loc| loc.to_lsp_location())
                                            .collect();

                                        // Add declaration if requested
                                        if include_decl {
                                            let key = &*node as *const RholangNode as usize;
                                            if let Some((start, _)) = (*doc.positions).get(&key) {
                                                let decl_range = Self::position_to_range(*start, contract_name.len());
                                                locations.push(Location { uri: uri.clone(), range: decl_range });
                                            }
                                        }

                                        return Ok(Some(locations));
                                    } else {
                                        debug!("No references found in GlobalSymbolIndex, falling back");
                                    }
                                }
                            }
                        }

                        if let RholangNode::Contract { name: contract_name_node, formals, .. } = &*parent {
                            let contract = parent.clone();

                            // Extract contract name for logging
                            let contract_name = Self::extract_contract_name(contract_name_node);
                            let arg_count = formals.len();

                            // Check all calls - the match_contract function below does the actual filtering
                            // Note: We don't use filter_contracts_by_pattern here because global_calls
                            // contains Send/SendSync nodes, not Contract nodes
                            let candidates = workspace.global_calls.iter().collect::<Vec<_>>();

                            debug!("Checking {} candidate calls for contract {:?} with arity {}",
                                   candidates.len(), contract_name, arg_count);

                            let matching_calls = candidates.iter().filter_map(|(uri, call)| {
                                use crate::ir::rholang_node::match_contract;
                                match &**call {
                                    RholangNode::Send { channel, inputs, .. } | RholangNode::SendSync { channel, inputs, .. } => {
                                        if match_contract(channel, inputs, &contract) {
                                            Some((uri.clone(), call.clone()))
                                        } else {
                                            None
                                        }
                                    }
                                    _ => None,
                                }
                            }).collect::<Vec<_>>();
                            debug!("Found {} matching calls for contract", matching_calls.len());
                            let mut locations = matching_calls.iter().map(|(call_uri, call)| {
                                let call_doc = workspace.documents.get(call_uri).expect("Document not found");
                                let call_positions = call_doc.positions.clone();
                                debug!("Matched call in {}: '{}'", call_uri, call.text(&call_doc.text, &call_doc.ir).to_string());
                                let channel = match &**call {
                                    RholangNode::Send { channel, .. } | RholangNode::SendSync { channel, .. } => channel.clone(),
                                    _ => unreachable!()
                                };
                                let key = &*channel as *const RholangNode as usize;
                                let (start, _) = (*call_positions).get(&key).unwrap();
                                Location {
                                    uri: call_uri.clone(),
                                    range: Self::position_to_range(*start, channel.text(&call_doc.text, &call_doc.ir).len_chars()),
                                }
                            }).collect::<Vec<_>>();
                            if include_decl {
                                let key = &*node as *const RholangNode as usize;
                                let (start, _) = (*doc.positions).get(&key).unwrap();
                                let decl_range = Self::position_to_range(*start, node.text(&doc.text, root).len_chars());
                                locations.push(Location { uri: uri.clone(), range: decl_range });
                            }
                            Ok(Some(locations))
                        } else {
                            unreachable!()
                        }
                    } else {
                        drop(workspace);
                        debug!("Not a contract name; falling back to symbol references");
                        if let Some(symbol) = self.get_symbol_at_position(&uri, lsp_pos).await {
                            let refs = self.get_symbol_references(&symbol, include_decl).await;
                            let locations = refs.into_iter().map(|(u, r)| Location { uri: u, range: r }).collect();
                            Ok(Some(locations))
                        } else {
                            Ok(None)
                        }
                    }
                } else {
                    drop(workspace);
                    debug!("Path too short; falling back to symbol references");
                    if let Some(symbol) = self.get_symbol_at_position(&uri, lsp_pos).await {
                        let refs = self.get_symbol_references(&symbol, include_decl).await;
                        let locations = refs.into_iter().map(|(u, r)| Location { uri: u, range: r }).collect();
                        Ok(Some(locations))
                    } else {
                        Ok(None)
                    }
                }
            } else {
                debug!("No node found at position {:?} in {}", ir_pos, uri);
                Ok(None)
            }
        } else {
            debug!("Document {} not found in workspace for references", uri);
            Ok(None)
        }
    }

    /// Provides document symbols for the given document.
    async fn document_symbol(&self, params: DocumentSymbolParams) -> LspResult<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri;
        debug!("Handling documentSymbol request for {}", uri);
        let workspace = self.workspace.read().await;
        if let Some(doc) = workspace.documents.get(&uri) {
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
        let workspace = self.workspace.read().await;

        // Ultra-fast path: Use suffix array for O(m log n + k) substring search
        // This is significantly faster than O(documents × symbols × name_length) filtering
        let symbols: Vec<SymbolInformation> = workspace.documents
            .values()
            .flat_map(|doc| doc.symbol_index.search(&query))
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
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        debug!("Hover request at {}:{:?}", uri, position);

        // Check if position is within a virtual document (embedded language)
        let virtual_docs = self.virtual_docs.read().await;
        if let Some((virtual_uri, virtual_position, virtual_doc)) =
            virtual_docs.find_virtual_document_at_position(&uri, position)
        {
            debug!(
                "Position {:?} is in virtual document {} at virtual position {:?}",
                position, virtual_uri, virtual_position
            );

            // Get hover from virtual document
            if let Some(mut hover) = virtual_doc.hover(virtual_position) {
                // Map hover range back to parent coordinates
                if let Some(range) = hover.range {
                    hover.range = Some(virtual_doc.map_range_to_parent(range));
                }
                debug!("Returning hover from virtual document");
                return Ok(Some(hover));
            }
        }
        drop(virtual_docs); // Release the lock

        // Get the document
        let workspace = self.workspace.read().await;
        let doc = match workspace.documents.get(&uri) {
            Some(doc) => doc,
            None => {
                debug!("Document not found: {}", uri);
                return Ok(None);
            }
        };

        use crate::lsp::models::DocumentLanguage;

        // Route to language-specific hover handler
        match doc.language {
            DocumentLanguage::Metta => {
                return self.hover_metta(&doc, position).await;
            }
            DocumentLanguage::Rholang | DocumentLanguage::Unknown => {
                // Continue with Rholang hover logic below
            }
        }

        // Find the node at the cursor position (Rholang)
        let byte_offset = Self::byte_offset_from_position(&doc.text, position.line as usize, position.character as usize)
            .unwrap_or(0);

        let ir_position = IrPosition {
            row: position.line as usize,
            column: position.character as usize,
            byte: byte_offset,
        };

        let node = find_node_at_position(&doc.ir, &doc.positions, ir_position)
            .ok_or_else(|| jsonrpc::Error::internal_error())?;

        // Check if this is a variable (contract name or local variable)
        if let RholangNode::Var { name: var_name, .. } = node.as_ref() {
            debug!("Hovering over variable: {}", var_name);

            // Try to find contract definition in global index
            let global_index = workspace.global_index.clone();

            if let Ok(global_index_guard) = global_index.read() {
                if let Ok(Some(symbol_loc)) = global_index_guard.find_contract_definition(var_name) {
                    // Build hover content with signature
                    if let Some(signature) = &symbol_loc.signature {
                        let mut hover_text = format!("```rholang\n{}\n```", signature);

                        // Add documentation if available
                        if let Some(doc_text) = &symbol_loc.documentation {
                            hover_text.push_str("\n\n---\n\n");
                            hover_text.push_str(doc_text);
                        }

                        // Add location information
                        hover_text.push_str(&format!("\n\n*Defined in: {}*", symbol_loc.uri.path()));

                        debug!("Returning hover for contract '{}': {}", var_name, signature);

                        return Ok(Some(Hover {
                            contents: HoverContents::Markup(MarkupContent {
                                kind: MarkupKind::Markdown,
                                value: hover_text,
                            }),
                            range: Some(Range {
                                start: position,
                                end: LspPosition {
                                    line: position.line,
                                    character: position.character + var_name.len() as u32,
                                },
                            }),
                        }));
                    }
                }
            }

            // Fallback: provide enhanced hover info using symbol table
            // This prevents VSCode from clearing document highlights when hover returns None
            debug!("Looking up symbol information for variable '{}'", var_name);

            // Check if this is a contract with overloads
            let global_table = workspace.global_table.clone();
            let overloads = global_table.lookup_all_contract_overloads(var_name);

            drop(workspace); // Release workspace lock before get_symbol_at_position

            if !overloads.is_empty() {
                // Show contract overload information
                let mut hover_text = format!("**{}**\n\n*contract*", var_name);

                // Add all overload signatures
                hover_text.push_str("\n\n**Overloads:**");
                for (idx, symbol) in overloads.iter().enumerate() {
                    let arity = symbol.arity().unwrap_or(0);
                    let variadic = if symbol.is_variadic() { "..." } else { "" };
                    hover_text.push_str(&format!("\n{}. `{}({}){}`",
                        idx + 1, var_name, arity, variadic));
                }

                // Add location for first overload
                let first = &overloads[0];
                hover_text.push_str(&format!("\n\nDeclared at line {}, column {}",
                    first.declaration_location.row + 1,
                    first.declaration_location.column + 1));

                if overloads.len() > 1 {
                    hover_text.push_str(&format!(" (+{} more)", overloads.len() - 1));
                }

                debug!("Providing overload hover for contract '{}' with {} overloads", var_name, overloads.len());
                return Ok(Some(Hover {
                    contents: HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: hover_text,
                    }),
                    range: Some(Range {
                        start: position,
                        end: LspPosition {
                            line: position.line,
                            character: position.character + var_name.len() as u32,
                        },
                    }),
                }));
            }

            if let Some(symbol) = self.get_symbol_at_position(&uri, position).await {
                let mut hover_text = format!("**{}**", var_name);

                // Add symbol type
                let symbol_type_str = match symbol.symbol_type {
                    SymbolType::Variable => "variable",
                    SymbolType::Contract => "contract",
                    SymbolType::Parameter => "parameter",
                };
                hover_text.push_str(&format!("\n\n*{}*", symbol_type_str));

                // Add declaration location
                let decl_loc = symbol.declaration_location;
                hover_text.push_str(&format!("\n\nDeclared at line {}, column {}",
                    decl_loc.row + 1, decl_loc.column + 1));

                debug!("Providing enhanced hover for variable '{}'", var_name);
                return Ok(Some(Hover {
                    contents: HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: hover_text,
                    }),
                    range: Some(Range {
                        start: position,
                        end: LspPosition {
                            line: position.line,
                            character: position.character + var_name.len() as u32,
                        },
                    }),
                }));
            } else {
                // Last resort: show just the variable name
                debug!("No symbol info found, providing basic hover for variable '{}'", var_name);
                return Ok(Some(Hover {
                    contents: HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: format!("**{}**\n\n*variable*", var_name),
                    }),
                    range: Some(Range {
                        start: position,
                        end: LspPosition {
                            line: position.line,
                            character: position.character + var_name.len() as u32,
                        },
                    }),
                }));
            }
        }

        debug!("No hover information available");
        Ok(None)
    }

    /// Provides signature help for contract calls
    async fn signature_help(&self, params: SignatureHelpParams) -> LspResult<Option<SignatureHelp>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        debug!("Signature help request at {}:{:?}", uri, position);

        // Get the document
        let workspace = self.workspace.read().await;
        let doc = match workspace.documents.get(&uri) {
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
                    let global_table = workspace.global_table.clone();
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

        let workspace = self.workspace.read().await;

        // Get document
        let doc = match workspace.documents.get(&uri) {
            Some(doc) => doc,
            None => {
                debug!("Document not found: {}", uri);
                return Ok(None);
            }
        };

        let mut completions = Vec::new();

        // Get all contract symbols from global table using pattern-based lookup
        // This is O(1) for accessing the entire contract index
        let global_table = workspace.global_table.clone();

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
        global_table: &std::sync::Arc<crate::ir::symbol_table::SymbolTable>,
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
