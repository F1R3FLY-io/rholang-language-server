//! Unified global symbol storage for Rholang
//!
//! This module provides a lock-free, efficient data structure for tracking
//! Rholang symbols across the entire workspace with the following constraints:
//!
//! - **Single declaration** per symbol (Rholang semantic rule)
//! - **At most one definition** per symbol (may equal declaration location)
//! - **Multiple references** per symbol (unlimited usages)
//! - **Cross-document linking** for global symbols (contracts)
//!
//! # Architecture
//!
//! This replaces the previous triple-redundant storage:
//! - OLD: `global_symbols` (DashMap<String, (Url, Position)>) - only 1 location
//! - OLD: `global_table` (RwLock<SymbolTable>) - lock contention
//! - OLD: `global_inverted_index` (DashMap<(Url, Position), Vec<(Url, Position)>>)
//!
//! NEW: Single unified structure with lock-free operations.

use dashmap::DashMap;
use std::sync::Arc;
use tower_lsp::lsp_types::Url;

use crate::ir::semantic_node::Position;
use crate::ir::symbol_table::SymbolType;

/// Location of a symbol in the source code
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolLocation {
    pub uri: Url,
    pub position: Position,
}

impl SymbolLocation {
    pub fn new(uri: Url, position: Position) -> Self {
        Self { uri, position }
    }
}

/// Complete information about a Rholang symbol's declaration, definition, and usages
#[derive(Debug, Clone)]
pub struct SymbolDeclaration {
    /// Symbol name
    pub name: String,

    /// Symbol type (Contract, Variable, Parameter, etc.)
    pub symbol_type: SymbolType,

    /// Declaration location (always present)
    pub declaration: SymbolLocation,

    /// Definition location (if different from declaration)
    /// For contracts: location of contract body
    /// For variables: location where value is bound
    pub definition: Option<SymbolLocation>,

    /// All usage/reference locations
    pub references: Vec<SymbolLocation>,
}

impl SymbolDeclaration {
    /// Create a new symbol declaration
    pub fn new(
        name: String,
        symbol_type: SymbolType,
        declaration: SymbolLocation,
    ) -> Self {
        Self {
            name,
            symbol_type,
            declaration,
            definition: None,
            references: Vec::new(),
        }
    }

    /// Set the definition location (must be different from declaration)
    pub fn set_definition(&mut self, definition: SymbolLocation) {
        if definition != self.declaration {
            self.definition = Some(definition);
        }
    }

    /// Add a reference/usage location
    pub fn add_reference(&mut self, reference: SymbolLocation) {
        self.references.push(reference);
    }

    /// Get all locations: declaration + optional definition
    pub fn definition_locations(&self) -> Vec<SymbolLocation> {
        let mut locations = vec![self.declaration.clone()];
        if let Some(def) = &self.definition {
            locations.push(def.clone());
        }
        locations
    }

    /// Get total number of references
    pub fn reference_count(&self) -> usize {
        self.references.len()
    }
}

/// Unified global symbol storage for Rholang workspace
///
/// # Concurrency
/// - Lock-free via DashMap
/// - Thread-safe: all operations are atomic
/// - No RwLock contention
///
/// # Memory Efficiency
/// - Single storage location (no redundancy)
/// - Symbols stored once with all their metadata
/// - References stored as compact Vec
#[derive(Debug)]
pub struct RholangGlobalSymbols {
    /// Maps symbol name -> SymbolDeclaration
    /// Lock-free concurrent hash map
    symbols: Arc<DashMap<String, SymbolDeclaration>>,
}

impl RholangGlobalSymbols {
    /// Create a new empty symbol storage
    pub fn new() -> Self {
        Self {
            symbols: Arc::new(DashMap::new()),
        }
    }

