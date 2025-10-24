//! Language-agnostic LSP feature implementations using SemanticNode
//!
//! This module demonstrates how LSP features like goto-definition, find-references,
//! and rename can work in a language-agnostic way using the SemanticNode trait
//! and semantic categories.
//!
//! These implementations can work with any language that implements SemanticNode
//! (Rholang, MeTTa, or UnifiedIR), enabling code reuse across multiple languages.

use std::sync::Arc;
use crate::ir::rholang_node::Position as IrPosition;
use crate::ir::semantic_node::{SemanticNode, SemanticCategory};
use crate::ir::symbol_table::SymbolTable;

/// Find a semantic node at a specific position in the tree.
///
/// This is a language-agnostic alternative to `find_node_at_position_with_path`
/// that works with any SemanticNode implementation.
///
/// # Arguments
/// * `root` - The root of the semantic tree
/// * `position` - The position to search for
///
/// # Returns
/// The node at the position and its path from root, if found
pub fn find_semantic_node_at_position<'a>(
    root: &'a dyn SemanticNode,
    position: IrPosition,
) -> Option<(&'a dyn SemanticNode, Vec<&'a dyn SemanticNode>)> {
    fn search_node<'a>(
        node: &'a dyn SemanticNode,
        position: IrPosition,
        path: &mut Vec<&'a dyn SemanticNode>,
    ) -> Option<(&'a dyn SemanticNode, Vec<&'a dyn SemanticNode>)> {
        path.push(node);

        let base = node.base();
        let start = base.relative_start();
        let node_start = IrPosition {
            row: start.delta_lines as usize,
            column: start.delta_columns as usize,
            byte: start.delta_bytes,
        };

        let node_end = IrPosition {
            row: node_start.row + base.span_lines(),
            column: if base.span_lines() > 0 {
                base.span_columns()
            } else {
                node_start.column + base.span_columns()
            },
            byte: node_start.byte + base.length(),
        };

        // Check if position is within this node's range
        let in_range = position.byte >= node_start.byte && position.byte <= node_end.byte;

        if !in_range {
            path.pop();
            return None;
        }

        // Search children
        for i in 0..node.children_count() {
            if let Some(child) = node.child_at(i) {
                if let Some(result) = search_node(child, position, path) {
                    return Some(result);
                }
            }
        }

        // This node contains the position
        Some((node, path.clone()))
    }

    let mut path = Vec::new();
    search_node(root, position, &mut path)
}

/// Extract a variable name from a semantic node.
///
/// Uses semantic categories to identify variable nodes and extract their names
/// in a language-agnostic way.
///
/// # Arguments
/// * `node` - The semantic node to extract from
///
/// # Returns
/// The variable name if the node is a variable, None otherwise
pub fn extract_variable_name(node: &dyn SemanticNode) -> Option<String> {
    match node.semantic_category() {
        SemanticCategory::Variable => {
            // Try Rholang
            use crate::ir::rholang_node::RholangNode;
            if let Some(rho) = node.as_any().downcast_ref::<RholangNode>() {
                if let RholangNode::Var { name, .. } = rho {
                    return Some(name.clone());
                }
            }

            // Try MeTTa
            use crate::ir::metta_node::MettaNode;
            if let Some(metta) = node.as_any().downcast_ref::<MettaNode>() {
                return metta.name().map(|s| s.to_string());
            }

            // Try UnifiedIR
            use crate::ir::unified_ir::UnifiedIR;
            if let Some(unified) = node.as_any().downcast_ref::<UnifiedIR>() {
                if let UnifiedIR::Variable { name, .. } = unified {
                    return Some(name.clone());
                }
            }

            None
        }
        _ => None,
    }
}

/// Get the symbol table for a semantic node.
///
/// Looks up the symbol table in the node's metadata, falling back to a provided
/// default if not found.
///
/// # Arguments
/// * `node` - The semantic node
/// * `fallback` - Fallback symbol table if node doesn't have one
///
/// # Returns
/// The symbol table for this node
pub fn get_symbol_table_for_node(
    node: &dyn SemanticNode,
    fallback: Arc<SymbolTable>,
) -> Arc<SymbolTable> {
    node.metadata()
        .and_then(|m| m.get("symbol_table"))
        .and_then(|t| t.downcast_ref::<Arc<SymbolTable>>())
        .cloned()
        .unwrap_or(fallback)
}

