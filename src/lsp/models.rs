use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use dashmap::DashMap;
use parking_lot::RwLock;
use ropey::Rope;

use tower_lsp::lsp_types::{TextDocumentContentChangeEvent, Url};
use tree_sitter::Tree;

use crate::ir::rholang_node::{RholangNode, Position as IrPosition};
use crate::ir::metta_node::MettaNode;
use crate::ir::semantic_node::SemanticNode;
use crate::ir::symbol_table::SymbolTable;
use crate::ir::transforms::symbol_table_builder::InvertedIndex;
use crate::ir::global_index::GlobalSymbolIndex;
use crate::lsp::symbol_index::SymbolIndex;
use crate::lsp::features::completion::incremental::DocumentCompletionState;

/// Language detected for a document based on file extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentLanguage {
    /// Rholang source file (.rho)
    Rholang,
    /// MeTTa source file (.metta, .metta2)
    Metta,
    /// Unknown or unsupported file type
    Unknown,
}

impl DocumentLanguage {
    /// Detects the language of a document from its URI.
    ///
    /// # Arguments
    /// * `uri` - The document's URI
    ///
    /// # Returns
    /// The detected language based on file extension
    ///
    /// # Examples
    /// ```ignore
    /// let lang = DocumentLanguage::from_uri(&uri);
    /// match lang {
    ///     DocumentLanguage::Rholang => { /* parse as Rholang */ }
    ///     DocumentLanguage::Metta => { /* parse as MeTTa */ }
    ///     DocumentLanguage::Unknown => { /* default to Rholang */ }
    /// }
    /// ```
    pub fn from_uri(uri: &Url) -> Self {
        match uri.path().rsplit('.').next() {
            Some("rho") => DocumentLanguage::Rholang,
            Some("metta") | Some("metta2") => DocumentLanguage::Metta,
            _ => DocumentLanguage::Unknown,
        }
    }
}

/// Changes associated with a specific version of the document.
#[derive(Debug)]
pub struct VersionedChanges {
    pub version: i32,
    pub changes: Vec<TextDocumentContentChangeEvent>,
}

/// Represents a cached document with IR, symbol table, and metadata for LSP queries.
///
/// Phase 4: Removed potential_global_refs - now handled by rholang_symbols in WorkspaceState.
#[derive(Debug)]
pub struct CachedDocument {
    /// Language-specific IR (RholangNode or MettaNode)
    ///
    /// DEPRECATED: This field will be replaced by `document_ir.root` in a future version.
    /// For backward compatibility, both fields are populated during the migration period.
    pub ir: Arc<RholangNode>,

    /// Position-indexed AST for O(log n) node lookups
    ///
    /// Phase 6 optimization: Enables fast position-based node queries using a BTreeMap
    /// instead of O(n) tree traversal. Expected 60-70% improvement for large files.
    pub position_index: Arc<crate::lsp::position_index::PositionIndex>,

    /// NEW: Document IR with separate comment channel
    ///
    /// This field contains the semantic tree (same as `ir`) plus a separate channel
    /// of comments sorted by position. The comment channel enables:
    /// - Directive parsing for embedded languages (e.g., `// @metta`)
    /// - Documentation extraction (e.g., `///` or `/**` doc comments)
    /// - Comment-aware features without polluting the semantic tree
    ///
    /// During the migration period (Phase 1), this field is optional and both
    /// `ir` and `document_ir.root` are kept in sync. In Phase 4, `ir` will be removed.
    pub document_ir: Option<Arc<crate::ir::DocumentIR>>,

    /// MeTTa-specific IR (only present for MeTTa files)
    pub metta_ir: Option<Vec<Arc<MettaNode>>>,
    /// Language-agnostic unified IR for cross-language features
    pub unified_ir: Arc<dyn SemanticNode>,
    /// Language detected from file extension
    pub language: DocumentLanguage,
    /// Tree-sitter parse tree
    pub tree: Arc<Tree>,
    /// Symbol table for this document
    pub symbol_table: Arc<SymbolTable>,
    /// Inverted index: Maps declaration position -> reference positions for local symbols
    /// Used for find-references and rename operations on local variables
    pub inverted_index: InvertedIndex,
    /// Document version number
    pub version: i32,
    /// Document text content
    pub text: Rope,
    /// Position mappings for IR nodes
    pub positions: Arc<std::collections::HashMap<usize, (IrPosition, IrPosition)>>,
    /// Suffix array-based symbol index for O(m log n + k) substring search
    pub symbol_index: Arc<SymbolIndex>,
    /// Fast hash of document content for change detection
    pub content_hash: u64,
    /// Phase 9: Incremental completion state for 10-50x faster code completion
    /// Caches completion dictionary + draft buffers to avoid re-indexing on every keystroke
    pub completion_state: Option<Arc<RwLock<DocumentCompletionState>>>,
}

/// State for an open text document managed by the LSP server.
#[derive(Debug)]
pub struct LspDocumentState {
    pub uri: Url,
    pub text: Rope,
    pub version: i32,
    pub history: LspDocumentHistory,
}