    /// Insert or update a symbol declaration
    ///
    /// # Arguments
    /// - `name`: Symbol name
    /// - `symbol_type`: Type of symbol (Contract, Variable, etc.)
    /// - `declaration`: Declaration location
    ///
    /// # Returns
    /// - `Ok(())` if inserted successfully
    /// - `Err(())` if symbol already exists with different declaration (conflict)
    ///
    /// # Constraints
    /// - Rholang allows only ONE declaration per symbol
    /// - If symbol exists, declaration must match
    pub fn insert_declaration(
        &self,
        name: String,
        symbol_type: SymbolType,
        declaration: SymbolLocation,
    ) -> Result<(), ()> {
        use dashmap::mapref::entry::Entry;

        match self.symbols.entry(name.clone()) {
            Entry::Occupied(entry) => {
                // Symbol already exists - verify declaration matches
                let existing = entry.get();
                if existing.declaration == declaration {
                    Ok(())
                } else {
                    // Conflict: different declaration for same symbol
                    Err(())
                }
            }
            Entry::Vacant(entry) => {
                // New symbol - insert
                entry.insert(SymbolDeclaration::new(name, symbol_type, declaration));
                Ok(())
            }
        }
    }

    /// Set the definition location for a symbol
    ///
    /// # Arguments
    /// - `name`: Symbol name
    /// - `definition`: Definition location
    ///
    /// # Returns
    /// - `Ok(())` if definition set successfully
    /// - `Err(())` if symbol not found
    ///
    /// # Constraints
    /// - Symbol must already be declared
    /// - Definition is ignored if it equals declaration location
    pub fn set_definition(
        &self,
        name: &str,
        definition: SymbolLocation,
    ) -> Result<(), ()> {
        match self.symbols.get_mut(name) {
            Some(mut symbol) => {
                symbol.set_definition(definition);
                Ok(())
            }
            None => Err(()),
        }
    }

    /// Add a reference/usage location for a symbol
    ///
    /// # Arguments
    /// - `name`: Symbol name
    /// - `reference`: Usage location
    ///
    /// # Returns
    /// - `Ok(())` if reference added
    /// - `Err(())` if symbol not found
    pub fn add_reference(
        &self,
        name: &str,
        reference: SymbolLocation,
    ) -> Result<(), ()> {
        match self.symbols.get_mut(name) {
            Some(mut symbol) => {
                symbol.add_reference(reference);
                Ok(())
            }
            None => Err(()),
        }
    }

    /// Look up a symbol by name
    ///
    /// # Returns
    /// - `Some(SymbolDeclaration)` if found
    /// - `None` if not found
    pub fn lookup(&self, name: &str) -> Option<SymbolDeclaration> {
        self.symbols.get(name).map(|entry| entry.value().clone())
    }

    /// Get definition locations (declaration + optional definition)
    ///
    /// # Returns
    /// - Vec of 1-2 locations
    /// - Empty vec if symbol not found
    pub fn get_definition_locations(&self, name: &str) -> Vec<SymbolLocation> {
        self.symbols
            .get(name)
            .map(|entry| entry.value().definition_locations())
            .unwrap_or_default()
    }

    /// Get all references for a symbol
    ///
    /// # Returns
    /// - Vec of reference locations
    /// - Empty vec if symbol not found
    pub fn get_references(&self, name: &str) -> Vec<SymbolLocation> {
        self.symbols
            .get(name)
            .map(|entry| entry.value().references.clone())
            .unwrap_or_default()
    }

    /// Get all references for a symbol at a specific declaration location
    ///
    /// This is used for find-references: given a definition location,
    /// find all usages of that symbol.
    ///
    /// # Arguments
    /// - `uri`: Document URI
    /// - `position`: Position in document
    ///
    /// # Returns
    /// - Vec of reference locations
    /// - Empty vec if no symbol declared at that location
    pub fn get_references_at(&self, uri: &Url, position: Position) -> Vec<SymbolLocation> {
        let target_location = SymbolLocation {
            uri: uri.clone(),
            position,
        };

        // Find symbol with matching declaration
        for entry in self.symbols.iter() {
            let symbol = entry.value();
            if symbol.declaration == target_location {
                return symbol.references.clone();
            }
            // Also check definition location
            if let Some(def) = &symbol.definition {
                if *def == target_location {
                    return symbol.references.clone();
                }
            }
        }

        Vec::new()
    }

    /// Remove all symbols from a specific document
    ///
    /// Used when a document is closed or deleted.
    ///
    /// # Arguments
    /// - `uri`: Document URI to remove symbols from
    pub fn remove_document(&self, uri: &Url) {
        self.symbols.retain(|_, symbol| {
            symbol.declaration.uri != *uri
        });
    }

