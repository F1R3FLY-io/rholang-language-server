use std::sync::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use crate::ir::node::Position;
use tower_lsp::lsp_types::Url;

/// Represents the type of a symbol in Rholang.
#[derive(Debug, Clone, PartialEq)]
pub enum SymbolType {
    Variable,
    Contract,
    Parameter,
}

/// Stores information about a symbol, including its declaration and definition locations.
#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
    pub symbol_type: SymbolType,
    pub declaration_uri: Url,
    pub declaration_location: Position,
    pub definition_location: Option<Position>,
}

impl Symbol {
    /// Creates a new symbol with the given attributes.
    pub fn new(name: String, symbol_type: SymbolType, declaration_uri: Url, declaration_location: Position) -> Self {
        Symbol {
            name,
            symbol_type,
            declaration_uri,
            declaration_location,
            definition_location: None,
        }
    }
}

/// A hierarchical symbol table with parent-child scoping.
#[derive(Debug, Clone)]
pub struct SymbolTable {
    pub symbols: Arc<RwLock<HashMap<String, Arc<Symbol>>>>,
    parent: Option<Arc<SymbolTable>>,
}

impl SymbolTable {
    /// Creates a new symbol table with an optional parent.
    pub fn new(parent: Option<Arc<SymbolTable>>) -> Self {
        SymbolTable {
            symbols: Arc::new(RwLock::new(HashMap::new())),
            parent,
        }
    }

    /// Inserts a symbol into the current scope.
    pub fn insert(&self, symbol: Arc<Symbol>) {
        self.symbols.write().unwrap().insert(symbol.name.clone(), symbol);
    }

    /// Looks up a symbol by name, traversing up the scope chain if necessary.
    pub fn lookup(&self, name: &str) -> Option<Arc<Symbol>> {
        self.symbols.read().unwrap().get(name).cloned()
            .or_else(|| self.parent.as_ref().and_then(|p| p.lookup(name)))
    }

    /// Updates the definition location of an existing symbol.
    pub fn update_definition(&self, name: &str, location: Position) {
        if let Some(symbol) = self.symbols.write().unwrap().get_mut(name) {
            Arc::make_mut(symbol).definition_location = Some(location);
        } else if let Some(parent) = &self.parent {
            parent.update_definition(name, location);
        }
    }

    /// Collects all symbols in the current scope and its parents for code completion.
    pub fn collect_all_symbols(&self) -> Vec<Arc<Symbol>> {
        let mut symbols = self.symbols.read().unwrap().values().cloned().collect::<Vec<_>>();
        if let Some(parent) = &self.parent {
            symbols.extend(parent.collect_all_symbols());
        }
        symbols
    }

    pub fn current_symbols(&self) -> Vec<Arc<Symbol>> {
        self.symbols.read().unwrap().values().cloned().collect()
    }

    /// Returns the parent symbol table, if any.
    pub fn parent(&self) -> Option<Arc<SymbolTable>> {
        self.parent.clone()
    }
}
