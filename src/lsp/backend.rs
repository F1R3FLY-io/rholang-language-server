use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, Sender};

use tokio::sync::{Mutex as AsyncMutex, RwLock};
use tokio::task;

use tonic::Request;
use tonic::transport::Channel;

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
    SymbolInformation,
};
use tower_lsp::lsp_types::request::{GotoDeclarationParams, GotoDeclarationResponse};
use tower_lsp::jsonrpc::Result as LspResult;

use tracing::{debug, error, info, trace, warn};

use ropey::Rope;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use walkdir::WalkDir;

use crate::ir::pipeline::Pipeline;
use crate::ir::node::{Node, Position as IrPosition, compute_absolute_positions, collect_contracts, collect_calls, match_contract, find_node_at_position_with_path, find_node_at_position};
use crate::ir::symbol_table::{Symbol, SymbolTable, SymbolType};
use crate::ir::transforms::symbol_table_builder::{SymbolTableBuilder, InvertedIndex};
use crate::ir::transforms::document_symbol_visitor::{collect_document_symbols, collect_workspace_symbols};
use crate::lsp::models::{CachedDocument, LspDocument, LspDocumentHistory, LspDocumentState, WorkspaceState};
use crate::lsp::semantic_validator::SemanticValidator;
use crate::lsp::diagnostic_provider::{BackendConfig, DiagnosticProvider, create_provider};
use crate::lsp::rust_validator::RustSemanticValidator;
use crate::tree_sitter::{parse_code, parse_to_ir};

use rholang_parser::RholangParser;
use rholang_parser::parser::errors::ParsingError;
use validated::Validated;

/// Document change event for debouncing
#[derive(Debug, Clone)]
struct DocumentChangeEvent {
    uri: Url,
    version: i32,
    document: Arc<LspDocument>,
    text: Arc<String>,
}

/// Workspace indexing task for progressive indexing
#[derive(Debug, Clone, Eq, PartialEq)]
struct IndexingTask {
    uri: Url,
    text: String,
    priority: u8,  // 0 = high (current file), 1 = normal
}

/// The Rholang language server backend, managing state and handling LSP requests.
#[derive(Clone)]
pub struct RholangBackend {
    client: Client,
    documents_by_uri: Arc<RwLock<HashMap<Url, Arc<LspDocument>>>>,
    documents_by_id: Arc<RwLock<HashMap<u32, Arc<LspDocument>>>>,
    serial_document_id: Arc<AtomicU32>,
    /// Pluggable diagnostic provider (Rust interpreter or gRPC to RNode)
    diagnostic_provider: Arc<Box<dyn DiagnosticProvider>>,
    /// Direct access to SemanticValidator for validate_parsed optimization (if using Rust backend)
    semantic_validator: Option<SemanticValidator>,
    client_process_id: Arc<Mutex<Option<u32>>>,
    pid_channel: Option<tokio::sync::mpsc::Sender<u32>>,
    // Reactive channels
    doc_change_tx: tokio::sync::mpsc::Sender<DocumentChangeEvent>,
    validation_cancel: Arc<Mutex<HashMap<Url, tokio::sync::oneshot::Sender<()>>>>,
    indexing_tx: tokio::sync::mpsc::Sender<IndexingTask>,
    workspace: Arc<RwLock<WorkspaceState>>,
    file_watcher: Arc<Mutex<Option<RecommendedWatcher>>>,
    file_events: Arc<Mutex<Receiver<notify::Result<notify::Event>>>>,
    file_sender: Arc<Mutex<Sender<notify::Result<notify::Event>>>>,
    version_counter: Arc<AtomicI32>,
    root_dir: Arc<RwLock<Option<PathBuf>>>,
    shutdown_tx: Arc<tokio::sync::broadcast::Sender<()>>,
}