    /// Clear all symbols
    pub fn clear(&self) {
        self.symbols.clear();
    }

    /// Get total number of symbols
    pub fn len(&self) -> usize {
        self.symbols.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.symbols.is_empty()
    }

    /// Get all symbol names
    pub fn symbol_names(&self) -> Vec<String> {
        self.symbols
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Get all symbols of a specific type
    pub fn symbols_of_type(&self, symbol_type: SymbolType) -> Vec<SymbolDeclaration> {
        self.symbols
            .iter()
            .filter(|entry| entry.value().symbol_type == symbol_type)
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Remove all symbols declared in a specific URI (incremental update support)
    ///
    /// This is used when a file is modified or deleted - we remove all symbols
    /// that were declared in that file, then re-index it.
    ///
    /// # Arguments
    /// - `uri`: Document URI to remove symbols from
    ///
    /// # Returns
    /// - Number of symbols removed
    pub fn remove_symbols_from_uri(&self, uri: &Url) -> usize {
        let mut removed_count = 0;

        // Collect symbol names to remove (avoid holding iter while mutating)
        let to_remove: Vec<String> = self.symbols
            .iter()
            .filter(|entry| &entry.value().declaration.uri == uri)
            .map(|entry| entry.key().clone())
            .collect();

        // Remove collected symbols
        for name in &to_remove {
            self.symbols.remove(name);
            removed_count += 1;
        }

        removed_count
    }

    /// Remove references from a specific URI for all symbols (incremental update support)
    ///
    /// This is used when a file is modified - we remove all references originating
    /// from that file, then re-index those references.
    ///
    /// # Arguments
    /// - `uri`: Document URI to remove references from
    ///
    /// # Returns
    /// - Number of references removed across all symbols
    pub fn remove_references_from_uri(&self, uri: &Url) -> usize {
        let mut removed_count = 0;

        // Iterate all symbols and remove references from the given URI
        for mut entry in self.symbols.iter_mut() {
            let symbol = entry.value_mut();
            let before_len = symbol.references.len();
            symbol.references.retain(|ref_loc| &ref_loc.uri != uri);
            removed_count += before_len - symbol.references.len();
        }

        removed_count
    }

    /// Remove a specific symbol by name (for fine-grained delta tracking)
    ///
    /// # Returns
    /// - `Some(SymbolDeclaration)` if symbol existed and was removed
    /// - `None` if symbol didn't exist
    pub fn remove_symbol(&self, name: &str) -> Option<SymbolDeclaration> {
        self.symbols.remove(name).map(|(_, v)| v)
    }
}

impl Default for RholangGlobalSymbols {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::symbol_table::SymbolType;

    fn test_uri(path: &str) -> Url {
        Url::parse(&format!("file:///test/{}", path)).unwrap()
    }

    fn test_position(row: usize, col: usize) -> Position {
        Position { row, column: col, byte: 0 }
    }

    #[test]
    fn test_insert_and_lookup() {
        let symbols = RholangGlobalSymbols::new();

        let result = symbols.insert_declaration(
            "myContract".to_string(),
            SymbolType::Contract,
            SymbolLocation::new(test_uri("main.rho"), test_position(10, 5)),
        );

        assert!(result.is_ok());

        let found = symbols.lookup("myContract");
        assert!(found.is_some());
        let symbol = found.unwrap();
        assert_eq!(symbol.name, "myContract");
        assert_eq!(symbol.symbol_type, SymbolType::Contract);
    }

    #[test]
    fn test_duplicate_declaration_same_location() {
        let symbols = RholangGlobalSymbols::new();
        let loc = SymbolLocation::new(test_uri("main.rho"), test_position(10, 5));

        symbols.insert_declaration("x".to_string(), SymbolType::Variable, loc.clone()).unwrap();

        // Same location - should succeed
        let result = symbols.insert_declaration("x".to_string(), SymbolType::Variable, loc);
        assert!(result.is_ok());
    }

    #[test]
    fn test_duplicate_declaration_different_location() {
        let symbols = RholangGlobalSymbols::new();

        symbols.insert_declaration(
            "x".to_string(),
            SymbolType::Variable,
            SymbolLocation::new(test_uri("main.rho"), test_position(10, 5)),
        ).unwrap();

        // Different location - should fail (conflict)
        let result = symbols.insert_declaration(
            "x".to_string(),
            SymbolType::Variable,
            SymbolLocation::new(test_uri("main.rho"), test_position(20, 10)),
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_set_definition() {
        let symbols = RholangGlobalSymbols::new();

        symbols.insert_declaration(
            "myContract".to_string(),
            SymbolType::Contract,
            SymbolLocation::new(test_uri("main.rho"), test_position(10, 5)),
        ).unwrap();

        symbols.set_definition(
            "myContract",
            SymbolLocation::new(test_uri("main.rho"), test_position(15, 10)),
        ).unwrap();

        let locs = symbols.get_definition_locations("myContract");
        assert_eq!(locs.len(), 2); // declaration + definition
    }

    #[test]
    fn test_definition_same_as_declaration() {
        let symbols = RholangGlobalSymbols::new();
        let loc = SymbolLocation::new(test_uri("main.rho"), test_position(10, 5));

        symbols.insert_declaration(
            "x".to_string(),
            SymbolType::Variable,
            loc.clone(),
        ).unwrap();

        // Set definition to same location - should be ignored
        symbols.set_definition("x", loc).unwrap();

        let locs = symbols.get_definition_locations("x");
        assert_eq!(locs.len(), 1); // Only declaration
    }

    #[test]
    fn test_add_references() {
        let symbols = RholangGlobalSymbols::new();

        symbols.insert_declaration(
            "x".to_string(),
            SymbolType::Variable,
            SymbolLocation::new(test_uri("main.rho"), test_position(5, 0)),
        ).unwrap();

        symbols.add_reference(
            "x",
            SymbolLocation::new(test_uri("main.rho"), test_position(10, 5)),
        ).unwrap();

        symbols.add_reference(
            "x",
            SymbolLocation::new(test_uri("main.rho"), test_position(15, 10)),
        ).unwrap();

        let refs = symbols.get_references("x");
        assert_eq!(refs.len(), 2);
    }

    #[test]
    fn test_get_references_at_declaration() {
        let symbols = RholangGlobalSymbols::new();
        let decl_loc = SymbolLocation::new(test_uri("main.rho"), test_position(5, 0));

        symbols.insert_declaration("x".to_string(), SymbolType::Variable, decl_loc.clone()).unwrap();
        symbols.add_reference("x", SymbolLocation::new(test_uri("main.rho"), test_position(10, 5))).unwrap();

        let refs = symbols.get_references_at(&test_uri("main.rho"), test_position(5, 0));
        assert_eq!(refs.len(), 1);
    }

    #[test]
    fn test_remove_document() {
        let symbols = RholangGlobalSymbols::new();

        symbols.insert_declaration(
            "x".to_string(),
            SymbolType::Variable,
            SymbolLocation::new(test_uri("main.rho"), test_position(5, 0)),
        ).unwrap();

        symbols.insert_declaration(
            "y".to_string(),
            SymbolType::Variable,
            SymbolLocation::new(test_uri("other.rho"), test_position(5, 0)),
        ).unwrap();

        assert_eq!(symbols.len(), 2);

        symbols.remove_document(&test_uri("main.rho"));

        assert_eq!(symbols.len(), 1);
        assert!(symbols.lookup("x").is_none());
        assert!(symbols.lookup("y").is_some());
    }

    #[test]
    fn test_symbols_of_type() {
        let symbols = RholangGlobalSymbols::new();

        symbols.insert_declaration(
            "c1".to_string(),
            SymbolType::Contract,
            SymbolLocation::new(test_uri("main.rho"), test_position(5, 0)),
        ).unwrap();

        symbols.insert_declaration(
            "c2".to_string(),
            SymbolType::Contract,
            SymbolLocation::new(test_uri("main.rho"), test_position(10, 0)),
        ).unwrap();

        symbols.insert_declaration(
            "x".to_string(),
            SymbolType::Variable,
            SymbolLocation::new(test_uri("main.rho"), test_position(15, 0)),
        ).unwrap();

        let contracts = symbols.symbols_of_type(SymbolType::Contract);
        assert_eq!(contracts.len(), 2);
    }
}
