//! Common utilities for position tracking across all language parsers
//!
//! This module provides standardized functions for converting absolute positions
//! from parsers (like Tree-Sitter or MeTTaTron) into relative positions stored
//! in the IR's NodeBase structures.
//!
//! # Position Tracking System
//!
//! The IR uses a delta-encoding system where each node stores:
//! - **relative_start**: Offset from previous sibling's end (delta encoding)
//! - **content_length**: Semantic extent up to last child
//! - **syntactic_length**: Full extent including closing delimiters
//!
//! This enables:
//! - Efficient memory usage through structural sharing
//! - On-demand position reconstruction
//! - Accurate position tracking for LSP operations
//!
//! # Usage
//!
//! All language parsers should use these functions when converting from their
//! native AST to our IR:
//!
//! ```rust,ignore
//! use crate::parsers::position_utils::{create_node_base_from_absolute, create_simple_node_base};
//!
//! // For nodes with closing delimiters (blocks, lists, etc.)
//! let base = create_node_base_from_absolute(
//!     absolute_start,
//!     absolute_end,
//!     &mut prev_end,
//!     Some(content_end),  // Where last child ends
//! );
//!
//! // For simple nodes without closing delimiters (variables, literals, etc.)
//! let base = create_simple_node_base(
//!     absolute_start,
//!     absolute_end,
//!     &mut prev_end,
//! );
//! ```

use crate::ir::semantic_node::{NodeBase, Position, SemanticNode};
use crate::ir::rholang_node::{RelativePosition, RholangNode};
use std::sync::Arc;

/// Convert absolute positions to NodeBase with relative positioning and dual-length tracking.
///
/// This is the standard way all parsers should create NodeBase instances to ensure
/// consistent position tracking across languages.
///
/// # Parameters
///
/// - `absolute_start`: Absolute position where the node starts (from parser)
/// - `absolute_end`: Absolute position where the node ends, including closing delimiters (from parser)
/// - `prev_end`: Mutable reference to the end position of the previous sibling.
///   This will be updated to `absolute_end` for the next sibling's delta calculation.
/// - `content_end`: Optional position where semantic content ends (before closing delimiters).
///   If `None`, uses `absolute_end` (appropriate for nodes without closing delimiters).
///
/// # Dual-Length System
///
/// The dual-length system distinguishes between:
/// - **content_length**: Semantic extent from start to end of last child (for operations)
/// - **syntactic_length**: Full extent including closing delimiters (for reconstruction)
///
/// Example for `{ x = 5 }`:
/// - `absolute_start`: position of `{`
/// - `content_end`: position after `5` (last semantic element)
/// - `absolute_end`: position after `}` (includes closing delimiter)
/// - `content_length`: distance from `{` to after `5`
/// - `syntactic_length`: distance from `{` to after `}`
///
/// # Position Reconstruction
///
/// The stored relative positions enable reconstruction of absolute positions:
/// ```text
/// reconstructed_start = prev_end + delta_bytes
/// reconstructed_end = reconstructed_start + syntactic_length
/// ```
///
/// # Returns
///
/// A `NodeBase` with:
/// - Relative position deltas (lines, columns, bytes)
/// - Content length (semantic extent)
/// - Syntactic length (full extent)
/// - Span metrics (lines and columns)
///
/// # Side Effects
///
/// Updates `*prev_end` to `absolute_end` so the next sibling can compute its delta.
///
/// # Example
///
/// ```rust,ignore
/// let mut prev_end = Position { row: 0, column: 0, byte: 0 };
///
/// // Parse: "{ x = 5 }"
/// // Tree-Sitter reports:
/// // - block_start: byte 0
/// // - last_child_end: byte 7 (after "5")
/// // - block_end: byte 9 (after "}")
///
/// let base = create_node_base_from_absolute(
///     Position { row: 0, column: 0, byte: 0 },   // absolute_start (at "{")
///     Position { row: 0, column: 9, byte: 9 },   // absolute_end (after "}")
///     &mut prev_end,
///     Some(Position { row: 0, column: 7, byte: 7 }),  // content_end (after "5")
/// );
///
/// // Result:
/// // - relative_start.delta_bytes = 0 (first node)
/// // - content_length = 7 (semantic extent)
/// // - syntactic_length = 9 (includes closing "}")
/// // - prev_end now = Position { row: 0, column: 9, byte: 9 }
/// ```
pub fn create_node_base_from_absolute(
    absolute_start: Position,
    absolute_end: Position,
    prev_end: &mut Position,
    content_end: Option<Position>,
) -> NodeBase {
    let content_end = content_end.unwrap_or(absolute_end);

    // Compute relative deltas from previous sibling's end
    let delta_bytes = absolute_start.byte.saturating_sub(prev_end.byte);
    let delta_lines = (absolute_start.row as i32) - (prev_end.row as i32);
    let delta_columns = if delta_lines == 0 {
        // Same line: delta is column difference
        (absolute_start.column as i32) - (prev_end.column as i32)
    } else {
        // Different line: delta is absolute column (line-relative)
        absolute_start.column as i32
    };

    // Compute dual lengths
    let content_length = content_end.byte.saturating_sub(absolute_start.byte);
    let syntactic_length = absolute_end.byte.saturating_sub(absolute_start.byte);

    // Compute span metrics (for display purposes)
    let span_lines = absolute_end.row.saturating_sub(absolute_start.row);
    let span_columns = if span_lines > 0 {
        // Multi-line: span columns is the final column
        absolute_end.column
    } else {
        // Single line: span is column difference
        absolute_end.column.saturating_sub(absolute_start.column)
    };

    // Update prev_end for next sibling's delta calculation
    *prev_end = absolute_end;

    NodeBase::new(
        RelativePosition {
            delta_lines,
            delta_columns,
            delta_bytes,
        },
        content_length,
        syntactic_length,
        span_lines,
        span_columns,
    )
}

