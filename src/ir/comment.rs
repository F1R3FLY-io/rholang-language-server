//! Comment node representation for the separate comment channel
//!
//! Comments are stored separately from the semantic IR tree to avoid:
//! - Polluting visitor traversals with non-semantic nodes
//! - Complicating position tracking with interleaved comments
//! - Adding overhead to symbol table and semantic analysis
//!
//! This module provides `CommentNode` which stores comment information with precise
//! position tracking, enabling:
//! - Directive parsing (e.g., `// @metta` language markers)
//! - Documentation extraction (e.g., `///` or `/**` doc comments)
//! - Source code reconstruction

use crate::ir::rholang_node::node_types::CommentKind;
use crate::ir::semantic_node::{NodeBase, Position};
use ropey::Rope;
use tree_sitter::Node as TSNode;

/// Represents a comment in the source code with position and content
///
/// Comments are stored in a separate channel from the main IR tree,
/// maintaining precise position information for directive parsing and
/// documentation extraction.
///
/// # Examples
///
/// ```rust,ignore
/// // Creating from Tree-Sitter node
/// let comment = CommentNode::from_ts_node(ts_node, &rope, prev_end);
///
/// // Checking for directives
/// if let Some(lang) = comment.parse_directive() {
///     println!("Language directive: {}", lang);
/// }
///
/// // Extracting doc comment text
/// if comment.is_doc_comment {
///     if let Some(doc_text) = comment.doc_text() {
///         println!("Documentation: {}", doc_text);
///     }
/// }
/// ```
/// Phase B-3: Added Serialize/Deserialize for persistent cache
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CommentNode {
    /// Kind of comment (line comment or block comment)
    pub kind: CommentKind,

    /// Position tracking (relative to previous comment or document start)
    pub base: NodeBase,

    /// Raw text content including delimiters (e.g., `// comment` or `/* block */`)
    pub text: String,

    /// Cached parsed directive if this is a language directive comment
    /// Set to Some(language) after calling `parse_directive()`
    pub cached_directive: Option<String>,

    /// Whether this is a documentation comment (starts with `///` or `/**`)
    pub is_doc_comment: bool,
}

impl CommentNode {
    /// Create a CommentNode from a Tree-Sitter node
    ///
    /// # Arguments
    /// * `ts_node` - Tree-Sitter node representing the comment
    /// * `rope` - Source code rope for text extraction
    /// * `prev_end` - End position of the previous comment or document start
    ///
    /// # Returns
    /// A new `CommentNode` with position tracking and text content
    ///
    /// # Panics
    /// Panics if `ts_node` is not a `line_comment` or `block_comment` node
    pub fn from_ts_node(ts_node: TSNode, rope: &Rope, prev_end: Position) -> Self {
        // Extract text from source
        let start_byte = ts_node.start_byte();
        let end_byte = ts_node.end_byte();
        let text = rope
            .byte_slice(start_byte..end_byte)
            .to_string();

        // Determine comment kind
        let kind = match ts_node.kind() {
            "line_comment" => CommentKind::Line,
            "block_comment" => CommentKind::Block,
            _ => panic!("CommentNode::from_ts_node called with non-comment node: {}", ts_node.kind()),
        };

        // Check if this is a documentation comment
        let is_doc_comment = text.starts_with("///") || text.starts_with("/**");

        // Compute absolute positions from Tree-Sitter
        let absolute_start = Position {
            row: ts_node.start_position().row,
            column: ts_node.start_position().column,
            byte: start_byte,
        };

        let absolute_end = Position {
            row: ts_node.end_position().row,
            column: ts_node.end_position().column,
            byte: end_byte,
        };

        // Create NodeBase with position tracking
        let length = end_byte - start_byte;
        let span_lines = absolute_end.row - absolute_start.row;
        let span_columns = if span_lines == 0 {
            absolute_end.column - absolute_start.column
        } else {
            absolute_end.column
        };

        let base = NodeBase::new_simple(absolute_start, length, span_lines, span_columns);

        Self {
            kind,
            base,
            text,
            cached_directive: None,
            is_doc_comment,
        }
    }

    /// Parse and cache language directive from comment text
    ///
    /// Looks for patterns like:
    /// - `// @metta`
    /// - `// @language: python`
    /// - `/* @metta */`
    ///
    /// # Returns
    /// `Some(language_name)` if a directive is found, `None` otherwise
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let comment = CommentNode { text: "// @metta".to_string(), .. };
    /// assert_eq!(comment.parse_directive(), Some("metta"));
    /// ```
    pub fn parse_directive(&mut self) -> Option<&str> {
        if self.cached_directive.is_none() {
            self.cached_directive = Self::extract_directive(&self.text);
        }
        self.cached_directive.as_deref()
    }

