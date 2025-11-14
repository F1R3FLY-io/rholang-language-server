//! Document indexing operations for the LSP backend
//!
//! This module contains all functionality related to parsing, indexing,
//! and caching documents including:
//! - Document processing pipeline (IR transformation, symbol building)
//! - File indexing (Rholang and MeTTa language support)
//! - Embedded language region detection
//! - File system change handling
//! - Directory-wide indexing
//! - Parallel batch indexing using Rayon

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use rayon::prelude::*;
use tower_lsp::lsp_types::Url;
use tracing::{debug, info, warn};

use ropey::Rope;
use walkdir::WalkDir;

use crate::ir::pipeline::Pipeline;
use crate::ir::rholang_node::{RholangNode, compute_absolute_positions, collect_contracts, collect_calls};
use crate::ir::symbol_table::SymbolTable;
use crate::ir::transforms::symbol_table_builder::SymbolTableBuilder;
use crate::ir::transforms::symbol_index_builder::SymbolIndexBuilder;
use crate::ir::transforms::documentation_attacher::DocumentationAttacher;
use crate::language_regions::{ChannelFlowAnalyzer, DirectiveParser, SemanticDetector};
use crate::lsp::models::{CachedDocument, DocumentLanguage};
use crate::tree_sitter::{parse_code, parse_to_ir, parse_to_document_ir};

use super::state::{RholangBackend, WorkspaceChangeEvent, WorkspaceChangeType};