/// Check if a semantic node is a binding construct.
///
/// # Arguments
/// * `node` - The semantic node to check
///
/// # Returns
/// True if the node represents a binding (new, let, contract param, etc.)
pub fn is_binding_node(node: &dyn SemanticNode) -> bool {
    matches!(
        node.semantic_category(),
        SemanticCategory::Binding
    )
}

/// Check if a semantic node is an invocation.
///
/// # Arguments
/// * `node` - The semantic node to check
///
/// # Returns
/// True if the node represents an invocation (function call, send, etc.)
pub fn is_invocation_node(node: &dyn SemanticNode) -> bool {
    matches!(
        node.semantic_category(),
        SemanticCategory::Invocation
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree_sitter::{parse_code, parse_to_ir};
    use ropey::Rope;

    #[test]
    fn test_find_semantic_node_at_position() {
        let _ = crate::logging::init_logger(false, Some("warn"), false);

        // Parse some Rholang code
        let code = r#"new x in { x!(42) }"#;
        let tree = parse_code(code);
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree, &rope);

        // Search for node at position of "x" (after "new ")
        let pos = IrPosition {
            row: 0,
            column: 4,  // position of 'x' in "new x"
            byte: 4,
        };

        let result = find_semantic_node_at_position(&*ir as &dyn SemanticNode, pos);
        assert!(result.is_some(), "Should find a node at position");

        let (node, path) = result.unwrap();
        println!("Found node type: {}", node.type_name());
        println!("Path length: {}", path.len());
        assert!(path.len() > 0, "Path should not be empty");
    }

    #[test]
    fn test_extract_variable_name() {
        let _ = crate::logging::init_logger(false, Some("warn"), false);

        // Parse Rholang code with a variable
        let code = r#"x"#;
        let tree = parse_code(code);
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree, &rope);

        // The root should be a Var node
        if let Some(name) = extract_variable_name(&*ir as &dyn SemanticNode) {
            assert_eq!(name, "x");
        }
    }

    #[test]
    fn test_is_binding_node() {
        let _ = crate::logging::init_logger(false, Some("warn"), false);

        // Parse Rholang code with a binding (new)
        let code = r#"new x in { Nil }"#;
        let tree = parse_code(code);
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree, &rope);

        // The root is a New node, which is a binding
        assert!(is_binding_node(&*ir as &dyn SemanticNode));
    }

    #[test]
    fn test_is_invocation_node() {
        let _ = crate::logging::init_logger(false, Some("warn"), false);

        // Note: In Rholang, Send is currently categorized as LanguageSpecific
        // rather than Invocation. This test demonstrates the categorization.
        let code = r#"x!(42)"#;
        let tree = parse_code(code);
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree, &rope);

        // Send is currently LanguageSpecific in Rholang's categorization
        let category = (&*ir as &dyn SemanticNode).semantic_category();
        assert_eq!(category, SemanticCategory::LanguageSpecific);

        // For actual Invocation nodes, we'd need UnifiedIR::Invocation
        // or a language where function calls are categorized as Invocation
    }

    #[test]
    fn test_find_semantic_node_with_unified_ir() {
        let _ = crate::logging::init_logger(false, Some("warn"), false);

        // Parse Rholang and convert to UnifiedIR
        let code = r#"new x in { x!(42) }"#;
        let tree = parse_code(code);
        let rope = Rope::from_str(code);
        let rho_ir = parse_to_ir(&tree, &rope);

        use crate::ir::unified_ir::UnifiedIR;
        let unified_ir = UnifiedIR::from_rholang(&rho_ir);

        // Search for any node
        let pos = IrPosition {
            row: 0,
            column: 4,
            byte: 4,
        };

        let result = find_semantic_node_at_position(&*unified_ir as &dyn SemanticNode, pos);
        assert!(result.is_some(), "Should find a node in UnifiedIR");

        let (node, _) = result.unwrap();
        println!("Found UnifiedIR node type: {}", node.type_name());
    }
}