/// Convenience wrapper for nodes without closing delimiters.
///
/// This is equivalent to calling `create_node_base_from_absolute` with `content_end = None`,
/// which sets `content_length = syntactic_length`.
///
/// Use this for simple nodes like:
/// - Variables: `x`, `myVar`
/// - Literals: `42`, `"hello"`, `true`
/// - Operators without container syntax: `+`, `-`, `*`
///
/// # Parameters
///
/// - `absolute_start`: Absolute position where the node starts
/// - `absolute_end`: Absolute position where the node ends
/// - `prev_end`: Mutable reference to previous sibling's end position (will be updated)
///
/// # Example
///
/// ```rust,ignore
/// let mut prev_end = Position { row: 0, column: 0, byte: 0 };
///
/// // Parse: "x" (variable, no closing delimiter)
/// let base = create_simple_node_base(
///     Position { row: 0, column: 0, byte: 0 },  // start at "x"
///     Position { row: 0, column: 1, byte: 1 },  // end after "x"
///     &mut prev_end,
/// );
///
/// // Result:
/// // - content_length = 1
/// // - syntactic_length = 1 (same as content_length)
/// ```
pub fn create_simple_node_base(
    absolute_start: Position,
    absolute_end: Position,
    prev_end: &mut Position,
) -> NodeBase {
    create_node_base_from_absolute(absolute_start, absolute_end, prev_end, None)
}