impl RholangBackend {
    /// Processes a parsed IR node through the transformation pipeline to build symbols and metadata (blocking version for CPU-bound work on Rayon).
    ///
    /// This is a synchronous version of `process_document` that can be called from Rayon's thread pool
    /// without blocking the tokio runtime. It takes cloned Arc references to workspace state instead of
    /// acquiring async locks.
    ///
    /// # Performance
    /// This function performs CPU-intensive work (parsing, IR transformation, symbol building) and should
    /// be called via `tokio::task::spawn_blocking` or from a Rayon thread pool.
    pub(super) fn process_document_blocking(
        document_ir: Arc<crate::ir::DocumentIR>,
        uri: &Url,
        text: &Rope,
        content_hash: u64,
        global_table: Arc<SymbolTable>,
        global_index: Arc<std::sync::RwLock<crate::ir::global_index::GlobalSymbolIndex>>,
        version_counter: &Arc<std::sync::atomic::AtomicI32>,
        rholang_symbols: Option<Arc<crate::lsp::rholang_contracts::RholangContracts>>,
    ) -> Result<CachedDocument, String> {
        // Extract semantic IR from DocumentIR
        let ir = document_ir.root.clone();
        // Priority 1: Incremental symbol updates
        // Clear old symbols for this URI from global_table BEFORE indexing
        // This ensures we don't have stale symbols from previous versions of the file
        // Lock-free retain operation using DashMap
        global_table.symbols.retain(|_, s| &s.declaration_uri != uri);

        // Clear old contracts from rholang_contracts index (incremental update)
        if let Some(ref rholang_syms) = rholang_symbols {
            let removed_contracts = rholang_syms.remove_contracts_from_uri(uri);
            let removed_refs = rholang_syms.remove_references_from_uri(uri);
            debug!("Incremental update for {}: removed {} contracts and {} references",
                   uri, removed_contracts, removed_refs);
        }

        let mut pipeline = Pipeline::new();

        // Symbol table builder for local symbol tracking
        // Phase 3.4: Pass rholang_symbols for direct indexing
        let builder = Arc::new(SymbolTableBuilder::new(ir.clone(), uri.clone(), global_table.clone(), rholang_symbols));
        pipeline.add_transform(crate::ir::pipeline::Transform {
            id: "symbol_table_builder".to_string(),
            dependencies: vec![],
            kind: crate::ir::pipeline::TransformKind::Specific(builder.clone()),
        });

        // Documentation attacher for doc comment attachment (Phase 3)
        let doc_attacher = Arc::new(DocumentationAttacher::new(document_ir.clone()));
        pipeline.add_transform(crate::ir::pipeline::Transform {
            id: "documentation_attacher".to_string(),
            dependencies: vec![],
            kind: crate::ir::pipeline::TransformKind::Specific(doc_attacher),
        });

        // Apply pipeline transformations first to get transformed IR
        let transformed_ir = pipeline.apply(&ir);

        // Compute positions from transformed IR (structural positions are unchanged, but node addresses differ)
        let positions = Arc::new(compute_absolute_positions(&transformed_ir));
        debug!("Cached {} node positions for {}", positions.len(), uri);

        // Symbol index builder for global pattern-based lookups (needs positions)
        let mut index_builder = SymbolIndexBuilder::new(global_index.clone(), uri.clone(), positions.clone());
        index_builder.index_tree(&transformed_ir);

        // Extract inverted_index from symbol table builder for local variable references
        let inverted_index = builder.get_inverted_index();
        debug!("Built inverted index with {} declaration->references mappings", inverted_index.len());

        let symbol_table = transformed_ir.metadata()
            .and_then(|m| m.get("symbol_table"))
            .map(|st| Arc::clone(st.downcast_ref::<Arc<SymbolTable>>().unwrap()))
            .unwrap_or_else(|| {
                debug!("No symbol table found on root for {}, using default empty table", uri);
                Arc::new(SymbolTable::new(Some(global_table.clone())))
            });
        let version = version_counter.fetch_add(1, Ordering::SeqCst);

        let symbol_count = symbol_table.collect_all_symbols().len();
        debug!("Processed document {}: {} symbols, version {}",
            uri, symbol_count, version);

        let mut contracts = Vec::new();
        let mut calls = Vec::new();
        collect_contracts(&transformed_ir, &mut contracts);
        collect_calls(&transformed_ir, &mut calls);
        debug!("Collected {} contracts and {} calls in {}", contracts.len(), calls.len(), uri);

        // Detect language and create UnifiedIR
        let language = DocumentLanguage::from_uri(uri);
        let unified_ir: Arc<dyn crate::ir::semantic_node::SemanticNode> = match language {
            DocumentLanguage::Rholang | DocumentLanguage::Unknown => {
                use crate::ir::unified_ir::UnifiedIR;
                UnifiedIR::from_rholang(&transformed_ir)
            }
            DocumentLanguage::Metta => {
                use crate::ir::unified_ir::UnifiedIR;
                use crate::ir::semantic_node::{NodeBase, Position};
                Arc::new(UnifiedIR::Error {
                    base: NodeBase::new_simple(
                        Position { row: 0, column: 0, byte: 0 },
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

        // Build position index for O(log n) node lookups (Phase 6)
        let position_index = Arc::new(crate::lsp::position_index::PositionIndex::build(&transformed_ir));
        debug!("Built position index for {} nodes ({} unique positions) in {}",
            position_index.node_count(), position_index.position_count(), uri);

        Ok(CachedDocument {
            ir: transformed_ir,
            position_index,
            document_ir: Some(document_ir),  // Phase 1: Populated with comment channel
            metta_ir: None,
            unified_ir,
            language,
            tree: Arc::new(parse_code("")),
            symbol_table,
            inverted_index,
            version,
            text: text.clone(),
            positions,
            symbol_index,
            content_hash,
            completion_state: None,  // Phase 9: Initialized lazily on first completion request
        })
    }

    /// Processes a parsed IR node through the transformation pipeline to build symbols and metadata.
    ///
    /// This async wrapper delegates CPU-intensive work to `process_document_blocking` via `spawn_blocking`
    /// to prevent blocking the tokio runtime.
    pub(super) async fn process_document(&self, document_ir: Arc<crate::ir::DocumentIR>, uri: &Url, text: &Rope, content_hash: u64) -> Result<CachedDocument, String> {
        // Lock and clone global_table for use in blocking task
        let global_table = Arc::new(self.workspace.global_table.read().await.clone());
        let global_index = self.workspace.global_index.clone();
        let rholang_symbols = Some(self.workspace.rholang_symbols.clone());

        // Delegate CPU-intensive work to blocking thread pool
        let uri_clone = uri.clone();
        let text_clone = text.clone();
        let version_counter = self.version_counter.clone();

        tokio::task::spawn_blocking(move || {
            Self::process_document_blocking(
                document_ir,
                &uri_clone,
                &text_clone,
                content_hash,
                global_table,
                global_index,
                &version_counter,
                rholang_symbols,
            )
        })
        .await
        .map_err(|e| format!("Failed to spawn blocking task: {}", e))?
    }

    /// Processes a parsed IR node through the transformation pipeline to build symbols and metadata (DEPRECATED - use process_document instead).
    #[allow(dead_code)]
    async fn process_document_old(&self, ir: Arc<RholangNode>, uri: &Url, text: &Rope, content_hash: u64) -> Result<CachedDocument, String> {
        let mut pipeline = Pipeline::new();
        // Lock and clone global_table for use in transforms
        let global_table = Arc::new(self.workspace.global_table.read().await.clone());
        let global_index = self.workspace.global_index.clone();

        // Symbol table builder for local symbol tracking
        // TODO Phase 3: Pass Some(rholang_symbols) for direct indexing (deprecated method)
        let builder = Arc::new(SymbolTableBuilder::new(ir.clone(), uri.clone(), global_table.clone(), None));
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

        // Extract inverted_index from symbol table builder for local variable references
        let inverted_index = builder.get_inverted_index();
        debug!("Built inverted index with {} declaration->references mappings", inverted_index.len());

        let symbol_table = transformed_ir.metadata()
            .and_then(|m| m.get("symbol_table"))
            .map(|st| Arc::clone(st.downcast_ref::<Arc<SymbolTable>>().unwrap()))
            .unwrap_or_else(|| {
                debug!("No symbol table found on root for {}, using default empty table", uri);
                Arc::new(SymbolTable::new(Some(global_table.clone())))
            });
        let version = self.version_counter.fetch_add(1, Ordering::SeqCst);

        let symbol_count = symbol_table.collect_all_symbols().len();
        debug!("Processed document {}: {} symbols, version {}",
            uri, symbol_count, version);

        let mut contracts = Vec::new();
        let mut calls = Vec::new();
        collect_contracts(&transformed_ir, &mut contracts);
        collect_calls(&transformed_ir, &mut calls);
        debug!("Collected {} contracts and {} calls in {}", contracts.len(), calls.len(), uri);

        // Detect language and create UnifiedIR
        let language = DocumentLanguage::from_uri(uri);
        let unified_ir: Arc<dyn crate::ir::semantic_node::SemanticNode> = match language {
            DocumentLanguage::Rholang | DocumentLanguage::Unknown => {
                // Convert RholangNode to UnifiedIR
                use crate::ir::unified_ir::UnifiedIR;
                UnifiedIR::from_rholang(&transformed_ir)
            }
            DocumentLanguage::Metta => {
                // MeTTa support not yet implemented - for now, just wrap as UnifiedIR
                // TODO: Implement MeTTa parsing and conversion
                use crate::ir::unified_ir::UnifiedIR;
                use crate::ir::semantic_node::{NodeBase, Position};
                Arc::new(UnifiedIR::Error {
                    base: NodeBase::new_simple(
                        Position { row: 0, column: 0, byte: 0 },
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

        // Build position index for O(log n) node lookups (Phase 6)
        let position_index = Arc::new(crate::lsp::position_index::PositionIndex::build(&transformed_ir));
        debug!("Built position index for {} nodes ({} unique positions) in {}",
            position_index.node_count(), position_index.position_count(), uri);

        Ok(CachedDocument {
            ir: transformed_ir,
            position_index,
            document_ir: None, // TODO: Populate in Phase 1 implementation
            metta_ir: None, // Will be populated for MeTTa files
            unified_ir,
            language,
            tree: Arc::new(parse_code("")), // Tree is not used for text, but keep for other
            symbol_table,
            inverted_index,
            version,
            text: text.clone(),
            positions,
            symbol_index,
            content_hash,
            completion_state: None,  // Phase 9: Initialized lazily on first completion request
        })
    }

    /// Indexes a document by parsing its text and processing it, using an existing syntax tree if provided for incremental updates.
    pub(super) async fn index_file(
        &self,
        uri: &Url,
        text: &str,
        _version: i32,
        tree: Option<tree_sitter::Tree>,
    ) -> Result<CachedDocument, String> {
        use crate::lsp::backend::document_cache::ContentHash;
        use std::collections::hash_map::DefaultHasher;

        // Phase B-2: Compute content hash for cache lookup (blake3 for fast hashing)
        let content_hash_blake3 = ContentHash::from_str(text);

        // Check cache first (Phase B-2 optimization)
        if let Some(cached_doc) = self.workspace.document_cache.get(uri, &content_hash_blake3) {
            debug!("Cache HIT for {} - returning cached document", uri);
            return Ok((*cached_doc).clone());
        }

        debug!("Cache MISS for {} - parsing and indexing", uri);

        // Compute fallback hash for backward compatibility with content_hash field
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        let content_hash = hasher.finish();

        // Check if we already have this exact content indexed
        // Note: We can't early-return here because we need to re-index to update workspace state
        // However, we can log the hash check for debugging
        if let Some(existing) = self.workspace.documents.get(uri) {
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
                // Note: We intentionally do NOT clear old symbols here - that will be done
                // in a single batched workspace update by the caller to minimize lock duration

                let tree = Arc::new(tree.unwrap_or_else(|| parse_code(text)));
                let rope = Rope::from_str(text);
                let document_ir = parse_to_document_ir(&tree, &rope);
                let cached = self.process_document(document_ir, uri, &rope, content_hash).await?;

                // Detect embedded language regions asynchronously using hybrid rayon worker
                // This approach provides 18-19x better throughput than synchronous detection
                let detection_result = self.detection_worker
                    .detect(uri.clone(), text.to_string())
                    .await
                    .map_err(|_| "Detection worker receiver dropped")?;

                debug!(
                    "Async detection completed for {} in {}ms: {} regions detected",
                    detection_result.uri,
                    detection_result.elapsed_ms,
                    detection_result.regions.len()
                );

                // DetectorRegistry already handles:
                // - Priority-based execution (DirectiveParser > SemanticDetector > ChannelFlowAnalyzer)
                // - Deduplication with directive priority override
                // - Parallel detection via rayon
                let all_regions = detection_result.regions;

                if !all_regions.is_empty() {
                    debug!("Registering {} virtual documents for {}", all_regions.len(), uri);
                    let mut virtual_docs = self.virtual_docs.write().await;
                    virtual_docs.register_regions(uri, &all_regions);

                    // Validate virtual documents and get diagnostics
                    // Note: We don't publish diagnostics here; that's done in validate()
                    let _virtual_diagnostics = virtual_docs.validate_all_for_parent(uri);
                    debug!("Validated {} virtual documents for {}", all_regions.len(), uri);
                }

                // Collect contracts and calls (CPU-bound work without holding lock)
                let mut contracts = Vec::new();
                collect_contracts(&cached.ir, &mut contracts);
                let mut calls = Vec::new();
                collect_calls(&cached.ir, &mut calls);

                // Note: We intentionally do NOT update workspace here to minimize lock duration.
                // Caller is responsible for batching workspace updates (documents, contracts, calls)
                // into a single write lock and calling link_symbols() afterward.

                // Phase B-2: Insert into cache before returning
                let modified_at = std::time::SystemTime::now(); // Use current time for in-memory documents
                self.workspace.document_cache.insert(
                    uri.clone(),
                    content_hash_blake3,
                    Arc::new(cached.clone()),
                    modified_at,
                );
                debug!("Cached document for {} (hash: {:?})", uri, content_hash_blake3.as_blake3());

                Ok(cached)
            }
        }
    }

    /// Indexes a MeTTa file by parsing and creating a cached document
    pub(super) async fn index_metta_file(
        &self,
        uri: &Url,
        text: &str,
        version: i32,
        content_hash: u64,
    ) -> Result<CachedDocument, String> {
        use crate::lsp::backend::document_cache::ContentHash;

        // Phase B-2: Check cache first
        let content_hash_blake3 = ContentHash::from_str(text);
        if let Some(cached_doc) = self.workspace.document_cache.get(uri, &content_hash_blake3) {
            debug!("Cache HIT for MeTTa file {} - returning cached document", uri);
            return Ok((*cached_doc).clone());
        }

        debug!("Cache MISS for MeTTa file {} - parsing and indexing", uri);
        use crate::parsers::MettaParser;
        use crate::ir::semantic_node::{NodeBase, Position};
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
        let placeholder_ir = Arc::new(RholangNode::Nil {
            base: NodeBase::new_simple(
                Position { row: 0, column: 0, byte: 0 },
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
                base: NodeBase::new_simple(
                    Position { row: 0, column: 0, byte: 0 },
                    0,
                    0,
                    0,
                ),
                message: "Empty MeTTa file".to_string(),
                children: vec![],
                metadata: None,
            })
        };

        // Create empty symbol table for now
        // TODO: Implement symbol table building for MeTTa
        let global_table = Arc::new(self.workspace.global_table.read().await.clone());
        let symbol_table = Arc::new(SymbolTable::new(Some(global_table)));

        // Create empty inverted_index and symbol index for MeTTa (not yet supported)
        let inverted_index = HashMap::new();
        let symbol_index = Arc::new(crate::lsp::symbol_index::SymbolIndex::new(Vec::new()));

        // Create empty position index for MeTTa placeholder
        let position_index = Arc::new(crate::lsp::position_index::PositionIndex::new());

        let rope = Rope::from_str(text);
        let positions = Arc::new(HashMap::new());

        let cached_doc = CachedDocument {
            ir: placeholder_ir,
            position_index,
            document_ir: None, // TODO: Populate in Phase 1 implementation
            metta_ir: Some(metta_nodes),
            unified_ir,
            language: DocumentLanguage::Metta,
            tree: Arc::new(parse_code("")), // Placeholder tree
            symbol_table,
            inverted_index,
            version,
            text: rope,
            positions,
            symbol_index,
            content_hash,
            completion_state: None,  // Phase 9: Initialized lazily on first completion request
        };

        // Phase B-2: Insert into cache before returning
        let modified_at = std::time::SystemTime::now();
        self.workspace.document_cache.insert(
            uri.clone(),
            content_hash_blake3,
            Arc::new(cached_doc.clone()),
            modified_at,
        );
        debug!("Cached MeTTa document for {} (hash: {:?})", uri, content_hash_blake3.as_blake3());

        // Broadcast workspace change event (ReactiveX Phase 2)
        let file_count = self.workspace.documents.len();
        let symbol_count = self.workspace.rholang_symbols.len();

        let _ = self.workspace_changes.send(WorkspaceChangeEvent {
            file_count,
            symbol_count,
            change_type: WorkspaceChangeType::FileIndexed,
        });

        Ok(cached_doc)
    }

    /// Handles file system events by re-indexing changed .rho files that are not open.
    pub(super) async fn handle_file_change(&self, path: PathBuf) {
        if path.extension().map_or(false, |ext| ext == "rho") {
            if let Ok(uri) = Url::from_file_path(&path) {
                // DashMap::contains_key is lock-free
                if self.documents_by_uri.contains_key(&uri) {
                    debug!("Skipping update for opened document: {}", uri);
                    return;
                }
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                match self.index_file(&uri, &text, 0, None).await {
                    Ok(cached_doc) => {
                        self.update_workspace_document(&uri, Arc::new(cached_doc)).await;
                        self.link_symbols().await;
                        info!("Updated cache for file: {}", uri);
                    }
                    Err(e) => warn!("Failed to index file {}: {}", uri, e),
                }
            }
        }
    }

    /// Indexes all .rho files in the given directory (non-recursively).
    ///
    /// This version uses sequential processing. For parallel batch indexing of many files,
    /// use `index_directory_parallel` instead for 4-8x speedup on multi-core systems.
    pub(super) async fn index_directory(&self, dir: &Path) {
        for result in WalkDir::new(dir) {
            match result {
                Ok(entry) => {
                    if entry.file_type().is_file() && entry.path().extension().map_or(false, |ext| ext == "rho") {
                        let uri = Url::from_file_path(entry.path()).expect("Failed to create URI from path");
                        // DashMap::contains_key is lock-free
                        if !self.documents_by_uri.contains_key(&uri)
                            && !self.workspace.documents.contains_key(&uri) {
                            if let Ok(text) = std::fs::read_to_string(entry.path()) {
                                match self.index_file(&uri, &text, 0, None).await {
                                    Ok(cached_doc) => {
                                        self.update_workspace_document(&uri, Arc::new(cached_doc)).await;
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

        // Populate completion index eagerly (Phase 4 optimization)
        debug!("Populating completion index after directory indexing");
        crate::lsp::features::completion::add_keywords(&self.workspace.completion_index);
        let global_table = self.workspace.global_table.read().await;
        crate::lsp::features::completion::populate_from_symbol_table(
            &self.workspace.completion_index,
            &*global_table,
        );
        drop(global_table);
        for doc_entry in self.workspace.documents.iter() {
            let (doc_uri, doc) = (doc_entry.key(), doc_entry.value());
            crate::lsp::features::completion::populate_from_symbol_table_with_tracking(
                &self.workspace.completion_index,
                &doc.symbol_table,
                doc_uri,
            );
        }
        info!("Completion index populated with {} symbols", self.workspace.completion_index.len());
    }

    /// Indexes all .rho files in the given directory using parallel processing (Rayon).
    ///
    /// This version provides 4-8x speedup on multi-core systems by:
    /// 1. Collecting all file paths first
    /// 2. Parsing and indexing files in parallel using Rayon
    /// 3. Batch-inserting results into workspace
    /// 4. Linking symbols once after all files are indexed
    ///
    /// # Performance
    /// - Expected speedup: 4-8x on 8+ core systems
    /// - Scales linearly with CPU cores
    /// - CPU utilization: ~95% vs ~25% sequential
    pub(super) async fn index_directory_parallel(&self, dir: &Path) {
        use std::time::Instant;
        let start = Instant::now();

        // Phase 1: Collect all .rho file paths (fast, single-threaded)
        let paths: Vec<PathBuf> = WalkDir::new(dir)
            .into_iter()
            .filter_map(|result| result.ok())
            .filter(|entry| {
                entry.file_type().is_file()
                    && entry.path().extension().map_or(false, |ext| ext == "rho")
            })
            .map(|entry| entry.path().to_path_buf())
            .collect();

        info!("Found {} .rho files to index in {:?}", paths.len(), dir);

        // Get workspace state snapshot for filtering
        // DashMap::iter() provides lock-free iteration
        let existing_docs: Vec<Url> = self.documents_by_uri.iter().map(|entry| entry.key().clone()).collect();
        let workspace_docs: Vec<Url> = self.workspace.documents.iter().map(|entry| entry.key().clone()).collect();

        // Phase 2: Parse and process files in parallel using Rayon
        // CRITICAL: Wrap Rayon work in spawn_blocking to prevent blocking Tokio runtime
        // Lock and clone global_table for use in blocking task
        let global_table = Arc::new(self.workspace.global_table.read().await.clone());
        let global_index = self.workspace.global_index.clone();
        let version_counter = self.version_counter.clone();
        let rholang_symbols = Some(self.workspace.rholang_symbols.clone());

        let results: Vec<(Url, Result<CachedDocument, String>)> = tokio::task::spawn_blocking(move || {
            paths
                .par_iter()
                .filter_map(|path| {
                    // Skip if already indexed
                    if let Ok(uri) = Url::from_file_path(path) {
                        if existing_docs.contains(&uri) || workspace_docs.contains(&uri) {
                            debug!("Skipping already indexed file: {}", uri);
                            return None;
                        }

                        // Read file and parse/index on Rayon thread pool
                        if let Ok(text) = std::fs::read_to_string(path) {
                            let rope = Rope::from_str(&text);
                            let tree = Arc::new(parse_code(&text));
                            let document_ir = parse_to_document_ir(&tree, &rope);

                            use std::collections::hash_map::DefaultHasher;
                            let mut hasher = DefaultHasher::new();
                            text.hash(&mut hasher);
                            let content_hash = hasher.finish();

                            // CPU-intensive work happens here in parallel
                            let result = Self::process_document_blocking(
                                document_ir,
                                &uri,
                                &rope,
                                content_hash,
                                global_table.clone(),
                                global_index.clone(),
                                &version_counter,
                                rholang_symbols.clone(),
                            );

                            return Some((uri, result));
                        }
                    }
                    None
                })
                .collect()
        })
        .await
        .expect("Rayon parallel indexing task panicked");

        let elapsed = start.elapsed();
        info!("Parallel indexing of {} files completed in {:?} ({:.1} files/sec)",
            results.len(), elapsed, results.len() as f64 / elapsed.as_secs_f64());

        // Phase 3: Batch insert into workspace using batched updates
        let indexed_uris: Vec<Url> = results.iter()
            .filter_map(|(uri, result)| if result.is_ok() { Some(uri.clone()) } else { None })
            .collect();

        for (uri, result) in results {
            match result {
                Ok(cached_doc) => {
                    self.update_workspace_document(&uri, Arc::new(cached_doc)).await;
                    debug!("Indexed file: {}", uri);
                }
                Err(e) => warn!("Failed to index file {}: {}", uri, e),
            }
        }

        // Phase 3.5: Register and validate virtual documents for all indexed files
        // This phase must happen BEFORE Phase 5 (link_virtual_symbols)
        for uri in &indexed_uris {
            // Detect embedded language regions asynchronously
            if let Ok(text) = std::fs::read_to_string(uri.path()) {
                let detection_result = self.detection_worker
                    .detect(uri.clone(), text)
                    .await
                    .unwrap_or_else(|_| {
                        warn!("Detection worker receiver dropped for {}", uri);
                        crate::language_regions::async_detection::DetectionResult {
                            uri: uri.clone(),
                            regions: vec![],
                            elapsed_ms: 0,
                        }
                    });

                if !detection_result.regions.is_empty() {
                    debug!("Registering {} virtual documents for {} during workspace indexing",
                        detection_result.regions.len(), uri);
                    let mut virtual_docs = self.virtual_docs.write().await;
                    virtual_docs.register_regions(uri, &detection_result.regions);

                    // Validate virtual documents
                    let _virtual_diagnostics = virtual_docs.validate_all_for_parent(uri);
                    debug!("Validated {} virtual documents for {}", detection_result.regions.len(), uri);
                }
            }
        }

        // Phase 4: Link symbols across all indexed files
        self.link_symbols().await;

        // Phase 5: Link symbols across all virtual documents
        self.link_virtual_symbols().await;

        // Phase 6: Populate completion index eagerly (Phase 4 optimization)
        // This eliminates the 10-50ms first-completion penalty
        let completion_start = Instant::now();
        debug!("Populating completion index during workspace initialization");

        // Add keywords (always available)
        crate::lsp::features::completion::add_keywords(&self.workspace.completion_index);

        // Add symbols from global table
        let global_table = self.workspace.global_table.read().await;
        crate::lsp::features::completion::populate_from_symbol_table(
            &self.workspace.completion_index,
            &*global_table,
        );
        drop(global_table);

        // Add symbols from all indexed documents
        for doc_entry in self.workspace.documents.iter() {
            let (doc_uri, doc) = (doc_entry.key(), doc_entry.value());
            crate::lsp::features::completion::populate_from_symbol_table_with_tracking(
                &self.workspace.completion_index,
                &doc.symbol_table,
                doc_uri,
            );
        }

        info!("Completion index populated with {} symbols in {:?}",
            self.workspace.completion_index.len(), completion_start.elapsed());

        info!("Total indexing time (including symbol linking and completion): {:?}", start.elapsed());
    }

    /// Generates the next unique document ID.
    pub(super) fn next_document_id(&self) -> u32 {
        self.serial_document_id.fetch_add(1, Ordering::SeqCst)
    }

    /// Updates workspace with a newly indexed document in a single batched write lock.
    ///
    /// This function performs all workspace mutations in one atomic operation:
    /// 1. Removes old symbols, contracts, and calls for the URI
    /// 2. Inserts the new cached document
    /// 3. Collects and inserts new contracts and calls
    /// 4. Broadcasts workspace change event
    ///
    /// This minimizes lock contention by:
    /// - Reducing multiple sequential write locks to one
    /// - Performing all CPU-bound work (collecting contracts/calls) before acquiring lock
    /// - Using in-place mutations to avoid cloning large HashMaps
    pub(super) async fn update_workspace_document(&self, uri: &Url, cached_doc: Arc<CachedDocument>) {
        // Collect contracts and calls outside the lock (CPU-bound work)
        let mut contracts = Vec::new();
        collect_contracts(&cached_doc.ir, &mut contracts);
        let mut calls = Vec::new();
        collect_calls(&cached_doc.ir, &mut calls);

        // Prepare new data to insert (outside lock)
        let new_contracts: Vec<_> = contracts.into_iter().map(|c| (uri.clone(), c)).collect();
        let new_calls: Vec<_> = calls.into_iter().map(|c| (uri.clone(), c)).collect();

        // Lock-free document and symbol updates using DashMap
        // NOTE: We do NOT clear symbols from global_table here because:
        // 1. global_table is shared across all documents via Arc<SymbolTable>
        // 2. SymbolTableBuilder already manages inserting/updating symbols during indexing
        // 3. Clearing here would delete symbols that were just added by SymbolTableBuilder
        // 4. global_table uses interior mutability, so changes are visible across all Arc clones

        // Priority 2b: Cleanup now handled by rholang_symbols.remove_symbols_from_uri() and
        // remove_references_from_uri() which are called in process_document_blocking()
        // No need for global_inverted_index cleanup

        // Lock-free contract and call updates
        self.workspace.global_contracts.remove(uri);
        self.workspace.global_calls.remove(uri);

        // Insert new data (lock-free)
        self.workspace.documents.insert(uri.clone(), cached_doc);
        for (contract_uri, contract) in new_contracts {
            self.workspace.global_contracts
                .entry(contract_uri)
                .or_insert_with(Vec::new)
                .push(contract);
        }
        for (call_uri, call) in new_calls {
            self.workspace.global_calls
                .entry(call_uri)
                .or_insert_with(Vec::new)
                .push(call);
        }

        let file_count = self.workspace.documents.len();
        let symbol_count = self.workspace.rholang_symbols.len();

        // Broadcast workspace change event (outside lock)
        let _ = self.workspace_changes.send(WorkspaceChangeEvent {
            file_count,
            symbol_count,
            change_type: WorkspaceChangeType::FileIndexed,
        });
    }

    /// Phase 9.3: Update incremental completion state based on document changes
    ///
    /// Processes content changes incrementally to update the completion draft buffer
    /// without full re-indexing. Handles single-character edits optimally.
    ///
    /// # Arguments
    /// * `uri` - Document URI
    /// * `changes` - List of content changes from LSP client
    /// * `cached_doc` - Updated cached document with new symbol table
    pub(super) async fn update_completion_state_incremental(
        &self,
        _uri: &tower_lsp::lsp_types::Url,
        changes: &[tower_lsp::lsp_types::TextDocumentContentChangeEvent],
        mut cached_doc_arc: std::sync::Arc<crate::lsp::models::CachedDocument>,
    ) {
        use crate::lsp::features::completion::incremental::get_or_init_completion_state;

        // Get or initialize completion state (requires mut access to cached_doc)
        // SAFETY: We need mutable access briefly to initialize if needed
        let cached_doc_mut = std::sync::Arc::get_mut(&mut cached_doc_arc);
        if cached_doc_mut.is_none() {
            // Doc is shared elsewhere, skip incremental update for now
            // This is rare - only happens if another request is accessing doc simultaneously
            tracing::warn!("Skipping incremental completion update - document is shared");
            return;
        }
        let cached_doc_mut = cached_doc_mut.unwrap();
        let state_arc = get_or_init_completion_state(cached_doc_mut);
        let mut state = state_arc.write();

        // Process each content change
        for change in changes {
            if let Some(range) = &change.range {
                // Incremental change with range
                let start = &range.start;
                let end = &range.end;

                // Check if this is on a single line (incremental edit)
                if start.line == end.line {
                    let char_delta = end.character as i32 - start.character as i32;

                    if char_delta == 0 && !change.text.is_empty() {
                        // Character insertion (single or multiple)
                        if let Err(e) = state.insert_str(&change.text, *start) {
                            tracing::warn!("Failed to insert text into completion state: {}", e);
                        }
                    } else if char_delta > 0 && change.text.is_empty() {
                        // Character deletion (single or multiple)
                        for _ in 0..char_delta {
                            if let Err(e) = state.handle_char_delete(*start) {
                                tracing::warn!("Failed to delete character from completion state: {}", e);
                                break;
                            }
                        }
                    } else if char_delta > 0 && !change.text.is_empty() {
                        // Replacement - delete then insert
                        for _ in 0..char_delta {
                            if let Err(e) = state.handle_char_delete(*start) {
                                tracing::warn!("Failed to delete during replacement: {}", e);
                                break;
                            }
                        }
                        if let Err(e) = state.insert_str(&change.text, *start) {
                            tracing::warn!("Failed to insert during replacement: {}", e);
                        }
                    } else {
                        // Complex edit - clear draft and rebuild context tree
                        if let Err(e) = state.clear_draft() {
                            tracing::warn!("Failed to clear draft buffer: {}", e);
                        }
                        if let Err(e) = state.rebuild_context_tree(&cached_doc_mut.symbol_table) {
                            tracing::warn!("Failed to rebuild context tree: {}", e);
                        }
                        // Phase 9.5: Invalidate scope cache after structural change
                        state.invalidate_scope_cache();
                    }
                } else {
                    // Multi-line edit - clear draft and rebuild context tree
                    if let Err(e) = state.clear_draft() {
                        tracing::warn!("Failed to clear draft buffer: {}", e);
                    }
                    if let Err(e) = state.rebuild_context_tree(&cached_doc_mut.symbol_table) {
                        tracing::warn!("Failed to rebuild context tree: {}", e);
                    }
                    // Phase 9.5: Invalidate scope cache after structural change
                    state.invalidate_scope_cache();
                }
            } else {
                // Full document replacement - clear draft and rebuild context tree
                if let Err(e) = state.clear_draft() {
                    tracing::warn!("Failed to clear draft buffer: {}", e);
                }
                if let Err(e) = state.rebuild_context_tree(&cached_doc_mut.symbol_table) {
                    tracing::warn!("Failed to rebuild context tree: {}", e);
                }
                // Phase 9.5: Invalidate scope cache after structural change
                state.invalidate_scope_cache();
            }
        }
    }
}