/// History of changes for incremental parsing and validation.
#[derive(Debug)]
pub struct LspDocumentHistory {
    pub text: String,
    pub changes: Vec<VersionedChanges>,
}

/// Workspace indexing state for Phase 2 lazy initialization optimization.
///
/// This enum tracks the progress of background workspace indexing:
/// - Idle: No indexing in progress
/// - InProgress: Currently indexing workspace files
/// - Complete: Indexing finished successfully
/// - Failed: Indexing encountered an error
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexingState {
    /// No indexing operation in progress
    Idle,
    /// Indexing in progress
    /// - total: Total number of files to index
    /// - completed: Number of files indexed so far
    InProgress { total: usize, completed: usize },
    /// Indexing completed successfully
    Complete,
    /// Indexing failed with error message
    Failed(String),
}

/// LSP document with state for open files.
#[derive(Debug)]
pub struct LspDocument {
    pub id: u32,
    pub state: tokio::sync::RwLock<LspDocumentState>,
}

/// Workspace state for cached documents and global symbols.
///
/// Optimized for concurrent access with lock-free data structures:
/// - DashMap for high-frequency reads/writes (documents, symbols)
/// - Separate RwLocks for infrequent bulk updates (indexes, tables)
///
/// This design eliminates lock contention on hot paths (goto_definition, references)
/// while maintaining consistency for batch operations (workspace indexing).
#[derive(Debug)]
pub struct WorkspaceState {
    /// Lock-free concurrent document cache
    /// Most frequently accessed - needs zero-contention reads
    pub documents: Arc<DashMap<Url, Arc<CachedDocument>>>,

    /// Symbol table for global scope
    /// Infrequent updates (only during workspace indexing)
    pub global_table: Arc<tokio::sync::RwLock<SymbolTable>>,

    /// REMOVED (Priority 2b): global_inverted_index - now stored in rholang_symbols
    // pub global_inverted_index: Arc<DashMap<(Url, IrPosition), Vec<(Url, IrPosition)>>>,

    /// Lock-free contract tracking by URI
    /// Allows concurrent contract discovery without blocking
    pub global_contracts: Arc<DashMap<Url, Vec<Arc<RholangNode>>>>,

    /// Lock-free call tracking by URI
    /// Allows concurrent call tracking without blocking
    pub global_calls: Arc<DashMap<Url, Vec<Arc<RholangNode>>>>,

    /// Global symbol index using MORK pattern matching for O(k) lookups
    /// Separate lock as it's updated less frequently than document cache
    /// Uses std::sync::RwLock because it's accessed from blocking/sync code (Rayon threads)
    pub global_index: Arc<std::sync::RwLock<GlobalSymbolIndex>>,

    /// Global symbols from all virtual documents across the workspace, organized by language
    /// Lock-free nested structure: language -> symbol -> locations
    /// Enables concurrent virtual document indexing without blocking
    /// Example: global_virtual_symbols.get("metta").get("get_neighbors") = [(virtual_uri_1, range1), ...]
    pub global_virtual_symbols: Arc<DashMap<String, Arc<DashMap<String, Vec<(Url, tower_lsp::lsp_types::Range)>>>>>,

    /// NEW: Unified Rholang symbol storage (replaces global_symbols + global_table + global_inverted_index)
    /// Lock-free, single-source-of-truth for all Rholang symbols
    /// Enforces Rholang constraints: 1 declaration + 0-1 definition + N references
    pub rholang_symbols: Arc<crate::lsp::rholang_contracts::RholangContracts>,

    /// Phase 2 optimization: Track workspace indexing state for lazy initialization
    /// Wrapped in RwLock as it's updated infrequently (only during indexing lifecycle changes)
    pub indexing_state: Arc<tokio::sync::RwLock<IndexingState>>,

    /// Fuzzy completion index using liblevenshtein DynamicDawg
    /// Lock-free concurrent access for fast completion queries
    pub completion_index: Arc<crate::lsp::features::completion::WorkspaceCompletionIndex>,
}

impl WorkspaceState {
    /// Create a new empty workspace state with lock-free concurrent data structures
    pub fn new() -> Self {
        Self {
            documents: Arc::new(DashMap::new()),
            global_table: Arc::new(tokio::sync::RwLock::new(SymbolTable::new(None))),
            // REMOVED (Priority 2b): global_inverted_index initialization
            global_contracts: Arc::new(DashMap::new()),
            global_calls: Arc::new(DashMap::new()),
            global_index: Arc::new(std::sync::RwLock::new(GlobalSymbolIndex::new())),
            global_virtual_symbols: Arc::new(DashMap::new()),
            rholang_symbols: Arc::new(crate::lsp::rholang_contracts::RholangContracts::new()),
            indexing_state: Arc::new(tokio::sync::RwLock::new(IndexingState::Idle)),
            completion_index: Arc::new(crate::lsp::features::completion::WorkspaceCompletionIndex::new()),
        }
    }
}

impl Default for WorkspaceState {
    fn default() -> Self {
        Self::new()
    }
}
