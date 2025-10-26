//! Helper functions for Rholang Tree-Sitter parsing
//!
//! This module provides utility functions for collecting and processing Tree-Sitter nodes
//! during the conversion to IR.

use std::sync::Arc;
use tree_sitter::Node as TSNode;
use tracing::{trace, warn};
use rpds::Vector;
use archery::ArcK;
use ropey::Rope;

use crate::ir::rholang_node::{
    RholangNode, NodeBase, Position, RelativePosition,
};
use super::conversion::convert_ts_node_to_ir;

/// Safely slice a rope by byte range, returning empty string on invalid range
///
/// # Arguments
/// * `rope` - The source code rope
/// * `start` - Starting byte offset
/// * `end` - Ending byte offset
///
/// # Returns
/// The sliced string, or empty string if the range is invalid
pub(crate) fn safe_byte_slice(rope: &Rope, start: usize, end: usize) -> String {
    if end > rope.len_bytes() || start > end {
        warn!(
            "Invalid byte range {}-{} (rope len={})",
            start,
            end,
            rope.len_bytes()
        );
        return String::new();
    }
    rope.byte_slice(start..end).to_string()
}

/// Collect named descendant nodes, updating prev_end sequentially
///
/// This function iterates through all named children of a node, converts them to IR,
/// and tracks position information for delta encoding.
///
/// # Arguments
/// * `node` - The Tree-Sitter parent node
/// * `rope` - The source code rope
/// * `prev_end` - The position where the previous node ended
///
/// # Returns
/// A tuple of (collected IR nodes, final position after last node)
pub(crate) fn collect_named_descendants(
    node: TSNode,
    rope: &Rope,
    prev_end: Position,
) -> (Vector<Arc<RholangNode>, ArcK>, Position) {
    let mut nodes: Vector<Arc<RholangNode>, ArcK> =
        Vector::<Arc<RholangNode>, ArcK>::new_with_ptr_kind();
    let mut current_prev_end = prev_end;
    let mut cursor = node.walk();

    for child in node.named_children(&mut cursor) {
        // Skip comments - they don't belong in the IR
        if is_comment(child.kind_id()) {
            continue;
        }
        let (child_node, child_end) = convert_ts_node_to_ir(child, rope, current_prev_end);
        nodes = nodes.push_back(child_node);
        current_prev_end = child_end;
    }

    (nodes, current_prev_end)
}