// Manual Debug implementation since DiagnosticProvider doesn't implement Debug
impl std::fmt::Debug for RholangBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RholangBackend")
            .field("backend", &self.diagnostic_provider.backend_name())
            .field("documents_count", &"<HashMap>")
            .finish()
    }
}

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
            })),
            file_watcher: Arc::new(Mutex::new(None)),
            file_events: Arc::new(Mutex::new(rx)),
            file_sender: Arc::new(Mutex::new(tx)),
            version_counter: Arc::new(AtomicI32::new(0)),
            root_dir: Arc::new(RwLock::new(None)),
            shutdown_tx: Arc::new(shutdown_tx),
        };

        // Spawn document change debouncer
        Self::spawn_document_debouncer(backend.clone(), doc_change_rx);

        // Spawn progressive indexer
        Self::spawn_progressive_indexer(backend.clone(), indexing_rx);

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
    async fn process_document(&self, ir: Arc<Node>, uri: &Url, text: &Rope) -> Result<CachedDocument, String> {
        let mut pipeline = Pipeline::new();
        let global_table = self.workspace.read().await.global_table.clone();
        let builder = Arc::new(SymbolTableBuilder::new(ir.clone(), uri.clone(), global_table.clone()));
        pipeline.add_transform(crate::ir::pipeline::Transform {
            id: "symbol_table_builder".to_string(),
            dependencies: vec![],
            visitor: builder.clone(),
        });
        let transformed_ir = pipeline.apply(&ir);
        let positions = Arc::new(compute_absolute_positions(&transformed_ir));
        debug!("Cached {} node positions for {}", positions.len(), uri);
        let inverted_index = builder.get_inverted_index();
        let potential_global_refs = builder.get_potential_global_refs();
        let symbol_table = transformed_ir.metadata()
            .and_then(|m| m.data.get("symbol_table"))
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

        Ok(CachedDocument {
            ir: transformed_ir,
            tree: Arc::new(parse_code("")), // Tree is not used for text, but keep for other
            symbol_table,
            inverted_index,
            version,
            text: text.clone(),
            positions,
            potential_global_refs,
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
        let cached = self.process_document(ir, uri, &rope).await?;
        let mut workspace = self.workspace.write().await;
        let mut contracts = Vec::new();
        collect_contracts(&cached.ir, &mut contracts);
        let mut calls = Vec::new();
        collect_calls(&cached.ir, &mut calls);
        workspace.global_contracts.extend(contracts.into_iter().map(|c| (uri.clone(), c)));
        workspace.global_calls.extend(calls.into_iter().map(|c| (uri.clone(), c)));
        Ok(cached)
    }

    /// Links contract symbols across all documents in the workspace for cross-file resolution.
    async fn link_symbols(&self) {
        let mut workspace = self.workspace.write().await;
        let mut global_symbols = HashMap::new();
        let documents = workspace.documents.clone();
        for (_uri, doc) in &documents {
            for symbol in doc.symbol_table.collect_all_symbols() {
                if matches!(symbol.symbol_type, SymbolType::Contract) {
                    global_symbols.insert(symbol.name.clone(), (symbol.declaration_uri.clone(), symbol.declaration_location));
                }
            }
        }
        workspace.global_symbols = global_symbols;
        info!("Linked symbols across {} files", documents.len());

        // Resolve potentials
        let mut resolutions = Vec::new();
        for (doc_uri, doc) in &documents {
            for (name, use_pos) in &doc.potential_global_refs {
                if let Some((def_uri, def_pos)) = workspace.global_symbols.get(name).cloned() {
                    if (doc_uri.clone(), *use_pos) != (def_uri.clone(), def_pos) {
                        resolutions.push(((def_uri, def_pos), (doc_uri.clone(), *use_pos)));
                        trace!("Resolved potential global usage of '{}' at {:?} to def at {:?}", name, use_pos, def_pos);
                    } else {
                        trace!("Skipping self-reference potential for '{}' at {:?}", name, use_pos);
                    }
                }
            }
        }
        for ((def_uri, def_pos), (use_uri, use_pos)) in resolutions {
            workspace.global_inverted_index
                .entry((def_uri, def_pos))
                .or_insert_with(Vec::new)
                .push((use_uri, use_pos));
        }

        // No additional linking for contracts/calls, as linear match is used
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

        // Local validation with parser reuse for semantic validation
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
                            return Ok(semantic_diagnostics);
                        }
                        debug!("Semantic validation passed for URI={}", state.uri);
                        return Ok(vec![]);
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
                            return Ok(all_diagnostics);
                        }
                        debug!("Semantic validation passed for all {} processes", num_procs);
                        return Ok(vec![]);
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

            Ok(semantic_diagnostics)
        } else {
            // Return syntax errors if present
            debug!("Syntax errors found for URI={}, skipping semantic validation", state.uri);
            Ok(local_diagnostics)
        }
    }

    /// Looks up the IR node, its symbol table, and inverted index at a given position in the document.
    pub async fn lookup_node_at_position(&self, uri: &Url, position: IrPosition) -> Option<(Arc<Node>, Arc<SymbolTable>, InvertedIndex)> {
        let opt_doc = {
            debug!("Acquiring workspace read lock for symbol at {}:{:?}", uri, position);
            let workspace = self.workspace.read().await;
            debug!("Workspace read lock acquired for {}:{:?}", uri, position);
            workspace.documents.get(uri).cloned()
        };
        if let Some(doc) = opt_doc {
            if let Some(node) = find_node_at_position(&doc.ir, &*doc.positions, position) {
                let symbol_table = node.metadata()
                    .and_then(|m| m.data.get("symbol_table"))
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
    async fn get_symbol_at_position(&self, uri: &Url, position: LspPosition) -> Option<Arc<Symbol>> {
        let opt_doc = {
            debug!("Acquiring workspace read lock for symbol at {}:{:?}", uri, position);
            let workspace = self.workspace.read().await;
            debug!("Workspace read lock acquired for {}:{:?}", uri, position);
            workspace.documents.get(uri).cloned()
        };
        if let Some(doc) = opt_doc {
            let text = &doc.text;
            let byte_offset = Self::byte_offset_from_position(text, position.line as usize, position.character as usize);
            if let Some(byte) = byte_offset {
                let pos = IrPosition {
                    row: position.line as usize,
                    column: position.character as usize,
                    byte,
                };

                // Get node with path for parent checking
                let (node_path_opt, symbol_table_opt) = {
                    let opt_doc = {
                        let workspace = self.workspace.read().await;
                        workspace.documents.get(&uri).cloned()
                    };
                    if let Some(doc) = opt_doc {
                        let path_result = find_node_at_position_with_path(&doc.ir, &*doc.positions, pos);
                        let symbol_table = path_result.as_ref().and_then(|(node, _)| {
                            node.metadata()
                                .and_then(|m| m.data.get("symbol_table"))
                                .and_then(|t| t.downcast_ref::<Arc<SymbolTable>>())
                                .cloned()
                        }).unwrap_or_else(|| doc.symbol_table.clone());
                        (path_result, Some(symbol_table))
                    } else {
                        (None, None)
                    }
                };

                if let (Some((node, path)), Some(symbol_table)) = (node_path_opt, symbol_table_opt) {
                    debug!("Node at position: {}", match &*node {
                        Node::Var {..} => "Var",
                        Node::Contract {..} => "Contract",
                        Node::Send {..} => "Send",
                        Node::SendSync {..} => "SendSync",
                        Node::Par {..} => "Par",
                        Node::New {..} => "New",
                        Node::Bundle {..} => "Bundle",
                        Node::Match {..} => "Match",
                        _ => "Other"
                    });
                    match &*node {
                        Node::Var { name, .. } => {
                            // Check if this Var is the name of a Contract (path should be [..., Contract, Var])
                            if path.len() >= 2 {
                                if let Node::Contract { name: contract_name, .. } = &*path[path.len() - 2] {
                                    if Arc::ptr_eq(contract_name, &node) {
                                        // This Var is a contract name - handle as global symbol
                                        debug!("Var '{}' is a contract name", name);
                                        let workspace = self.workspace.read().await;
                                        if let Some((def_uri, def_pos)) = workspace.global_symbols.get(name).cloned() {
                                            debug!("Found global contract symbol '{}' at {}:{} in {}",
                                                name, position.line, position.character, uri);
                                            return Some(Arc::new(Symbol {
                                                name: name.to_string(),
                                                symbol_type: SymbolType::Contract,
                                                declaration_uri: def_uri.clone(),
                                                declaration_location: def_pos,
                                                definition_location: Some(def_pos),
                                            }));
                                        }
                                    }
                                }
                            }

                            // Handle regular variables
                            if let Some(symbol) = symbol_table.lookup(name) {
                                debug!("Found symbol '{}' at {}:{} in {}",
                                    name, position.line, position.character, uri);
                                return Some(symbol);
                            } else {
                                // Search global symbols for unbound references
                                let workspace = self.workspace.read().await;
                                if let Some((def_uri, def_pos)) = workspace.global_symbols.get(name).cloned() {
                                    debug!("Found global symbol '{}' for unbound reference at {}:{} in {}",
                                        name, position.line, position.character, uri);
                                    return Some(Arc::new(Symbol {
                                        name: name.to_string(),
                                        symbol_type: SymbolType::Contract,
                                        declaration_uri: def_uri.clone(),
                                        declaration_location: def_pos,
                                        definition_location: Some(def_pos),
                                    }));
                                } else {
                                    debug!("Symbol '{}' at {}:{} in {} not found in symbol table or global",
                                        name, position.line, position.character, uri);
                                }
                            }
                        }
                        Node::Contract { name, .. } => {
                            // Handle contract declarations
                            if let Node::Var { name: contract_name, .. } = &**name {
                                let workspace = self.workspace.read().await;
                                if let Some((def_uri, def_pos)) = workspace.global_symbols.get(contract_name).cloned() {
                                    debug!("Found contract symbol '{}' at {}:{} in {}",
                                        contract_name, position.line, position.character, uri);
                                    return Some(Arc::new(Symbol {
                                        name: contract_name.to_string(),
                                        symbol_type: SymbolType::Contract,
                                        declaration_uri: def_uri.clone(),
                                        declaration_location: def_pos,
                                        definition_location: Some(def_pos),
                                    }));
                                }
                            }
                        }
                        Node::Send { channel, inputs, .. } | Node::SendSync { channel, inputs, .. } => {
                            // Handle contract calls like foo!(42) and positions on send inputs
                            let workspace = self.workspace.read().await;
                            if let Some(doc) = workspace.documents.get(&uri) {
                                // First check if position is within the channel node
                                let channel_key = &**channel as *const Node as usize;
                                if let Some(&(ch_start, ch_end)) = doc.positions.get(&channel_key) {
                                    debug!("Send channel position: start={:?}, end={:?}, cursor={}",
                                        ch_start, ch_end, byte);
                                    if ch_start.byte <= byte && byte <= ch_end.byte {
                                        // Position is within the channel, extract the name
                                        if let Node::Var { name: channel_name, .. } = &**channel {
                                            debug!("Send channel is Var '{}'", channel_name);
                                            if let Some((def_uri, def_pos)) = workspace.global_symbols.get(channel_name).cloned() {
                                                debug!("Found global contract symbol '{}' for Send at {}:{} in {}",
                                                    channel_name, position.line, position.character, uri);
                                                return Some(Arc::new(Symbol {
                                                    name: channel_name.to_string(),
                                                    symbol_type: SymbolType::Contract,
                                                    declaration_uri: def_uri.clone(),
                                                    declaration_location: def_pos,
                                                    definition_location: Some(def_pos),
                                                }));
                                            } else {
                                                // Check symbol table for local variables
                                                if let Some(symbol) = symbol_table.lookup(channel_name) {
                                                    debug!("Found local symbol '{}' for Send at {}:{} in {}",
                                                        channel_name, position.line, position.character, uri);
                                                    return Some(symbol);
                                                }
                                            }
                                        }
                                    }
                                }

                                // Check if position is within any of the Send's inputs
                                for input in inputs {
                                    let input_key = &**input as *const Node as usize;
                                    if let Some(&(input_start, input_end)) = doc.positions.get(&input_key) {
                                        debug!("Send input position: start={:?}, end={:?}, cursor={}",
                                            input_start, input_end, byte);
                                        // Allow a small tolerance for position matching
                                        let tolerance = 2; // bytes
                                        if input_start.byte.saturating_sub(tolerance) <= byte && byte <= input_end.byte {
                                            if let Node::Var { name: input_name, .. } = &**input {
                                                debug!("Position within Send input Var '{}'", input_name);
                                                // Use the input node's own symbol table if it has one, which should include all parent scopes
                                                let input_symbol_table = input.metadata()
                                                    .and_then(|m| m.data.get("symbol_table"))
                                                    .and_then(|t| t.downcast_ref::<Arc<SymbolTable>>())
                                                    .cloned()
                                                    .unwrap_or_else(|| doc.symbol_table.clone());

                                                if let Some(symbol) = input_symbol_table.lookup(input_name) {
                                                    debug!("Found local symbol '{}' for Send input at {}:{} in {}",
                                                        input_name, position.line, position.character, uri);
                                                    return Some(symbol);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Node::Par { .. } => {
                            // When clicking on a contract name or call site in a Par, we might get a Par node.
                            // The node returned by find_node_at_position might be a nested Par,
                            // so we need to search from the document root to find all relevant nodes.
                            let workspace = self.workspace.read().await;
                            if let Some(doc) = workspace.documents.get(&uri) {
                                // First, check if position is within any Send/SendSync channel or inputs
                                let mut sends = Vec::new();
                                collect_calls(&doc.ir, &mut sends);
                                debug!("Found {} send nodes in document", sends.len());
                                for send in sends {
                                    let (channel, inputs) = match &*send {
                                        Node::Send { channel, inputs, .. } => (channel, inputs),
                                        Node::SendSync { channel, inputs, .. } => (channel, inputs),
                                        _ => continue,
                                    };

                                    // Check channel first
                                    let channel_key = &**channel as *const Node as usize;
                                    if let Some(&(ch_start, ch_end)) = doc.positions.get(&channel_key) {
                                        debug!("Send channel position: start={:?}, end={:?}, cursor={}",
                                            ch_start, ch_end, byte);
                                        // Check if position is within or just before the channel
                                        // (allowing for whitespace/offset differences)
                                        let tolerance = 5; // bytes
                                        if ch_start.byte.saturating_sub(tolerance) <= byte && byte <= ch_end.byte {
                                            if let Node::Var { name: channel_name, .. } = &**channel {
                                                debug!("Position within Send channel Var '{}'", channel_name);
                                                // First try symbol table for local variables
                                                if let Some(symbol) = symbol_table.lookup(channel_name) {
                                                    debug!("Found local symbol '{}' for Send at {}:{} in {}",
                                                        channel_name, position.line, position.character, uri);
                                                    return Some(symbol);
                                                } else if let Some((def_uri, def_pos)) = workspace.global_symbols.get(channel_name).cloned() {
                                                    debug!("Found global contract symbol '{}' for Send at {}:{} in {}",
                                                        channel_name, position.line, position.character, uri);
                                                    return Some(Arc::new(Symbol {
                                                        name: channel_name.to_string(),
                                                        symbol_type: SymbolType::Contract,
                                                        declaration_uri: def_uri.clone(),
                                                        declaration_location: def_pos,
                                                        definition_location: Some(def_pos),
                                                    }));
                                                }
                                            }
                                        }
                                    }

                                    // Check if position is within any of the Send's inputs
                                    for input in inputs {
                                        let input_key = &**input as *const Node as usize;
                                        if let Some(&(input_start, input_end)) = doc.positions.get(&input_key) {
                                            debug!("Send input position: start={:?}, end={:?}, cursor={}",
                                                input_start, input_end, byte);
                                            // Allow a small tolerance for position matching
                                            let tolerance = 2; // bytes
                                            if input_start.byte.saturating_sub(tolerance) <= byte && byte <= input_end.byte {
                                                if let Node::Var { name: input_name, .. } = &**input {
                                                    debug!("Position within Send input Var '{}'", input_name);
                                                    if let Some(symbol) = symbol_table.lookup(input_name) {
                                                        debug!("Found local symbol '{}' for Send input at {}:{} in {}",
                                                            input_name, position.line, position.character, uri);
                                                        return Some(symbol);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }

                                // Next, check if position is within any contract's name
                                let mut contracts = Vec::new();
                                collect_contracts(&doc.ir, &mut contracts);
                                debug!("Found {} contracts in document", contracts.len());
                                for contract in contracts {
                                    if let Node::Contract { name, .. } = &*contract {
                                        if let Node::Var { name: contract_name, .. } = &**name {
                                            let key = &**name as *const Node as usize;
                                            if let Some(&(start, end)) = doc.positions.get(&key) {
                                                debug!("Contract '{}' name position: start={:?}, end={:?}, byte={}",
                                                    contract_name, start, end, byte);
                                                if start.byte <= byte && byte <= end.byte {
                                                    debug!("Position is within contract name '{}' in document", contract_name);
                                                    if let Some((def_uri, def_pos)) = workspace.global_symbols.get(contract_name).cloned() {
                                                        debug!("Found global contract symbol '{}' at {}:{} in {}",
                                                            contract_name, position.line, position.character, uri);
                                                        return Some(Arc::new(Symbol {
                                                            name: contract_name.to_string(),
                                                            symbol_type: SymbolType::Contract,
                                                            declaration_uri: def_uri.clone(),
                                                            declaration_location: def_pos,
                                                            definition_location: Some(def_pos),
                                                        }));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            debug!("Node at {}:{} in {} is Par but position not in any contract names or send channels",
                                position.line, position.character, uri);
                        }
                        _ => {
                            debug!("Node at {}:{} in {} is not a supported node type",
                                position.line, position.character, uri);
                        }
                    }
                } else {
                    debug!("Invalid position {}:{} in {}",
                        position.line, position.character, uri);
                }
            } else {
                debug!("Document not found: {}", uri);
            }
        }
        None
    }

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

#[tower_lsp::async_trait]
impl LanguageServer for RholangBackend {
    /// Handles the LSP initialize request, setting up capabilities and indexing workspace files.
    async fn initialize(&self, params: InitializeParams) -> jsonrpc::Result<InitializeResult> {
        info!("Received initialize: {:?}", params);

        if let Some(client_pid) = params.process_id {
            {
                let mut locked_pid = self.client_process_id.lock().unwrap();
                if let Some(cmdline_pid) = *locked_pid {
                    if cmdline_pid != client_pid {
                        warn!("Client PID mismatch: command line ({}) vs LSP ({})", cmdline_pid, client_pid);
                    }
                }
                *locked_pid = Some(client_pid);
            } // Drop the lock here before await

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

                // Spawn file watcher event batcher
                Self::spawn_file_watcher(self.clone(), self.file_events.clone());
            } else {
                warn!("Failed to convert root_uri to path: {}. Skipping workspace indexing and file watching.", root_uri);
            }
        }

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
                    self.index_directory(&dir).await;

                    let tx = self.file_sender.lock().unwrap().clone();
                    let mut watcher = RecommendedWatcher::new(
                        move |res| { let _ = tx.send(res); },
                        notify::Config::default()
                    ).map_err(|_| jsonrpc::Error::internal_error()).expect("Failed to initialize watcher");
                    if let Err(e) = watcher.watch(parent, RecursiveMode::NonRecursive) {
                        warn!("Failed to watch directory {:?}: {}", parent, e);
                    }
                    *self.file_watcher.lock().unwrap() = Some(watcher);

                    // Spawn file watcher event batcher
                    Self::spawn_file_watcher(self.clone(), self.file_events.clone());
                }
            }
        } else {
            drop(root_guard);
        }

        let document_id = self.next_document_id();
        let document = Arc::new(LspDocument {
            id: document_id,
            state: RwLock::new(LspDocumentState {
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
        self.documents_by_uri.write().await.insert(uri.clone(), document.clone());
        self.documents_by_id.write().await.insert(document_id, document.clone());
        match self.index_file(&uri, &text, version, None).await {
            Ok(cached_doc) => {
                self.workspace.write().await.documents.insert(uri.clone(), Arc::new(cached_doc));
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
        if let Some(document) = self.documents_by_uri.read().await.get(&uri) {
            if let Some((text, tree)) = document.apply(params.content_changes, version).await {
                match self.index_file(&uri, &text, version, Some(tree)).await {
                    Ok(cached_doc) => {
                        self.workspace.write().await.documents.insert(uri.clone(), Arc::new(cached_doc));
                        self.link_symbols().await;
                    }
                    Err(e) => warn!("Failed to update {}: {}", uri, e),
                }

                // Send change event to debouncer instead of immediate validation
                let text_arc = Arc::new(text.to_string());
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
        if let Some(document) = self.documents_by_uri.write().await.remove(&uri) {
            self.documents_by_id.write().await.remove(&document.id);
            info!("Closed document: {}, id: {}", uri, document.id);
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
        let uri = params.text_document_position_params.text_document.uri.clone();
        let lsp_pos = params.text_document_position_params.position;

        debug!("goto_definition request for {} at {:?}", uri, lsp_pos);

        let byte = {
            let workspace = self.workspace.read().await;
            if let Some(doc) = workspace.documents.get(&uri) {
                let text = &doc.text;
                Self::byte_offset_from_position(text, lsp_pos.line as usize, lsp_pos.character as usize)
            } else {
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
                debug!("Found node at position: '{}'", node.text(&doc.text, root).to_string());
                if path.len() >= 2 {
                    let parent = path[path.len() - 2].clone();
                    let is_channel = match &*parent {
                        Node::Send { channel, .. } | Node::SendSync { channel, .. } => Arc::ptr_eq(channel, &node),
                        _ => false,
                    };
                    debug!("Is channel in Send/SendSync: {}", is_channel);
                    if is_channel {
                        if let Node::Send { channel, inputs, .. } | Node::SendSync { channel, inputs, .. } = &*parent {
                            let matching = workspace.global_contracts.iter().filter(|(_, contract)| match_contract(channel, inputs, contract)).map(|(u, c)| {
                                let cached_doc = workspace.documents.get(u).expect("Document not found");
                                let positions = cached_doc.positions.clone();
                                debug!("Matched contract in {}: '{}'", u, c.text(&cached_doc.text, &cached_doc.ir).to_string());
                                let name = if let Node::Contract { name, .. } = &**c {
                                    debug!("Contact name: {:?}", name);
                                    name
                                } else {
                                    debug!("Unreachable!");
                                    unreachable!()
                                };
                                debug!("Found contract name");
                                let key = &**name as *const Node as usize;
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
                                if let Some(symbol) = self.get_symbol_at_position(&uri, lsp_pos).await {
                                    let pos = symbol.definition_location.unwrap_or(symbol.declaration_location);
                                    let range = Self::position_to_range(pos, symbol.name.len());
                                    let loc = Location { uri: symbol.declaration_uri.clone(), range };
                                    Ok(Some(GotoDefinitionResponse::Scalar(loc)))
                                } else {
                                    Ok(None)
                                }
                            } else if matching.len() == 1 {
                                Ok(Some(GotoDefinitionResponse::Scalar(matching[0].clone())))
                            } else {
                                Ok(Some(GotoDefinitionResponse::Array(matching)))
                            }
                        } else {
                            unreachable!()
                        }
                    } else {
                        drop(workspace);
                        debug!("Not a channel; falling back to symbol lookup");
                        if let Some(symbol) = self.get_symbol_at_position(&uri, lsp_pos).await {
                            let pos = symbol.definition_location.unwrap_or(symbol.declaration_location);
                            let range = Self::position_to_range(pos, symbol.name.len());
                            let loc = Location { uri: symbol.declaration_uri.clone(), range };
                            Ok(Some(GotoDefinitionResponse::Scalar(loc)))
                        } else {
                            Ok(None)
                        }
                    }
                } else {
                    drop(workspace);
                    debug!("Path too short; falling back to symbol lookup");
                    if let Some(symbol) = self.get_symbol_at_position(&uri, lsp_pos).await {
                        let pos = symbol.definition_location.unwrap_or(symbol.declaration_location);
                        let range = Self::position_to_range(pos, symbol.name.len());
                        let loc = Location { uri: symbol.declaration_uri.clone(), range };
                        Ok(Some(GotoDefinitionResponse::Scalar(loc)))
                    } else {
                        Ok(None)
                    }
                }
            } else {
                debug!("No node found at position {:?} in {}", ir_pos, uri);
                Ok(None)
            }
        } else {
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
                        Node::Contract { name, .. } => Arc::ptr_eq(name, &node),
                        _ => false,
                    };
                    debug!("Is name in Contract: {}", is_name);
                    if is_name {
                        if let Node::Contract { .. } = &*parent {
                            let contract = parent.clone();
                            let matching_calls = workspace.global_calls.iter().filter(|(_, call)| {
                                match &**call {
                                    Node::Send { channel, inputs, .. } | Node::SendSync { channel, inputs, .. } => {
                                        match_contract(channel, inputs, &contract)
                                    }
                                    _ => false,
                                }
                            }).cloned().collect::<Vec<_>>();
                            debug!("Found {} matching calls for contract", matching_calls.len());
                            let mut locations = matching_calls.iter().map(|(call_uri, call)| {
                                let call_doc = workspace.documents.get(call_uri).expect("Document not found");
                                let call_positions = call_doc.positions.clone();
                                debug!("Matched call in {}: '{}'", call_uri, call.text(&call_doc.text, &call_doc.ir).to_string());
                                let channel = match &**call {
                                    Node::Send { channel, .. } | Node::SendSync { channel, .. } => channel.clone(),
                                    _ => unreachable!()
                                };
                                let key = &*channel as *const Node as usize;
                                let (start, _) = (*call_positions).get(&key).unwrap();
                                Location {
                                    uri: call_uri.clone(),
                                    range: Self::position_to_range(*start, channel.text(&call_doc.text, &call_doc.ir).len_chars()),
                                }
                            }).collect::<Vec<_>>();
                            if include_decl {
                                let key = &*node as *const Node as usize;
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
            let symbols = collect_document_symbols(&doc.ir, &*doc.positions);
            debug!("Found {} symbols in document {}", symbols.len(), uri);
            Ok(Some(DocumentSymbolResponse::Nested(symbols)))
        } else {
            debug!("Document not found: {}", uri);
            Ok(None)
        }
    }

    /// Searches for workspace symbols matching the query.
    async fn symbol(&self, params: WorkspaceSymbolParams) -> LspResult<Option<Vec<SymbolInformation>>> {
        let query = params.query.to_lowercase();
        debug!("Handling workspace symbol request with query '{}'", query);
        let workspace = self.workspace.read().await;
        let mut symbols = Vec::new();
        for (uri, doc) in &workspace.documents {
            let doc_symbols = collect_workspace_symbols(&doc.symbol_table, uri);
            symbols.extend(doc_symbols.into_iter().filter(|s| s.name.to_lowercase().contains(&query)));
        }
        debug!("Found {} matching workspace symbols", symbols.len());
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
}
