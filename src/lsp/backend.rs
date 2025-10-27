use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};
use std::sync::mpsc::Receiver;

use tokio::sync::RwLock;
use tokio::task;


use tower_lsp::{Client, LanguageServer, jsonrpc};
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
    SemanticTokensOptions,
};
use tower_lsp::lsp_types::request::{GotoDeclarationParams, GotoDeclarationResponse};
use tower_lsp::jsonrpc::Result as LspResult;

use tracing::{debug, error, info, trace, warn};

use ropey::Rope;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use walkdir::WalkDir;

use crate::ir::pipeline::Pipeline;
use crate::ir::rholang_node::{RholangNode, Position as IrPosition, compute_absolute_positions, collect_contracts, collect_calls, match_contract, find_node_at_position_with_path, find_node_at_position};
use crate::ir::symbol_table::{Symbol, SymbolTable, SymbolType};
use crate::ir::transforms::symbol_table_builder::{SymbolTableBuilder, InvertedIndex};
use crate::ir::transforms::symbol_index_builder::SymbolIndexBuilder;
use crate::ir::transforms::document_symbol_visitor::collect_document_symbols;
use crate::language_regions::{ChannelFlowAnalyzer, DirectiveParser, SemanticDetector, VirtualDocumentRegistry};
use crate::lsp::models::{CachedDocument, LspDocument, LspDocumentHistory, LspDocumentState, WorkspaceState};
use crate::lsp::semantic_validator::SemanticValidator;
use crate::lsp::diagnostic_provider::{BackendConfig, DiagnosticProvider, create_provider};
use crate::tree_sitter::{parse_code, parse_to_ir};

use rholang_parser::RholangParser;
use rholang_parser::parser::errors::ParsingError;
use validated::Validated;

// Import types from backend submodules
mod state;
mod utils;
mod streams;
mod reactive;
mod metta;
mod symbols;
mod handlers;

pub use state::RholangBackend;
use state::{DocumentChangeEvent, IndexingTask, WorkspaceChangeEvent, WorkspaceChangeType};
use utils::SemanticTokensBuilder;

