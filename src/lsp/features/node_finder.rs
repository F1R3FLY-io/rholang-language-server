//! Generic node finding utilities for language-agnostic LSP features
//!
//! This module provides utilities to find semantic nodes at specific positions
//! in a language-agnostic way, working with `&dyn SemanticNode`.

use tower_lsp::lsp_types::Position as LspPosition;

use crate::ir::semantic_node::{Position, SemanticNode};

/// Find the semantic node at a given position
///
/// This performs a depth-first search through the semantic tree to find the
/// innermost (most specific) node that contains the given position.
///
/// # Arguments
/// * `root` - The root node of the semantic tree
/// * `position` - The position to search for (in IR coordinates)
///
/// # Returns
/// `Some(&dyn SemanticNode)` if a node is found at the position, `None` otherwise
///
/// # Algorithm
/// 1. Traverse tree depth-first
/// 2. For each node, check if position is within its span
/// 3. Return the innermost (deepest) matching node
///
/// # Example
/// ```rust,ignore
/// let node = find_node_at_position(&root, &position)?;
/// match node.semantic_category() {
///     SemanticCategory::Variable => { /* handle variable */ }
///     SemanticCategory::Invocation => { /* handle invocation */ }
///     _ => {}
/// }
/// ```
pub fn find_node_at_position<'a>(
    root: &'a dyn SemanticNode,
    position: &Position,
) -> Option<&'a dyn SemanticNode> {
    find_node_at_position_recursive(root, position, &Position { row: 0, column: 0, byte: 0 })
}

/// Recursive helper for find_node_at_position
fn find_node_at_position_recursive<'a>(
    node: &'a dyn SemanticNode,
    target: &Position,
    prev_end: &Position,
) -> Option<&'a dyn SemanticNode> {
    use tracing::trace;

    // Compute this node's absolute start and end positions
    let start = node.absolute_position(*prev_end);
    let end = node.absolute_end(start);

    trace!(
        "Checking node type={}, start=({}, {}), end=({}, {}), target=({}, {})",
        node.type_name(),
        start.row, start.column,
        end.row, end.column,
        target.row, target.column
    );

    // Check if target position is within this node's span
    if !position_in_range(target, &start, &end) {
        trace!("Position not in range, skipping");
        return None;
    }

    trace!("Position IS in range, checking {} children", node.children_count());

    // This node contains the position - check children for more specific match
    let mut child_prev_end = start;
    for i in 0..node.children_count() {
        if let Some(child) = node.child_at(i) {
            // Recursively search child
            if let Some(found) = find_node_at_position_recursive(child, target, &child_prev_end) {
                trace!("Found in child: {}", found.type_name());
                return Some(found); // Found more specific node in child
            }
            // Update prev_end for next sibling
            let child_start = child.absolute_position(child_prev_end);
            child_prev_end = child.absolute_end(child_start);
        }
    }

    trace!("No child found, returning this node: {}", node.type_name());
    // No child contains the position, so this node is the most specific
    Some(node)
}

/// Check if a position is within a range (inclusive start, exclusive end)
fn position_in_range(pos: &Position, start: &Position, end: &Position) -> bool {
    // Only use byte offset comparison if the target position has a computed byte offset
    // (i.e., byte > 0). If target has byte == 0, it means the byte offset was not computed
    // from the LSP position, so we must use line/column comparison.
    if pos.byte > 0 && start.byte > 0 && end.byte > 0 {
        // All positions have byte offsets - use them for precise comparison
        // Position must be >= start
        if pos.byte < start.byte {
            return false;
        }

        // Position must be < end
        if pos.byte >= end.byte {
            return false;
        }

        return true;
    }

    // Fall back to line/column comparison when byte offsets are unavailable
    // Position must be >= start (line-first comparison)
    if pos.row < start.row {
        return false;
    }
    if pos.row == start.row && pos.column < start.column {
        return false;
    }

    // Position must be < end (line-first comparison)
    if pos.row > end.row {
        return false;
    }
    if pos.row == end.row && pos.column >= end.column {
        return false;
    }

    true
}

/// Find the node at a position along with its parent path
///
/// This returns both the target node and the path from root to that node,
/// which is useful for understanding context.
///
/// # Arguments
/// * `root` - The root node of the semantic tree
/// * `position` - The position to search for
///
/// # Returns
/// `Some((&dyn SemanticNode, Vec<&dyn SemanticNode>))` where:
/// - First element is the target node
/// - Second element is the path from root to target (including target)
///
/// # Example
/// ```rust,ignore
/// if let Some((node, path)) = find_node_with_path(&root, &position) {
///     let parent = path.get(path.len() - 2); // Get parent node
///     // Use parent context for better symbol resolution
/// }
/// ```
pub fn find_node_with_path<'a>(
    root: &'a dyn SemanticNode,
    position: &Position,
) -> Option<(&'a dyn SemanticNode, Vec<&'a dyn SemanticNode>)> {
    let mut path = Vec::new();
    let result = find_node_with_path_recursive(
        root,
        position,
        &Position { row: 0, column: 0, byte: 0 },
        &mut path,
    );

    result.map(|node| (node, path))
}

