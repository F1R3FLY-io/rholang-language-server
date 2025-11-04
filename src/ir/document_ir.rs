//! Document IR with separate comment channel
//!
//! This module provides `DocumentIR`, a container that separates semantic nodes
//! from comments. This architecture enables:
//!
//! - Clean visitor traversal (visitors don't see comments by default)
//! - Efficient directive parsing (DirectiveParser uses comment channel)
//! - Documentation attachment (attach comments as metadata to following nodes)
//! - Position tracking integrity (comments don't interrupt sequential computation)
//!
//! # Architecture
//!
//! ```text
//! DocumentIR
//! ├── root: Arc<RholangNode>  // Semantic tree (no comments)
//! └── comments: Vec<CommentNode>  // Sorted by position
//! ```
//!
//! Comments are collected during parsing and stored separately, allowing them to be:
//! - Queried by position for directive parsing
//! - Attached as metadata to semantic nodes for documentation
//! - Ignored during semantic analysis and symbol resolution

use crate::ir::comment::CommentNode;
use crate::ir::rholang_node::node_types::RholangNode;
use crate::ir::semantic_node::Position;
use std::sync::Arc;

/// Document IR with separate comment channel
///
/// Separates semantic nodes (executable code) from comments (documentation/directives),
/// enabling clean visitor traversal and efficient comment processing.
///
/// # Examples
///
/// ```rust,ignore
/// // Parse source to DocumentIR
/// let document_ir = parse_to_ir(source, &tree, &rope);
///
/// // Access semantic tree (no comments)
/// let semantic_root = &document_ir.root;
///
/// // Access comments
/// for comment in &document_ir.comments {
///     if let Some(lang) = comment.parse_directive() {
///         println!("Found language directive: {}", lang);
///     }
/// }
///
/// // Find comment at specific position
/// let pos = Position { row: 10, column: 5, byte: 150 };
/// if let Some(comment) = document_ir.comment_at_position(&pos) {
///     println!("Comment: {}", comment.text);
/// }
/// ```
#[derive(Debug, Clone)]
pub struct DocumentIR {
    /// Primary semantic tree (without comments)
    ///
    /// This is the IR tree used by all semantic analysis, symbol resolution,
    /// and LSP features. Comments are not included to keep it clean.
    pub root: Arc<RholangNode>,

    /// Comment channel (sorted by position)
    ///
    /// All comments from the source file, sorted by their absolute byte position.
    /// This enables efficient querying and directive parsing.
    pub comments: Vec<CommentNode>,
}

impl DocumentIR {
    /// Create a new DocumentIR with semantic tree and comments
    ///
    /// # Arguments
    /// * `root` - The semantic IR tree (without comments)
    /// * `comments` - Vector of comments, sorted by position
    ///
    /// # Note
    /// Comments should be sorted by byte position for efficient queries.
    /// The `parse_to_ir` function ensures this invariant.
    pub fn new(root: Arc<RholangNode>, comments: Vec<CommentNode>) -> Self {
        Self { root, comments }
    }

    /// Get comment at a specific position
    ///
    /// Performs binary search on sorted comments to find the comment
    /// that contains the given position.
    ///
    /// # Arguments
    /// * `pos` - The position to search for
    ///
    /// # Returns
    /// `Some(&CommentNode)` if a comment contains the position, `None` otherwise
    ///
    /// # Complexity
    /// O(log n) where n is the number of comments
    pub fn comment_at_position(&self, pos: &Position) -> Option<&CommentNode> {
        // Binary search for comment containing position
        let mut left = 0;
        let mut right = self.comments.len();
        let mut prev_end = Position {
            row: 0,
            column: 0,
            byte: 0,
        };

        while left < right {
            let mid = (left + right) / 2;
            let comment = &self.comments[mid];

            let start = comment.absolute_position(prev_end);
            let end = comment.absolute_end(start);

            if pos.byte < start.byte {
                right = mid;
            } else if pos.byte >= end.byte {
                left = mid + 1;
                prev_end = end;
            } else {
                // Position is within this comment
                return Some(comment);
            }
        }

        None
    }

    /// Get all comments in a range
    ///
    /// Returns all comments whose start position falls within the given range.
    ///
    /// # Arguments
    /// * `start` - Start of the range (inclusive)
    /// * `end` - End of the range (exclusive)
    ///
    /// # Returns
    /// Vector of comments in the range
    ///
    /// # Complexity
    /// O(n) where n is the number of comments
    pub fn comments_in_range(&self, start: &Position, end: &Position) -> Vec<&CommentNode> {
        let mut result = Vec::new();
        let mut prev_end = Position {
            row: 0,
            column: 0,
            byte: 0,
        };

        for comment in &self.comments {
            let comment_start = comment.absolute_position(prev_end);
            let comment_end = comment.absolute_end(comment_start);

            // Check if comment starts within range
            if comment_start.byte >= start.byte && comment_start.byte < end.byte {
                result.push(comment);
            }

            // Early exit if we've passed the range
            if comment_start.byte >= end.byte {
                break;
            }

            prev_end = comment_end;
        }

        result
    }

