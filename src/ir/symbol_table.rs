use dashmap::DashMap;
use rustc_hash::FxBuildHasher;  // Phase 2 optimization: ~2x faster than default hasher
use std::sync::Arc;
use crate::ir::rholang_node::{Position, RholangNode};
use tower_lsp::lsp_types::Url;
use rpds::Vector;
use archery::ArcK;

/// Represents the type of a symbol in Rholang.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub enum SymbolType {
    Variable,
    Contract,
    Parameter,
}

/// Stores contract pattern information for pattern matching.
/// This represents the formal parameters and remainder of a contract definition.
#[derive(Debug, Clone)]
pub struct ContractPattern {
    /// The formal parameters (patterns) of the contract
    pub formals: Vector<Arc<RholangNode>, ArcK>,
    /// Optional remainder parameter for variadic contracts
    pub formals_remainder: Option<Arc<RholangNode>>,
    /// The contract body/process
    pub proc: Arc<RholangNode>,
}

/// Stores information about a symbol, including its declaration and definition locations.
/// For contracts, also stores the pattern signature for efficient pattern matching.
#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
    pub symbol_type: SymbolType,
    pub declaration_uri: Url,
    pub declaration_location: Position,
    pub definition_location: Option<Position>,
    /// For Contract symbols: stores the pattern signature
    /// This enables O(1) pattern lookup instead of O(n) IR tree traversal
    pub contract_pattern: Option<ContractPattern>,
    /// For Contract symbols with complex identifiers: stores the full identifier node
    /// Simple identifiers (Var, Quote(StringLiteral)) are stored in `name` only.
    /// Complex identifiers (Quote(Map), Quote(List), etc.) are stored here for structural matching.
    pub contract_identifier_node: Option<Arc<RholangNode>>,
    /// Documentation extracted from doc comments (Phase 5: Completion Item Documentation)
    pub documentation: Option<String>,
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
            contract_pattern: None,
            contract_identifier_node: None,
            documentation: None,
        }
    }

    /// Creates a new contract symbol with pattern information.
    pub fn new_contract(
        name: String,
        declaration_uri: Url,
        declaration_location: Position,
        formals: Vector<Arc<RholangNode>, ArcK>,
        formals_remainder: Option<Arc<RholangNode>>,
        proc: Arc<RholangNode>,
    ) -> Self {
        Symbol {
            name,
            symbol_type: SymbolType::Contract,
            declaration_uri,
            declaration_location,
            definition_location: Some(declaration_location),
            contract_pattern: Some(ContractPattern {
                formals,
                formals_remainder,
                proc,
            }),
            contract_identifier_node: None,
            documentation: None,
        }
    }

    /// Returns the arity (number of parameters) for contract symbols.
    /// Returns None for non-contract symbols.
    pub fn arity(&self) -> Option<usize> {
        self.contract_pattern.as_ref().map(|p| p.formals.len())
    }

    /// Returns true if this contract accepts variadic arguments.
    pub fn is_variadic(&self) -> bool {
        self.contract_pattern.as_ref()
            .map(|p| p.formals_remainder.is_some())
            .unwrap_or(false)
    }
}

// Manual implementations to avoid requiring RholangNode to implement these traits
impl PartialEq for Symbol {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.symbol_type == other.symbol_type
            && self.declaration_uri == other.declaration_uri
            && self.declaration_location == other.declaration_location
            && self.definition_location == other.definition_location
    }
}

impl Eq for Symbol {}

impl std::hash::Hash for Symbol {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.symbol_type.hash(state);
        self.declaration_uri.hash(state);
        self.declaration_location.hash(state);
        self.definition_location.hash(state);
    }
}

impl PartialOrd for Symbol {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Symbol {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.name.cmp(&other.name)
            .then(self.declaration_uri.cmp(&other.declaration_uri))
            .then(self.declaration_location.cmp(&other.declaration_location))
    }
}

/// Pattern signature for efficient contract lookup.
/// Groups contracts by name and arity for O(1) pattern matching queries.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PatternSignature {
    /// Contract name
    pub name: String,
    /// Number of formal parameters (arity)
    pub arity: usize,
    /// Whether the contract accepts variadic arguments
    pub is_variadic: bool,
}