/// Recursive helper for find_node_with_path
fn find_node_with_path_recursive<'a>(
    node: &'a dyn SemanticNode,
    target: &Position,
    prev_end: &Position,
    path: &mut Vec<&'a dyn SemanticNode>,
) -> Option<&'a dyn SemanticNode> {
    // Compute this node's absolute start and end positions
    let start = node.absolute_position(*prev_end);
    let end = node.absolute_end(start);

    // Check if target position is within this node's span
    if !position_in_range(target, &start, &end) {
        return None;
    }

    // Add this node to path
    path.push(node);

    // This node contains the position - check children for more specific match
    let mut child_prev_end = start;
    for i in 0..node.children_count() {
        if let Some(child) = node.child_at(i) {
            // Recursively search child
            if let Some(found) = find_node_with_path_recursive(child, target, &child_prev_end, path) {
                return Some(found); // Found more specific node in child
            }
            // Update prev_end for next sibling
            let child_start = child.absolute_position(child_prev_end);
            child_prev_end = child.absolute_end(child_start);
        }
    }

    // No child contains the position, so this node is the most specific
    Some(node)
}

/// Convert LSP position to IR position
///
/// LSP positions are 0-based line/character coordinates.
/// This converts them to IR Position format.
///
/// # Arguments
/// * `lsp_pos` - LSP position
///
/// # Returns
/// IR `Position` (note: byte offset will be 0, should be computed separately if needed)
pub fn lsp_to_ir_position(lsp_pos: LspPosition) -> Position {
    Position {
        row: lsp_pos.line as usize,
        column: lsp_pos.character as usize,
        byte: 0, // Byte offset requires full text scan - caller should compute if needed
    }
}

/// Convert IR position to LSP position
///
/// # Arguments
/// * `ir_pos` - IR position
///
/// # Returns
/// LSP `Position`
pub fn ir_to_lsp_position(ir_pos: &Position) -> LspPosition {
    LspPosition {
        line: ir_pos.row as u32,
        character: ir_pos.column as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::Any;
    use crate::ir::semantic_node::{NodeBase, RelativePosition, SemanticCategory, Metadata};

    // Mock node for testing
    #[derive(Debug)]
    struct MockNode {
        base: NodeBase,
        category: SemanticCategory,
        children: Vec<MockNode>,
    }

    impl MockNode {
        fn new(
            relative_start: RelativePosition,
            length: usize,
            span_lines: usize,
            span_columns: usize,
            category: SemanticCategory,
        ) -> Self {
            Self {
                base: NodeBase::new_simple(relative_start, length, span_lines, span_columns),
                category,
                children: vec![],
            }
        }

        fn with_children(mut self, children: Vec<MockNode>) -> Self {
            self.children = children;
            self
        }
    }

    impl SemanticNode for MockNode {
        fn base(&self) -> &NodeBase {
            &self.base
        }

        fn metadata(&self) -> Option<&Metadata> {
            None
        }

        fn metadata_mut(&mut self) -> Option<&mut Metadata> {
            None
        }

        fn semantic_category(&self) -> SemanticCategory {
            self.category
        }

        fn type_name(&self) -> &'static str {
            "MockNode"
        }

        fn children_count(&self) -> usize {
            self.children.len()
        }

        fn child_at(&self, index: usize) -> Option<&dyn SemanticNode> {
            self.children.get(index).map(|n| n as &dyn SemanticNode)
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    #[test]
    fn test_find_node_at_position_root() {
        // Root node: starts at (0, 0), length 10
        let root = MockNode::new(
            RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 },
            10,
            0,
            10,
            SemanticCategory::Block,
        );

        // Position within root
        let pos = Position { row: 0, column: 5, byte: 5 };
        let found = find_node_at_position(&root, &pos);

        assert!(found.is_some());
        assert_eq!(found.unwrap().semantic_category(), SemanticCategory::Block);
    }

    #[test]
    fn test_find_node_at_position_child() {
        // Root node: (0, 0), length 20
        // Child node: starts after root start + 5 bytes
        let child = MockNode::new(
            RelativePosition { delta_lines: 0, delta_columns: 5, delta_bytes: 5 },
            5,
            0,
            5,
            SemanticCategory::Variable,
        );

        let root = MockNode::new(
            RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 },
            20,
            0,
            20,
            SemanticCategory::Block,
        ).with_children(vec![child]);

        // Position within child (byte 7 = root start + 5 + 2)
        let pos = Position { row: 0, column: 7, byte: 7 };
        let found = find_node_at_position(&root, &pos);

        assert!(found.is_some());
        assert_eq!(found.unwrap().semantic_category(), SemanticCategory::Variable);
    }

    #[test]
    fn test_find_node_with_path() {
        let child = MockNode::new(
            RelativePosition { delta_lines: 0, delta_columns: 5, delta_bytes: 5 },
            5,
            0,
            5,
            SemanticCategory::Variable,
        );

        let root = MockNode::new(
            RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 },
            20,
            0,
            20,
            SemanticCategory::Block,
        ).with_children(vec![child]);

        // Position within child
        let pos = Position { row: 0, column: 7, byte: 7 };
        let result = find_node_with_path(&root, &pos);

        assert!(result.is_some());
        let (node, path) = result.unwrap();
        assert_eq!(node.semantic_category(), SemanticCategory::Variable);
        assert_eq!(path.len(), 2); // Root + child
        assert_eq!(path[0].semantic_category(), SemanticCategory::Block);
        assert_eq!(path[1].semantic_category(), SemanticCategory::Variable);
    }

    #[test]
    fn test_position_conversion() {
        let lsp_pos = LspPosition { line: 10, character: 5 };
        let ir_pos = lsp_to_ir_position(lsp_pos);

        assert_eq!(ir_pos.row, 10);
        assert_eq!(ir_pos.column, 5);

        let back_to_lsp = ir_to_lsp_position(&ir_pos);
        assert_eq!(back_to_lsp.line, 10);
        assert_eq!(back_to_lsp.character, 5);
    }
}
