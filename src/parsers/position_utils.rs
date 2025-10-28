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

use crate::ir::semantic_node::{NodeBase, Position};
use crate::ir::rholang_node::RelativePosition;

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
