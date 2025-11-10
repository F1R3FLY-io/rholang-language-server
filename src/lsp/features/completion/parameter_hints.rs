//! Parameter hints and context detection for contract calls
//!
//! This module provides parameter-aware completion by analyzing contract calls
//! and determining which parameter position the cursor is in, along with the
//! expected pattern type for that parameter.
//!
//! Features:
//! - Position-based parameter detection (1st, 2nd, 3rd argument, etc.)
//! - Pattern type analysis (Int, String, List, Map, etc.)
//! - Type-based completion filtering

use crate::ir::rholang_node::RholangNode;
use crate::ir::semantic_node::{Position, SemanticNode};
use crate::ir::symbol_table::SymbolTable;
use std::sync::Arc;

/// Expected pattern type for a parameter
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpectedPatternType {
    /// Any type accepted (@x or @_)
    Any,

    /// Expects an integer (@{x : Int})
    Int,

    /// Expects a string (@{x : String})
    String,

    /// Expects a boolean (@{x : Bool})
    Bool,

    /// Expects a byte array (@{x : ByteArray})
    ByteArray,

    /// Expects a URI (@{x : Uri})
    Uri,

    /// Expects a list (@[...])
    List,

    /// Expects a map with optional required keys
    Map { required_keys: Option<Vec<String>> },

    /// Expects a set
    Set,

    /// Expects a PathMap
    PathMap,

    /// Expects a tuple
    Tuple,

    /// Custom/unknown pattern type
    Custom(String),
}

impl ExpectedPatternType {
    /// Check if this pattern type accepts a specific type
    pub fn accepts(&self, other: &ExpectedPatternType) -> bool {
        match (self, other) {
            // Any accepts everything
            (ExpectedPatternType::Any, _) => true,
            // Exact matches
            (ExpectedPatternType::Int, ExpectedPatternType::Int) => true,
            (ExpectedPatternType::String, ExpectedPatternType::String) => true,
            (ExpectedPatternType::Bool, ExpectedPatternType::Bool) => true,
            (ExpectedPatternType::ByteArray, ExpectedPatternType::ByteArray) => true,
            (ExpectedPatternType::Uri, ExpectedPatternType::Uri) => true,
            (ExpectedPatternType::List, ExpectedPatternType::List) => true,
            (ExpectedPatternType::Set, ExpectedPatternType::Set) => true,
            (ExpectedPatternType::PathMap, ExpectedPatternType::PathMap) => true,
            (ExpectedPatternType::Tuple, ExpectedPatternType::Tuple) => true,
            // Map with key requirements
            (ExpectedPatternType::Map { required_keys: Some(req) }, ExpectedPatternType::Map { .. }) => {
                // TODO: Check if provided map has required keys
                true
            }
            (ExpectedPatternType::Map { .. }, ExpectedPatternType::Map { .. }) => true,
            // Custom types - conservative match
            (ExpectedPatternType::Custom(_), _) => true,
            (_, ExpectedPatternType::Custom(_)) => true,
            // No match
            _ => false,
        }
    }
}

/// Context information for a parameter in a contract call
#[derive(Debug, Clone)]
pub struct ParameterContext {
    /// Name of the contract being called
    pub contract_name: String,

    /// Position of the parameter (0-indexed)
    pub parameter_position: usize,

    /// Name of the parameter (if known from contract definition)
    pub parameter_name: Option<String>,

    /// Expected pattern type for this parameter
    pub expected_pattern: ExpectedPatternType,

    /// Documentation for this parameter (if available)
    pub documentation: Option<String>,
}

/// Detect parameter context at a given cursor position
///
/// This function analyzes the IR tree to determine if the cursor is inside
/// a contract call's argument list, and if so, which parameter position.
///
/// # Arguments
/// * `node` - The IR node at the cursor position
/// * `position` - Cursor position
/// * `symbol_table` - Symbol table for looking up contract definitions
///
/// # Returns
/// Parameter context if cursor is in a contract call, None otherwise
pub fn get_parameter_context(
    node: &dyn SemanticNode,
    _position: &Position,
    symbol_table: &SymbolTable,
) -> Option<ParameterContext> {
    use crate::ir::rholang_node::RholangNode;
    use crate::ir::semantic_node::SemanticNodeExt;

    // Try to downcast to RholangNode using the helper method
    let rholang_node = node.as_rholang()?;

    // Walk up the IR tree to find parent Send node
    // For now, we check if the current node itself is a Send
    // TODO: Implement proper parent traversal when parent pointers are available
    if let RholangNode::Send { channel, .. } = rholang_node {
        // Extract contract name from channel
        let contract_name = extract_contract_name(channel.as_ref())?;

        // TODO: Determine which parameter position the cursor is in
        // This requires comparing cursor position with input positions
        // For now, we assume cursor is at position 0
        let parameter_position = 0;

        // Look up contract definition in symbol table
        // Use collect_all_symbols and filter by name and type
        let all_symbols = symbol_table.collect_all_symbols();
        for symbol in all_symbols {
            if symbol.name == contract_name
                && symbol.symbol_type == crate::ir::symbol_table::SymbolType::Contract {
                // Get the contract node from metadata
                // TODO: Retrieve contract node to extract formals
                // For now, return basic context without pattern analysis
                return Some(ParameterContext {
                    contract_name: contract_name.clone(),
                    parameter_position,
                    parameter_name: None,
                    expected_pattern: ExpectedPatternType::Any,
                    documentation: symbol.documentation.clone(),
                });
            }
        }
    }

    None
}