    /// Get doc comment immediately before a position
    ///
    /// Finds a documentation comment (starts with `///` or `/**`) that appears
    /// immediately before the given position, with only whitespace in between.
    ///
    /// This is used to attach documentation to declarations.
    ///
    /// # Arguments
    /// * `pos` - The position to search before
    ///
    /// # Returns
    /// `Some(&CommentNode)` if a doc comment precedes the position, `None` otherwise
    ///
    /// # Examples
    ///
    /// ```rholang
    /// /// This is a doc comment
    /// contract foo() = { ... }
    /// //   ^ position here should find the doc comment above
    /// ```
    pub fn doc_comment_before(&self, pos: &Position) -> Option<&CommentNode> {
        let mut prev_end = Position {
            row: 0,
            column: 0,
            byte: 0,
        };

        let mut last_doc_comment: Option<&CommentNode> = None;

        for comment in &self.comments {
            let comment_start = comment.absolute_position(prev_end);
            let comment_end = comment.absolute_end(comment_start);

            // Stop if we've passed the target position (use rows for robustness)
            // Note: We use row-based comparison instead of byte-based because semantic
            // tree positions may not account for skipped comments, causing byte positions
            // to overlap with comment positions.
            if comment_start.row > pos.row {
                break;
            }

            // Check if this is a doc comment
            if comment.is_doc_comment {
                // Check if it's immediately before the position
                // (within 1 line, accounting for whitespace)
                let lines_between = pos.row.saturating_sub(comment_end.row);
                if lines_between <= 1 {
                    last_doc_comment = Some(comment);
                }
            }

            prev_end = comment_end;
        }

        last_doc_comment
    }

    /// Get all consecutive doc comments before a position (Phase 7)
    ///
    /// Unlike `doc_comment_before()` which returns only the last doc comment,
    /// this method returns ALL consecutive doc comments that appear immediately
    /// before the given position, in order from first to last.
    ///
    /// This enables proper multi-line doc comment aggregation for structured documentation.
    ///
    /// # Arguments
    /// * `pos` - The position to search before
    ///
    /// # Returns
    /// Vector of consecutive doc comments before the position, or empty vec if none found
    ///
    /// # Example
    /// ```rust,ignore
    /// /// Line 1
    /// /// Line 2
    /// /// Line 3
    /// contract foo() = { Nil }
    /// //   ^ position here returns all 3 doc comments
    /// ```
    pub fn doc_comments_before(&self, pos: &Position) -> Vec<&CommentNode> {
        let mut prev_end = Position {
            row: 0,
            column: 0,
            byte: 0,
        };

        let mut consecutive_docs = Vec::new();
        let mut last_doc_end_row: Option<usize> = None;

        for comment in &self.comments {
            let comment_start = comment.absolute_position(prev_end);
            let comment_end = comment.absolute_end(comment_start);

            // Stop if we've passed the target position
            if comment_start.row > pos.row {
                break;
            }

            // Check if this is a doc comment
            if comment.is_doc_comment {
                // Check if it's consecutive with previous doc comments
                if let Some(last_row) = last_doc_end_row {
                    // Allow 1 blank line between consecutive doc comments
                    if comment_start.row > last_row + 2 {
                        // Not consecutive - clear previous docs
                        consecutive_docs.clear();
                    }
                }

                // Add this comment to the consecutive group
                consecutive_docs.push(comment);
                last_doc_end_row = Some(comment_end.row);
            }

            prev_end = comment_end;
        }

        // Now check if the LAST comment in the consecutive group is immediately
        // before the target position (within 1 line)
        if let Some(&last_comment) = consecutive_docs.last() {
            // Recompute last comment's end position
            let mut prev = Position { row: 0, column: 0, byte: 0 };
            for comment in &self.comments {
                if std::ptr::eq(comment, last_comment) {
                    let end = last_comment.absolute_end(last_comment.absolute_position(prev));
                    let lines_between = pos.row.saturating_sub(end.row);
                    if lines_between > 1 {
                        // Last comment is too far - return empty
                        return Vec::new();
                    }
                    break;
                }
                prev = comment.absolute_end(comment.absolute_position(prev));
            }
        }

        consecutive_docs
    }