/// Collect patterns from a names node, separating elements and optional remainder
///
/// This handles the complex pattern syntax in Rholang including:
/// - Simple patterns: `x, y, z`
/// - Quoted patterns: `x, @y, z`
/// - Remainder patterns: `x, ...rest`
///
/// # Arguments
/// * `node` - The Tree-Sitter names node
/// * `rope` - The source code rope
/// * `prev_end` - The position where the previous node ended
///
/// # Returns
/// A tuple of (pattern elements, optional remainder, final position)
pub(crate) fn collect_patterns(
    node: TSNode,
    rope: &Rope,
    prev_end: Position,
) -> (
    Vector<Arc<RholangNode>, ArcK>,
    Option<Arc<RholangNode>>,
    Position,
) {
    let mut elements: Vector<Arc<RholangNode>, ArcK> =
        Vector::<Arc<RholangNode>, ArcK>::new_with_ptr_kind();
    let mut remainder: Option<Arc<RholangNode>> = None;
    let mut current_prev_end = prev_end;
    let mut cursor = node.walk();
    let mut is_remainder = false;
    let mut is_quote = false;
    let mut quote_delta: Option<RelativePosition> = None;
    let mut quote_start_byte: Option<usize> = None;

    for child in node.children(&mut cursor) {
        let child_kind = child.kind();
        trace!(
            "Pattern child: '{}' at start={:?}, end={:?}",
            child_kind,
            child.start_position(),
            child.end_position()
        );

        if child_kind == "," {
            continue;
        } else if child_kind == "..." {
            is_remainder = true;
            continue;
        } else if child_kind == "@" {
            is_quote = true;
            let absolute_start = Position {
                row: child.start_position().row,
                column: child.start_position().column,
                byte: child.start_byte(),
            };
            let delta_lines = absolute_start.row as i32 - current_prev_end.row as i32;
            let delta_columns = if delta_lines == 0 {
                absolute_start.column as i32 - current_prev_end.column as i32
            } else {
                absolute_start.column as i32
            };
            let delta_bytes = absolute_start.byte - current_prev_end.byte;
            quote_delta = Some(RelativePosition {
                delta_lines,
                delta_columns,
                delta_bytes,
            });
            quote_start_byte = Some(absolute_start.byte);
            current_prev_end = Position {
                row: child.end_position().row,
                column: child.end_position().column,
                byte: child.end_byte(),
            };
            continue;
        } else if child.is_named() {
            let (mut child_node, child_end) = convert_ts_node_to_ir(child, rope, current_prev_end);

            // Skip empty variable names
            if let RholangNode::Var { ref name, .. } = *child_node {
                if name.is_empty() {
                    trace!("Skipped empty variable name at {:?}", current_prev_end);
                    continue;
                }
            }

            // Wrap in Quote if @-prefixed
            if is_quote {
                let q_delta = quote_delta.take().expect("Quote delta not set");
                let q_start_byte = quote_start_byte.take().expect("Quote start byte not set");
                let length = child.end_byte() - q_start_byte;
                let span_lines = child.end_position().row - child.start_position().row;
                let span_columns = if span_lines == 0 {
                    child.end_position().column - child.start_position().column + 1 // +1 for '@'
                } else {
                    child.end_position().column
                };
                let quote_base = NodeBase::new(q_delta, length, span_lines, span_columns);
                child_node = Arc::new(RholangNode::Quote {
                    base: quote_base,
                    quotable: child_node,
                    metadata: None,
                });
                is_quote = false;
            }

            // Add to remainder or elements
            if is_remainder {
                remainder = Some(child_node);
                is_remainder = false;
            } else {
                elements = elements.push_back(child_node);
            }
            current_prev_end = child_end;
        } else {
            warn!("Unhandled child in patterns: {}", child_kind);
            current_prev_end = Position {
                row: child.end_position().row,
                column: child.end_position().column,
                byte: child.end_byte(),
            };
        }
    }

    (elements, remainder, current_prev_end)
}

/// Collect linear binds for Choice nodes, maintaining position continuity
///
/// # Arguments
/// * `branch_node` - The Tree-Sitter branch node
/// * `rope` - The source code rope
/// * `prev_end` - The position where the previous node ended
///
/// # Returns
/// A tuple of (linear bind nodes, final position)
pub(crate) fn collect_linear_binds(
    branch_node: TSNode,
    rope: &Rope,
    prev_end: Position,
) -> (Vector<Arc<RholangNode>, ArcK>, Position) {
    let mut linear_binds: Vector<Arc<RholangNode>, ArcK> =
        Vector::<Arc<RholangNode>, ArcK>::new_with_ptr_kind();
    let mut current_prev_end = prev_end;
    let mut cursor = branch_node.walk();

    for child in branch_node.children(&mut cursor) {
        if child.kind() == "linear_bind" {
            let (bind_node, bind_end) = convert_ts_node_to_ir(child, rope, current_prev_end);
            linear_binds = linear_binds.push_back(bind_node);
            current_prev_end = bind_end;
        } else if child.kind() == "=>" {
            break;
        }
    }

    (linear_binds, current_prev_end)
}

/// Optimized comment detection using kind_id for O(1) comparison
///
/// Uses OnceLock to cache the kind IDs for line and block comments,
/// making subsequent checks very fast.
///
/// # Arguments
/// * `kind_id` - The Tree-Sitter kind ID to check
///
/// # Returns
/// true if the kind ID represents a comment node
#[inline(always)]
pub(crate) fn is_comment(kind_id: u16) -> bool {
    // Get the kind IDs for comment nodes
    // These are compile-time constants after the first call
    static LINE_COMMENT_KIND: std::sync::OnceLock<u16> = std::sync::OnceLock::new();
    static BLOCK_COMMENT_KIND: std::sync::OnceLock<u16> = std::sync::OnceLock::new();

    let language: tree_sitter::Language = rholang_tree_sitter::LANGUAGE.into();
    let line_comment_kind = *LINE_COMMENT_KIND.get_or_init(|| {
        language.id_for_node_kind("line_comment", true)
    });
    let block_comment_kind = *BLOCK_COMMENT_KIND.get_or_init(|| {
        language.id_for_node_kind("block_comment", true)
    });

    kind_id == line_comment_kind || kind_id == block_comment_kind
}
