//! Pattern matching infrastructure for MeTTa using MORK
//!
//! This module provides pattern-based indexing and matching for MeTTa function definitions,
//! enabling efficient pattern-aware go-to-definition and other LSP features.
//!
//! Coordinates with MeTTaTron's MORK-based pattern matching.

use std::sync::Arc;
use std::collections::HashMap;
use tower_lsp::lsp_types::{Location, Url, Range, Position};
use mork::space::Space;

use crate::ir::metta_node::MettaNode;

/// A MeTTa function definition with its pattern
#[derive(Debug, Clone)]
pub struct MettaDefinition {
    /// The function name (e.g., "is_connected")
    pub name: String,
    /// The full pattern s-expression (e.g., (is_connected $from $to))
    pub pattern: Arc<MettaNode>,
    /// Location of this definition in the source
    pub location: Location,
    /// Arity (number of parameters) for quick filtering
    pub arity: usize,
}

/// Pattern matcher for MeTTa function definitions
///
/// Uses MORK's Space for efficient O(k) pattern matching where k is the number
/// of matching patterns (vs O(n) for iteration over all patterns).
///
/// This enables pattern-aware go-to-definition: given a call like `(is_connected room_a room_b)`,
/// find all definitions with matching patterns like `(is_connected $from $to)`.
pub struct MettaPatternMatcher {
    /// MORK Space for pattern storage
    space: Space,
    /// Map from pattern Expr to definition info
    pattern_to_def: HashMap<String, Vec<MettaDefinition>>,
    /// Index by function name for quick initial filtering
    name_index: HashMap<String, Vec<MettaDefinition>>,
}

impl std::fmt::Debug for MettaPatternMatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MettaPatternMatcher")
            .field("space", &"<MORK Space>")
            .field("definitions", &self.name_index.len())
            .finish()
    }
}

impl MettaPatternMatcher {
    /// Create a new pattern matcher with an empty MORK Space
    pub fn new() -> Self {
        MettaPatternMatcher {
            space: Space::new(),
            pattern_to_def: HashMap::new(),
            name_index: HashMap::new(),
        }
    }

    /// Add a function definition pattern to the index
    ///
    /// # Arguments
    /// * `name` - The function name (e.g., "is_connected")
    /// * `pattern` - The pattern node (e.g., `(is_connected $from $to)`)
    /// * `location` - Source location of this definition
    ///
    /// # Returns
    /// Ok(()) if pattern was added successfully
    pub fn add_definition(
        &mut self,
        name: String,
        pattern: Arc<MettaNode>,
        location: Location,
    ) -> Result<(), String> {
        // Compute arity from pattern
        let arity = self.compute_arity(&pattern);

        let def = MettaDefinition {
            name: name.clone(),
            pattern: pattern.clone(),
            location,
            arity,
        };

        // Add to name index
        self.name_index
            .entry(name.clone())
            .or_insert_with(Vec::new)
            .push(def.clone());

        // TODO: Add to MORK space for pattern matching
        // For now, we'll use simple name-based + arity filtering
        // Future: convert pattern to MORK Expr and store in space

        Ok(())
    }

    /// Find all definitions matching a call site pattern
    ///
    /// Given a call like `(is_connected room_a room_b)`, find all definitions
    /// with matching patterns like `(is_connected $from $to)`.
    ///
    /// # Arguments
    /// * `call_pattern` - The call site pattern node
    ///
    /// # Returns
    /// Vector of matching definition locations
    pub fn find_matching_definitions(&self, call_pattern: &MettaNode) -> Vec<Location> {
        // Extract function name and arity from call site
        let (name, arity) = match self.extract_call_info(call_pattern) {
            Some(info) => info,
            None => return Vec::new(),
        };

        // Find candidates by name
        let candidates = match self.name_index.get(&name) {
            Some(defs) => defs,
            None => return Vec::new(),
        };

        // Filter by arity (simple heuristic for now)
        let mut matches = Vec::new();
        for def in candidates {
            if def.arity == arity {
                matches.push(def.location.clone());
            }
        }

        // TODO: Use MORK pattern matching for more precise filtering
        // This would handle cases like:
        // - (is_connected $x room_b) should match (is_connected $from $to)
        // - but not (is_connected $x $y $z) with different arity

        matches
    }

