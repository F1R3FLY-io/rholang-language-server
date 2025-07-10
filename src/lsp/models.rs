use std::cmp;
use std::collections::HashMap;
use std::sync::Arc;
use ropey::Rope;
use tower_lsp::lsp_types::{TextDocumentContentChangeEvent, Url};
use tree_sitter::Tree;
use crate::ir::node::{Node, Position};
use crate::ir::symbol_table::SymbolTable;
use crate::ir::transforms::symbol_table_builder::InvertedIndex;

/// Changes associated with a specific version of the document.
#[derive(Debug)]
pub struct VersionedChanges {
    pub version: i32,
    pub changes: Vec<TextDocumentContentChangeEvent>,
}

impl PartialEq for VersionedChanges {
    fn eq(&self, other: &Self) -> bool {
        self.version == other.version
    }
}

impl Eq for VersionedChanges {}

impl PartialOrd for VersionedChanges {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(other.version.cmp(&self.version))
    }
}

impl Ord for VersionedChanges {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        other.version.cmp(&self.version)
    }
}

/// History of changes to a document, including initial text and versioned changes.
#[derive(Debug)]
pub struct LspDocumentHistory {
    pub text: String,
    pub changes: Vec<VersionedChanges>,
}

/// Mutable state of an LSP document, including text rope and version.
#[derive(Debug)]
pub struct LspDocumentState {
    pub uri: Url,
    pub text: Rope,
    pub version: i32,
    pub history: LspDocumentHistory,
}

/// Represents an open LSP document with a unique ID and shared state.
#[derive(Debug)]
pub struct LspDocument {
    pub id: u32,
    pub state: tokio::sync::RwLock<LspDocumentState>,
}

/// Cached processed data for a document, including IR, symbols, and positions.
#[derive(Debug, Clone)]
pub struct CachedDocument {
    pub ir: Arc<Node<'static>>, // Lifetime cast safe with Arc<Tree>
    pub tree: Arc<Tree>,        // Shared ownership to extend lifetime
    pub symbol_table: Arc<SymbolTable>,
    pub inverted_index: InvertedIndex,
    pub version: i32,
    pub text: String,
    pub positions: Arc<HashMap<usize, (Position, Position)>>, // Cached start and end positions
}

/// State of the entire workspace, including all documents and global symbols.
#[derive(Debug)]
pub struct WorkspaceState {
    pub documents: HashMap<Url, Arc<CachedDocument>>, // Cached parsed data
    pub global_symbols: HashMap<String, (Url, Position)>, // Global symbol table
    pub global_table: Arc<SymbolTable>, // Global scope for contracts
    pub global_inverted_index: HashMap<(Url, Position), Vec<(Url, Position)>>, // Cross-file usages
}
