//! Symbol table builder for MeTTa
//!
//! Builds scoped symbol tables from MeTTa IR for LSP features like
//! document highlights, go-to-definition, and rename.

use std::collections::HashMap;
use std::sync::Arc;
use tower_lsp::lsp_types::{Location, Position as LspPosition, Range, Url};

use crate::ir::metta_node::MettaNode;
use crate::ir::metta_pattern_matching::MettaPatternMatcher;
use crate::ir::semantic_node::Position;

/// Type of symbol in MeTTa
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MettaSymbolKind {
    /// Variable binding ($x, &y, 'z)
    Variable,
    /// Function definition (left side of =)
    Definition,
    /// Lambda parameter
    Parameter,
    /// Let binding variable
    LetBinding,
    /// Match pattern variable
    MatchPattern,
}

/// Information about a symbol occurrence
#[derive(Debug, Clone)]
pub struct SymbolOccurrence {
    /// The symbol name (without $ or & prefix for variables)
    pub name: String,
    /// Kind of symbol
    pub kind: MettaSymbolKind,
    /// Position in the document
    pub range: Range,
    /// Whether this is the definition site
    pub is_definition: bool,
    /// Scope ID this symbol belongs to
    pub scope_id: usize,
}

/// A scope in MeTTa code
#[derive(Debug, Clone)]
pub struct MettaScope {
    /// Unique ID for this scope
    pub id: usize,
    /// Parent scope ID (None for global scope)
    pub parent_id: Option<usize>,
    /// Symbols defined in this scope
    pub symbols: HashMap<String, Vec<SymbolOccurrence>>,
}

/// Symbol table for a MeTTa document
#[derive(Debug)]
pub struct MettaSymbolTable {
    /// All scopes in the document
    pub scopes: Vec<MettaScope>,
    /// All symbol occurrences (for quick lookup by position)
    pub all_occurrences: Vec<SymbolOccurrence>,
    /// Pattern matcher for function definitions (wrapped in Arc for sharing)
    pub pattern_matcher: Arc<MettaPatternMatcher>,
    /// Document URI (for creating locations)
    pub uri: Url,
    /// The IR nodes (for looking up call sites)
    pub ir_nodes: Vec<Arc<MettaNode>>,
}

impl MettaSymbolTable {
    /// Find the symbol occurrence at a given position
    pub fn find_symbol_at_position(&self, position: &LspPosition) -> Option<&SymbolOccurrence> {
        self.all_occurrences.iter().find(|occ| {
            position_in_range(position, &occ.range)
        })
    }

    /// Find all occurrences of a symbol in its scope
    pub fn find_symbol_references(&self, symbol: &SymbolOccurrence) -> Vec<&SymbolOccurrence> {
        let scope = &self.scopes[symbol.scope_id];

        // Collect from current scope and all child scopes
        let mut refs = Vec::new();
        self.collect_references_in_scope(scope.id, &symbol.name, &mut refs);
        refs
    }

    fn collect_references_in_scope<'a>(&'a self, scope_id: usize, name: &str, refs: &mut Vec<&'a SymbolOccurrence>) {
        // Add from current scope
        if let Some(occurrences) = self.scopes[scope_id].symbols.get(name) {
            refs.extend(occurrences.iter());
        }

        // Add from child scopes (scopes that have this as parent)
        for scope in &self.scopes {
            if scope.parent_id == Some(scope_id) {
                self.collect_references_in_scope(scope.id, name, refs);
            }
        }
    }

    /// Find the definition of a symbol
    ///
    /// For variables, finds the binding definition in the same scope.
    /// For function calls, uses pattern matching to find matching definitions.
    pub fn find_definition<'a>(&'a self, symbol: &'a SymbolOccurrence) -> Option<&'a SymbolOccurrence> {
        match symbol.kind {
            MettaSymbolKind::Variable | MettaSymbolKind::Parameter | MettaSymbolKind::LetBinding | MettaSymbolKind::MatchPattern => {
                // For variables, find the definition in the same scope
                let refs = self.find_symbol_references(symbol);
                refs.into_iter().find(|occ| occ.is_definition)
            }
            MettaSymbolKind::Definition => {
                // For function definitions, already at the definition
                Some(symbol)
            }
        }
    }

    /// Find all definitions matching a function call pattern
    ///
    /// Uses pattern matching to find all definitions that could match the call site.
    /// For example, `(is_connected room_a room_b)` matches `(is_connected $from $to)`.
    ///
    /// # Arguments
    /// * `call_node` - The function call node
    ///
    /// # Returns
    /// Vector of locations for all matching definitions
    pub fn find_function_definitions(&self, call_node: &MettaNode) -> Vec<Location> {
        self.pattern_matcher.find_matching_definitions(call_node)
    }
}