    /// Get all doc comments in the document
    ///
    /// Returns an iterator over all comments that are documentation comments
    /// (start with `///` or `/**`).
    ///
    /// # Returns
    /// Iterator over documentation comments
    pub fn doc_comments(&self) -> impl Iterator<Item = &CommentNode> {
        self.comments.iter().filter(|c| c.is_doc_comment)
    }

    /// Get all directive comments in the document
    ///
    /// Returns a vector of comments that contain language directives
    /// (e.g., `// @metta`, `/* @language: python */`).
    ///
    /// # Returns
    /// Vector of directive comments with their parsed language names
    ///
    /// # Note
    /// This creates mutable copies to parse directives, which may be less
    /// efficient than caching. Consider caching the results if called frequently.
    pub fn directive_comments(&self) -> Vec<(CommentNode, String)> {
        self.comments
            .iter()
            .filter_map(|c| {
                // Clone to get mutable copy for parsing
                let mut comment_copy = c.clone();
                // Parse and convert to String to avoid borrow issue
                let lang_string = comment_copy.parse_directive().map(|s| s.to_string())?;
                Some((comment_copy, lang_string))
            })
            .collect()
    }

    /// Get total number of comments
    pub fn comment_count(&self) -> usize {
        self.comments.len()
    }

    /// Check if document has any comments
    pub fn has_comments(&self) -> bool {
        !self.comments.is_empty()
    }

    /// Check if document has any doc comments
    pub fn has_doc_comments(&self) -> bool {
        self.comments.iter().any(|c| c.is_doc_comment)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::rholang_node::node_types::CommentKind;
    use crate::ir::semantic_node::{Position, NodeBase};

    fn create_test_comment(byte: usize, text: &str, is_doc: bool) -> CommentNode {
        CommentNode {
            kind: CommentKind::Line,
            base: NodeBase::new_simple(
                Position {
                    row: 0,
                    column: 0,
                    byte: byte,
                },
                text.len(),
                0,
                text.len(),
            ),
            text: text.to_string(),
            cached_directive: None,
            is_doc_comment: is_doc,
        }
    }

    fn create_test_root() -> Arc<RholangNode> {
        Arc::new(RholangNode::Nil {
            base: NodeBase::new_simple(
                Position {
                    row: 0,
                    column: 0,
                    byte: 0,
                },
                0,
                0,
                0,
            ),
            metadata: None,
        })
    }

    #[test]
    fn test_new_document_ir() {
        let root = create_test_root();
        let comments = vec![
            create_test_comment(0, "// comment 1", false),
            create_test_comment(20, "// comment 2", false),
        ];

        let doc_ir = DocumentIR::new(root.clone(), comments.clone());

        assert_eq!(doc_ir.comment_count(), 2);
        assert!(doc_ir.has_comments());
    }

    #[test]
    fn test_comment_at_position() {
        let root = create_test_root();
        let comments = vec![
            create_test_comment(0, "// comment at byte 0", false),
            create_test_comment(50, "// comment at byte 50", false),
        ];

        let doc_ir = DocumentIR::new(root, comments);

        // Position within first comment
        let pos1 = Position {
            row: 0,
            column: 5,
            byte: 5,
        };
        assert!(doc_ir.comment_at_position(&pos1).is_some());

        // Position between comments
        let pos2 = Position {
            row: 0,
            column: 25,
            byte: 25,
        };
        assert!(doc_ir.comment_at_position(&pos2).is_none());

        // Position within second comment
        let pos3 = Position {
            row: 0,
            column: 55,
            byte: 55,
        };
        assert!(doc_ir.comment_at_position(&pos3).is_some());
    }

    #[test]
    fn test_doc_comments() {
        let root = create_test_root();
        let comments = vec![
            create_test_comment(0, "// regular comment", false),
            create_test_comment(30, "/// doc comment 1", true),
            create_test_comment(60, "// another regular", false),
            create_test_comment(90, "/// doc comment 2", true),
        ];

        let doc_ir = DocumentIR::new(root, comments);

        let doc_comments: Vec<_> = doc_ir.doc_comments().collect();
        assert_eq!(doc_comments.len(), 2);
        assert!(doc_comments[0].is_doc_comment);
        assert!(doc_comments[1].is_doc_comment);
    }

    #[test]
    fn test_has_doc_comments() {
        let root = create_test_root();

        let comments_without_docs = vec![create_test_comment(0, "// regular", false)];
        let doc_ir1 = DocumentIR::new(root.clone(), comments_without_docs);
        assert!(!doc_ir1.has_doc_comments());

        let comments_with_docs = vec![
            create_test_comment(0, "// regular", false),
            create_test_comment(30, "/// doc", true),
        ];
        let doc_ir2 = DocumentIR::new(root, comments_with_docs);
        assert!(doc_ir2.has_doc_comments());
    }
}
