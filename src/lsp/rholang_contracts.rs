//! Global contract storage for Rholang
//!
//! This module provides a lock-free, efficient data structure for tracking
//! Rholang contracts (global symbols) across the entire workspace with the following constraints:
//!
//! - **Single declaration** per contract (Rholang semantic rule)
//! - **At most one definition** per contract (may equal declaration location)
//! - **Multiple references** per contract (unlimited usages)
//! - **Cross-document linking** for contracts visible across files
//!
//! Note: Local symbols (variables, let bindings) are tracked per-document via SymbolTable
//! and inverted_index, not in this global structure.
//!
//! # Architecture
//!
//! This replaces the previous triple-redundant storage:
//! - OLD: `global_symbols` (DashMap<String, (Url, Position)>) - only 1 location
//! - OLD: `global_table` (RwLock<SymbolTable>) - lock contention
//! - OLD: `global_inverted_index` (DashMap<(Url, Position), Vec<(Url, Position)>>)
//! - OLD: per-document `inverted_index` (HashMap<Position, Vec<Position>>) - local symbols
//!
//! NEW: Single unified structure with lock-free operations supporting both global and local symbols.
//!
//! # Contract Keys
//!
//! Contracts are indexed by name only (global scope, visible across files).

use dashmap::DashMap;
use std::sync::Arc;
use std::hash::{Hash, Hasher};
use tower_lsp::lsp_types::Url;

use crate::ir::semantic_node::Position;
use crate::ir::symbol_table::SymbolType;

// SymbolKey removed - contracts are now keyed by String name only.
// Local symbols are handled per-document via SymbolTable and inverted_index.

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
        // Only add if not already present (deduplicate)
        if !self.references.iter().any(|r| r.uri == reference.uri && r.position == reference.position) {
            self.references.push(reference);
        }
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

/// Global contract storage for Rholang workspace
///
/// Stores only **global contracts** (visible across all files).
/// Local symbols (variables, let bindings) are tracked per-document.
///
/// # Concurrency
/// - Lock-free via DashMap
/// - Thread-safe: all operations are atomic
/// - No RwLock contention
///
/// # Memory Efficiency
/// - Single storage location for global contracts
/// - Contracts stored once with all their metadata
/// - References stored as compact Vec
#[derive(Debug)]
pub struct RholangContracts {
    /// Maps contract name -> ContractDeclaration
    /// Lock-free concurrent hash map
    contracts: Arc<DashMap<String, SymbolDeclaration>>,
}

impl RholangContracts {
    /// Create a new empty contract storage
    pub fn new() -> Self {
        Self {
            contracts: Arc::new(DashMap::new()),
        }
    }

    /// Insert or update a contract declaration
    ///
    /// # Arguments
    /// - `name`: Contract name (extracted from Var or Quote)
    /// - `symbol_type`: Should be SymbolType::Contract
    /// - `declaration`: Declaration location
    ///
    /// # Returns
    /// - `Ok(())` if inserted successfully
    /// - `Err(())` if contract already exists with different declaration (conflict)
    ///
    /// # Constraints
    /// - Rholang allows only ONE declaration per contract
    /// - If contract exists, declaration must match
    ///
    /// # Note
    /// This only stores global contracts. Local symbols (variables) should use
    /// per-document SymbolTable and inverted_index instead.
    pub fn insert_declaration(
        &self,
        name: String,
        symbol_type: SymbolType,
        declaration: SymbolLocation,
    ) -> Result<(), ()> {
        use dashmap::mapref::entry::Entry;

        // Only store contracts (global symbols)
        if !matches!(symbol_type, SymbolType::Contract) {
            // Local symbols should not be stored here
            return Err(());
        }

        match self.contracts.entry(name.clone()) {
            Entry::Occupied(entry) => {
                // Contract already exists - verify declaration matches
                let existing = entry.get();
                if existing.declaration == declaration {
                    Ok(())
                } else {
                    // Conflict: different declaration for same contract
                    Err(())
                }
            }
            Entry::Vacant(entry) => {
                // New contract - insert
                entry.insert(SymbolDeclaration::new(name, symbol_type, declaration));
                Ok(())
            }
        }
    }

    /// Set the definition location for a contract
    ///
    /// # Arguments
    /// - `name`: Contract name
    /// - `definition`: Definition location
    ///
    /// # Returns
    /// - `Ok(())` if definition set successfully
    /// - `Err(())` if contract not found
    ///
    /// # Constraints
    /// - Contract must already be declared
    /// - Definition is ignored if it equals declaration location
    pub fn set_definition(
        &self,
        name: &str,
        definition: SymbolLocation,
    ) -> Result<(), ()> {
        if let Some(mut entry) = self.contracts.get_mut(name) {
            entry.set_definition(definition);
            Ok(())
        } else {
            Err(())
        }
    }

    /// Add a reference/usage location for a contract
    ///
    /// # Arguments
    /// - `name`: Contract name
    /// - `reference`: Usage location
    ///
    /// # Returns
    /// - `Ok(())` if reference added
    /// - `Err(())` if contract not found
    pub fn add_reference(
        &self,
        name: &str,
        reference: SymbolLocation,
    ) -> Result<(), ()> {
        if let Some(mut contract) = self.contracts.get_mut(name) {
            contract.add_reference(reference);
            Ok(())
        } else {
            Err(())
        }
    }

    /// Look up a contract by name
    ///
    /// # Returns
    /// - `Some(SymbolDeclaration)` if found
    /// - `None` if not found
    pub fn lookup(&self, name: &str) -> Option<SymbolDeclaration> {
        self.contracts.get(name).map(|entry| entry.value().clone())
    }