/// Builder for MeTTa symbol tables
pub struct MettaSymbolTableBuilder {
    scopes: Vec<MettaScope>,
    all_occurrences: Vec<SymbolOccurrence>,
    next_scope_id: usize,
    /// Absolute positions cache
    positions: HashMap<usize, (Position, Position)>,
    /// Pattern matcher for function definitions
    pattern_matcher: MettaPatternMatcher,
    /// Document URI
    uri: Url,
}

impl MettaSymbolTableBuilder {
    /// Create a new symbol table builder for a document
    ///
    /// # Arguments
    /// * `uri` - The document URI (for creating definition locations)
    pub fn new(uri: Url) -> Self {
        // Create global scope
        let global_scope = MettaScope {
            id: 0,
            parent_id: None,
            symbols: HashMap::new(),
        };

        Self {
            scopes: vec![global_scope],
            all_occurrences: Vec::new(),
            next_scope_id: 1,
            positions: HashMap::new(),
            pattern_matcher: MettaPatternMatcher::new(),
            uri,
        }
    }

    /// Build symbol table from MeTTa IR nodes
    pub fn build(mut self, nodes: &[Arc<MettaNode>]) -> MettaSymbolTable {
        // Compute absolute positions for all nodes together
        // Each node's position is relative to the previous node's end
        let mut prev_end = Position { row: 0, column: 0, byte: 0 };

        for node in nodes {
            // Import the helper function from metta_node module
            use crate::ir::metta_node::compute_positions_with_prev_end;
            let (node_positions, new_prev_end) = compute_positions_with_prev_end(node, prev_end);
            self.positions.extend(node_positions);
            prev_end = new_prev_end;
        }

        // Process each top-level node in global scope
        for node in nodes {
            self.process_node(node, 0);
        }

        MettaSymbolTable {
            scopes: self.scopes,
            all_occurrences: self.all_occurrences,
            pattern_matcher: Arc::new(self.pattern_matcher),
            uri: self.uri,
            ir_nodes: nodes.to_vec(),
        }
    }

    fn create_scope(&mut self, parent_id: usize) -> usize {
        let id = self.next_scope_id;
        self.next_scope_id += 1;

        self.scopes.push(MettaScope {
            id,
            parent_id: Some(parent_id),
            symbols: HashMap::new(),
        });

        id
    }

    fn add_symbol(&mut self, occurrence: SymbolOccurrence) {
        // Add to scope's symbol list
        self.scopes[occurrence.scope_id]
            .symbols
            .entry(occurrence.name.clone())
            .or_insert_with(Vec::new)
            .push(occurrence.clone());

        // Add to global list
        self.all_occurrences.push(occurrence);
    }

    /// Index a function definition for pattern-based lookup
    ///
    /// Extracts the function name from the pattern and adds it to the pattern matcher.
    /// For example, from `(is_connected $from $to)`, extracts "is_connected".
    fn index_function_definition(&mut self, pattern: &Arc<MettaNode>, _base: &crate::ir::semantic_node::NodeBase) {
        // Extract function name from pattern
        // Pattern is typically an SExpr like (function_name $arg1 $arg2)
        let name = match &**pattern {
            MettaNode::SExpr { elements, .. } if elements.len() > 0 => {
                // First element is the function name
                match elements[0].name() {
                    Some(n) => n.to_string(),
                    None => return, // Not a valid function pattern
                }
            }
            _ => return, // Not a function definition
        };

        // Get the pattern's position to create a location
        let node_ptr = &**pattern as *const MettaNode as usize;
        let (start, end) = match self.positions.get(&node_ptr) {
            Some(pos) => pos,
            None => return, // Position not computed yet
        };

        // Create location for this definition
        let location = Location {
            uri: self.uri.clone(),
            range: Range::new(
                LspPosition::new(start.row as u32, start.column as u32),
                LspPosition::new(end.row as u32, end.column as u32),
            ),
        };

        // Add to pattern matcher
        if let Err(e) = self.pattern_matcher.add_definition(name, pattern.clone(), location) {
            eprintln!("Warning: Failed to index function definition: {}", e);
        }
    }