impl RholangBackend {
    /// Creates a new instance of the Rholang backend with the given client and connections.
    ///
    /// If `grpc_address` is provided, uses gRPC backend to connect to RNode server.
    /// Otherwise, uses the Rust interpreter backend (if available).
    /// Backend can also be selected via RHOLANG_VALIDATOR_BACKEND environment variable.
    pub async fn new(
        client: Client,
        grpc_address: Option<String>,
        client_process_id: Option<u32>,
        pid_channel: Option<tokio::sync::mpsc::Sender<u32>>,
    ) -> anyhow::Result<Self> {
        // Determine backend configuration
        let backend_config = if let Some(addr) = grpc_address {
            info!("Using gRPC backend at {}", addr);
            BackendConfig::Grpc(addr)
        } else {
            // Check environment variable, otherwise use default
            BackendConfig::from_env_or_default(None)
        };

        info!("Creating diagnostic provider with backend: {:?}", backend_config);

        // Create the diagnostic provider
        let diagnostic_provider = create_provider(backend_config.clone()).await?;
        let diagnostic_provider = Arc::new(diagnostic_provider);

        info!("Using {} backend for validation", diagnostic_provider.backend_name());

        // If using Rust backend, keep direct access to SemanticValidator for optimize_parsed optimization
        let semantic_validator = if matches!(backend_config, BackendConfig::Rust) {
            #[cfg(feature = "interpreter")]
            {
                match SemanticValidator::new() {
                    Ok(validator) => Some(validator),
                    Err(e) => {
                        warn!("Failed to get SemanticValidator for optimization: {}", e);
                        None
                    }
                }
            }
            #[cfg(not(feature = "interpreter"))]
            {
                None
            }
        } else {
            None
        };

        let (tx, rx) = std::sync::mpsc::channel();

        // Create reactive channels
        let (doc_change_tx, doc_change_rx) = tokio::sync::mpsc::channel::<DocumentChangeEvent>(100);
        let (indexing_tx, indexing_rx) = tokio::sync::mpsc::channel::<IndexingTask>(100);
        let validation_cancel = Arc::new(Mutex::new(HashMap::new()));
        let (shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(1);

        // Create hot observable for workspace changes (ReactiveX Phase 2)
        let (workspace_tx, _workspace_rx) = tokio::sync::watch::channel(WorkspaceChangeEvent {
            file_count: 0,
            symbol_count: 0,
            change_type: WorkspaceChangeType::Initialized,
        });

        let backend = Self {
            client: client.clone(),
            documents_by_uri: Arc::new(RwLock::new(HashMap::new())),
            documents_by_id: Arc::new(RwLock::new(HashMap::new())),
            serial_document_id: Arc::new(AtomicU32::new(0)),
            diagnostic_provider,
            semantic_validator,
            client_process_id: Arc::new(Mutex::new(client_process_id)),
            pid_channel,
            doc_change_tx: doc_change_tx.clone(),
            validation_cancel: validation_cancel.clone(),
            indexing_tx: indexing_tx.clone(),
            workspace: Arc::new(RwLock::new(WorkspaceState {
                documents: HashMap::new(),
                global_symbols: HashMap::new(),
                global_table: Arc::new(SymbolTable::new(None)),
                global_inverted_index: HashMap::new(),
                global_contracts: Vec::new(),
                global_calls: Vec::new(),
                global_index: Arc::new(std::sync::RwLock::new(crate::ir::global_index::GlobalSymbolIndex::new())),
            })),
            file_watcher: Arc::new(Mutex::new(None)),
            file_events: Arc::new(Mutex::new(rx)),
            file_sender: Arc::new(Mutex::new(tx)),
            version_counter: Arc::new(AtomicI32::new(0)),
            root_dir: Arc::new(RwLock::new(None)),
            shutdown_tx: Arc::new(shutdown_tx),
            virtual_docs: Arc::new(RwLock::new(VirtualDocumentRegistry::new())),
            workspace_changes: Arc::new(workspace_tx),
        };

        // Spawn reactive document change debouncer
        Self::spawn_reactive_document_debouncer(backend.clone(), doc_change_rx);

        // Spawn reactive progressive indexer
        Self::spawn_reactive_progressive_indexer(backend.clone(), indexing_rx);

        Ok(backend)
    }

    /// Spawns the document change debouncer task
    fn spawn_document_debouncer(
        backend: RholangBackend,
        mut doc_change_rx: tokio::sync::mpsc::Receiver<DocumentChangeEvent>,
    ) {
        let mut shutdown_rx = backend.shutdown_tx.subscribe();

        tokio::spawn(async move {
            use std::collections::HashMap;
            use tokio::time::{sleep, Duration};

            let mut pending_changes: HashMap<Url, DocumentChangeEvent> = HashMap::new();
            let debounce_duration = Duration::from_millis(300);

            loop {
                // Wait for a change, timeout, or shutdown signal
                tokio::select! {
                    Some(event) = doc_change_rx.recv() => {
                        // Store/update pending change
                        pending_changes.insert(event.uri.clone(), event);
                    }
                    _ = sleep(debounce_duration), if !pending_changes.is_empty() => {
                        // Timeout reached, process all pending changes
                        for (uri, event) in pending_changes.drain() {
                            // Cancel any in-flight validation for this URI
                            if let Some(cancel_tx) = backend.validation_cancel.lock().unwrap().remove(&uri) {
                                let _ = cancel_tx.send(());
                                trace!("Cancelled previous validation for {}", uri);
                            }

                            // Create cancellation token for this validation
                            let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
                            backend.validation_cancel.lock().unwrap().insert(uri.clone(), cancel_tx);

                            // Spawn validation with cancellation
                            let backend_clone = backend.clone();
                            let uri_clone = uri.clone();
                            let document = event.document.clone();
                            let text = event.text.clone();
                            let version = event.version;

                            tokio::spawn(async move {
                                tokio::select! {
                                    result = backend_clone.validate(document.clone(), &text, version) => {
                                        match result {
                                            Ok(diagnostics) => {
                                                if document.version().await == version {
                                                    backend_clone.client.publish_diagnostics(
                                                        uri_clone.clone(),
                                                        diagnostics,
                                                        Some(version)
                                                    ).await;
                                                }
                                                // Remove cancellation token
                                                backend_clone.validation_cancel.lock().unwrap().remove(&uri_clone);
                                            }
                                            Err(e) => error!("Validation failed for {}: {}", uri_clone, e),
                                        }
                                    }
                                    _ = cancel_rx => {
                                        debug!("Validation cancelled for {}", uri_clone);
                                    }
                                }
                            });
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        info!("Document debouncer received shutdown signal, exiting gracefully");
                        break;
                    }
                }
            }
            debug!("Document debouncer task terminated");
        });
    }

    /// Spawns the progressive workspace indexer task
    fn spawn_progressive_indexer(
        backend: RholangBackend,
        mut indexing_rx: tokio::sync::mpsc::Receiver<IndexingTask>,
    ) {
        let mut shutdown_rx = backend.shutdown_tx.subscribe();

        tokio::spawn(async move {
            use std::collections::BinaryHeap;
            use std::cmp::Ordering;

            #[derive(Eq, PartialEq)]
            struct PrioritizedTask(u8, IndexingTask);

            impl Ord for PrioritizedTask {
                fn cmp(&self, other: &Self) -> Ordering {
                    // Reverse order: lower priority value = higher priority
                    other.0.cmp(&self.0)
                }
            }

            impl PartialOrd for PrioritizedTask {
                fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
                    Some(self.cmp(other))
                }
            }

            let mut queue = BinaryHeap::new();

            loop {
                // Collect tasks or shutdown
                tokio::select! {
                    Some(task) = indexing_rx.recv() => {
                    queue.push(PrioritizedTask(task.priority, task));

                    // Drain any immediately available tasks
                    while let Ok(task) = indexing_rx.try_recv() {
                        queue.push(PrioritizedTask(task.priority, task));
                    }

                    // Process queue by priority
                    while let Some(PrioritizedTask(_, task)) = queue.pop() {
                        match backend.index_file(&task.uri, &task.text, 0, None).await {
                            Ok(cached_doc) => {
                                backend.workspace.write().await.documents.insert(
                                    task.uri.clone(),
                                    Arc::new(cached_doc)
                                );
                                trace!("Indexed {} (priority {})", task.uri, task.priority);
                            }
                            Err(e) => warn!("Failed to index {}: {}", task.uri, e),
                        }
                    }

                    // After batch, link symbols
                    backend.link_symbols().await;
                    }
                    _ = shutdown_rx.recv() => {
                        info!("Progressive indexer received shutdown signal, exiting gracefully");
                        break;
                    }
                }
            }
            debug!("Progressive indexer task terminated");
        });
    }

    /// Spawns a file watcher event batcher to handle rapid file system changes
    fn spawn_file_watcher(
        backend: RholangBackend,
        file_events: Arc<Mutex<Receiver<notify::Result<notify::Event>>>>,
    ) {
        use std::collections::HashSet;
        use tokio::time::{Duration, Instant};
        use std::sync::atomic::{AtomicBool, Ordering};

        let shutdown_flag = Arc::new(AtomicBool::new(false));
        let shutdown_flag_clone = shutdown_flag.clone();
        let shutdown_tx = backend.shutdown_tx.clone();

        // Spawn a task to watch for shutdown signal and set the flag
        tokio::spawn(async move {
            let mut shutdown_rx = shutdown_tx.subscribe();
            let _ = shutdown_rx.recv().await;
            shutdown_flag_clone.store(true, Ordering::Relaxed);
            info!("File watcher received shutdown signal");
        });

        task::spawn_blocking(move || {
            let mut pending_paths: HashSet<PathBuf> = HashSet::new();
            let batch_duration = Duration::from_millis(100);
            let mut last_event_time = Instant::now();

            loop {
                // Check for shutdown
                if shutdown_flag.load(Ordering::Relaxed) {
                    info!("File watcher task exiting gracefully");
                    break;
                }

                // Try to receive an event with timeout
                match file_events.lock().unwrap().recv_timeout(batch_duration) {
                    Ok(Ok(event)) => {
                        // Collect paths from event
                        for path in event.paths {
                            if path.extension().map_or(false, |ext| ext == "rho") {
                                pending_paths.insert(path);
                            }
                        }
                        last_event_time = Instant::now();
                    }
                    Ok(Err(e)) => {
                        warn!("File watcher error: {}", e);
                    }
                    Err(_timeout) => {
                        // Timeout - check if we should process batch
                        if !pending_paths.is_empty() && last_event_time.elapsed() >= batch_duration {
                            // Process batch
                            let paths: Vec<PathBuf> = pending_paths.drain().collect();
                            info!("Processing batch of {} file changes", paths.len());

                            for path in paths {
                                let backend = backend.clone();
                                tokio::spawn(async move {
                                    backend.handle_file_change(path).await;
                                });
                            }
                        }
                    }
                }
            }
            debug!("File watcher task terminated");
        });
    }

    /// Processes a parsed IR node through the transformation pipeline to build symbols and metadata.
    async fn process_document(&self, ir: Arc<RholangNode>, uri: &Url, text: &Rope, content_hash: u64) -> Result<CachedDocument, String> {
        let mut pipeline = Pipeline::new();
        let global_table = self.workspace.read().await.global_table.clone();
        let global_index = self.workspace.read().await.global_index.clone();

        // Symbol table builder for local symbol tracking
        let builder = Arc::new(SymbolTableBuilder::new(ir.clone(), uri.clone(), global_table.clone()));
        pipeline.add_transform(crate::ir::pipeline::Transform {
            id: "symbol_table_builder".to_string(),
            dependencies: vec![],
            kind: crate::ir::pipeline::TransformKind::Specific(builder.clone()),
        });

        // Apply pipeline transformations first to get transformed IR
        let transformed_ir = pipeline.apply(&ir);

        // Compute positions from transformed IR (structural positions are unchanged, but node addresses differ)
        let positions = Arc::new(compute_absolute_positions(&transformed_ir));
        debug!("Cached {} node positions for {}", positions.len(), uri);

        // Symbol index builder for global pattern-based lookups (needs positions)
        // MUST use transformed_ir because positions HashMap is keyed by transformed_ir node addresses.
        // SymbolTableBuilder.with_metadata() creates new Arc allocations, so ir and transformed_ir
        // have different memory addresses. Using ir would cause position lookups to fail.
        let mut index_builder = SymbolIndexBuilder::new(global_index.clone(), uri.clone(), positions.clone());
        index_builder.index_tree(&transformed_ir);
        let inverted_index = builder.get_inverted_index();
        let potential_global_refs = builder.get_potential_global_refs();
        let symbol_table = transformed_ir.metadata()
            .and_then(|m| m.get("symbol_table"))
            .map(|st| Arc::clone(st.downcast_ref::<Arc<SymbolTable>>().unwrap()))
            .unwrap_or_else(|| {
                debug!("No symbol table found on root for {}, using default empty table", uri);
                Arc::new(SymbolTable::new(Some(global_table.clone())))
            });
        builder.resolve_local_potentials(&symbol_table);
        let version = self.version_counter.fetch_add(1, Ordering::SeqCst);

        debug!("Processed document {}: {} symbols, {} usages, version {}",
            uri, symbol_table.collect_all_symbols().len(), inverted_index.len(), version);

        let mut contracts = Vec::new();
        let mut calls = Vec::new();
        collect_contracts(&transformed_ir, &mut contracts);
        collect_calls(&transformed_ir, &mut calls);
        debug!("Collected {} contracts and {} calls in {}", contracts.len(), calls.len(), uri);

        // Detect language and create UnifiedIR
        let language = crate::lsp::models::DocumentLanguage::from_uri(uri);
        let unified_ir: Arc<dyn crate::ir::semantic_node::SemanticNode> = match language {
            crate::lsp::models::DocumentLanguage::Rholang | crate::lsp::models::DocumentLanguage::Unknown => {
                // Convert RholangNode to UnifiedIR
                use crate::ir::unified_ir::UnifiedIR;
                UnifiedIR::from_rholang(&transformed_ir)
            }
            crate::lsp::models::DocumentLanguage::Metta => {
                // MeTTa support not yet implemented - for now, just wrap as UnifiedIR
                // TODO: Implement MeTTa parsing and conversion
                use crate::ir::unified_ir::UnifiedIR;
                use crate::ir::semantic_node::{NodeBase, RelativePosition};
                Arc::new(UnifiedIR::Error {
                    base: NodeBase::new(
                        RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 },
                        0,
                        0,
                        0
                    ),
                    message: "MeTTa support not yet implemented".to_string(),
                    children: Vec::new(),
                    metadata: None,
                }) as Arc<dyn crate::ir::semantic_node::SemanticNode>
            }
        };
        debug!("Created UnifiedIR for {} (language: {:?})", uri, language);

        // Build suffix array-based symbol index for O(m log n + k) substring search
        let workspace_symbols = crate::ir::transforms::document_symbol_visitor::collect_workspace_symbols(&symbol_table, uri);
        let symbol_index = Arc::new(crate::lsp::symbol_index::SymbolIndex::new(workspace_symbols));
        debug!("Built suffix array index for {} symbols in {}", symbol_index.len(), uri);

        Ok(CachedDocument {
            ir: transformed_ir,
            metta_ir: None, // Will be populated for MeTTa files
            unified_ir,
            language,
            tree: Arc::new(parse_code("")), // Tree is not used for text, but keep for other
            symbol_table,
            inverted_index,
            version,
            text: text.clone(),
            positions,
            potential_global_refs,
            symbol_index,
            content_hash,
        })
    }