impl PatternSignature {
    pub fn from_symbol(symbol: &Symbol) -> Option<Self> {
        if symbol.symbol_type == SymbolType::Contract {
            Some(PatternSignature {
                name: symbol.name.clone(),
                arity: symbol.arity().unwrap_or(0),
                is_variadic: symbol.is_variadic(),
            })
        } else {
            None
        }
    }

    /// Check if this signature can match a call with the given number of arguments.
    pub fn matches_arity(&self, arg_count: usize) -> bool {
        if self.is_variadic {
            arg_count >= self.arity
        } else {
            arg_count == self.arity
        }
    }
}

/// A hierarchical symbol table with parent-child scoping.
/// Includes PathMap-based pattern indexing for efficient contract lookups.
/// Uses lock-free DashMap for concurrent access from multiple threads.
#[derive(Debug, Clone)]
pub struct SymbolTable {
    /// Lock-free concurrent symbol storage with FxHasher for performance
    /// Eliminates lock contention during symbol lookups from multiple LSP requests
    pub symbols: Arc<DashMap<String, Arc<Symbol>, FxBuildHasher>>,
    /// Lock-free pattern index: maps name -> (signature -> list of contract symbols)
    /// Phase 4 optimization: Two-level index for O(1) name lookup instead of O(n) iteration
    /// First level: contract name -> inner map
    /// Second level: PatternSignature -> list of contract symbols with that signature
    pattern_index: Arc<DashMap<String, DashMap<PatternSignature, Vec<Arc<Symbol>>, FxBuildHasher>, FxBuildHasher>>,
    parent: Option<Arc<SymbolTable>>,
}

impl SymbolTable {
    /// Creates a new symbol table with an optional parent.
    /// Uses lock-free DashMap with FxHasher for optimal concurrent performance.
    pub fn new(parent: Option<Arc<SymbolTable>>) -> Self {
        SymbolTable {
            symbols: Arc::new(DashMap::with_hasher(FxBuildHasher::default())),
            pattern_index: Arc::new(DashMap::with_hasher(FxBuildHasher::default())),
            parent,
        }
    }

    /// Inserts a symbol into the current scope.
    /// If the symbol is a contract, also updates the pattern index.
    /// Lock-free operation using DashMap.
    pub fn insert(&self, symbol: Arc<Symbol>) {
        let name = symbol.name.clone();
        self.symbols.insert(name.clone(), symbol.clone());

        // Update pattern index for contract symbols (Phase 4: two-level index)
        if let Some(sig) = PatternSignature::from_symbol(&symbol) {
            // Get or create the inner map for this contract name
            let inner_map = self.pattern_index
                .entry(name)
                .or_insert_with(|| DashMap::with_hasher(FxBuildHasher::default()));

            // Insert into the inner map using the signature as key
            inner_map.entry(sig).or_insert_with(Vec::new).push(symbol);
        }
    }

    /// Looks up contracts by pattern signature (name + arity).
    /// Phase 4 optimization: O(1) name lookup + O(k) arity filtering where k = overloads for this name.
    /// Previously: O(n) iteration over all contract signatures.
    /// Traverses up the scope chain if necessary.
    /// Lock-free iteration using DashMap.
    pub fn lookup_contracts_by_pattern(&self, name: &str, arg_count: usize) -> Vec<Arc<Symbol>> {
        let mut results = Vec::new();

        // Phase 4: O(1) name lookup, then iterate only that contract's overloads
        if let Some(inner_map) = self.pattern_index.get(name) {
            for entry in inner_map.iter() {
                let (sig, symbols) = entry.pair();
                if sig.matches_arity(arg_count) {
                    results.extend(symbols.iter().cloned());
                }
            }
        }

        // Search parent scope if available
        if let Some(parent) = &self.parent {
            results.extend(parent.lookup_contracts_by_pattern(name, arg_count));
        }

        results
    }

    /// Looks up all contract overloads for a given name (all arities).
    /// Phase 4 optimization: O(1) name lookup instead of O(n) iteration.
    /// Useful for code completion and documentation.
    /// Lock-free iteration using DashMap.
    pub fn lookup_all_contract_overloads(&self, name: &str) -> Vec<Arc<Symbol>> {
        let mut results = Vec::new();

        // Phase 4: Direct O(1) lookup by name
        if let Some(inner_map) = self.pattern_index.get(name) {
            for entry in inner_map.iter() {
                let (_sig, symbols) = entry.pair();
                results.extend(symbols.iter().cloned());
            }
        }

        if let Some(parent) = &self.parent {
            results.extend(parent.lookup_all_contract_overloads(name));
        }

        results
    }