    fn process_node(&mut self, node: &Arc<MettaNode>, scope_id: usize) {
        match &**node {
            // Definition: (= pattern body)
            // Creates a new scope where pattern variables are bound
            MettaNode::Definition { pattern, body, base, .. } => {
                let def_scope = self.create_scope(scope_id);

                // Index this function definition for pattern matching
                self.index_function_definition(pattern, base);

                // Process pattern to collect bound variables
                self.process_pattern(pattern, def_scope, true);

                // Process body in the new scope
                self.process_node(body, def_scope);
            }

            // Lambda: (lambda (params) body)
            MettaNode::Lambda { params, body, .. } => {
                let lambda_scope = self.create_scope(scope_id);

                // Parameters are definitions in the lambda scope
                for param in params {
                    self.process_pattern(param, lambda_scope, true);
                }

                self.process_node(body, lambda_scope);
            }

            // Let: (let ((var val)...) body)
            MettaNode::Let { bindings, body, .. } => {
                let let_scope = self.create_scope(scope_id);

                // Process bindings
                for (var, val) in bindings {
                    // Process the value expression in parent scope
                    self.process_node(val, scope_id);
                    // Bind the variable in the let scope
                    self.process_pattern(var, let_scope, true);
                }

                self.process_node(body, let_scope);
            }

            // Match: (match scrutinee (pattern body)...)
            MettaNode::Match { scrutinee, cases, .. } => {
                self.process_node(scrutinee, scope_id);

                // Check if this is a grounded query: (match & space pattern return)
                // In grounded queries, the pattern is a query that references existing variables,
                // not a pattern that defines new bindings
                let is_grounded_query = if let MettaNode::SExpr { elements, .. } = &**scrutinee {
                    let has_ampersand = elements.len() == 2
                        && matches!(&*elements[0], MettaNode::Variable { var_type: crate::ir::metta_node::MettaVariableType::Grounded, .. });
                    has_ampersand && cases.len() == 1
                } else {
                    false
                };

                if is_grounded_query {
                    // Grounded query: process pattern and body in the current scope
                    // Variables in the pattern are references, not definitions
                    for (pattern, case_body) in cases {
                        self.process_node(pattern, scope_id);  // Process as references
                        self.process_node(case_body, scope_id);
                    }
                } else {
                    // Regular match: create new scope for each case
                    for (pattern, case_body) in cases {
                        let case_scope = self.create_scope(scope_id);
                        self.process_pattern(pattern, case_scope, true);
                        self.process_node(case_body, case_scope);
                    }
                }
            }

            // If: (if cond then else)
            MettaNode::If { condition, consequence, alternative, .. } => {
                self.process_node(condition, scope_id);
                self.process_node(consequence, scope_id);
                if let Some(alt) = alternative {
                    self.process_node(alt, scope_id);
                }
            }

            // S-expression: recursively process elements
            MettaNode::SExpr { elements, .. } => {
                for elem in elements {
                    self.process_node(elem, scope_id);
                }
            }

            // Type annotation
            MettaNode::TypeAnnotation { expr, type_expr, .. } => {
                self.process_node(expr, scope_id);
                self.process_node(type_expr, scope_id);
            }

            // Eval
            MettaNode::Eval { expr, .. } => {
                self.process_node(expr, scope_id);
            }

            // Variable reference
            MettaNode::Variable { name, .. } => {
                if let Some(range) = self.get_node_range(node) {
                    let occurrence = SymbolOccurrence {
                        name: name.clone(),
                        kind: MettaSymbolKind::Variable,
                        range,
                        is_definition: false,
                        scope_id,
                    };
                    self.add_symbol(occurrence);
                }
            }

            // Atom - could be a function reference
            MettaNode::Atom { name, .. } => {
                if let Some(range) = self.get_node_range(node) {
                    let occurrence = SymbolOccurrence {
                        name: name.clone(),
                        kind: MettaSymbolKind::Definition,
                        range,
                        is_definition: false,
                        scope_id,
                    };
                    self.add_symbol(occurrence);
                }
            }

            // Literals and other leaf nodes - no symbols
            _ => {}
        }
    }

    fn process_pattern(&mut self, pattern: &Arc<MettaNode>, scope_id: usize, is_definition: bool) {
        match &**pattern {
            MettaNode::Variable { name, .. } => {
                if let Some(range) = self.get_node_range(pattern) {
                    let occurrence = SymbolOccurrence {
                        name: name.clone(),
                        kind: MettaSymbolKind::Variable,
                        range,
                        is_definition,
                        scope_id,
                    };
                    self.add_symbol(occurrence);
                }
            }

            MettaNode::Atom { name, .. } if is_definition => {
                if let Some(range) = self.get_node_range(pattern) {
                    let occurrence = SymbolOccurrence {
                        name: name.clone(),
                        kind: MettaSymbolKind::Definition,
                        range,
                        is_definition,
                        scope_id,
                    };
                    self.add_symbol(occurrence);
                }
            }

            MettaNode::SExpr { elements, .. } => {
                for elem in elements {
                    self.process_pattern(elem, scope_id, is_definition);
                }
            }

            _ => {}
        }
    }

    fn get_node_range(&self, node: &Arc<MettaNode>) -> Option<Range> {
        let key = &**node as *const MettaNode as usize;
        self.positions.get(&key).map(|(start, end)| {
            Range {
                start: LspPosition {
                    line: start.row as u32,
                    character: start.column as u32,
                },
                end: LspPosition {
                    line: end.row as u32,
                    character: end.column as u32,
                },
            }
        })
    }
}