/// Extract contract name from a channel expression
fn extract_contract_name(channel: &RholangNode) -> Option<String> {
    use crate::ir::rholang_node::RholangNode;

    match channel {
        // Direct variable reference: foo!(x)
        RholangNode::Var { name, .. } => Some(name.clone()),

        // Quote reference: @foo!(x) - extract from quoted process
        RholangNode::Quote { quotable, .. } => extract_contract_name(quotable.as_ref()),

        // Other cases: method calls, expressions, etc.
        _ => None,
    }
}

/// Extract parameter patterns from a contract definition
///
/// # Arguments
/// * `contract_node` - Contract IR node
///
/// # Returns
/// Vector of parameter names and their expected pattern types
pub fn extract_parameter_patterns(
    contract_node: &RholangNode,
) -> Vec<(String, ExpectedPatternType)> {
    use crate::ir::rholang_node::RholangNode;

    if let RholangNode::Contract { formals, .. } = contract_node {
        formals
            .iter()
            .filter_map(|formal| {
                // Each formal is a pattern that can be:
                // - A simple variable: @x
                // - A typed pattern: @{x : Int}
                // - A collection pattern: @[x, y, z]
                // etc.

                match formal.as_ref() {
                    // Simple variable pattern
                    RholangNode::Var { name, .. } => {
                        Some((name.clone(), ExpectedPatternType::Any))
                    }

                    // Quote pattern - analyze the quoted content
                    RholangNode::Quote { quotable, .. } => {
                        let pattern_type = analyze_pattern_type(quotable);
                        // Try to extract name from quoted pattern
                        if let Some(name) = extract_pattern_name(quotable) {
                            Some((name, pattern_type))
                        } else {
                            // Anonymous pattern (e.g., @_)
                            Some((format!("param_{}", formals.iter().position(|f| Arc::ptr_eq(f, formal)).unwrap_or(0)), pattern_type))
                        }
                    }

                    // Other pattern types
                    _ => {
                        let pattern_type = analyze_pattern_type(formal);
                        if let Some(name) = extract_pattern_name(formal) {
                            Some((name, pattern_type))
                        } else {
                            None
                        }
                    }
                }
            })
            .collect()
    } else {
        Vec::new()
    }
}

/// Extract variable name from a pattern node
fn extract_pattern_name(pattern: &RholangNode) -> Option<String> {
    use crate::ir::rholang_node::RholangNode;

    match pattern {
        RholangNode::Var { name, .. } => Some(name.clone()),
        RholangNode::Quote { quotable, .. } => extract_pattern_name(quotable),
        RholangNode::Wildcard { .. } => None,  // Wildcard has no name
        _ => None,
    }
}

/// Analyze a pattern node to determine its expected type
///
/// In Rholang, patterns are expressed using regular nodes (not separate pattern types).
/// This function analyzes a node used in a pattern position (e.g., contract formals)
/// to infer what type of value it expects.
///
/// # Arguments
/// * `pattern_node` - IR node used in pattern position
///
/// # Returns
/// Expected pattern type
pub fn analyze_pattern_type(pattern_node: &RholangNode) -> ExpectedPatternType {
    use crate::ir::rholang_node::RholangNode::*;

    match pattern_node {
        // Simple variable or wildcard - accepts any type
        Var { .. } | Wildcard { .. } => ExpectedPatternType::Any,

        // Literal types
        BoolLiteral { .. } => ExpectedPatternType::Bool,
        LongLiteral { .. } => ExpectedPatternType::Int,
        StringLiteral { .. } => ExpectedPatternType::String,
        UriLiteral { .. } => ExpectedPatternType::Uri,

        // Collection types
        List { .. } => ExpectedPatternType::List,
        Set { .. } => ExpectedPatternType::Set,
        Map { .. } => ExpectedPatternType::Map { required_keys: None },
        Pathmap { .. } => ExpectedPatternType::PathMap,
        Tuple { .. } => ExpectedPatternType::Tuple,

        // Other nodes default to Any (including Send, Par, etc.)
        _ => ExpectedPatternType::Any,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expected_pattern_type_accepts() {
        assert!(ExpectedPatternType::Any.accepts(&ExpectedPatternType::Int));
        assert!(ExpectedPatternType::Int.accepts(&ExpectedPatternType::Int));
        assert!(!ExpectedPatternType::Int.accepts(&ExpectedPatternType::String));
        assert!(ExpectedPatternType::Map { required_keys: None }
            .accepts(&ExpectedPatternType::Map { required_keys: None }));
    }

    #[test]
    fn test_parameter_context_creation() {
        let ctx = ParameterContext {
            contract_name: "foo".to_string(),
            parameter_position: 0,
            parameter_name: Some("x".to_string()),
            expected_pattern: ExpectedPatternType::Int,
            documentation: Some("First parameter".to_string()),
        };

        assert_eq!(ctx.contract_name, "foo");
        assert_eq!(ctx.parameter_position, 0);
        assert_eq!(ctx.expected_pattern, ExpectedPatternType::Int);
    }
}