    /// Extract language directive from comment text (internal helper)
    fn extract_directive(text: &str) -> Option<String> {
        // Remove comment delimiters
        let content = match text {
            s if s.starts_with("//") => s.trim_start_matches('/').trim(),
            s if s.starts_with("/*") => {
                s.trim_start_matches("/*").trim_end_matches("*/").trim()
            }
            _ => return None,
        };

        // Check for @language directive
        if let Some(stripped) = content.strip_prefix('@') {
            // Handle both `@metta` and `@language: metta` formats
            let lang = if let Some(colon_pos) = stripped.find(':') {
                // Format: @language: metta
                stripped[colon_pos + 1..].trim()
            } else {
                // Format: @metta
                stripped.trim()
            };

            // Extract just the language name (stop at first whitespace)
            let lang_name = lang.split_whitespace().next()?;

            if !lang_name.is_empty() {
                return Some(lang_name.to_string());
            }
        }

        None
    }

    /// Get documentation text with comment delimiters stripped
    ///
    /// For line comments (`///`), strips the leading `///` and any trailing whitespace.
    /// For block comments (`/**`), strips `/**` and `*/`, and removes leading `*` from each line.
    ///
    /// # Returns
    /// `Some(doc_text)` if this is a documentation comment, `None` otherwise
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let comment = CommentNode {
    ///     text: "/// This is a doc comment".to_string(),
    ///     is_doc_comment: true,
    ///     ..
    /// };
    /// assert_eq!(comment.doc_text(), Some("This is a doc comment".to_string()));
    /// ```
    pub fn doc_text(&self) -> Option<String> {
        if !self.is_doc_comment {
            return None;
        }

        Some(match self.kind {
            CommentKind::Line => {
                // Strip /// prefix and trim
                self.text
                    .trim_start_matches("///")
                    .trim()
                    .to_string()
            }
            CommentKind::Block => {
                // Strip /** and */ delimiters
                let content = self.text
                    .trim_start_matches("/**")
                    .trim_end_matches("*/");

                // Process multi-line doc comments, strip leading * on each line
                content
                    .lines()
                    .map(|line| {
                        line.trim_start()
                            .trim_start_matches('*')
                            .trim()
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        })
    }

    /// Get absolute position of this comment
    ///
    /// # Arguments
    /// * `_prev_end` - The end position of the previous element (unused, kept for API compatibility)
    ///
    /// # Returns
    /// Absolute `Position` of the start of this comment
    pub fn absolute_position(&self, _prev_end: Position) -> Position {
        self.base.start()
    }

    /// Compute absolute end position of this comment
    ///
    /// # Arguments
    /// * `start` - The absolute start position of this comment
    ///
    /// # Returns
    /// Absolute `Position` of the end of this comment
    pub fn absolute_end(&self, start: Position) -> Position {
        Position {
            row: start.row + self.base.span_lines(),
            column: if self.base.span_lines() > 0 {
                self.base.span_columns()
            } else {
                start.column + self.base.span_columns()
            },
            byte: start.byte + self.base.syntactic_length(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_directive_simple() {
        assert_eq!(
            CommentNode::extract_directive("// @metta"),
            Some("metta".to_string())
        );
        assert_eq!(
            CommentNode::extract_directive("/* @python */"),
            Some("python".to_string())
        );
    }

    #[test]
    fn test_extract_directive_with_colon() {
        assert_eq!(
            CommentNode::extract_directive("// @language: metta"),
            Some("metta".to_string())
        );
        assert_eq!(
            CommentNode::extract_directive("/* @language: python */"),
            Some("python".to_string())
        );
    }

    #[test]
    fn test_extract_directive_no_directive() {
        assert_eq!(CommentNode::extract_directive("// regular comment"), None);
        assert_eq!(CommentNode::extract_directive("/* block comment */"), None);
    }

    #[test]
    fn test_doc_text_line_comment() {
        let comment = CommentNode {
            kind: CommentKind::Line,
            base: NodeBase::new_simple(
                Position {
                    row: 0,
                    column: 0,
                    byte: 0,
                },
                20,
                0,
                20,
            ),
            text: "/// This is a doc comment".to_string(),
            cached_directive: None,
            is_doc_comment: true,
        };

        assert_eq!(comment.doc_text(), Some("This is a doc comment".to_string()));
    }

    #[test]
    fn test_doc_text_block_comment() {
        let comment = CommentNode {
            kind: CommentKind::Block,
            base: NodeBase::new_simple(
                Position {
                    row: 0,
                    column: 0,
                    byte: 0,
                },
                30,
                0,
                30,
            ),
            text: "/** Multi\n * line\n * doc */".to_string(),
            cached_directive: None,
            is_doc_comment: true,
        };

        assert_eq!(
            comment.doc_text(),
            Some("Multi\nline\ndoc".to_string())
        );
    }

    #[test]
    fn test_doc_text_non_doc_comment() {
        let comment = CommentNode {
            kind: CommentKind::Line,
            base: NodeBase::new_simple(
                Position {
                    row: 0,
                    column: 0,
                    byte: 0,
                },
                20,
                0,
                20,
            ),
            text: "// regular comment".to_string(),
            cached_directive: None,
            is_doc_comment: false,
        };

        assert_eq!(comment.doc_text(), None);
    }
}