fn position_in_range(position: &LspPosition, range: &Range) -> bool {
    if position.line < range.start.line || position.line > range.end.line {
        return false;
    }
    if position.line == range.start.line && position.character < range.start.character {
        return false;
    }
    if position.line == range.end.line && position.character > range.end.character {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::semantic_node::{NodeBase, RelativePosition};

    fn test_base() -> NodeBase {
        NodeBase::new_simple(
            RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 },
            10,
            0,
            10,
        )
    }

    #[test]
    fn test_lambda_scope() {
        // (lambda ($x) $x)
        let param = Arc::new(MettaNode::Variable {
            base: test_base(),
            name: "x".to_string(),
            var_type: crate::ir::metta_node::MettaVariableType::Regular,
            metadata: None,
        });
        let body = Arc::new(MettaNode::Variable {
            base: test_base(),
            name: "x".to_string(),
            var_type: crate::ir::metta_node::MettaVariableType::Regular,
            metadata: None,
        });
        let lambda = Arc::new(MettaNode::Lambda {
            base: test_base(),
            params: vec![param],
            body,
            metadata: None,
        });

        let test_uri = tower_lsp::lsp_types::Url::parse("file:///test.metta").unwrap();
        let builder = MettaSymbolTableBuilder::new(test_uri);
        let table = builder.build(&[lambda]);

        // Should have 2 scopes: global + lambda
        assert_eq!(table.scopes.len(), 2);

        // Should have 2 occurrences of "x": definition and reference
        let x_refs: Vec<_> = table.all_occurrences.iter()
            .filter(|occ| occ.name == "x")
            .collect();
        assert_eq!(x_refs.len(), 2);
        assert!(x_refs.iter().any(|occ| occ.is_definition));
    }

    #[test]
    fn test_parser_to_symbol_table() {
        use crate::parsers::MettaParser;

        // Simple MeTTa code with variable usage
        let metta_code = r#"
(= (factorial $n)
   (if (== $n 0)
       1
       (* $n (factorial (- $n 1)))))
"#;

        // Parse the code
        let mut parser = MettaParser::new().expect("Failed to create parser");
        let nodes = parser.parse_to_ir(metta_code).expect("Failed to parse MeTTa code");

        // Build symbol table
        let test_uri = tower_lsp::lsp_types::Url::parse("file:///test.metta").unwrap();
        let builder = MettaSymbolTableBuilder::new(test_uri);
        let table = builder.build(&nodes);

        // Should have at least 2 scopes: global + definition
        assert!(table.scopes.len() >= 2, "Expected at least 2 scopes, got {}", table.scopes.len());

        // Should have some symbol occurrences
        assert!(!table.all_occurrences.is_empty(), "Expected some symbol occurrences");

        // Should have references to variable 'n'
        let n_refs: Vec<_> = table.all_occurrences.iter()
            .filter(|occ| occ.name == "n")
            .collect();
        assert!(!n_refs.is_empty(), "Expected references to variable 'n'");

        // At least one should be a definition
        assert!(n_refs.iter().any(|occ| occ.is_definition), "Expected at least one definition of 'n'");
    }

    #[test]
    fn test_grounded_query_syntax() {
        use crate::parsers::MettaParser;

        // MeTTa code with grounded query syntax (match & self pattern return)
        let metta_code = r#"
(= (get_neighbors $room)
   (match & self (connected $room $target) $target))
"#;

        // Parse the code - this should NOT fail
        let mut parser = MettaParser::new().expect("Failed to create parser");
        let nodes = parser.parse_to_ir(metta_code).expect("Failed to parse grounded query syntax");

        // Build symbol table
        let test_uri = tower_lsp::lsp_types::Url::parse("file:///test.metta").unwrap();
        let builder = MettaSymbolTableBuilder::new(test_uri);
        let table = builder.build(&nodes);

        // Should have parsed successfully
        assert!(!table.all_occurrences.is_empty(), "Expected symbol occurrences from grounded query");

        // Should have references to variables 'room' and 'target'
        let room_refs: Vec<_> = table.all_occurrences.iter()
            .filter(|occ| occ.name == "room")
            .collect();
        assert!(!room_refs.is_empty(), "Expected references to 'room'");

        let target_refs: Vec<_> = table.all_occurrences.iter()
            .filter(|occ| occ.name == "target")
            .collect();
        assert!(!target_refs.is_empty(), "Expected references to 'target'");

        // Test position-based lookup
        if let Some(first_room) = room_refs.first() {
            let found = table.find_symbol_at_position(&first_room.range.start);
            assert!(found.is_some(), "Should find symbol 'room' at its own start position");
        }
    }
}