    /// Extract function name and arity from a call site pattern
    ///
    /// For example, `(is_connected room_a room_b)` -> ("is_connected", 2)
    fn extract_call_info(&self, node: &MettaNode) -> Option<(String, usize)> {
        match node {
            MettaNode::SExpr { elements, .. } if elements.len() > 0 => {
                // First element should be the function name
                let name = elements[0].name()?;
                let arity = elements.len() - 1; // Subtract 1 for function name
                Some((name.to_string(), arity))
            }
            _ => None,
        }
    }

    /// Compute the arity of a pattern (number of parameters)
    ///
    /// For example, `(is_connected $from $to)` has arity 2
    fn compute_arity(&self, node: &MettaNode) -> usize {
        match node {
            MettaNode::SExpr { elements, .. } if elements.len() > 0 => {
                elements.len() - 1 // Subtract 1 for function name
            }
            _ => 0,
        }
    }

    /// Get all definitions for a given function name
    ///
    /// This is used for simple name-based lookup without pattern matching
    pub fn get_definitions_by_name(&self, name: &str) -> Vec<&MettaDefinition> {
        self.name_index
            .get(name)
            .map(|defs| defs.iter().collect())
            .unwrap_or_else(Vec::new)
    }

    /// Clear all indexed patterns
    pub fn clear(&mut self) {
        self.space = Space::new();
        self.pattern_to_def.clear();
        self.name_index.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::semantic_node::NodeBase;
    use crate::ir::rholang_node::RelativePosition;

    fn test_base() -> NodeBase {
        NodeBase::new(
            RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            0,
            0,
            0,
        )
    }

    fn test_location() -> Location {
        Location {
            uri: Url::parse("file:///test.metta").unwrap(),
            range: Range::new(Position::new(0, 0), Position::new(0, 10)),
        }
    }

    #[test]
    fn test_add_definition() {
        let mut matcher = MettaPatternMatcher::new();

        // Create pattern: (is_connected $from $to)
        let pattern = Arc::new(MettaNode::SExpr {
            base: test_base(),
            elements: vec![
                Arc::new(MettaNode::Atom {
                    base: test_base(),
                    name: "is_connected".to_string(),
                    metadata: None,
                }),
                Arc::new(MettaNode::Variable {
                    base: test_base(),
                    name: "from".to_string(),
                    var_type: crate::ir::metta_node::MettaVariableType::Regular,
                    metadata: None,
                }),
                Arc::new(MettaNode::Variable {
                    base: test_base(),
                    name: "to".to_string(),
                    var_type: crate::ir::metta_node::MettaVariableType::Regular,
                    metadata: None,
                }),
            ],
            metadata: None,
        });

        let result = matcher.add_definition(
            "is_connected".to_string(),
            pattern,
            test_location(),
        );

        assert!(result.is_ok());
        assert_eq!(matcher.name_index.len(), 1);
        assert_eq!(matcher.name_index.get("is_connected").unwrap().len(), 1);
    }

    #[test]
    fn test_find_by_name() {
        let mut matcher = MettaPatternMatcher::new();

        let pattern = Arc::new(MettaNode::Atom {
            base: test_base(),
            name: "test".to_string(),
            metadata: None,
        });

        matcher.add_definition("test_fn".to_string(), pattern, test_location()).unwrap();

        let defs = matcher.get_definitions_by_name("test_fn");
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "test_fn");
    }

    #[test]
    fn test_extract_call_info() {
        let matcher = MettaPatternMatcher::new();

        // Create call: (is_connected room_a room_b)
        let call = MettaNode::SExpr {
            base: test_base(),
            elements: vec![
                Arc::new(MettaNode::Atom {
                    base: test_base(),
                    name: "is_connected".to_string(),
                    metadata: None,
                }),
                Arc::new(MettaNode::Atom {
                    base: test_base(),
                    name: "room_a".to_string(),
                    metadata: None,
                }),
                Arc::new(MettaNode::Atom {
                    base: test_base(),
                    name: "room_b".to_string(),
                    metadata: None,
                }),
            ],
            metadata: None,
        };

        let info = matcher.extract_call_info(&call);
        assert_eq!(info, Some(("is_connected".to_string(), 2)));
    }
}
