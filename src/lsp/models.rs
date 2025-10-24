use std::collections::HashMap;
use std::sync::Arc;

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
#[derive(Debug)]
pub struct CachedDocument {
    /// Language-specific IR (RholangNode or MettaNode)
    pub ir: Arc<RholangNode>,
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
    /// Inverted index for rename/references
    pub inverted_index: InvertedIndex,
    /// Document version number
    pub version: i32,
    /// Document text content
    pub text: Rope,
    /// Position mappings for IR nodes
    pub positions: Arc<std::collections::HashMap<usize, (IrPosition, IrPosition)>>,
    /// Potential cross-file symbol references
    pub potential_global_refs: Vec<(String, IrPosition)>,
    /// Suffix array-based symbol index for O(m log n + k) substring search
    pub symbol_index: Arc<SymbolIndex>,
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

/// LSP document with state for open files.
#[derive(Debug)]
pub struct LspDocument {
    pub id: u32,
    pub state: tokio::sync::RwLock<LspDocumentState>,
}

/// Workspace state for cached documents and global symbols.
#[derive(Debug)]
pub struct WorkspaceState {
    pub documents: HashMap<Url, Arc<CachedDocument>>,
    pub global_symbols: HashMap<String, (Url, IrPosition)>,
    pub global_table: Arc<SymbolTable>,
    pub global_inverted_index: HashMap<(Url, IrPosition), Vec<(Url, IrPosition)>>,
    pub global_contracts: Vec<(Url, Arc<RholangNode>)>,
    pub global_calls: Vec<(Url, Arc<RholangNode>)>,
    /// Global symbol index using MORK pattern matching for O(k) lookups
    pub global_index: Arc<std::sync::RwLock<GlobalSymbolIndex>>,
}
