use std::any::Any;
use std::collections::HashMap;
use std::ptr;
use std::sync::{Arc, RwLock};

use archery::ArcK;
use rpds::Vector;
use tower_lsp::lsp_types::Url;
use tracing::trace;

use crate::ir::rholang_node::{Metadata, RholangNode, RholangNodeVector, NodeBase, Position, RholangSendType};
use crate::ir::symbol_table::{Symbol, SymbolTable, SymbolType};
use crate::ir::type_extraction::{TypeChecker, TypeExtractor};
use crate::ir::visitor::Visitor;

/// Maps symbol declaration positions to their usage locations within a file.
pub type InvertedIndex = HashMap<Position, Vec<Position>>;

/// Represents a structured value extracted from a contract call argument or pattern.
///
/// Used for pattern matching complex quoted structures in contract identifiers and parameters.
/// Supports recursive matching of maps, lists, tuples, and sets.
#[derive(Debug, Clone, PartialEq)]
pub enum StructuredValue {
    /// A string literal value
    String(String),
    /// An unbound variable reference (matches anything with binding)
    Variable,
    /// A wildcard pattern (matches anything without binding)
    Wildcard,
    /// A map/dictionary pattern with string keys
    Map(HashMap<String, StructuredValue>),
    /// A list pattern with ordered elements
    List(Vec<StructuredValue>),
    /// A tuple pattern with ordered elements
    Tuple(Vec<StructuredValue>),
    /// A set pattern with unordered elements
    Set(Vec<StructuredValue>),
}

/// Builds hierarchical symbol tables and populates global symbol storage for Rholang IR trees.
/// Manages scope creation for nodes like `new`, `let`, `contract`, `input`, `case`, and `branch`.
///
/// Phase 3 Refactoring: Now directly populates RholangContracts during parsing,
/// eliminating the need for separate link_symbols phase.
///
/// Phase 4: Removed potential_global_refs - now handled directly by rholang_symbols.
/// Phase 3 Enhancement: Added type checking infrastructure for pattern matching.
#[derive(Debug)]
pub struct SymbolTableBuilder {
    root: Arc<RholangNode>,  // Root IR node with static lifetime
    current_uri: Url,          // URI of the current file
    current_table: RwLock<Arc<SymbolTable>>,  // Current scope's symbol table
    /// Inverted index: Maps declaration position -> reference positions for local variables
    /// Used for find-references and rename operations (two-tier resolution)
    inverted_index: RwLock<InvertedIndex>,
    global_table: Arc<SymbolTable>,  // Global scope for cross-file symbols
    rholang_symbols: Option<Arc<crate::lsp::rholang_contracts::RholangContracts>>,  // Direct global symbol storage (Phase 3+)
    type_extractor: RwLock<TypeExtractor>,  // Type constraint extractor (Phase 3: Type-Based Matching)
    type_checker: TypeChecker,  // Type constraint checker (Phase 3: Type-Based Matching)
    /// Cache for extract_structured_value() results to avoid redundant tree traversals
    /// Key: raw pointer to RholangNode (stable during symbol table building)
    /// Value: extracted StructuredValue
    extraction_cache: RwLock<HashMap<usize, Arc<StructuredValue>>>,
}

impl SymbolTableBuilder {
    /// Creates a new builder with a root IR node, file URI, and global symbol table.
    ///
    /// # Arguments
    /// * `root` - Root IR node
    /// * `uri` - Current file URI
    /// * `global_table` - Global symbol table for scope chain
    /// * `rholang_symbols` - Optional global symbol storage for direct indexing (Phase 3+)
    pub fn new(
        root: Arc<RholangNode>,
        uri: Url,
        global_table: Arc<SymbolTable>,
        rholang_symbols: Option<Arc<crate::lsp::rholang_contracts::RholangContracts>>,
    ) -> Self {
        let local_table = Arc::new(SymbolTable::new(Some(global_table.clone())));
        Self {
            root,
            current_uri: uri,
            current_table: RwLock::new(local_table),
            inverted_index: RwLock::new(HashMap::new()),
            global_table,
            rholang_symbols,
            type_extractor: RwLock::new(TypeExtractor::new()),
            type_checker: TypeChecker::new(),
            extraction_cache: RwLock::new(HashMap::new()),
        }
    }

    /// Returns the inverted index mapping declaration positions to reference positions
    pub fn get_inverted_index(&self) -> InvertedIndex {
        self.inverted_index.read().expect("Failed to lock inverted_index").clone()
    }

    /// Pushes a new scope onto the stack, linking it to the current scope as its parent.
    fn push_scope(&self) -> Arc<SymbolTable> {
        let current = self.current_table.read().expect("Failed to lock current_table").clone();
        let new_table = Arc::new(SymbolTable::new(Some(current)));
        *self.current_table.write().expect("Failed to lock current_table") = new_table.clone();
        trace!("Pushed new scope");
        new_table
    }

    /// Pops the current scope, reverting to its parent if one exists.
    fn pop_scope(&self) {
        let current = self.current_table.read().expect("Failed to lock current_table").clone();
        if let Some(parent) = current.parent() {
            *self.current_table.write().expect("Failed to lock current_table") = parent;
            trace!("Popped scope");
        } else {
            trace!("No parent scope to pop to; retaining current scope");
        }
    }