    /// Get definition locations (declaration + optional definition) for a contract
    ///
    /// # Returns
    /// - Vec of 1-2 locations
    /// - Empty vec if contract not found
    pub fn get_definition_locations(&self, name: &str) -> Vec<SymbolLocation> {
        self.contracts
            .get(name)
            .map(|entry| entry.value().definition_locations())
            .unwrap_or_default()
    }

    /// Get all references for a contract
    ///
    /// # Returns
    /// - Vec of reference locations
    /// - Empty vec if contract not found
    pub fn get_references(&self, name: &str) -> Vec<SymbolLocation> {
        self.contracts
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
        for entry in self.contracts.iter() {
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
        self.contracts.retain(|_, symbol| {
            symbol.declaration.uri != *uri
        });
    }

    /// Clear all symbols
    pub fn clear(&self) {
        self.contracts.clear();
    }

    /// Get total number of symbols
    pub fn len(&self) -> usize {
        self.contracts.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.contracts.is_empty()
    }

    /// Get all contract names
    pub fn contract_names(&self) -> Vec<String> {
        self.contracts
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Get all contracts of a specific type (typically all will be Contract type)
    pub fn contracts_of_type(&self, symbol_type: SymbolType) -> Vec<SymbolDeclaration> {
        self.contracts
            .iter()
            .filter(|entry| entry.value().symbol_type == symbol_type)
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Remove all contracts declared in a specific URI (incremental update support)
    ///
    /// This is used when a file is modified or deleted - we remove all contracts
    /// that were declared in that file, then re-index it.
    ///
    /// # Arguments
    /// - `uri`: Document URI to remove contracts from
    ///
    /// # Returns
    /// - Number of contracts removed
    pub fn remove_contracts_from_uri(&self, uri: &Url) -> usize {
        let mut removed_count = 0;

        // Collect contract names to remove (avoid holding iter while mutating)
        let to_remove: Vec<String> = self.contracts
            .iter()
            .filter(|entry| &entry.value().declaration.uri == uri)
            .map(|entry| entry.key().clone())
            .collect();

        // Remove collected contracts
        for name in &to_remove {
            self.contracts.remove(name);
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
        for mut entry in self.contracts.iter_mut() {
            let symbol = entry.value_mut();
            let before_len = symbol.references.len();
            symbol.references.retain(|ref_loc| &ref_loc.uri != uri);
            removed_count += before_len - symbol.references.len();
        }

        removed_count
    }

    /// Remove a specific contract by name
    ///
    /// # Returns
    /// - `Some(SymbolDeclaration)` if contract existed and was removed
    /// - `None` if contract didn't exist
    pub fn remove_contract(&self, name: &str) -> Option<SymbolDeclaration> {
        self.contracts.remove(name).map(|(_, v)| v)
    }
}

impl Default for RholangContracts {
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
        let symbols = RholangContracts::new();

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
        let symbols = RholangContracts::new();
        let loc = SymbolLocation::new(test_uri("main.rho"), test_position(10, 5));

        symbols.insert_declaration("x".to_string(), SymbolType::Contract, loc.clone()).unwrap();

        // Same location - should succeed
        let result = symbols.insert_declaration("x".to_string(), SymbolType::Contract, loc);
        assert!(result.is_ok());
    }

    #[test]
    fn test_duplicate_declaration_different_location() {
        let symbols = RholangContracts::new();

        symbols.insert_declaration(
            "x".to_string(),
            SymbolType::Contract,
            SymbolLocation::new(test_uri("main.rho"), test_position(10, 5)),
        ).unwrap();

        // Different location - should fail (conflict)
        let result = symbols.insert_declaration(
            "x".to_string(),
            SymbolType::Contract,
            SymbolLocation::new(test_uri("main.rho"), test_position(20, 10)),
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_set_definition() {
        let symbols = RholangContracts::new();

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
        let symbols = RholangContracts::new();
        let loc = SymbolLocation::new(test_uri("main.rho"), test_position(10, 5));

        symbols.insert_declaration(
            "x".to_string(),
            SymbolType::Contract,
            loc.clone(),
        ).unwrap();

        // Set definition to same location - should be ignored
        symbols.set_definition("x", loc).unwrap();

        let locs = symbols.get_definition_locations("x");
        assert_eq!(locs.len(), 1); // Only declaration
    }

    #[test]
    fn test_add_references() {
        let symbols = RholangContracts::new();

        symbols.insert_declaration(
            "x".to_string(),
            SymbolType::Contract,
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
        let symbols = RholangContracts::new();
        let decl_loc = SymbolLocation::new(test_uri("main.rho"), test_position(5, 0));

        symbols.insert_declaration("x".to_string(), SymbolType::Contract, decl_loc.clone()).unwrap();
        symbols.add_reference("x", SymbolLocation::new(test_uri("main.rho"), test_position(10, 5))).unwrap();

        let refs = symbols.get_references_at(&test_uri("main.rho"), test_position(5, 0));
        assert_eq!(refs.len(), 1);
    }

    #[test]
    fn test_remove_document() {
        let symbols = RholangContracts::new();

        symbols.insert_declaration(
            "x".to_string(),
            SymbolType::Contract,
            SymbolLocation::new(test_uri("main.rho"), test_position(5, 0)),
        ).unwrap();

        symbols.insert_declaration(
            "y".to_string(),
            SymbolType::Contract,
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
        let symbols = RholangContracts::new();

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
            SymbolType::Contract,
            SymbolLocation::new(test_uri("main.rho"), test_position(15, 0)),
        ).unwrap();

        let contracts = symbols.contracts_of_type(SymbolType::Contract);
        assert_eq!(contracts.len(), 3); // All 3 symbols are contracts
    }
}
