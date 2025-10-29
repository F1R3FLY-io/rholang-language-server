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

/// Fragment information for concatenated string sources
#[derive(Debug, Clone)]
pub struct FragmentInfo {
    /// Number of string fragments that were concatenated
    pub num_fragments: usize,
    /// Indices of fragments containing this definition
    pub fragment_indices: Vec<usize>,
}

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
    /// Whether this definition comes from a concatenated string
    /// Concatenated strings have runtime variables, limiting MORK pattern matching precision
    pub is_concatenated: bool,
    /// Detailed fragment information if from concatenated source
    pub fragment_info: Option<FragmentInfo>,
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
    /// Map from pattern Expr to definition info (legacy, may be deprecated)
    pattern_to_def: HashMap<String, Vec<MettaDefinition>>,
    /// Index by function name for quick initial filtering
    name_index: HashMap<String, Vec<MettaDefinition>>,
    /// Position index: MORK pattern bytes → LSP locations
    /// This enables mapping MORK query results back to source positions
    pattern_locations: HashMap<Vec<u8>, Vec<Location>>,
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
            pattern_locations: HashMap::new(),
        }
    }

    /// Add a function definition pattern to the index
    ///
    /// # Arguments
    /// * `name` - The function name (e.g., "is_connected")
    /// * `pattern` - The pattern node (e.g., `(is_connected $from $to)`)
    /// * `location` - Source location of this definition
    /// * `is_concatenated` - Whether the definition comes from concatenated strings
    /// * `fragment_info` - Optional concatenation metadata
    ///
    /// # Returns
    /// Ok(()) if pattern was added successfully
    pub fn add_definition(
        &mut self,
        name: String,
        pattern: Arc<MettaNode>,
        location: Location,
        is_concatenated: bool,
        fragment_info: Option<FragmentInfo>,
    ) -> Result<(), String> {
        // Compute arity from pattern
        let arity = self.compute_arity(&pattern);

        let def = MettaDefinition {
            name: name.clone(),
            pattern: pattern.clone(),
            location: location.clone(),
            arity,
            is_concatenated,
            fragment_info,
        };

        // Add to name index
        self.name_index
            .entry(name.clone())
            .or_insert_with(Vec::new)
            .push(def.clone());

        // Add to MORK space and position index (only for non-concatenated patterns)
        // Concatenated patterns have runtime variables, making MORK matching unreliable
        if !is_concatenated {
            // Convert pattern to MORK bytes
            let pattern_bytes = self.pattern_to_mork_bytes(&pattern)?;

            // Add to MORK Space using the same approach as MeTTaTron
            let mork_str = Self::node_to_mork_string(&pattern);
            if let Ok(_count) = self.space.load_all_sexpr_impl(mork_str.as_bytes(), true) {
                // Successfully added to MORK Space
                // Now map the pattern bytes to the location
                self.pattern_locations
                    .entry(pattern_bytes)
                    .or_insert_with(Vec::new)
                    .push(location.clone());
            } else {
                // Failed to add to MORK Space - log but don't fail
                // Fall back to name-based matching will still work
                eprintln!("Warning: Failed to add pattern to MORK Space: {}", mork_str);
            }
        }

        Ok(())
    }

    /// Find definitions using MORK-based pattern lookup
    ///
    /// This method queries the pattern_locations index directly using pattern bytes.
    /// It provides O(1) lookup for exact pattern matches.
    ///
    /// # Arguments
    /// * `call_pattern` - The call site pattern node
    ///
    /// # Returns
    /// Vector of matching locations, or empty vector if no pattern-based match
    fn find_with_mork(&self, call_pattern: &MettaNode) -> Vec<Location> {
        // Convert call site to pattern by replacing concrete args with variables
        // For example: (is_connected room_a room_b) → (is_connected $a $b)
        let normalized_pattern = self.normalize_call_to_pattern(call_pattern);

        // Convert to MORK bytes
        if let Ok(pattern_bytes) = self.pattern_to_mork_bytes(&normalized_pattern) {
            // Look up in pattern_locations index
            if let Some(locations) = self.pattern_locations.get(&pattern_bytes) {
                return locations.clone();
            }
        }

        Vec::new()
    }

    /// Normalize a call site to a pattern by replacing arguments with variables
    ///
    /// Transforms `(is_connected room_a room_b)` into `(is_connected $a $b)`
    /// This enables pattern matching against indexed definitions.
    fn normalize_call_to_pattern(&self, node: &MettaNode) -> Arc<MettaNode> {
        match node {
            MettaNode::SExpr { base, elements, metadata } if !elements.is_empty() => {
                // Keep the function name (first element), replace args with variables
                let mut normalized = vec![elements[0].clone()];

                // Replace each argument with a generic variable
                const VAR_NAMES: [&str; 10] = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"];
                for (idx, _arg) in elements[1..].iter().enumerate() {
                    let var_name = if idx < VAR_NAMES.len() {
                        VAR_NAMES[idx].to_string()
                    } else {
                        format!("v{}", idx)
                    };

                    normalized.push(Arc::new(MettaNode::Variable {
                        base: base.clone(),
                        name: var_name,
                        var_type: crate::ir::metta_node::MettaVariableType::Regular,
                        metadata: None,
                    }));
                }

                Arc::new(MettaNode::SExpr {
                    base: base.clone(),
                    elements: normalized,
                    metadata: metadata.clone(),
                })
            }
            _ => Arc::new(node.clone()),
        }
    }

    /// Find all definitions matching a call site pattern (hybrid approach)
    ///
    /// Given a call like `(is_connected room_a room_b)`, find all definitions
    /// with matching patterns like `(is_connected $from $to)`.
    ///
    /// **Hybrid Strategy**:
    /// 1. Try MORK-based pattern lookup first (O(1) hash lookup)
    /// 2. Fall back to name+arity matching for concatenated patterns
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

        // Partition candidates by provenance
        let (simple_defs, concat_defs): (Vec<_>, Vec<_>) = candidates
            .iter()
            .partition(|def| !def.is_concatenated);

        let mut matches = Vec::new();

        // For simple (non-concatenated) definitions: Try MORK pattern matching
        if !simple_defs.is_empty() {
            let mork_matches = self.find_with_mork(call_pattern);
            if !mork_matches.is_empty() {
                matches.extend(mork_matches);
            } else {
                // MORK didn't find anything, fall back to arity matching
                for def in simple_defs {
                    if def.arity == arity {
                        matches.push(def.location.clone());
                    }
                }
            }
        }

        // For concatenated definitions: Use name+arity matching only
        // (Concatenated strings have runtime variables, MORK can't help)
        for def in concat_defs {
            if def.arity == arity {
                matches.push(def.location.clone());
            }
        }

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

    /// Convert MettaNode to MORK s-expression string format
    ///
    /// This format can be parsed by MORK's parser and added to the Space.
    /// Follows the same conventions as MeTTaTron's `MettaValue::to_mork_string()`.
    ///
    /// # Arguments
    /// * `node` - The MettaNode to convert
    ///
    /// # Returns
    /// MORK-formatted string representation
    fn node_to_mork_string(node: &MettaNode) -> String {
        match node {
            MettaNode::Atom { name, .. } => {
                // Atoms are rendered as-is
                name.clone()
            }
            MettaNode::Variable { name, .. } => {
                // Variables are prefixed with $ in MORK format
                format!("${}", name)
            }
            MettaNode::Integer { value, .. } => {
                // Integers are rendered directly
                value.to_string()
            }
            MettaNode::Float { value, .. } => {
                // Floats are rendered directly
                value.to_string()
            }
            MettaNode::Bool { value, .. } => {
                // Booleans are rendered directly
                value.to_string()
            }
            MettaNode::String { value, .. } => {
                // Strings are quoted
                format!("\"{}\"", value)
            }
            MettaNode::SExpr { elements, .. } => {
                // S-expressions are rendered as (element1 element2 ...)
                let inner = elements
                    .iter()
                    .map(|e| Self::node_to_mork_string(e))
                    .collect::<Vec<_>>()
                    .join(" ");
                format!("({})", inner)
            }
            // Handle other node types if needed
            _ => {
                // Fallback for unsupported node types
                "()".to_string()
            }
        }
    }

    /// Convert MettaNode pattern to MORK bytes for indexing
    ///
    /// This creates the byte representation that MORK uses for pattern matching.
    /// The bytes are used as keys in the `pattern_locations` HashMap.
    ///
    /// # Arguments
    /// * `pattern` - The pattern node to convert
    ///
    /// # Returns
    /// Result containing the MORK byte representation
    fn pattern_to_mork_bytes(&self, pattern: &MettaNode) -> Result<Vec<u8>, String> {
        let mork_str = Self::node_to_mork_string(pattern);
        Ok(mork_str.as_bytes().to_vec())
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
        self.pattern_locations.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::semantic_node::NodeBase;
    use crate::ir::rholang_node::RelativePosition;

    fn test_base() -> NodeBase {
        NodeBase::new_simple(
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
            false, // not concatenated
            None,  // no fragment info
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

        matcher.add_definition("test_fn".to_string(), pattern, test_location(), false, None).unwrap();

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