/// Recalculate children nodes' relative positions for wrapping in a new container.
///
/// This is used when wrapping nodes in a container (like Par) that starts at a different
/// position than the children's original reference. The function takes nodes that were
/// created with one `prev_end` reference and recalculates their positions for a new
/// reference point.
///
/// # Invariant
///
/// A node's children must have relative positions computed from the node's own start
/// position, NOT from any earlier reference point. This function enforces that invariant.
///
/// # Parameters
///
/// * `children` - Vector of child nodes to recalculate
/// * `original_prev_end` - The prev_end used when children were originally created
/// * `new_prev_end` - The new reference position (typically the container's start)
///
/// # Returns
///
/// A new vector of children with recalculated relative positions.
///
/// # Example
///
/// ```rust,ignore
/// // Children created with prev_end = byte 9 (Block's "{")
/// let children = collect_named_descendants(block_node, rope, Position { byte: 9, ... });
///
/// // But Par starts at byte 15 (first child's position)
/// let par_start = Position { byte: 15, ... };
///
/// // Recalculate children's positions to be relative to par_start
/// let adjusted_children = recalculate_children_positions(
///     children,
///     Position { byte: 9, ... },   // original reference
///     par_start,                    // new reference
/// );
/// // First child now has delta_bytes = 0 (not 6)
/// ```
pub fn recalculate_children_positions(
    children: &rpds::Vector<Arc<RholangNode>, archery::ArcK>,
    original_prev_end: Position,
    new_prev_end: Position,
) -> rpds::Vector<Arc<RholangNode>, archery::ArcK> {
    use rpds::Vector;
    use archery::ArcK;

    eprintln!("DEBUG: recalculate_children_positions called with {} children", children.len());
    eprintln!("DEBUG:   original_prev_end: byte {}", original_prev_end.byte);
    eprintln!("DEBUG:   new_prev_end: byte {}", new_prev_end.byte);

    let mut result = Vector::<Arc<RholangNode>, ArcK>::new_with_ptr_kind();
    let mut current_original_prev = original_prev_end;
    let mut current_new_prev = new_prev_end;

    for (i, child) in children.iter().enumerate() {
        // Compute child's absolute positions using original reference
        let child_abs_start = SemanticNode::absolute_position(child.as_ref(), current_original_prev);
        let child_abs_end = SemanticNode::absolute_end(child.as_ref(), child_abs_start);

        eprintln!("DEBUG:   Child {}: old_delta={}, abs_start={}, abs_end={}",
                  i, child.base().delta_bytes(), child_abs_start.byte, child_abs_end.byte);

        // Compute content end
        let content_end = Position {
            row: child_abs_start.row + child.base().span_lines(),
            column: if child.base().span_lines() > 0 {
                child.base().span_columns()
            } else {
                child_abs_start.column + child.base().span_columns()
            },
            byte: child_abs_start.byte + child.base().content_length(),
        };

        // Create new NodeBase with position relative to new reference
        let new_base = create_node_base_from_absolute(
            child_abs_start,
            child_abs_end,
            &mut current_new_prev,
            Some(content_end),
        );

        eprintln!("DEBUG:   Child {}: new_delta={}, current_new_prev={}",
                  i, new_base.delta_bytes(), current_new_prev.byte);

        // Clone child with new base (simplified - just update base, keep rest)
        let new_child = clone_node_with_new_base(&child, new_base);
        result = result.push_back(new_child);

        // Update references for next iteration
        current_original_prev = child_abs_end;
    }

    result
}