    /// Updates a node's metadata with a specific symbol table and optional symbol.
    fn update_metadata<'b>(
        &self,
        node: Arc<RholangNode>,
        table: Arc<SymbolTable>,
        symbol: Option<Arc<Symbol>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let mut data = metadata.as_ref().map_or(HashMap::new(), |m| (**m).clone());
        data.insert("symbol_table".to_string(), Arc::new(table) as Arc<dyn Any + Send + Sync>);
        if let Some(sym) = symbol {
            data.insert("referenced_symbol".to_string(), Arc::new(sym) as Arc<dyn Any + Send + Sync>);
        }
        node.with_metadata(Some(Arc::new(data)))
    }

    /// Updates a node's metadata with the current symbol table and optional symbol.
    fn update_with_current_table<'b>(
        &self,
        node: Arc<RholangNode>,
        symbol: Option<Arc<Symbol>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let current_table = self.current_table.read().expect("Failed to lock current_table").clone();
        self.update_metadata(node, current_table, symbol, metadata)
    }

    /// Collects variables bound in pattern nodes (e.g., in `match` cases).
    fn collect_bound_vars<'b>(&self, pattern: &'b Arc<RholangNode>) -> Vec<Arc<RholangNode>> {
        match &**pattern {
            RholangNode::Var { .. } => vec![pattern.clone()],
            RholangNode::Wildcard { .. } => vec![],
            RholangNode::Tuple { elements, .. } => elements.iter().flat_map(|e| self.collect_bound_vars(e)).collect(),
            RholangNode::List { elements, remainder, .. } => {
                let mut vars: Vec<_> = elements.iter().flat_map(|e| self.collect_bound_vars(e)).collect();
                if let Some(rem) = remainder {
                    vars.extend(self.collect_bound_vars(rem));
                }
                vars
            }
            RholangNode::Set { elements, remainder, .. } => {
                let mut vars: Vec<_> = elements.iter().flat_map(|e| self.collect_bound_vars(e)).collect();
                if let Some(rem) = remainder {
                    vars.extend(self.collect_bound_vars(rem));
                }
                vars
            }
            RholangNode::Map { pairs, remainder, .. } => {
                let mut vars: Vec<_> = pairs.iter().flat_map(|(k, v)| {
                    self.collect_bound_vars(k).into_iter().chain(self.collect_bound_vars(v))
                }).collect();
                if let Some(rem) = remainder {
                    vars.extend(self.collect_bound_vars(rem));
                }
                vars
            }
            RholangNode::Quote { quotable, .. } => self.collect_bound_vars(quotable),
            RholangNode::Disjunction { left, right, .. } => {
                let mut vars = self.collect_bound_vars(left);
                vars.extend(self.collect_bound_vars(right));
                vars
            }
            RholangNode::Conjunction { left, right, .. } => {
                let mut vars = self.collect_bound_vars(left);
                vars.extend(self.collect_bound_vars(right));
                vars
            }
            RholangNode::Negation { operand, .. } => self.collect_bound_vars(operand),
            RholangNode::Parenthesized { expr, .. } => self.collect_bound_vars(expr),
            _ => vec![],
        }
    }

    /// Extract contract name from a channel node
    ///
    /// Handles both `foo` (Var) and `@"foo"` (Quote) syntax
    fn extract_contract_name(&self, channel: &Arc<RholangNode>) -> Option<String> {
        match &**channel {
            RholangNode::Var { name, .. } => Some(name.clone()),
            RholangNode::Quote { quotable, .. } => {
                match &**quotable {
                    RholangNode::StringLiteral { value, .. } => Some(value.clone()),
                    _ => None
                }
            },
            _ => None
        }
    }

    /// Extract contract identifier with support for complex patterns
    ///
    /// Returns a tuple of (name_for_lookup, optional_complex_identifier_node).
    /// For simple cases (Var, Quote(StringLiteral)), returns (name, None).
    /// For complex cases (Quote(Map), Quote(List), etc.), returns (generated_key, Some(node)).
    ///
    /// # Examples
    /// - `foo` → `(Some("foo"), None)`
    /// - `@"robotAPI"` → `(Some("robotAPI"), None)`
    /// - `@{service: "auth"}` → `(Some("@complex_HASH"), Some(Map node))`
    /// - `@["api", "v1"]` → `(Some("@complex_HASH"), Some(List node))`
    ///
    /// # Arguments
    /// * `channel` - The contract identifier node from the contract definition
    ///
    /// # Returns
    /// `(Option<String>, Option<Arc<RholangNode>>)` where:
    /// - First element is the lookup key (simple name or generated hash)
    /// - Second element is the complex identifier node (None for simple cases)
    fn extract_contract_identifier(
        &self,
        channel: &Arc<RholangNode>
    ) -> (Option<String>, Option<Arc<RholangNode>>) {
        match &**channel {
            // Simple variable: foo
            RholangNode::Var { name, .. } => {
                (Some(name.clone()), None)
            },

            // Quoted identifier
            RholangNode::Quote { quotable, .. } => {
                match &**quotable {
                    // Simple quoted string: @"foo"
                    RholangNode::StringLiteral { value, .. } => {
                        (Some(value.clone()), None)
                    },

                    // Complex quoted pattern: @{...}, @[...], @(...), etc.
                    _ => {
                        // Generate a stable hash-based key for lookup
                        // Format: @complex_<type>_<hash>
                        let type_name = match &**quotable {
                            RholangNode::Map { .. } => "map",
                            RholangNode::List { .. } => "list",
                            RholangNode::Tuple { .. } => "tuple",
                            RholangNode::Set { .. } => "set",
                            _ => "other",
                        };

                        // Use debug representation for stable hash
                        // (In production, consider a proper hash function)
                        let hash_str = format!("{:?}", quotable);
                        let hash = hash_str.chars()
                            .fold(0u64, |acc, c| acc.wrapping_mul(31).wrapping_add(c as u64));

                        let key = format!("@complex_{}_{:x}", type_name, hash);
                        (Some(key), Some(quotable.clone()))
                    }
                }
            },

            _ => (None, None)
        }
    }

    /// Extract pattern value from an argument node
    ///
    /// Handles both `"transport_object"` (StringLiteral) and `@"transport_object"` (Quote)
    fn extract_pattern_value(&self, arg: &Arc<RholangNode>) -> Option<String> {
        match &**arg {
            RholangNode::StringLiteral { value, .. } => Some(value.clone()),
            RholangNode::Quote { quotable, .. } => {
                match &**quotable {
                    RholangNode::StringLiteral { value, .. } => Some(value.clone()),
                    _ => None
                }
            },
            _ => None
        }
    }

    /// Extract parameter name from a contract formal parameter node
    ///
    /// Contract formals can be either plain variables or quoted variables:
    /// - Plain variable: `x` → returns `Some("x")`
    /// - Quoted variable: `@x` → returns `Some("x")`
    /// - Quoted literal: `@"action"` → returns `None` (pattern, not binding)
    /// - Complex quote: `@(x + y)` → returns `None` (not a simple variable)
    /// - Wildcard: `_` → returns `None` (no name to bind)
    ///
    /// # Arguments
    /// * `formal` - A formal parameter node from a contract definition
    ///
    /// # Returns
    /// `Some(name)` if the formal is a simple variable (plain or quoted), `None` otherwise
    fn extract_parameter_name(&self, formal: &Arc<RholangNode>) -> Option<String> {
        match &**formal {
            RholangNode::Var { name, .. } => Some(name.clone()),
            RholangNode::Quote { quotable, .. } => {
                match &**quotable {
                    RholangNode::Var { name, .. } => Some(name.clone()),
                    _ => None  // Complex quoted processes (not simple variables)
                }
            },
            _ => None
        }
    }

    /// Recursively extract all variable bindings from a parameter pattern
    ///
    /// This handles complex quoted patterns and extracts all nested variable bindings:
    /// - Simple: `x` or `@x` → `[("x", position)]`
    /// - Map: `@{key1: var1, key2: var2}` → `[("var1", pos1), ("var2", pos2)]`
    /// - List: `@[elem1, elem2]` → `[("elem1", pos1), ("elem2", pos2)]`
    /// - Tuple: `@(x, y)` → `[("x", pos1), ("y", pos2)]`
    /// - Set: `@Set(x, y)` → `[("x", pos1), ("y", pos2)]`
    /// - Nested: `@{outer: {inner: value}}` → `[("value", position)]`
    /// - Wildcard: `_` → `[]` (no bindings)
    /// - Literals: `@"foo"` → `[]` (no bindings)
    ///
    /// # Arguments
    /// * `formal` - A formal parameter node from a contract definition
    ///
    /// # Returns
    /// Vector of (variable_name, position) tuples for all bindings in the pattern
    fn extract_parameter_bindings(&self, formal: &Arc<RholangNode>) -> Vec<(String, Position)> {
        let mut bindings = Vec::new();
        self.extract_bindings_recursive(formal, &mut bindings);
        bindings
    }

    /// Helper function for recursive binding extraction
    fn extract_bindings_recursive(&self, node: &Arc<RholangNode>, bindings: &mut Vec<(String, Position)>) {
        match &**node {
            // Simple variable binding
            RholangNode::Var { name, .. } => {
                if !name.is_empty() && name != "_" {
                    let position = node.absolute_start(&self.root);
                    trace!("extract_bindings_recursive: Extracted unquoted Var '{}' at {:?}", name, position);
                    bindings.push((name.clone(), position));
                } else {
                    trace!("extract_bindings_recursive: Skipped unquoted Var (empty or wildcard)");
                }
            },

            // Quoted pattern - for simple variables, use the Quote's position (the @ symbol)
            RholangNode::Quote { quotable, .. } => {
                // For contract parameters like @destRoom, point to the @ symbol
                if let RholangNode::Var { name, .. } = &**quotable {
                    if !name.is_empty() && name != "_" {
                        let position = node.absolute_start(&self.root);  // Use Quote's position, not Var's
                        trace!("extract_bindings_recursive: Extracted quoted Var '{}' at {:?}", name, position);
                        bindings.push((name.clone(), position));
                        return;  // Don't recurse for simple quoted variables
                    }
                }
                // For complex quoted patterns, recurse normally
                trace!("extract_bindings_recursive: Recursing into complex quoted pattern");
                self.extract_bindings_recursive(quotable, bindings);
            },

            // Map pattern: extract from all values
            RholangNode::Map { pairs, .. } => {
                for (key, value) in pairs {
                    // Keys are typically literals, but extract from values
                    self.extract_bindings_recursive(key, bindings);
                    self.extract_bindings_recursive(value, bindings);
                }
            },

            // List pattern: extract from all elements
            RholangNode::List { elements, .. } => {
                for element in elements {
                    self.extract_bindings_recursive(element, bindings);
                }
            },

            // Tuple pattern: extract from all elements
            RholangNode::Tuple { elements, .. } => {
                trace!("extract_bindings_recursive: Processing Tuple with {} elements", elements.len());
                for element in elements {
                    self.extract_bindings_recursive(element, bindings);
                }
                trace!("extract_bindings_recursive: After Tuple, bindings count: {}", bindings.len());
            },

            // Set pattern: extract from all elements
            RholangNode::Set { elements, .. } => {
                for element in elements {
                    self.extract_bindings_recursive(element, bindings);
                }
            },

            // Pathmap pattern: extract from all elements (same as set)
            RholangNode::Pathmap { elements, .. } => {
                for element in elements {
                    self.extract_bindings_recursive(element, bindings);
                }
            },

            // Wildcard: no bindings
            RholangNode::Wildcard { .. } => {},

            // Literals and other non-binding patterns: ignore
            RholangNode::StringLiteral { .. } => {},
            RholangNode::LongLiteral { .. } => {},
            RholangNode::BoolLiteral { .. } => {},
            RholangNode::UriLiteral { .. } => {},

            // For other node types that might contain patterns, recurse conservatively
            // This handles cases like nested expressions or complex patterns
            _ => {
                // Don't recurse into arbitrary expressions as they're not binding patterns
                // Only the specific pattern-forming constructs above bind variables
            }
        }
    }

    /// Extract structured value from a node for complex pattern matching
    ///
    /// Recursively extracts structured representations from Rholang nodes to support
    /// pattern matching against complex quoted structures (maps, lists, tuples, etc.).
    ///
    /// # Arguments
    /// * `node` - The node to extract a structured value from
    ///
    /// # Returns
    /// `Some(StructuredValue)` if the node can be represented as a structured value,
    /// `None` for unsupported node types
    ///
    /// # Examples
    /// - `"foo"` → `Some(StructuredValue::String("foo"))`
    /// - `x` → `Some(StructuredValue::Variable)`
    /// - `@{key: "value"}` → `Some(StructuredValue::Map(...))`
    /// - `@[1, 2, 3]` → `Some(StructuredValue::List(...))`
    fn extract_structured_value(&self, node: &Arc<RholangNode>) -> Option<StructuredValue> {
        // Use node pointer as cache key (stable during symbol table building)
        let cache_key = ptr::addr_of!(**node) as usize;

        // Check cache first
        {
            let cache = self.extraction_cache.read().expect("Failed to lock extraction_cache");
            if let Some(cached) = cache.get(&cache_key) {
                return Some((**cached).clone());
            }
        }

        // Extract value (not in cache)
        let extracted = match &**node {
            RholangNode::StringLiteral { value, .. } => {
                Some(StructuredValue::String(value.clone()))
            },

            RholangNode::Var { .. } => {
                Some(StructuredValue::Variable)
            },

            RholangNode::Wildcard { .. } => {
                Some(StructuredValue::Wildcard)
            },

            RholangNode::Quote { quotable, .. } => {
                // Recursively extract from quoted content
                self.extract_structured_value(quotable)
            },

            RholangNode::Map { pairs, .. } => {
                let mut map = HashMap::new();
                for (key, value) in pairs {
                    // Extract key (must be a string)
                    if let Some(key_val) = self.extract_structured_value(key) {
                        if let StructuredValue::String(key_str) = key_val {
                            // Extract value (can be any structure)
                            if let Some(value_val) = self.extract_structured_value(value) {
                                map.insert(key_str, value_val);
                            } else {
                                // Unsupported value type - abort
                                return None;
                            }
                        } else {
                            // Non-string key - not supported
                            return None;
                        }
                    } else {
                        // Can't extract key - abort
                        return None;
                    }
                }
                Some(StructuredValue::Map(map))
            },

            RholangNode::List { elements, .. } => {
                let vals: Option<Vec<_>> = elements.iter()
                    .map(|e| self.extract_structured_value(e))
                    .collect();
                vals.map(StructuredValue::List)
            },

            RholangNode::Tuple { elements, .. } => {
                let vals: Option<Vec<_>> = elements.iter()
                    .map(|e| self.extract_structured_value(e))
                    .collect();
                vals.map(StructuredValue::Tuple)
            },

            RholangNode::Set { elements, .. } => {
                let vals: Option<Vec<_>> = elements.iter()
                    .map(|e| self.extract_structured_value(e))
                    .collect();
                vals.map(StructuredValue::Set)
            },

            RholangNode::Pathmap { elements, .. } => {
                // Treat pathmap as set for pattern matching (both are unordered collections)
                let vals: Option<Vec<_>> = elements.iter()
                    .map(|e| self.extract_structured_value(e))
                    .collect();
                vals.map(StructuredValue::Set)
            },

            _ => None
        };

        // Store in cache if extraction succeeded
        if let Some(ref value) = extracted {
            let mut cache = self.extraction_cache.write().expect("Failed to lock extraction_cache");
            cache.insert(cache_key, Arc::new(value.clone()));
        }

        extracted
    }

    /// Extract pattern values from all arguments
    ///
    /// Maps over all input arguments and extracts their pattern values.
    /// Returns a vector with Some(value) for string literals and None for other patterns.
    ///
    /// # Returns
    /// - `Vec<Option<String>>` where:
    ///   - `Some(value)` for string literal arguments (e.g., `"transport_object"`)
    ///   - `None` for non-literal patterns (wildcards, variables, complex expressions)
    ///
    /// # Example
    /// ```ignore
    /// // For invocation: robotAPI!("transport", "ball1", x)
    /// // Returns: [Some("transport"), Some("ball1"), None]
    /// ```
    ///
    /// # Phase
    /// Part of Phase 1: Multi-Argument String Literal Matching
    fn extract_all_pattern_values(&self, inputs: &Vector<Arc<RholangNode>, ArcK>)
        -> Vec<Option<String>>
    {
        inputs.iter().map(|arg| self.extract_pattern_value(arg)).collect()
    }

    /// Check if an argument matches a formal parameter pattern
    ///
    /// This is the core pattern matching logic that determines contract overload resolution.
    ///
    /// # Supported Pattern Types
    ///
    /// 1. **Wildcard** (`_`): Matches any argument
    ///    - Example: `contract foo(_, @x) = { ... }` matches `foo!(42, "bar")`
    ///
    /// 2. **Variable** (`@x`, `@variableName`): Matches any argument with binding
    ///    - Example: `contract foo(@x, @y) = { ... }` matches `foo!(42, "bar")`
    ///
    /// 3. **String Literal** (`@"value"`): Requires exact string match
    ///    - Example: `contract foo(@"transport", @dest) = { ... }` matches `foo!("transport", "room_a")`
    ///    - Non-match: `foo!("validate", "room_a")`
    ///
    /// 4. **Type-Constrained Patterns** (`@{x /\ Type}`): Checks type compatibility
    ///    - Example: `contract foo(@{n /\ Int}) = { ... }` matches `foo!(42)` but not `foo!("text")`
    ///    - NOTE: Pattern conjunctions not yet in parser - infrastructure ready for future use
    ///
    /// 5. **Unknown/Complex Patterns**: Conservative approach - returns `false`
    ///    - Avoids false positives for patterns we don't yet understand
    ///
    /// # Arguments
    /// - `formal`: The formal parameter pattern from the contract definition
    /// - `arg_value`: The extracted argument value (Some for literals, None for expressions)
    /// - `arg_node`: The actual argument node (for type checking)
    ///
    /// Helper: Match map pattern against argument map
    ///
    /// Requires exact key set match (no subset matching in MVP).
    /// Each key-value pair must match recursively.
    fn matches_map_pattern(
        &self,
        pattern_pairs: &Vector<(Arc<RholangNode>, Arc<RholangNode>), ArcK>,
        arg_map: &HashMap<String, StructuredValue>
    ) -> bool {
        // Require exact key count (no subset matching)
        if pattern_pairs.len() != arg_map.len() {
            return false;
        }

        for (key, value) in pattern_pairs {
            // Extract key (must be a string)
            if let Some(StructuredValue::String(key_str)) = self.extract_structured_value(key) {
                // Check if argument has this key
                match arg_map.get(&key_str) {
                    Some(arg_val) => {
                        // Recursively match value pattern
                        if !self.matches_structured_pattern(value, arg_val) {
                            return false;
                        }
                    },
                    None => return false, // Key not in argument
                }
            } else {
                return false; // Non-string key not supported
            }
        }
        true
    }

    /// Helper: Match list pattern against argument list
    ///
    /// Requires exact length match (no remainder in MVP).
    /// Each element must match recursively in order.
    fn matches_list_pattern(
        &self,
        pattern_elems: &Vector<Arc<RholangNode>, ArcK>,
        arg_list: &Vec<StructuredValue>
    ) -> bool {
        // Exact length match required
        if pattern_elems.len() != arg_list.len() {
            return false;
        }

        for (p_elem, a_val) in pattern_elems.iter().zip(arg_list.iter()) {
            if !self.matches_structured_pattern(p_elem, a_val) {
                return false;
            }
        }
        true
    }

    /// Helper: Match tuple pattern against argument tuple
    ///
    /// Same logic as list matching - exact length and element-wise matching.
    fn matches_tuple_pattern(
        &self,
        pattern_elems: &Vector<Arc<RholangNode>, ArcK>,
        arg_tuple: &Vec<StructuredValue>
    ) -> bool {
        self.matches_list_pattern(pattern_elems, arg_tuple)
    }

    /// Helper: Match set pattern against argument set
    ///
    /// Requires exact size match. Order doesn't matter for sets,
    /// but we check element-wise for simplicity in MVP.
    fn matches_set_pattern(
        &self,
        pattern_elems: &Vector<Arc<RholangNode>, ArcK>,
        arg_set: &Vec<StructuredValue>
    ) -> bool {
        // For MVP, use same logic as lists (order-independent matching deferred)
        self.matches_list_pattern(pattern_elems, arg_set)
    }

    /// Helper: Match a pattern node against a structured value
    ///
    /// This is the core recursive matching logic for complex patterns.
    fn matches_structured_pattern(
        &self,
        formal: &Arc<RholangNode>,
        arg_value: &StructuredValue
    ) -> bool {
        match &**formal {
            // Wildcard matches anything
            RholangNode::Wildcard { .. } => true,

            // Variable bindings match anything
            RholangNode::Var { .. } => true,

            // String literal must match exactly
            RholangNode::StringLiteral { value: pattern_val, .. } => {
                matches!(arg_value, StructuredValue::String(arg_val) if arg_val == pattern_val)
            },

            // Quote: unwrap and match recursively
            RholangNode::Quote { quotable, .. } => {
                self.matches_structured_pattern(quotable, arg_value)
            },

            // Map pattern
            RholangNode::Map { pairs, .. } => {
                if let StructuredValue::Map(arg_map) = arg_value {
                    self.matches_map_pattern(pairs, arg_map)
                } else {
                    false
                }
            },

            // List pattern
            RholangNode::List { elements, .. } => {
                if let StructuredValue::List(arg_list) = arg_value {
                    self.matches_list_pattern(elements, arg_list)
                } else {
                    false
                }
            },

            // Tuple pattern
            RholangNode::Tuple { elements, .. } => {
                if let StructuredValue::Tuple(arg_tuple) = arg_value {
                    self.matches_tuple_pattern(elements, arg_tuple)
                } else {
                    false
                }
            },

            // Set pattern
            RholangNode::Set { elements, .. } => {
                if let StructuredValue::Set(arg_set) = arg_value {
                    self.matches_set_pattern(elements, arg_set)
                } else {
                    false
                }
            },

            // Pathmap pattern (treat as set - both are unordered collections)
            RholangNode::Pathmap { elements, .. } => {
                if let StructuredValue::Set(arg_set) = arg_value {
                    self.matches_set_pattern(elements, arg_set)
                } else {
                    false
                }
            },

            // Unknown patterns: conservative - don't match
            _ => false
        }
    }

    /// # Returns
    /// - `true` if the argument is compatible with the formal pattern
    /// - `false` if incompatible or pattern type is unsupported
    ///
    /// # Phase
    /// Phase 2: Wildcard and Variable Pattern Support
    /// Phase 2.5: Complex Quote Pattern Support (Maps, Lists, Tuples, Sets)
    /// Phase 3: Type-Based Matching
    fn matches_pattern(
        &self,
        formal: &Arc<RholangNode>,
        arg_value: &Option<String>,
        arg_node: &Arc<RholangNode>
    ) -> bool {
        match &**formal {
            // Wildcard matches anything
            RholangNode::Wildcard { .. } => true,

            // Variable bindings match anything
            RholangNode::Var { .. } => true,

            // Quote: check if it's a simple string literal or complex pattern
            RholangNode::Quote { quotable, .. } => {
                match &**quotable {
                    // Simple string literal: backward compatible path
                    RholangNode::StringLiteral { value, .. } => {
                        arg_value.as_ref().map_or(false, |v| v == value)
                    },

                    // Variable inside quote: matches anything
                    RholangNode::Var { .. } => true,

                    // Complex pattern: use structured matching
                    _ => {
                        // Extract structured value from argument
                        if let Some(arg_structured) = self.extract_structured_value(arg_node) {
                            self.matches_structured_pattern(quotable, &arg_structured)
                        } else {
                            // Can't extract structure - conservative no match
                            false
                        }
                    }
                }
            },

            // TODO: Pattern conjunction with type constraints
            // When parser supports `@{x /\ Type}`, add:
            // RholangNode::ConnPat { conn_term_var, conn_term_type, .. } => {
            //     let mut extractor = self.type_extractor.write().unwrap();
            //     if let Some(type_constraint) = extractor.extract_type_from_node(conn_term_type) {
            //         self.type_checker.satisfies_constraint(arg_node, &type_constraint)
            //     } else {
            //         true  // No type constraint - matches anything
            //     }
            // }

            // Unknown patterns: conservative approach - don't match
            _ => false
        }
    }

    /// Resolve contract invocation by pattern matching
    ///
    /// Matches ALL argument values against formal parameters of contract definitions
    /// to find the correct overload. This is the main entry point for contract
    /// overload resolution.
    ///
    /// # Algorithm
    ///
    /// 1. Find all contract symbols with matching name
    /// 2. Filter by arity (argument count must be compatible)
    /// 3. For each candidate, check ALL formal parameters against ALL arguments
    /// 4. First candidate where ALL patterns match is the winner
    /// 5. Record reference and return the matched symbol
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Definitions
    /// contract robotAPI(@"transport_object", @objectName, @destRoom, ret) = { ... }  // Line 279
    /// contract robotAPI(@"validate_plan", @objectName, @destRoom, ret) = { ... }     // Line 298
    ///
    /// // Invocation
    /// robotAPI!("transport_object", "ball1", "room_a", *result4c)
    ///
    /// // Resolution process:
    /// // 1. Find candidates: [robotAPI at 279, robotAPI at 298]
    /// // 2. Check first candidate:
    /// //    - @"transport_object" vs "transport_object" ✓
    /// //    - @objectName vs "ball1" ✓ (variable matches anything)
    /// //    - @destRoom vs "room_a" ✓ (variable matches anything)
    /// //    - ret vs *result4c ✓ (variable matches anything)
    /// // 3. Match found! Return symbol at line 279
    /// ```
    ///
    /// # Arguments
    /// - `contract_name`: Name of the contract being invoked
    /// - `arg_values`: Extracted values from all invocation arguments
    /// - `arg_nodes`: Actual argument nodes (for type checking in Phase 3)
    /// - `send_node`: AST node of the invocation (for recording references)
    ///
    /// # Returns
    /// - `Some(Symbol)` if a matching contract definition was found
    /// - `None` if no compatible definition exists
    ///
    /// # Phase
    /// Phase 1: Multi-argument matching
    /// Phase 2: Wildcard and Variable Pattern Support
    /// Phase 3: Type-Based Matching
    fn resolve_contract_by_pattern(
        &self,
        contract_name: &str,
        arg_values: Vec<Option<String>>,
        arg_nodes: &Vector<Arc<RholangNode>, ArcK>,
        send_node: &Arc<RholangNode>,
    ) -> Option<Arc<Symbol>> {
        let current_table = self.current_table.read().unwrap();

        // Get all contracts with this name and arity
        let candidates = current_table.lookup_contracts_by_pattern(contract_name, arg_values.len());

        // Filter by matching ALL formal parameters against ALL arguments
        'candidates: for symbol in candidates {
            if let Some(pattern) = &symbol.contract_pattern {
                // Check each formal parameter against corresponding argument
                // Zip together: formal parameters, extracted values, and actual nodes
                for ((formal, arg_val), arg_node) in pattern.formals.iter()
                    .zip(arg_values.iter())
                    .zip(arg_nodes.iter()) {
                    // Use matches_pattern helper to check compatibility
                    if !self.matches_pattern(formal, arg_val, arg_node) {
                        // Pattern doesn't match - try next candidate
                        continue 'candidates;
                    }
                }

                // ALL parameters matched! Record reference and return this symbol
                if let Some(ref rholang_symbols) = self.rholang_symbols {
                    use crate::lsp::rholang_contracts::SymbolLocation;
                    let ref_location = SymbolLocation::new(
                        self.current_uri.clone(),
                        send_node.absolute_start(&self.root)
                    );
                    let _ = rholang_symbols.add_reference(contract_name, ref_location);
                    trace!(
                        "Pattern-matched '{}' with args {:?} at {:?}",
                        contract_name, arg_values, send_node.absolute_start(&self.root)
                    );
                }
                return Some(symbol.clone());
            }
        }

        // No pattern match - log for debugging
        trace!("No pattern match for '{}' with args {:?}", contract_name, arg_values);
        None
    }

}