    /// Indexes a document by parsing its text and processing it, using an existing syntax tree if provided for incremental updates.
    async fn index_file(
        &self,
        uri: &Url,
        text: &str,
        _version: i32,
        tree: Option<tree_sitter::Tree>,
    ) -> Result<CachedDocument, String> {
        use crate::lsp::models::DocumentLanguage;
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        // Compute fast hash of content for change detection
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        let content_hash = hasher.finish();

        // Check if we already have this exact content indexed
        // Note: We can't early-return here because we need to re-index to update workspace state
        // However, we can log the hash check for debugging
        if let Some(existing) = self.workspace.read().await.documents.get(uri) {
            if existing.content_hash == content_hash {
                debug!("Content unchanged for {} (hash: {}), but reindexing to update workspace", uri, content_hash);
            } else {
                debug!("Reindexing {} - content changed (old hash: {}, new hash: {})",
                    uri, existing.content_hash, content_hash);
            }
        }

        // Detect language from file extension
        let language = DocumentLanguage::from_uri(uri);

        // Route to appropriate parser based on language
        match language {
            DocumentLanguage::Metta => {
                // Handle MeTTa files
                self.index_metta_file(uri, text, _version, content_hash).await
            }
            DocumentLanguage::Rholang | DocumentLanguage::Unknown => {
                // Handle Rholang files (existing logic)
                let uri_clone = uri.clone();
                let mut workspace = self.workspace.write().await;
                workspace.global_table.symbols.write().unwrap().retain(|_, s| &s.declaration_uri != &uri_clone);
                let mut global_symbols = workspace.global_symbols.clone();
                global_symbols.retain(|_, (u, _)| u != &uri_clone);
                workspace.global_symbols = global_symbols;
                workspace.global_inverted_index.retain(|(d_uri, _), us| {
                    if d_uri == &uri_clone {
                        false
                    } else {
                        us.retain(|(u_uri, _)| u_uri != &uri_clone);
                        !us.is_empty()
                    }
                });
                workspace.global_contracts.retain(|(u, _)| u != &uri_clone);
                workspace.global_calls.retain(|(u, _)| u != &uri_clone);
                drop(workspace);

                let tree = Arc::new(tree.unwrap_or_else(|| parse_code(text)));
                let rope = Rope::from_str(text);
                let ir = parse_to_ir(&tree, &rope);
                let cached = self.process_document(ir, uri, &rope, content_hash).await?;

                // Scan for embedded language regions using multiple detection methods
                let mut all_regions = Vec::new();

                // 1. Comment directive detection (e.g., // @metta)
                let directive_regions = DirectiveParser::scan_directives(text, &tree, &rope);
                debug!("Found {} regions via comment directives", directive_regions.len());
                all_regions.extend(directive_regions);

                // 2. Semantic detection (e.g., strings sent to @"rho:metta:compile")
                let semantic_regions = SemanticDetector::detect_regions(text, &tree, &rope);
                debug!("Found {} regions via semantic analysis", semantic_regions.len());

                // Merge semantic regions, avoiding duplicates
                // (directive regions take precedence if there's overlap)
                for semantic_region in semantic_regions {
                    // Check if this region overlaps with any directive region
                    let overlaps = all_regions.iter().any(|r| {
                        (semantic_region.start_byte >= r.start_byte && semantic_region.start_byte < r.end_byte)
                            || (semantic_region.end_byte > r.start_byte && semantic_region.end_byte <= r.end_byte)
                            || (semantic_region.start_byte <= r.start_byte && semantic_region.end_byte >= r.end_byte)
                    });

                    if !overlaps {
                        all_regions.push(semantic_region);
                    }
                }

                // 3. Channel flow analysis (e.g., variables bound to compiler channels)
                let flow_regions = ChannelFlowAnalyzer::analyze(text, &tree, &rope);
                debug!("Found {} regions via channel flow analysis", flow_regions.len());

                // Merge flow regions, avoiding duplicates
                for flow_region in flow_regions {
                    let overlaps = all_regions.iter().any(|r| {
                        (flow_region.start_byte >= r.start_byte && flow_region.start_byte < r.end_byte)
                            || (flow_region.end_byte > r.start_byte && flow_region.end_byte <= r.end_byte)
                            || (flow_region.start_byte <= r.start_byte && flow_region.end_byte >= r.end_byte)
                    });

                    if !overlaps {
                        all_regions.push(flow_region);
                    }
                }

                if !all_regions.is_empty() {
                    debug!("Total {} embedded language regions detected in {}", all_regions.len(), uri);
                    let mut virtual_docs = self.virtual_docs.write().await;
                    virtual_docs.register_regions(uri, &all_regions);

                    // Validate virtual documents and get diagnostics
                    // Note: We don't publish diagnostics here; that's done in validate()
                    let _virtual_diagnostics = virtual_docs.validate_all_for_parent(uri);
                    debug!("Validated {} virtual documents for {}", all_regions.len(), uri);
                }

                let mut workspace = self.workspace.write().await;
                let mut contracts = Vec::new();
                collect_contracts(&cached.ir, &mut contracts);
                let mut calls = Vec::new();
                collect_calls(&cached.ir, &mut calls);
                workspace.global_contracts.extend(contracts.into_iter().map(|c| (uri.clone(), c)));
                workspace.global_calls.extend(calls.into_iter().map(|c| (uri.clone(), c)));

                // Broadcast workspace change event (ReactiveX Phase 2)
                let file_count = workspace.documents.len();
                let symbol_count = workspace.global_symbols.len();
                drop(workspace); // Release lock before broadcasting

                let _ = self.workspace_changes.send(WorkspaceChangeEvent {
                    file_count,
                    symbol_count,
                    change_type: WorkspaceChangeType::FileIndexed,
                });

                Ok(cached)
            }
        }
    }