/// Helper to clone a node with a new NodeBase.
/// This is a simplified version that works for the Par children use case.
pub fn clone_node_with_new_base(node: &Arc<RholangNode>, new_base: NodeBase) -> Arc<RholangNode> {
    use crate::ir::rholang_node::node_types::*;

    match &**node {
        RholangNode::Par { left, right, processes, metadata, .. } => {
            Arc::new(RholangNode::Par {
                base: new_base,
                left: left.clone(),
                right: right.clone(),
                processes: processes.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::Send { channel, send_type, send_type_delta, inputs, metadata, .. } => {
            Arc::new(RholangNode::Send {
                base: new_base,
                channel: channel.clone(),
                send_type: send_type.clone(),
                send_type_delta: send_type_delta.clone(),
                inputs: inputs.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::SendSync { channel, inputs, cont, metadata, .. } => {
            Arc::new(RholangNode::SendSync {
                base: new_base,
                channel: channel.clone(),
                inputs: inputs.clone(),
                cont: cont.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::New { decls, proc, metadata, .. } => {
            Arc::new(RholangNode::New {
                base: new_base,
                decls: decls.clone(),
                proc: proc.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::Block { proc, metadata, .. } => {
            Arc::new(RholangNode::Block {
                base: new_base,
                proc: proc.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::Var { name, metadata, .. } => {
            Arc::new(RholangNode::Var {
                base: new_base,
                name: name.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::Nil { metadata, .. } => {
            Arc::new(RholangNode::Nil {
                base: new_base,
                metadata: metadata.clone(),
            })
        }
        RholangNode::IfElse { condition, consequence, alternative, metadata, .. } => {
            Arc::new(RholangNode::IfElse {
                base: new_base,
                condition: condition.clone(),
                consequence: consequence.clone(),
                alternative: alternative.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::Let { decls, proc, metadata, .. } => {
            Arc::new(RholangNode::Let {
                base: new_base,
                decls: decls.clone(),
                proc: proc.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::Bundle { bundle_type, proc, metadata, .. } => {
            Arc::new(RholangNode::Bundle {
                base: new_base,
                bundle_type: bundle_type.clone(),
                proc: proc.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::Match { expression, cases, metadata, .. } => {
            Arc::new(RholangNode::Match {
                base: new_base,
                expression: expression.clone(),
                cases: cases.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::Choice { branches, metadata, .. } => {
            Arc::new(RholangNode::Choice {
                base: new_base,
                branches: branches.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::Contract { name, formals, formals_remainder, proc, metadata, .. } => {
            Arc::new(RholangNode::Contract {
                base: new_base,
                name: name.clone(),
                formals: formals.clone(),
                formals_remainder: formals_remainder.clone(),
                proc: proc.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::Input { receipts, proc, metadata, .. } => {
            Arc::new(RholangNode::Input {
                base: new_base,
                receipts: receipts.clone(),
                proc: proc.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::Parenthesized { expr, metadata, .. } => {
            Arc::new(RholangNode::Parenthesized {
                base: new_base,
                expr: expr.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::BinOp { op, left, right, metadata, .. } => {
            Arc::new(RholangNode::BinOp {
                base: new_base,
                op: op.clone(),
                left: left.clone(),
                right: right.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::UnaryOp { op, operand, metadata, .. } => {
            Arc::new(RholangNode::UnaryOp {
                base: new_base,
                op: op.clone(),
                operand: operand.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::Method { receiver, name, args, metadata, .. } => {
            Arc::new(RholangNode::Method {
                base: new_base,
                receiver: receiver.clone(),
                name: name.clone(),
                args: args.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::Eval { name, metadata, .. } => {
            Arc::new(RholangNode::Eval {
                base: new_base,
                name: name.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::Quote { quotable, metadata, .. } => {
            Arc::new(RholangNode::Quote {
                base: new_base,
                quotable: quotable.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::VarRef { kind, var, metadata, .. } => {
            Arc::new(RholangNode::VarRef {
                base: new_base,
                kind: kind.clone(),
                var: var.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::BoolLiteral { value, metadata, .. } => {
            Arc::new(RholangNode::BoolLiteral {
                base: new_base,
                value: *value,
                metadata: metadata.clone(),
            })
        }
        RholangNode::LongLiteral { value, metadata, .. } => {
            Arc::new(RholangNode::LongLiteral {
                base: new_base,
                value: *value,
                metadata: metadata.clone(),
            })
        }
        RholangNode::StringLiteral { value, metadata, .. } => {
            Arc::new(RholangNode::StringLiteral {
                base: new_base,
                value: value.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::UriLiteral { value, metadata, .. } => {
            Arc::new(RholangNode::UriLiteral {
                base: new_base,
                value: value.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::List { elements, remainder, metadata, .. } => {
            Arc::new(RholangNode::List {
                base: new_base,
                elements: elements.clone(),
                remainder: remainder.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::Set { elements, remainder, metadata, .. } => {
            Arc::new(RholangNode::Set {
                base: new_base,
                elements: elements.clone(),
                remainder: remainder.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::Map { pairs, remainder, metadata, .. } => {
            Arc::new(RholangNode::Map {
                base: new_base,
                pairs: pairs.clone(),
                remainder: remainder.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::Pathmap { elements, remainder, metadata, .. } => {
            Arc::new(RholangNode::Pathmap {
                base: new_base,
                elements: elements.clone(),
                remainder: remainder.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::Tuple { elements, metadata, .. } => {
            Arc::new(RholangNode::Tuple {
                base: new_base,
                elements: elements.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::NameDecl { var, uri, metadata, .. } => {
            Arc::new(RholangNode::NameDecl {
                base: new_base,
                var: var.clone(),
                uri: uri.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::Decl { names, names_remainder, procs, metadata, .. } => {
            Arc::new(RholangNode::Decl {
                base: new_base,
                names: names.clone(),
                names_remainder: names_remainder.clone(),
                procs: procs.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::LinearBind { names, remainder, source, metadata, .. } => {
            Arc::new(RholangNode::LinearBind {
                base: new_base,
                names: names.clone(),
                remainder: remainder.clone(),
                source: source.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::RepeatedBind { names, remainder, source, metadata, .. } => {
            Arc::new(RholangNode::RepeatedBind {
                base: new_base,
                names: names.clone(),
                remainder: remainder.clone(),
                source: source.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::PeekBind { names, remainder, source, metadata, .. } => {
            Arc::new(RholangNode::PeekBind {
                base: new_base,
                names: names.clone(),
                remainder: remainder.clone(),
                source: source.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::Comment { kind, metadata, .. } => {
            Arc::new(RholangNode::Comment {
                base: new_base,
                kind: kind.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::Wildcard { metadata, .. } => {
            Arc::new(RholangNode::Wildcard {
                base: new_base,
                metadata: metadata.clone(),
            })
        }
        RholangNode::SimpleType { value, metadata, .. } => {
            Arc::new(RholangNode::SimpleType {
                base: new_base,
                value: value.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::ReceiveSendSource { name, metadata, .. } => {
            Arc::new(RholangNode::ReceiveSendSource {
                base: new_base,
                name: name.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::SendReceiveSource { name, inputs, metadata, .. } => {
            Arc::new(RholangNode::SendReceiveSource {
                base: new_base,
                name: name.clone(),
                inputs: inputs.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::Error { children, metadata, .. } => {
            Arc::new(RholangNode::Error {
                base: new_base,
                children: children.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::Disjunction { left, right, metadata, .. } => {
            Arc::new(RholangNode::Disjunction {
                base: new_base,
                left: left.clone(),
                right: right.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::Conjunction { left, right, metadata, .. } => {
            Arc::new(RholangNode::Conjunction {
                base: new_base,
                left: left.clone(),
                right: right.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::Negation { operand, metadata, .. } => {
            Arc::new(RholangNode::Negation {
                base: new_base,
                operand: operand.clone(),
                metadata: metadata.clone(),
            })
        }
        RholangNode::Unit { metadata, .. } => {
            Arc::new(RholangNode::Unit {
                base: new_base,
                metadata: metadata.clone(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_first_node_has_zero_delta() {
        let mut prev_end = Position {
            row: 0,
            column: 0,
            byte: 0,
        };

        let base = create_simple_node_base(
            Position {
                row: 0,
                column: 0,
                byte: 0,
            },
            Position {
                row: 0,
                column: 3,
                byte: 3,
            },
            &mut prev_end,
        );

        assert_eq!(base.delta_bytes(), 0);
        assert_eq!(base.delta_lines(), 0);
        assert_eq!(base.delta_columns(), 0);
        assert_eq!(base.content_length(), 3);
        assert_eq!(base.syntactic_length(), 3);
    }

    #[test]
    fn test_sibling_computes_delta_from_previous() {
        let mut prev_end = Position {
            row: 0,
            column: 0,
            byte: 0,
        };

        // First node: "foo" (bytes 0-3)
        create_simple_node_base(
            Position {
                row: 0,
                column: 0,
                byte: 0,
            },
            Position {
                row: 0,
                column: 3,
                byte: 3,
            },
            &mut prev_end,
        );

        // Second node: "bar" (bytes 4-7, one space after "foo")
        let base = create_simple_node_base(
            Position {
                row: 0,
                column: 4,
                byte: 4,
            },
            Position {
                row: 0,
                column: 7,
                byte: 7,
            },
            &mut prev_end,
        );

        assert_eq!(base.delta_bytes(), 1); // One space between nodes
        assert_eq!(base.delta_lines(), 0); // Same line
        assert_eq!(base.delta_columns(), 1); // One column delta
        assert_eq!(base.content_length(), 3);
        assert_eq!(base.syntactic_length(), 3);
    }

    #[test]
    fn test_dual_length_for_block_with_delimiter() {
        let mut prev_end = Position {
            row: 0,
            column: 0,
            byte: 0,
        };

        // Block: "{ x }" where x ends at byte 3, closing "}" at byte 4
        let base = create_node_base_from_absolute(
            Position {
                row: 0,
                column: 0,
                byte: 0,
            }, // Start at "{"
            Position {
                row: 0,
                column: 5,
                byte: 5,
            }, // End after "}"
            &mut prev_end,
            Some(Position {
                row: 0,
                column: 3,
                byte: 3,
            }), // Content ends after "x"
        );

        assert_eq!(base.content_length(), 3); // Up to "x"
        assert_eq!(base.syntactic_length(), 5); // Includes closing "}"
        assert_eq!(prev_end.byte, 5); // Next sibling starts after "}"
    }

    #[test]
    fn test_multiline_span_calculation() {
        let mut prev_end = Position {
            row: 0,
            column: 0,
            byte: 0,
        };

        // Node spanning multiple lines
        let base = create_simple_node_base(
            Position {
                row: 1,
                column: 2,
                byte: 10,
            },
            Position {
                row: 3,
                column: 5,
                byte: 50,
            },
            &mut prev_end,
        );

        assert_eq!(base.span_lines(), 2); // row 3 - row 1
        assert_eq!(base.span_columns(), 5); // Final column on multi-line node
        assert_eq!(base.syntactic_length(), 40); // 50 - 10 bytes
    }

    #[test]
    fn test_prev_end_updated_after_creation() {
        let mut prev_end = Position {
            row: 0,
            column: 0,
            byte: 0,
        };

        let end_pos = Position {
            row: 0,
            column: 10,
            byte: 10,
        };

        create_simple_node_base(
            Position {
                row: 0,
                column: 0,
                byte: 0,
            },
            end_pos,
            &mut prev_end,
        );

        // prev_end should be updated to the node's end (end_pos)
        assert_eq!(prev_end.row, 0);    // end_pos.row
        assert_eq!(prev_end.column, 10); // end_pos.column
        assert_eq!(prev_end.byte, 10);   // end_pos.byte
    }

    #[test]
    fn test_saturating_sub_prevents_underflow() {
        // Test that if somehow absolute_start < prev_end, we don't panic
        let mut prev_end = Position {
            row: 0,
            column: 10,
            byte: 10,
        };

        let base = create_simple_node_base(
            Position {
                row: 0,
                column: 5,
                byte: 5,
            }, // Start before prev_end (unusual but shouldn't panic)
            Position {
                row: 0,
                column: 8,
                byte: 8,
            },
            &mut prev_end,
        );

        // Should not panic - saturating_sub prevents byte underflow
        // Bytes use saturating_sub (usize), so clamps to 0
        assert_eq!(base.delta_bytes(), 0);
        // Lines/columns use i32, so can be negative (representing backwards movement)
        assert_eq!(base.delta_columns(), -5); // 5 - 10 = -5
    }
}