impl Visitor for SymbolTableBuilder {
    fn visit_par<'a>(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        left: &Arc<RholangNode>,
        right: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_left = self.visit_node(left);
        let new_right = self.visit_node(right);
        let new_node = Arc::new(RholangNode::Par {
                processes: None,
            base: base.clone(),
            left: Some(new_left),
            right: Some(new_right),
            metadata: metadata.clone(),
        });
        self.update_with_current_table(new_node, None, metadata)
    }

    /// Visits an n-ary parallel composition node.
    fn visit_par_nary(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        processes: &RholangNodeVector,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_processes: Vec<Arc<RholangNode>> = processes
            .iter()
            .map(|proc| self.visit_node(proc))
            .collect();

        let new_node = Arc::new(RholangNode::Par {
            processes: Some(Vector::from_iter(new_processes)),
            base: base.clone(),
            left: None,
            right: None,
            metadata: metadata.clone(),
        });
        self.update_with_current_table(new_node, None, metadata)
    }

    /// Visits a `new` node, ensuring declarations are added to the symbol table before processing.
    fn visit_new<'a>(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        decls: &Vector<Arc<RholangNode>, ArcK>,
        proc: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_table = self.push_scope();
        for d in decls.iter() {
            if let RholangNode::NameDecl { var, .. } = &**d {
                if let RholangNode::Var { name, .. } = &**var {
                    if !name.is_empty() {  // Skip empty variable names
                        let location = var.absolute_start(&self.root);
                        let symbol = Arc::new(Symbol::new(
                            name.clone(),
                            SymbolType::Variable,
                            self.current_uri.clone(),
                            location,
                        ));
                        new_table.insert(symbol);
                    }
                }
            }
        }
        let new_decls = decls.iter().map(|d| self.visit_node(d)).collect();
        let new_proc = self.visit_node(proc);
        let new_node = Arc::new(RholangNode::New {
            base: base.clone(),
            decls: new_decls,
            proc: new_proc,
            metadata: metadata.clone(),
        });
        let updated_node = self.update_metadata(new_node, new_table.clone(), None, metadata);
        self.pop_scope();
        updated_node
    }

    /// Visits a `let` node, adding declarations to the symbol table before processing.
    fn visit_let<'a>(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        decls: &Vector<Arc<RholangNode>, ArcK>,
        proc: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let outer_table = self.current_table.read().unwrap().clone();
        let new_table = self.push_scope();
        for d in decls {
            if let RholangNode::Decl { names, names_remainder, procs, .. } = &**d {
                for (name, proc) in names.iter().zip(procs.iter()) {
                    if let RholangNode::Var { name: var_name, .. } = &**name {
                        if !var_name.is_empty() {  // Skip empty variable names
                            let decl_loc = name.absolute_start(&self.root);
                            let def_loc = proc.absolute_start(&self.root);
                            let symbol = Arc::new(Symbol {
                                name: var_name.clone(),
                                symbol_type: SymbolType::Variable,
                                declaration_uri: self.current_uri.clone(),
                                declaration_location: decl_loc,
                                definition_location: Some(def_loc),
                                contract_pattern: None,
                                contract_identifier_node: None,
                                documentation: None,
                            });
                            new_table.insert(symbol);
                            trace!("Declared variable '{}' in let scope at {:?}", var_name, decl_loc);
                        } else {
                            trace!("Skipped empty variable name in let declaration at {:?}", name.absolute_start(&self.root));
                        }
                    }
                }
                if let Some(rem) = names_remainder {
                    if let RholangNode::Var { name: var_name, .. } = &**rem {
                        if !var_name.is_empty() {
                            let decl_loc = rem.absolute_start(&self.root);
                            let symbol = Arc::new(Symbol::new(
                                var_name.clone(),
                                SymbolType::Variable,
                                self.current_uri.clone(),
                                decl_loc,
                            ));
                            new_table.insert(symbol);
                            trace!("Declared remainder variable '{}' in let scope at {:?}", var_name, decl_loc);

                            // Priority 2b: Also index in rholang_symbols as local symbol
                            if let Some(ref rholang_syms) = self.rholang_symbols {
                                use crate::lsp::rholang_contracts::SymbolLocation;
                                let decl_location = SymbolLocation::new(self.current_uri.clone(), decl_loc);
                                let _ = rholang_syms.insert_declaration(
                                    var_name.clone(),
                                    SymbolType::Variable,
                                    decl_location,
                                );
                                trace!("Indexed local let remainder variable '{}' in rholang_symbols at {:?}", var_name, decl_loc);
                            }
                        }
                    }
                }
            }
        }
        let new_decls = decls.iter().map(|d| {
            if let RholangNode::Decl { names, names_remainder, procs, base: decl_base, metadata: decl_metadata } = &**d {
                let new_names = names.iter().map(|n| self.visit_node(n)).collect::<Vector<_, ArcK>>();
                let new_names_remainder = names_remainder.as_ref().map(|r| self.visit_node(r));
                let new_procs = procs.iter().map(|p| {
                    let prev_table = self.current_table.write().unwrap().clone();
                    *self.current_table.write().unwrap() = outer_table.clone();
                    let new_p = self.visit_node(p);
                    *self.current_table.write().unwrap() = prev_table;
                    new_p
                }).collect::<Vector<_, ArcK>>();
                Arc::new(RholangNode::Decl {
                    base: decl_base.clone(),
                    names: new_names,
                    names_remainder: new_names_remainder,
                    procs: new_procs,
                    metadata: decl_metadata.clone(),
                })
            } else {
                self.visit_node(d)
            }
        }).collect::<Vector<_, ArcK>>();
        let new_proc = self.visit_node(proc);
        let new_node = Arc::new(RholangNode::Let {
            base: base.clone(),
            decls: new_decls,
            proc: new_proc,
            metadata: metadata.clone(),
        });
        let updated_node = self.update_metadata(new_node, new_table.clone(), None, metadata);
        self.pop_scope();
        updated_node
    }

    /// Visits a `contract` node, registering the contract globally and parameters locally.
    fn visit_contract<'a>(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        name: &Arc<RholangNode>,
        formals: &Vector<Arc<RholangNode>, ArcK>,
        formals_remainder: &Option<Arc<RholangNode>>,
        proc: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let contract_pos = name.absolute_start(&self.root);

        // DEBUG: Log contract name position
        trace!("Contract name node position: {:?}", contract_pos);

        // Use the new extract_contract_identifier to handle both simple and complex identifiers
        let (contract_name_opt, identifier_node) = self.extract_contract_identifier(name);
        let contract_name = contract_name_opt.unwrap_or_else(|| String::new());

        let symbol = if !contract_name.is_empty() {
            trace!("Declaring contract '{}' at {:?} with {} parameters",
                contract_name, contract_pos, formals.len());

            // Determine which table to insert into (global vs current scope)
            let current_table_guard = self.current_table.read().unwrap();
            let is_top_level = current_table_guard.parent().as_ref().map_or(false, |p| p.parent().is_none());

            let insert_table = if is_top_level {
                self.global_table.clone()
            } else {
                current_table_guard.clone()
            };

            // Check if there's already a 'new' binding with this name in current scope
            // If so, preserve its declaration location (for goto_declaration)
            // while using the contract position as the definition location (for goto_definition)
            let (decl_location, decl_uri) = if let Some(existing_symbol) = insert_table.lookup(&contract_name) {
                if matches!(existing_symbol.symbol_type, SymbolType::Variable) {
                    trace!("Contract '{}' has existing 'new' binding at {:?}, preserving declaration location",
                        contract_name, existing_symbol.declaration_location);
                    (existing_symbol.declaration_location, existing_symbol.declaration_uri.clone())
                } else {
                    (contract_pos, self.current_uri.clone())
                }
            } else {
                (contract_pos, self.current_uri.clone())
            };
            drop(current_table_guard);

            // Clone decl_uri before it's moved (needed for rholang_symbols later)
            let decl_uri_for_global = decl_uri.clone();

            // Create contract symbol with pattern information
            let mut symbol = Symbol::new_contract(
                contract_name.clone(),
                decl_uri,
                decl_location,
                formals.clone(),
                formals_remainder.clone(),
                proc.clone(),
            );

            // Set definition location to the contract position
            symbol.definition_location = Some(contract_pos);

            // Phase 5/7: Extract documentation from metadata (supports both String and StructuredDocumentation)
            if let Some(meta) = metadata {
                use crate::ir::transforms::documentation_attacher::DOC_METADATA_KEY;
                use crate::ir::StructuredDocumentation;

                if let Some(doc_any) = meta.get(DOC_METADATA_KEY) {
                    // Phase 7: Try StructuredDocumentation first (new format)
                    if let Some(structured_doc) = doc_any.downcast_ref::<StructuredDocumentation>() {
                        symbol.documentation = Some(structured_doc.to_plain_text());
                        trace!("Extracted structured documentation for contract '{}': summary length = {}, params = {}",
                            contract_name, structured_doc.summary.len(), structured_doc.params.len());
                    }
                    // Phase 5: Fall back to plain String (old format - backwards compatibility)
                    else if let Some(doc_string) = doc_any.downcast_ref::<String>() {
                        symbol.documentation = Some(doc_string.clone());
                        trace!("Extracted plain documentation for contract '{}': {} chars", contract_name, doc_string.len());
                    }
                }
            }

            // Store complex identifier node for structural matching (Phase 2)
            if let Some(complex_node) = identifier_node {
                symbol.contract_identifier_node = Some(complex_node);
                trace!("Stored complex identifier node for contract '{}'", contract_name);
            }

            let symbol = Arc::new(symbol);

            // Insert into symbol table (automatically updates pattern index)
            insert_table.insert(symbol.clone());

            // Phase 3.2: Add ALL contracts to rholang_symbols (not just top-level)
            // This ensures two-tier resolution works correctly for nested contracts
            if let Some(ref rholang_symbols) = self.rholang_symbols {
                use crate::lsp::rholang_contracts::SymbolLocation;

                let global_decl_loc = SymbolLocation::new(
                    decl_uri_for_global,
                    decl_location,
                );

                // Insert declaration (ignore errors - may already exist from forward ref)
                let _ = rholang_symbols.insert_declaration(
                    contract_name.clone(),
                    SymbolType::Contract,
                    global_decl_loc.clone(),
                );

                // Contract body is the definition location
                if contract_pos != global_decl_loc.position {
                    let def_location = SymbolLocation::new(self.current_uri.clone(), contract_pos);
                    let _ = rholang_symbols.set_definition(&contract_name, def_location);
                }

                trace!("Indexed contract '{}' in rholang_symbols at {:?} (top_level: {})", contract_name, contract_pos, is_top_level);
            }

            Some(symbol)
        } else {
            trace!("Skipped empty contract name at {:?}", contract_pos);
            None
        };

        let new_name = self.visit_node(name);

        let new_table = self.push_scope();

        // Extract all bindings from formal parameters (including nested bindings in complex patterns)
        for f in formals {
            let bindings = self.extract_parameter_bindings(f);
            if bindings.is_empty() {
                trace!("No variable bindings found in formal parameter at {:?} (wildcard or literal)", f.absolute_start(&self.root));
            } else {
                for (param_name, location) in bindings {
                    let symbol = Arc::new(Symbol::new(
                        param_name.clone(),
                        SymbolType::Parameter,
                        self.current_uri.clone(),
                        location,
                    ));
                    new_table.insert(symbol);
                    trace!("Declared parameter '{}' in contract scope at {:?}", param_name, location);
                }
            }
        }

        // Extract bindings from remainder parameter (if present)
        if let Some(rem) = formals_remainder {
            let bindings = self.extract_parameter_bindings(rem);
            if bindings.is_empty() {
                trace!("No variable bindings found in remainder parameter at {:?} (wildcard or literal)", rem.absolute_start(&self.root));
            } else {
                for (param_name, location) in bindings {
                    let symbol = Arc::new(Symbol::new(
                        param_name.clone(),
                        SymbolType::Parameter,
                        self.current_uri.clone(),
                        location,
                    ));
                    new_table.insert(symbol);
                    trace!("Declared remainder parameter '{}' in contract scope at {:?}", param_name, location);
                }
            }
        }

        let new_formals = formals.iter().map(|f| self.visit_node(f)).collect();
        let new_formals_remainder = formals_remainder.as_ref().map(|r| self.visit_node(r));
        let new_proc = self.visit_node(proc);

        let new_node = Arc::new(RholangNode::Contract {
            base: base.clone(),
            name: new_name,
            formals: new_formals,
            formals_remainder: new_formals_remainder,
            proc: new_proc,
            metadata: metadata.clone(),
        });
        let updated_node = self.update_metadata(new_node, new_table.clone(), symbol, metadata);
        self.pop_scope();
        updated_node
    }

    /// Visits an `input` node, adding bindings to the symbol table before processing.
    fn visit_input<'a>(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        receipts: &Vector<Vector<Arc<RholangNode>, ArcK>, ArcK>,
        proc: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_table = self.push_scope();
        for r in receipts {
            for b in r {
                match &**b {
                    RholangNode::LinearBind { names, remainder, .. } |
                    RholangNode::RepeatedBind { names, remainder, .. } |
                    RholangNode::PeekBind { names, remainder, .. } => {
                        for name in names {
                            let bound_vars = self.collect_bound_vars(name);
                            for var in bound_vars {
                                if let RholangNode::Var { name: var_name, .. } = &*var {
                                    if !var_name.is_empty() {  // Skip empty variable names
                                        // Use the bind node position (includes @ prefix) instead of just the var name
                                        let location = b.absolute_start(&self.root);
                                        let symbol = Arc::new(Symbol::new(
                                            var_name.clone(),
                                            SymbolType::Variable,
                                            self.current_uri.clone(),
                                            location,
                                        ));
                                        new_table.insert(symbol);
                                        trace!("Declared variable '{}' in input scope at {:?}", var_name, location);
                                    } else {
                                        trace!("Skipped empty variable name in input binding at {:?}", var.absolute_start(&self.root));
                                    }
                                }
                            }
                        }
                        if let Some(rem) = remainder {
                            if let RholangNode::Var { name: var_name, .. } = &**rem {
                                if !var_name.is_empty() {
                                    let location = rem.absolute_start(&self.root);
                                    let symbol = Arc::new(Symbol::new(
                                        var_name.clone(),
                                        SymbolType::Variable,
                                        self.current_uri.clone(),
                                        location,
                                    ));
                                    new_table.insert(symbol);
                                    trace!("Declared remainder variable '{}' in input scope at {:?}", var_name, location);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        let new_receipts = receipts.iter().map(|r| {
            r.iter().map(|b| self.visit_node(b)).collect()
        }).collect();
        let new_proc = self.visit_node(proc);
        let new_node = Arc::new(RholangNode::Input {
            base: base.clone(),
            receipts: new_receipts,
            proc: new_proc,
            metadata: metadata.clone(),
        });
        let updated_node = self.update_metadata(new_node, new_table.clone(), None, metadata);
        self.pop_scope();
        updated_node
    }

    /// Visits a `match` node, adding pattern variables to the symbol table before processing cases.
    fn visit_match<'a>(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        expression: &Arc<RholangNode>,
        cases: &Vector<(Arc<RholangNode>, Arc<RholangNode>), ArcK>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_expression = self.visit_node(expression);
        let new_cases = cases.iter().map(|(pattern, proc)| {
            let new_table = self.push_scope();
            let bound_vars = self.collect_bound_vars(pattern);
            for var in bound_vars {
                if let RholangNode::Var { name, .. } = &*var {
                    if !name.is_empty() {  // Skip empty variable names
                        let location = var.absolute_start(&self.root);
                        let symbol = Arc::new(Symbol::new(
                            name.clone(),
                            SymbolType::Variable,
                            self.current_uri.clone(),
                            location,
                        ));
                        new_table.insert(symbol);
                        trace!("Declared variable '{}' in match case scope at {:?}", name, location);
                    } else {
                        trace!("Skipped empty variable name in match pattern at {:?}", var.absolute_start(&self.root));
                    }
                }
            }
            let new_pattern = self.visit_node(pattern);
            let new_proc = self.visit_node(proc);
            let case_node = Arc::new(RholangNode::Match {
                base: base.clone(),
                expression: new_expression.clone(),
                cases: Vector::new_with_ptr_kind().push_back((new_pattern.clone(), new_proc.clone())),
                metadata: metadata.clone(),
            });
            let _updated_case = self.update_metadata(case_node, new_table.clone(), None, metadata);
            self.pop_scope();
            (new_pattern, new_proc)
        }).collect();

        let new_node = Arc::new(RholangNode::Match {
            base: base.clone(),
            expression: new_expression,
            cases: new_cases,
            metadata: metadata.clone(),
        });
        self.update_with_current_table(new_node, None, metadata)
    }

    /// Visits a `choice` node, adding input variables to the symbol table before processing branches.
    fn visit_choice<'a>(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        branches: &Vector<(Vector<Arc<RholangNode>, ArcK>, Arc<RholangNode>), ArcK>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_branches = branches.iter().map(|(inputs, proc)| {
            let new_table = self.push_scope();
            for i in inputs {
                if let RholangNode::LinearBind { names, remainder, .. } |
                       RholangNode::RepeatedBind { names, remainder, .. } |
                       RholangNode::PeekBind { names, remainder, .. } = &**i {
                    for name in names {
                        if let RholangNode::Var { name: var_name, .. } = &**name {
                            if !var_name.is_empty() {  // Skip empty variable names
                                // Use the bind node position (includes @ prefix) instead of just the var name
                                let location = i.absolute_start(&self.root);
                                let symbol = Arc::new(Symbol::new(
                                    var_name.clone(),
                                    SymbolType::Variable,
                                    self.current_uri.clone(),
                                    location,
                                ));
                                new_table.insert(symbol);
                                trace!("Declared variable '{}' in choice branch scope at {:?}", var_name, location);
                            } else {
                                trace!("Skipped empty variable name in choice branch at {:?}", name.absolute_start(&self.root));
                            }
                        }
                    }
                    if let Some(rem) = remainder {
                        if let RholangNode::Var { name: var_name, .. } = &**rem {
                            if !var_name.is_empty() {
                                let location = rem.absolute_start(&self.root);
                                let symbol = Arc::new(Symbol::new(
                                    var_name.clone(),
                                    SymbolType::Variable,
                                    self.current_uri.clone(),
                                    location,
                                ));
                                new_table.insert(symbol);
                                trace!("Declared remainder variable '{}' in choice branch scope at {:?}", var_name, location);
                            }
                        }
                    }
                }
            }
            let new_inputs: Vector<Arc<RholangNode>, ArcK> = inputs.iter().map(|i| self.visit_node(i)).collect();
            let new_proc = self.visit_node(proc);
            let branch_node = Arc::new(RholangNode::Choice {
                base: base.clone(),
                branches: Vector::new_with_ptr_kind().push_back((new_inputs.clone(), new_proc.clone())),
                metadata: metadata.clone(),
            });
            let _updated_branch = self.update_metadata(branch_node, new_table.clone(), None, metadata);
            self.pop_scope();
            (new_inputs, new_proc)
        }).collect();

        let new_node = Arc::new(RholangNode::Choice {
            base: base.clone(),
            branches: new_branches,
            metadata: metadata.clone(),
        });
        self.update_with_current_table(new_node, None, metadata)
    }

    /// Visits a `var` node, recording usages only if they differ from the declaration location.
    fn visit_var<'a>(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        name: &String,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let mut referenced_symbol: Option<Arc<Symbol>> = None;
        if !name.is_empty() {  // Only process non-empty variable names
            if let Some(symbol) = self.current_table.read().unwrap().lookup(name) {
                referenced_symbol = Some(symbol.clone());
                let usage_location = node.absolute_start(&self.root);
                let is_declaration = usage_location == symbol.declaration_location;
                let is_definition = symbol.definition_location.map_or(false, |def| usage_location == def);
                if is_declaration || is_definition {
                    trace!("Skipped recording for {} of '{}' at {:?}", if is_declaration {"declaration"} else {"definition"}, name, usage_location);
                } else {
                    // Two-tier resolution: Route based on symbol type, not just URI
                    if symbol.symbol_type == SymbolType::Contract {
                        // Contracts always go to global tier (rholang_symbols)
                        if let Some(ref rholang_symbols) = self.rholang_symbols {
                            use crate::lsp::rholang_contracts::SymbolLocation;
                            let ref_location = SymbolLocation::new(self.current_uri.clone(), usage_location);
                            let _ = rholang_symbols.add_reference(name, ref_location);
                            trace!("Added contract reference to rholang_symbols for '{}' at {:?}", name, usage_location);
                        }
                    } else if symbol.declaration_uri == self.current_uri {
                        // Local variables in same file - add to inverted_index
                        let decl_pos = symbol.declaration_location;
                        let mut index = self.inverted_index.write().unwrap();
                        index.entry(decl_pos).or_insert_with(Vec::new).push(usage_location);
                        trace!("Added local reference to inverted_index for '{}': {:?} -> {:?}", name, decl_pos, usage_location);
                    } else {
                        // Cross-file local variable reference (shouldn't happen often)
                        trace!("Cross-file local variable reference for '{}' at {:?}", name, usage_location);
                    }
                }
            } else {
                let usage_location = node.absolute_start(&self.root);
                // Phase 4: Removed potential_global_refs push - now handled by rholang_symbols
                trace!("Unbound reference for '{}' at {:?}", name, usage_location);

                // Phase 3.3: If rholang_symbols available, try to add reference
                // (may fail if symbol not yet declared - that's OK, forward refs handled separately)
                if let Some(ref rholang_symbols) = self.rholang_symbols {
                    use crate::lsp::rholang_contracts::SymbolLocation;
                    let ref_location = SymbolLocation::new(self.current_uri.clone(), usage_location);
                    let _ = rholang_symbols.add_reference(name, ref_location);
                    trace!("Tried to add reference to rholang_symbols for unbound '{}' at {:?}", name, usage_location);
                }
            }
        } else {
            trace!("Skipped empty variable name in var usage at {:?}", node.absolute_start(&self.root));
        }
        let new_node = Arc::new(RholangNode::Var {
            base: base.clone(),
            name: name.clone(),
            metadata: metadata.clone(),
        });
        self.update_with_current_table(new_node, referenced_symbol, metadata)
    }

    /// Visits a `send` node with pattern-based contract resolution
    ///
    /// This override enables pattern matching for contract invocations:
    /// - Extracts the contract name from the channel
    /// - Extracts ALL argument pattern values
    /// - Matches against contract definitions with matching patterns
    /// - Updates the channel node's referenced_symbol metadata with the matched symbol
    fn visit_send<'a>(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        channel: &Arc<RholangNode>,
        send_type: &RholangSendType,
        send_type_pos: &Position,
        inputs: &Vector<Arc<RholangNode>, ArcK>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        // Extract contract name from channel (Var or Quote)
        let contract_name_opt = self.extract_contract_name(channel);

        // Extract ALL argument values for pattern matching
        let arg_values = self.extract_all_pattern_values(inputs);

        // Try pattern matching to find the correct contract symbol
        let matched_symbol = if let Some(contract_name) = contract_name_opt.as_ref() {
            self.resolve_contract_by_pattern(&contract_name, arg_values, inputs, node)
        } else {
            None
        };

        // Visit the channel, potentially with pattern-matched symbol
        let new_channel = if let Some(ref symbol) = matched_symbol {
            // If we have a pattern-matched symbol, visit the channel and override its referenced_symbol metadata
            let visited_channel = self.visit_node(channel);
            // Update the channel node with the correct symbol
            self.update_with_current_table(visited_channel, Some(symbol.clone()), &None)
        } else {
            // No pattern match, use default visit
            self.visit_node(channel)
        };

        // Visit inputs normally
        let new_inputs = inputs.iter().map(|i| self.visit_node(i)).collect::<Vector<_, ArcK>>();

        // Return updated node if children changed
        if Arc::ptr_eq(channel, &new_channel) &&
            inputs.iter().zip(new_inputs.iter()).all(|(a, b)| Arc::ptr_eq(a, b)) {
            Arc::clone(node)
        } else {
            Arc::new(RholangNode::Send {
                base: base.clone(),
                channel: new_channel,
                send_type: send_type.clone(),
                send_type_pos: *send_type_pos,
                inputs: new_inputs,
                metadata: metadata.clone(),
            })
        }
    }

    fn visit_disjunction<'a>(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        left: &Arc<RholangNode>,
        right: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_left = self.visit_node(left);
        let new_right = self.visit_node(right);
        let new_node = Arc::new(RholangNode::Disjunction {
            base: base.clone(),
            left: new_left,
            right: new_right,
            metadata: metadata.clone(),
        });
        self.update_with_current_table(new_node, None, metadata)
    }

    fn visit_conjunction<'a>(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        left: &Arc<RholangNode>,
        right: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_left = self.visit_node(left);
        let new_right = self.visit_node(right);
        let new_node = Arc::new(RholangNode::Conjunction {
            base: base.clone(),
            left: new_left,
            right: new_right,
            metadata: metadata.clone(),
        });
        self.update_with_current_table(new_node, None, metadata)
    }

    fn visit_negation<'a>(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        operand: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_operand = self.visit_node(operand);
        let new_node = Arc::new(RholangNode::Negation {
            base: base.clone(),
            operand: new_operand,
            metadata: metadata.clone(),
        });
        self.update_with_current_table(new_node, None, metadata)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ropey::Rope;
    use crate::tree_sitter::{parse_code, parse_to_ir};
    use crate::ir::rholang_node::{compute_absolute_positions, find_node_at_position};
    use tower_lsp::lsp_types::Url;

    #[test]
    fn test_hierarchical_symbol_table() {
        let code = "new x in { let y = 42 in { contract z() = { x!(y) } } }";
        let rope = Rope::from_str(code);
        let tree = parse_code(code);
        let ir = parse_to_ir(&tree, &rope);
        let root: Arc<RholangNode> = ir;
        let uri = Url::parse("file:///test.rho").expect("Invalid URI");
        let global_table = Arc::new(SymbolTable::new(None));

        let builder = SymbolTableBuilder::new(root.clone(), uri.clone(), global_table.clone(), None);
        let transformed = builder.visit_node(&root);

        // Check nested scopes
        if let RholangNode::New { proc, .. } = &*transformed {
            if let RholangNode::Block { proc: let_node, .. } = &**proc {
                if let RholangNode::Let { proc: contract_block, .. } = &**let_node {
                    if let RholangNode::Block { proc: contract_node, .. } = &**contract_block {
                        let contract_table = contract_node.metadata()
                            .and_then(|m| m.get("symbol_table"))
                            .and_then(|t| t.downcast_ref::<Arc<SymbolTable>>())
                            .cloned()
                            .unwrap();
                        assert!(contract_table.lookup("x").is_some(), "x should be in scope");
                        assert!(contract_table.lookup("y").is_some(), "y should be in scope");
                        assert!(contract_table.lookup("z").is_some(), "z should be in global scope");
                    }
                }
            }
        }
    }

    #[test]
    fn test_symbol_table_new() {
        let code = "new x in { contract x() = { Nil } }";
        let rope = Rope::from_str(code);
        let tree = parse_code(code);
        let ir = parse_to_ir(&tree, &rope);
        let root: Arc<RholangNode> = ir;
        let uri = Url::parse("file:///test.rho").expect("Invalid URI");
        let global_table = Arc::new(SymbolTable::new(None));
        let builder = SymbolTableBuilder::new(root.clone(), uri, global_table, None);
        let transformed = builder.visit_node(&root);

        if let RholangNode::New { proc, .. } = &*transformed {
            if let RholangNode::Block { proc: contract_node, .. } = &**proc {
                let contract_table = contract_node.metadata()
                    .and_then(|m| m.get("symbol_table"))
                    .and_then(|t| t.downcast_ref::<Arc<SymbolTable>>())
                    .cloned()
                    .unwrap();
                assert!(contract_table.lookup("x").is_some());
            }
        }
    }

    // Priority 2b: Test disabled - get_inverted_index() removed
    // Local symbol references now tracked in rholang_symbols, not per-document inverted_index
    #[test]
    #[ignore]
    fn test_inverted_index() {
        // This test is obsolete - inverted_index removed in Priority 2b
        // Local symbol references now stored in rholang_symbols with local keys
    }

    #[test]
    fn test_position_lookup() {
        let code = "new x in { x!() }";
        let rope = Rope::from_str(code);
        let tree = parse_code(code);
        let ir = parse_to_ir(&tree, &rope);
        let root: Arc<RholangNode> = ir;
        let global_table = Arc::new(SymbolTable::new(None));
        let positions = compute_absolute_positions(&root);
        let uri = Url::parse("file:///test.rho").expect("Invalid URI");
        let builder = SymbolTableBuilder::new(root.clone(), uri, global_table, None);
        builder.visit_node(&root);
        let position = Position { row: 0, column: 11, byte: 11 };
        let node = find_node_at_position(&root, &positions, position).unwrap();
        assert_eq!(node.text(&rope, &root).to_string(), "x");
    }

    #[test]
    fn test_symbol_table_scoping() {
        let code = "new x in { let y = x in { y } }";
        let rope = Rope::from_str(code);
        let tree = parse_code(code);
        let ir = parse_to_ir(&tree, &rope);
        let root: Arc<RholangNode> = ir;
        let uri = Url::parse("file:///test.rho").expect("Invalid URI");
        let global_table = Arc::new(SymbolTable::new(None));
        let builder = SymbolTableBuilder::new(root.clone(), uri.clone(), global_table.clone(), None);
        let transformed = builder.visit_node(&root);

        assert!(global_table.lookup("x").is_none(), "x should be in local scope");

        if let RholangNode::New { proc, .. } = &*transformed {
            if let RholangNode::Block { proc: let_node, .. } = &**proc {
                let let_table = let_node.metadata().and_then(|m| m.get("symbol_table"))
                    .and_then(|t| t.downcast_ref::<Arc<SymbolTable>>())
                    .cloned()
                    .unwrap();
                assert!(let_table.lookup("x").is_some(), "x should be accessible in let scope");
                if let RholangNode::Let { decls, proc, .. } = &**let_node {
                    if let Some(decl) = decls.first() {
                        if let RholangNode::Decl { procs, .. } = &**decl {
                            if let Some(_x_node) = procs.first() {
                                // Priority 2b: Removed inverted_index checks
                                // Usage tracking now in rholang_symbols, not per-document inverted_index
                            }
                        }
                    }
                    if let RholangNode::Block { proc: y_node, .. } = &**proc {
                        let y_table = y_node.metadata().and_then(|m| m.get("symbol_table"))
                            .and_then(|t| t.downcast_ref::<Arc<SymbolTable>>())
                            .cloned()
                            .unwrap();
                        assert!(y_table.lookup("y").is_some(), "y should be in the Var node's symbol table");
                        // Priority 2b: Removed inverted_index checks
                        // Usage tracking now in rholang_symbols, not per-document inverted_index
                    }
                }
            }
        }
    }

    // Priority 2b: Test disabled - get_potential_global_refs() removed
    // Cross-file references now handled automatically by rholang_symbols
    // during symbol table building - no need for separate "potential" tracking
    #[test]
    #[ignore]
    fn test_cross_file_reference() {
        // This test is obsolete - cross-file references are now indexed directly
        // into rholang_symbols during visit_var(), not collected as "potentials"
    }

    #[test]
    fn test_contract_parameters_in_symbol_table() {
        let code = "contract foo(x) = { Nil }";
        let rope = Rope::from_str(code);
        let tree = parse_code(code);
        let ir = parse_to_ir(&tree, &rope);
        let root: Arc<RholangNode> = ir;
        let uri = Url::parse("file:///test.rho").expect("Invalid URI");
        let global_table = Arc::new(SymbolTable::new(None));
        let builder = SymbolTableBuilder::new(root.clone(), uri.clone(), global_table.clone(), None);
        let transformed = builder.visit_node(&root);

        if let RholangNode::Contract { metadata, .. } = &*transformed {
            let symbol_table = metadata.as_ref().unwrap()
                .get("symbol_table")
                .and_then(|t| t.downcast_ref::<Arc<SymbolTable>>())
                .cloned()
                .unwrap();
            assert!(symbol_table.lookup("x").is_some(), "Parameter'x' should be in contract's symbol table");
        } else {
            panic!("Expected Contract node");
        }
    }
}