    /// Indexes a MeTTa file by parsing and creating a cached document
    async fn index_metta_file(
        &self,
        uri: &Url,
        text: &str,
        version: i32,
        content_hash: u64,
    ) -> Result<CachedDocument, String> {
        use crate::parsers::MettaParser;
        use crate::lsp::models::DocumentLanguage;
        use crate::ir::semantic_node::{NodeBase, RelativePosition};
        use crate::ir::unified_ir::UnifiedIR;

        debug!("Indexing MeTTa file: {}", uri);

        // Parse MeTTa source to IR
        let mut parser = MettaParser::new()
            .map_err(|e| format!("Failed to create MeTTa parser: {}", e))?;
        let metta_nodes = parser.parse_to_ir(text)
            .map_err(|e| format!("Failed to parse MeTTa file: {}", e))?;

        debug!("Parsed {} MeTTa expressions", metta_nodes.len());

        // Create a placeholder RholangNode for the ir field
        // This is temporary - in future we'll refactor CachedDocument to use Arc<dyn SemanticNode>
        let placeholder_ir = Arc::new(crate::ir::rholang_node::RholangNode::Nil {
            base: NodeBase::new(
                RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 },
                text.len(),
                0,
                text.len(),
            ),
            metadata: None,
        });

        // Create unified IR from first MeTTa node (or error if empty)
        let unified_ir: Arc<dyn crate::ir::semantic_node::SemanticNode> = if let Some(first_node) = metta_nodes.first() {
            use crate::ir::semantic_node::SemanticNode;
            Arc::new(UnifiedIR::MettaExt {
                base: first_node.as_ref().base().clone(),
                node: first_node.clone() as Arc<dyn std::any::Any + Send + Sync>,
                metadata: None,
            })
        } else {
            Arc::new(UnifiedIR::Error {
                base: NodeBase::new(
                    RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 },
                    0,
                    0,
                    0,
                ),
                message: "Empty MeTTa file".to_string(),
                children: vec![],
                metadata: None,
            })
        };

        // Create empty symbol table and inverted index for now
        // TODO: Implement symbol table building for MeTTa
        let global_table = self.workspace.read().await.global_table.clone();
        let symbol_table = Arc::new(crate::ir::symbol_table::SymbolTable::new(Some(global_table)));
        let inverted_index = std::collections::HashMap::new();
        let potential_global_refs = Vec::new();

        // Create empty symbol index
        let symbol_index = Arc::new(crate::lsp::symbol_index::SymbolIndex::new(Vec::new()));

        let rope = Rope::from_str(text);
        let positions = Arc::new(std::collections::HashMap::new());

        let cached_doc = CachedDocument {
            ir: placeholder_ir,
            metta_ir: Some(metta_nodes),
            unified_ir,
            language: DocumentLanguage::Metta,
            tree: Arc::new(parse_code("")), // Placeholder tree
            symbol_table,
            inverted_index,
            version,
            text: rope,
            positions,
            potential_global_refs,
            symbol_index,
            content_hash,
        };

        // Broadcast workspace change event (ReactiveX Phase 2)
        let workspace = self.workspace.read().await;
        let file_count = workspace.documents.len();
        let symbol_count = workspace.global_symbols.len();
        drop(workspace); // Release lock before broadcasting

        let _ = self.workspace_changes.send(WorkspaceChangeEvent {
            file_count,
            symbol_count,
            change_type: WorkspaceChangeType::FileIndexed,
        });

        Ok(cached_doc)
    }

    /// Handles file system events by re-indexing changed .rho files that are not open.
    async fn handle_file_change(&self, path: PathBuf) {
        if path.extension().map_or(false, |ext| ext == "rho") {
            if let Ok(uri) = Url::from_file_path(&path) {
                if self.documents_by_uri.read().await.contains_key(&uri) {
                    debug!("Skipping update for opened document: {}", uri);
                    return;
                }
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                match self.index_file(&uri, &text, 0, None).await {
                    Ok(cached_doc) => {
                        self.workspace.write().await.documents.insert(uri.clone(), Arc::new(cached_doc));
                        self.link_symbols().await;
                        info!("Updated cache for file: {}", uri);
                    }
                    Err(e) => warn!("Failed to index file {}: {}", uri, e),
                }
            }
        }
    }

    /// Indexes all .rho files in the given directory (non-recursively).
    async fn index_directory(&self, dir: &Path) {
        for result in WalkDir::new(dir) {
            match result {
                Ok(entry) => {
                    if entry.file_type().is_file() && entry.path().extension().map_or(false, |ext| ext == "rho") {
                        let uri = Url::from_file_path(entry.path()).expect("Failed to create URI from path");
                        if !self.documents_by_uri.read().await.contains_key(&uri)
                            && !self.workspace.read().await.documents.contains_key(&uri) {
                            if let Ok(text) = std::fs::read_to_string(entry.path()) {
                                match self.index_file(&uri, &text, 0, None).await {
                                    Ok(cached_doc) => {
                                        self.workspace.write().await.documents.insert(uri.clone(), Arc::new(cached_doc));
                                        debug!("Indexed file: {}", uri);
                                    }
                                    Err(e) => warn!("Failed to index file {}: {}", uri, e),
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to read directory {:?} for sibling indexing: {}", dir, e);
                }
            }
        }
        self.link_symbols().await;
    }

    /// Generates the next unique document ID.
    fn next_document_id(&self) -> u32 {
        self.serial_document_id.fetch_add(1, Ordering::SeqCst)
    }

    /// Validates the document text locally and remotely, returning diagnostics if any issues are found.
    async fn validate(
        &self,
        document: Arc<LspDocument>,
        text: &str,
        version: i32
    ) -> Result<Vec<Diagnostic>, String> {
        let state = document.state.read().await;
        if state.version != version {
            debug!("Skipping validation for outdated version {} (current: {})",
                   version, state.version);
            return Ok(Vec::new());
        }

        // Detect language and route to appropriate validator
        use crate::lsp::models::DocumentLanguage;
        let language = DocumentLanguage::from_uri(&state.uri);

        if language == DocumentLanguage::Metta {
            // Validate MeTTa file
            use crate::validators::MettaValidator;
            debug!("Validating MeTTa file: {}", state.uri);
            let validator = MettaValidator::new();
            let diagnostics = validator.validate(text);
            return Ok(diagnostics);
        }

        // Local validation with parser reuse for semantic validation (Rholang)
        let parser = RholangParser::new();
        let parse_result = parser.parse(&text);

        let (local_diagnostics, parsed_ast) = match parse_result {
            Validated::Good(procs) => {
                debug!("Syntax validation successful for code snippet");
                // Keep the parsed AST for semantic validation
                (Vec::new(), Some(procs))
            }
            Validated::Fail(failures) => {
                let total_errors: usize = failures.iter().map(|f| f.errors.len().get()).sum();
                error!("Syntax validation failed with {} errors", total_errors);
                let diagnostics = failures.into_iter().flat_map(|failure| {
                    failure.errors.into_iter().map(|err| {
                        let range = Range {
                            start: LspPosition {
                                line: (err.span.start.line - 1) as u32,
                                character: (err.span.start.col - 1) as u32,
                            },
                            end: LspPosition {
                                line: (err.span.end.line - 1) as u32,
                                character: (err.span.end.col - 1) as u32,
                            },
                        };
                        let message = match err.error {
                            ParsingError::SyntaxError { sexp } => format!("Syntax error: {}", sexp),
                            ParsingError::MissingToken(token) => format!("Missing token: {}", token),
                            ParsingError::Unexpected(c) => format!("Unexpected character: {}", c),
                            ParsingError::UnexpectedVar => "Unexpected variable".to_string(),
                            ParsingError::UnexpectedMatchAfter { rule, offender } => format!("Unexpected {} after {}", offender, rule),
                            ParsingError::NumberOutOfRange => "Number out of range".to_string(),
                            ParsingError::DuplicateNameDecl { first, second } => format!("Duplicate name declaration at {} and {}", first, second),
                            ParsingError::MalformedLetDecl { lhs_arity, rhs_arity } => format!("Malformed let declaration: LHS arity {} != RHS arity {}", lhs_arity, rhs_arity),
                            ParsingError::UnexpectedQuote => "Unexpected quote character".to_string(),
                        };
                        Diagnostic {
                            range,
                            severity: Some(DiagnosticSeverity::ERROR),
                            source: Some("rholang-parser".to_string()),
                            message,
                            ..Default::default()
                        }
                    }).collect::<Vec<_>>()
                }).collect::<Vec<_>>();
                (diagnostics, None)
            }
        };

        // Semantic validation (if no syntax errors)
        if local_diagnostics.is_empty() {
            // OPTIMIZATION: If using Rust backend and have pre-parsed AST, use validate_parsed to avoid re-parsing
            if let Some(validator) = &self.semantic_validator {
                if let Some(procs) = parsed_ast {
                    if procs.len() == 1 {
                        debug!("Running optimized semantic validation with pre-parsed AST for URI={}", state.uri);
                        let ast = procs.into_iter().next().unwrap();
                        let semantic_diagnostics = validator.validate_parsed(ast, &parser);
                        if !semantic_diagnostics.is_empty() {
                            info!("Semantic validation found {} errors for URI={} (version={})",
                                  semantic_diagnostics.len(), state.uri, version);
                            let all_diags = self.aggregate_with_virtual_diagnostics(&state.uri, semantic_diagnostics).await;
                            return Ok(all_diags);
                        }
                        debug!("Semantic validation passed for URI={}", state.uri);
                        let all_diags = self.aggregate_with_virtual_diagnostics(&state.uri, vec![]).await;
                        return Ok(all_diags);
                    } else {
                        // Multiple procs - validate each one separately
                        let num_procs = procs.len();
                        debug!("Multiple top-level processes detected ({}), validating each separately", num_procs);
                        let mut all_diagnostics = Vec::new();
                        for ast in &procs {
                            let diagnostics = validator.validate_parsed(*ast, &parser);
                            all_diagnostics.extend(diagnostics);
                        }
                        if !all_diagnostics.is_empty() {
                            info!("Semantic validation found {} errors across {} processes for URI={} (version={})",
                                  all_diagnostics.len(), num_procs, state.uri, version);
                            let final_diags = self.aggregate_with_virtual_diagnostics(&state.uri, all_diagnostics).await;
                            return Ok(final_diags);
                        }
                        debug!("Semantic validation passed for all {} processes", num_procs);
                        let final_diags = self.aggregate_with_virtual_diagnostics(&state.uri, vec![]).await;
                        return Ok(final_diags);
                    }
                }
            }

            // Use generic diagnostic provider (works for both Rust and gRPC backends)
            debug!("Running semantic validation via {} backend for URI={}",
                   self.diagnostic_provider.backend_name(), state.uri);
            let semantic_diagnostics = self.diagnostic_provider.validate(text).await;

            if !semantic_diagnostics.is_empty() {
                info!("{} validation found {} errors for URI={} (version={})",
                      self.diagnostic_provider.backend_name(),
                      semantic_diagnostics.len(), state.uri, version);
            } else {
                debug!("{} validation passed for URI={}",
                       self.diagnostic_provider.backend_name(), state.uri);
            }

            let all_diags = self.aggregate_with_virtual_diagnostics(&state.uri, semantic_diagnostics).await;
            Ok(all_diags)
        } else {
            // Return syntax errors if present
            debug!("Syntax errors found for URI={}, skipping semantic validation", state.uri);
            let all_diags = self.aggregate_with_virtual_diagnostics(&state.uri, local_diagnostics).await;
            Ok(all_diags)
        }
    }

    /// Aggregates diagnostics from parent document and virtual documents
    async fn aggregate_with_virtual_diagnostics(
        &self,
        uri: &Url,
        mut parent_diagnostics: Vec<Diagnostic>,
    ) -> Vec<Diagnostic> {
        let mut virtual_docs = self.virtual_docs.write().await;
        let virtual_diagnostics = virtual_docs.validate_all_for_parent(uri);
        if !virtual_diagnostics.is_empty() {
            debug!("Adding {} diagnostics from virtual documents", virtual_diagnostics.len());
            parent_diagnostics.extend(virtual_diagnostics);
        }
        parent_diagnostics
    }

    /// Looks up the IR node, its symbol table, and inverted index at a given position in the document.
    pub async fn lookup_node_at_position(&self, uri: &Url, position: IrPosition) -> Option<(Arc<RholangNode>, Arc<SymbolTable>, InvertedIndex)> {
        let opt_doc = {
            debug!("Acquiring workspace read lock for symbol at {}:{:?}", uri, position);
            let workspace = self.workspace.read().await;
            debug!("Workspace read lock acquired for {}:{:?}", uri, position);
            workspace.documents.get(uri).cloned()
        };
        if let Some(doc) = opt_doc {
            if let Some(node) = find_node_at_position(&doc.ir, &*doc.positions, position) {
                let symbol_table = node.metadata()
                    .and_then(|m| m.get("symbol_table"))
                    .and_then(|t| t.downcast_ref::<Arc<SymbolTable>>())
                    .cloned()
                    .unwrap_or_else(|| doc.symbol_table.clone());
                return Some((node, symbol_table, doc.inverted_index.clone()));
            }
        }
        None
    }

    fn position_to_range(position: IrPosition, name_len: usize) -> Range {
        Range {
            start: LspPosition {
                line: position.row as u32,
                character: position.column as u32,
            },
            end: LspPosition {
                line: position.row as u32,
                character: (position.column + name_len) as u32,
            },
        }
    }

    /// Retrieves the symbol at the specified LSP position in the document.
    /// Retrieves all occurrences of the symbol, including declaration (if requested), definition (if distinct), and usages.
    async fn get_symbol_references(&self, symbol: &Symbol, include_declaration: bool) -> Vec<(Url, Range)> {
        let mut locations = Vec::new();
        let decl_uri = symbol.declaration_uri.clone();
        let name_len = symbol.name.len();

        // Add declaration location
        let decl_pos = symbol.declaration_location;
        let decl_range = Self::position_to_range(decl_pos, name_len);
        if include_declaration {
            locations.push((decl_uri.clone(), decl_range));
            debug!("Added declaration of '{}' at {}:{:?}", symbol.name, decl_uri, decl_pos);
        }

        // Add definition location if it exists and differs from declaration
        if let Some(def_pos) = symbol.definition_location {
            if def_pos != decl_pos {
                let def_range = Self::position_to_range(def_pos, name_len);
                locations.push((decl_uri.clone(), def_range));
                debug!("Added definition of '{}' at {}:{:?}", symbol.name, decl_uri, def_pos);
            }
        }

        let workspace = self.workspace.read().await;

        // Add local usages from the declaration document
        if let Some(decl_doc) = workspace.documents.get(&decl_uri) {
            if let Some(usages) = decl_doc.inverted_index.get(&decl_pos) {
                for &usage_pos in usages {
                    let range = Self::position_to_range(usage_pos, name_len);
                    locations.push((decl_uri.clone(), range));
                    debug!("Added local usage of '{}' at {}:{:?}", symbol.name, decl_uri, usage_pos);
                }
            }
        }

        // Add global usages if the symbol is a contract
        if symbol.symbol_type == SymbolType::Contract {
            if let Some(global_usages) = workspace.global_inverted_index.get(&(decl_uri.clone(), decl_pos)) {
                for &(ref use_uri, use_pos) in global_usages {
                    let range = Self::position_to_range(use_pos, name_len);
                    locations.push((use_uri.clone(), range));
                    debug!("Added global usage of '{}' at {}:{:?}", symbol.name, use_uri, use_pos);
                }
            }
        }

        locations
    }

    /// Computes the byte offset from a line and character position in the source text.
    pub fn byte_offset_from_position(text: &Rope, line: usize, character: usize) -> Option<usize> {
        // Check if line is within bounds
        if line >= text.len_lines() {
            debug!("Line {} out of bounds (rope has {} lines)", line, text.len_lines());
            return None;
        }

        text.try_line_to_byte(line).ok().map(|line_start_byte| {
            let line_text = text.line(line);
            let char_offset = character.min(line_text.len_chars());
            let byte_in_line = line_text.char_to_byte(char_offset);
            let total_byte = line_start_byte + byte_in_line;
            debug!("byte_offset_from_position: line={}, character={}, line_start_byte={}, char_offset={}, byte_in_line={}, total_byte={}, line_text={:?}, total_text_len={}",
                line, character, line_start_byte, char_offset, byte_in_line, total_byte, line_text.to_string(), text.len_bytes());
            total_byte
        })
    }
}
