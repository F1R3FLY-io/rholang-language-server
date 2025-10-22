use std::collections::HashMap;
use std::sync::Arc;

use ropey::Rope;

use tower_lsp::lsp_types::{TextDocumentContentChangeEvent, Url};
use tree_sitter::Tree;

use crate::ir::node::{Node, Position as IrPosition};
use crate::ir::symbol_table::SymbolTable;
use crate::ir::transforms::symbol_table_builder::InvertedIndex;

/// Changes associated with a specific version of the document.
#[derive(Debug)]
pub struct VersionedChanges {
    pub version: i32,
    pub changes: Vec<TextDocumentContentChangeEvent>,
}

/// Represents a cached document with IR, symbol table, and metadata for LSP queries.
#[derive(Debug)]
pub struct CachedDocument {
    pub ir: Arc<Node>,
    pub tree: Arc<Tree>,
    pub symbol_table: Arc<SymbolTable>,
    pub inverted_index: InvertedIndex,
    pub version: i32,
    pub text: Rope,
    pub positions: Arc<std::collections::HashMap<usize, (IrPosition, IrPosition)>>,
    pub potential_global_refs: Vec<(String, IrPosition)>,
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
    pub global_contracts: Vec<(Url, Arc<Node>)>,
    pub global_calls: Vec<(Url, Arc<Node>)>,
}