    /// Looks up a symbol by name, traversing up the scope chain if necessary.
    /// Lock-free lookup using DashMap.
    pub fn lookup(&self, name: &str) -> Option<Arc<Symbol>> {
        use std::collections::HashSet;
        let mut visited = HashSet::new();
        self.lookup_with_visited(name, &mut visited)
    }

    /// Internal lookup with cycle detection via visited set.
    /// Tracks visited symbol tables to detect and break circular parent chains.
    fn lookup_with_visited(&self, name: &str, visited: &mut std::collections::HashSet<usize>) -> Option<Arc<Symbol>> {
        // Use the Arc pointer address as a unique identifier for this symbol table
        let self_id = Arc::as_ptr(&self.symbols) as usize;

        if !visited.insert(self_id) {
            // Already visited this symbol table - circular reference detected
            eprintln!("WARNING: Circular symbol table reference detected while looking up symbol '{}'", name);
            eprintln!("         This indicates a bug in symbol table construction (parent chain has a cycle)");
            return None;
        }

        // Look up in current table
        if let Some(symbol) = self.symbols.get(name).map(|entry| entry.value().clone()) {
            return Some(symbol);
        }

        // Recursively search parent, passing the visited set
        self.parent.as_ref().and_then(|p| p.lookup_with_visited(name, visited))
    }

    /// Updates the definition location of an existing symbol.
    /// Lock-free mutation using DashMap.
    pub fn update_definition(&self, name: &str, location: Position) {
        if let Some(mut entry) = self.symbols.get_mut(name) {
            Arc::make_mut(entry.value_mut()).definition_location = Some(location);
        } else if let Some(parent) = &self.parent {
            parent.update_definition(name, location);
        }
    }

    /// Collects all symbols in the current scope and its parents for code completion.
    /// Lock-free iteration using DashMap.
    pub fn collect_all_symbols(&self) -> Vec<Arc<Symbol>> {
        let mut symbols: Vec<Arc<Symbol>> = self.symbols.iter().map(|entry| entry.value().clone()).collect();
        if let Some(parent) = &self.parent {
            symbols.extend(parent.collect_all_symbols());
        }
        symbols
    }

    /// Returns all symbols in the current scope only (no parent traversal).
    /// Lock-free iteration using DashMap.
    pub fn current_symbols(&self) -> Vec<Arc<Symbol>> {
        self.symbols.iter().map(|entry| entry.value().clone()).collect()
    }

    /// Returns the parent symbol table, if any.
    pub fn parent(&self) -> Option<Arc<SymbolTable>> {
        self.parent.clone()
    }

    /// Resolves the best matching contract overload for a given call site.
    ///
    /// This function implements overload resolution by:
    /// 1. Using pattern index to find candidates with matching (name, arity)
    /// 2. Preferring exact arity matches over variadic matches
    /// 3. Returning the most specific match
    ///
    /// # Arguments
    /// * `name` - Contract name being called
    /// * `arg_count` - Number of arguments at the call site
    ///
    /// # Returns
    /// The best matching contract symbol, or None if no match found
    pub fn resolve_overload(&self, name: &str, arg_count: usize) -> Option<Arc<Symbol>> {
        let candidates = self.lookup_contracts_by_pattern(name, arg_count);

        if candidates.is_empty() {
            return None;
        }

        // Prefer exact arity match over variadic match
        // If multiple exact matches exist, return the first one
        let exact_match = candidates.iter()
            .find(|s| s.arity() == Some(arg_count) && !s.is_variadic());

        if let Some(exact) = exact_match {
            return Some((*exact).clone());
        }

        // Fall back to variadic match if no exact match
        let variadic_match = candidates.iter()
            .find(|s| s.is_variadic() && s.arity().map_or(false, |a| a <= arg_count));

        variadic_match.map(|s| (*s).clone())
    }

    /// Gets all matching overloads for hover/signature help display.
    ///
    /// Returns all contract overloads that could potentially match the call,
    /// sorted by arity for consistent display.
    pub fn get_matching_overloads(&self, name: &str, arg_count: usize) -> Vec<Arc<Symbol>> {
        let mut candidates = self.lookup_contracts_by_pattern(name, arg_count);

        // Sort by arity for consistent display
        candidates.sort_by_key(|s| s.arity().unwrap_or(0));

        candidates
    }
}
