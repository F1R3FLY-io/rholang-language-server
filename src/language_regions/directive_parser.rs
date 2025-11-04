//! Comment directive parser for detecting embedded language regions
//!
//! Scans Rholang source files for comment directives like `// @metta` or `// @language: metta`
//! that indicate embedded language regions within string literals.
//!
//! **Phase 2**: Migrated to use DocumentIR comment channel instead of Tree-Sitter traversal.

use std::sync::Arc;
use tree_sitter::{Node as TSNode, Tree};
use ropey::Rope;
use tracing::{debug, trace};

use crate::ir::{DocumentIR, CommentNode};
use crate::ir::semantic_node::Position;

/// Source of a language region detection
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegionSource {
    /// Comment directive above string literal (e.g., `// @metta`)
    CommentDirective,
    /// Semantic analysis (string passed to MeTTa compiler)
    SemanticAnalysis,
    /// Channel flow analysis (string sent to MeTTa compiler channel)
    ChannelFlow,
}

/// Represents a detected embedded language region
#[derive(Debug, Clone)]
pub struct LanguageRegion {
    /// Language name (e.g., "metta")
    pub language: String,
    /// Start byte offset in parent document (inside string quotes)
    pub start_byte: usize,
    /// End byte offset in parent document (inside string quotes)
    pub end_byte: usize,
    /// Start line in parent document
    pub start_line: usize,
    /// Start column in parent document
    pub start_column: usize,
    /// How this region was detected
    pub source: RegionSource,
    /// The actual text content of the region
    pub content: String,
    /// Optional concatenation chain for holed virtual documents
    /// When present, indicates the region is formed by concatenating string literals with expressions
    pub concatenation_chain: Option<super::concatenation::ConcatenationChain>,
}

/// Parser for detecting embedded language regions via comment directives
pub struct DirectiveParser;

impl DirectiveParser {
    /// Scans a document for comment directives above string literals
    ///
    /// **Phase 2**: Now uses DocumentIR comment channel for efficient directive access.
    ///
    /// Looks for patterns like:
    /// - `// @metta`
    /// - `// @language: metta`
    /// - `// @language:meta`
    /// - `/* @metta */`
    ///
    /// # Arguments
    /// * `source` - The source text to scan
    /// * `document_ir` - The DocumentIR with semantic tree and comment channel
    /// * `tree` - The Tree-Sitter parse tree (for string literal extraction)
    ///
    /// # Returns
    /// Vector of detected language regions
    pub fn scan_directives(source: &str, document_ir: &Arc<DocumentIR>, tree: &Tree) -> Vec<LanguageRegion> {
        let mut regions = Vec::new();

        // Phase 2: Use comment channel instead of Tree-Sitter traversal
        debug!("Found {} comments total via comment channel", document_ir.comments.len());

        // Collect all string literals with their positions (still need Tree-Sitter for this)
        let string_literals = Self::collect_string_literals(&tree.root_node(), source);
        debug!("Found {} string literal nodes", string_literals.len());

        // Match directive comments to string literals
        for (string_node, string_text) in &string_literals {
            // Look for a directive comment immediately before this string literal
            // Pass ALL comments, not just filtered directives, to maintain position tracking
            if let Some((directive_lang, _comment)) = Self::find_directive_before_v2(
                string_node.start_byte(),
                string_node.start_position().row,
                &document_ir.comments,
            ) {
                debug!(
                    "Found {} directive for string at line {}",
                    directive_lang,
                    string_node.start_position().row
                );

                // Extract the content inside the string quotes
                let content = Self::extract_string_content(string_text);

                regions.push(LanguageRegion {
                    language: directive_lang,
                    start_byte: string_node.start_byte() + 1, // Skip opening quote
                    end_byte: string_node.end_byte() - 1,     // Skip closing quote
                    start_line: string_node.start_position().row,
                    start_column: string_node.start_position().column,
                    source: RegionSource::CommentDirective,
                    content,
                    concatenation_chain: None,
                });
            }
        }

        regions
    }

    /// **REMOVED (Phase 2)**: Comment collection now handled by DocumentIR comment channel
    ///
    /// Old implementation traversed Tree-Sitter tree. Now we use:
    /// ```rust
    /// let directives = document_ir.directive_comments();
    /// ```

    /// Collects all string literal nodes from the tree
    fn collect_string_literals<'a>(
        node: &TSNode<'a>,
        source: &'a str,
    ) -> Vec<(TSNode<'a>, String)> {
        let mut literals = Vec::new();
        let mut cursor = node.walk();

        Self::walk_tree_for_strings(&mut cursor, source, &mut literals);

        literals
    }

    /// Recursively walks the tree to find all string literal nodes
    fn walk_tree_for_strings<'a>(
        cursor: &mut tree_sitter::TreeCursor<'a>,
        source: &'a str,
        literals: &mut Vec<(TSNode<'a>, String)>,
    ) {
        let node = cursor.node();

        // Check if this is a string literal node
        if node.kind() == "string_literal" {
            // Skip if parent is a quote (like @"rho:metta:compile") or uri_literal
            // We only want string literals that are data (e.g., arguments to sends)
            let should_skip = if let Some(parent) = node.parent() {
                matches!(parent.kind(), "quote" | "uri_literal")
            } else {
                false
            };

            if !should_skip {
                if let Ok(text) = node.utf8_text(source.as_bytes()) {
                    trace!("Found string literal: {:?}", text);
                    literals.push((node, text.to_string()));
                }
            }
        }

        // Recurse into children
        if cursor.goto_first_child() {
            loop {
                Self::walk_tree_for_strings(cursor, source, literals);
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
            cursor.goto_parent();
        }
    }

    /// **Phase 2**: Finds a language directive comment before a string literal
    ///
    /// Uses the comment channel from DocumentIR for efficient access.
    /// Iterates through ALL comments to maintain proper position tracking.
    ///
    /// Returns the language name if a directive is found
    fn find_directive_before_v2(
        string_start_byte: usize,
        string_line: usize,
        comments: &[CommentNode],
    ) -> Option<(String, CommentNode)> {
        let mut prev_end = Position {
            row: 0,
            column: 0,
            byte: 0,
        };

        // Look for a directive comment on the line immediately before the string literal
        // or on the same line before the string
        // IMPORTANT: Iterate through ALL comments to maintain position tracking
        for comment in comments {
            let comment_start = comment.absolute_position(prev_end);
            let comment_end = comment.absolute_end(comment_start);

            // Check if this comment is a directive
            let mut comment_copy = comment.clone();
            if let Some(lang) = comment_copy.parse_directive() {
                // Comment should be before the string (same line or previous line)
                let is_before = comment_end.byte < string_start_byte
                    && (comment_start.row == string_line || comment_start.row + 1 == string_line);

                if is_before {
                    // Directive already parsed by CommentNode - just use it!
                    return Some((lang.to_string(), comment.clone()));
                }
            }

            prev_end = comment_end;
        }

        None
    }

    /// Parses a comment to extract a language directive
    ///
    /// **DEPRECATED (Phase 2)**: Use `CommentNode::parse_directive()` from comment channel instead.
    ///
    /// This method is kept for backward compatibility and testing only.
    /// The comment channel now provides pre-parsed directives via `document_ir.directive_comments()`.
    ///
    /// Matches patterns:
    /// - `@metta`
    /// - `@language: metta` (whitespace irrelevant)
    /// - `@language:meta` (alias for metta, whitespace irrelevant)
    ///
    /// Whitespace between `@language`, `:`, and the language name is completely ignored.
    /// Examples: `@language:metta`, `@language: metta`, `@language :metta`, `@language : metta`
    #[deprecated(since = "0.1.0", note = "Use CommentNode::parse_directive() from comment channel")]
    fn parse_directive(comment_text: &str) -> Option<String> {
        // Remove comment markers and normalize whitespace
        let content = comment_text
            .trim_start_matches("//")
            .trim_start_matches("/*")
            .trim_end_matches("*/")
            .trim();

        trace!("Parsing directive from: {:?}", content);

        // Match @metta
        if content.contains("@metta") {
            return Some("metta".to_string());
        }

        // Match @language with flexible whitespace: @language<ws>:<ws>metta
        if let Some(idx) = content.find("@language") {
            // Get everything after @language
            let after_lang = &content[idx + "@language".len()..];

            // Remove all whitespace to normalize
            let normalized = after_lang.replace(" ", "").replace("\t", "");

            // Now check for :metta or :meta
            if normalized.starts_with(":metta") || normalized.starts_with(":meta") {
                return Some("metta".to_string());
            }
        }

        None
    }

    /// Extracts the content from inside a string literal (removes quotes and escapes)
    fn extract_string_content(string_with_quotes: &str) -> String {
        if string_with_quotes.len() < 2 {
            return String::new();
        }

        // Remove leading and trailing quotes
        let content = &string_with_quotes[1..string_with_quotes.len() - 1];

        // Unescape common escape sequences
        content
            .replace("\\\"", "\"")
            .replace("\\\\", "\\")
            .replace("\\n", "\n")
            .replace("\\r", "\r")
            .replace("\\t", "\t")
    }
}

/// Implementation of VirtualDocumentDetector trait for DirectiveParser
///
/// **Phase 2**: Internally creates DocumentIR from tree/rope for efficient comment access.
impl super::detector::VirtualDocumentDetector for DirectiveParser {
    fn name(&self) -> &str {
        "directive-parser"
    }

    fn detect(&self, source: &str, tree: &Tree, rope: &Rope) -> Vec<LanguageRegion> {
        // Phase 2: Create DocumentIR internally to access comment channel
        use crate::parsers::rholang::parse_to_document_ir;
        let document_ir = parse_to_document_ir(tree, rope);

        Self::scan_directives(source, &document_ir, tree)
    }

    fn priority(&self) -> i32 {
        // Highest priority - explicit directives should override semantic detection
        100
    }

    fn can_run_in_parallel(&self) -> bool {
        true
    }

    fn supports_incremental(&self) -> bool {
        // Comment directives don't change often and require full scan
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree_sitter::{parse_code, parse_to_document_ir};

    #[test]
    #[allow(deprecated)]
    fn test_parse_directive_metta() {
        assert_eq!(
            DirectiveParser::parse_directive("// @metta"),
            Some("metta".to_string())
        );
    }

    #[test]
    #[allow(deprecated)]
    fn test_parse_directive_language_metta() {
        assert_eq!(
            DirectiveParser::parse_directive("// @language: metta"),
            Some("metta".to_string())
        );
    }

    #[test]
    #[allow(deprecated)]
    fn test_parse_directive_language_meta() {
        assert_eq!(
            DirectiveParser::parse_directive("// @language:meta"),
            Some("metta".to_string())
        );
    }

    #[test]
    #[allow(deprecated)]
    fn test_parse_directive_whitespace_variations() {
        // Test all whitespace variations
        assert_eq!(
            DirectiveParser::parse_directive("// @language:metta"),
            Some("metta".to_string()),
            "@language:metta should work"
        );
        assert_eq!(
            DirectiveParser::parse_directive("// @language: metta"),
            Some("metta".to_string()),
            "@language: metta should work"
        );
        assert_eq!(
            DirectiveParser::parse_directive("// @language :metta"),
            Some("metta".to_string()),
            "@language :metta should work"
        );
        assert_eq!(
            DirectiveParser::parse_directive("// @language : metta"),
            Some("metta".to_string()),
            "@language : metta should work"
        );
        assert_eq!(
            DirectiveParser::parse_directive("// @language  :  metta"),
            Some("metta".to_string()),
            "@language  :  metta (multiple spaces) should work"
        );
        assert_eq!(
            DirectiveParser::parse_directive("/* @language\t:\tmetta */"),
            Some("metta".to_string()),
            "@language with tabs should work"
        );
    }

    #[test]
    #[allow(deprecated)]
    fn test_parse_directive_block_comment() {
        assert_eq!(
            DirectiveParser::parse_directive("/* @metta */"),
            Some("metta".to_string())
        );
    }

    #[test]
    #[allow(deprecated)]
    fn test_parse_directive_no_match() {
        assert_eq!(DirectiveParser::parse_directive("// regular comment"), None);
    }

    #[test]
    fn test_extract_string_content() {
        assert_eq!(
            DirectiveParser::extract_string_content(r#""hello world""#),
            "hello world"
        );
        assert_eq!(
            DirectiveParser::extract_string_content(r#""escaped \"quotes\"""#),
            r#"escaped "quotes""#
        );
    }

    #[test]
    fn test_scan_directives_simple() {
        let source = r#"
// @metta
@"rho:metta:compile"!("(= factorial (lambda (n) (if (< n 2) 1 (* n (factorial (- n 1))))))"))
"#;
        let tree = parse_code(source);
        let rope = Rope::from_str(source);
        let document_ir = parse_to_document_ir(&tree, &rope);

        let regions = DirectiveParser::scan_directives(source, &document_ir, &tree);

        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language, "metta");
        assert_eq!(regions[0].source, RegionSource::CommentDirective);
        assert!(regions[0]
            .content
            .contains("(= factorial (lambda (n)"));
    }

    #[test]
    fn test_scan_directives_multiple() {
        let source = r#"
// @language: metta
@"rho:metta:compile"!("(= foo 42)")

// Regular comment
Nil |

// @metta
@"rho:metta:compile"!("(= bar 24)")
"#;
        let tree = parse_code(source);
        let rope = Rope::from_str(source);
        let document_ir = parse_to_document_ir(&tree, &rope);

        let regions = DirectiveParser::scan_directives(source, &document_ir, &tree);

        assert_eq!(regions.len(), 2);
        assert_eq!(regions[0].language, "metta");
        assert!(regions[0].content.contains("(= foo 42)"));
        assert_eq!(regions[1].language, "metta");
        assert!(regions[1].content.contains("(= bar 24)"));
    }

    #[test]
    fn test_scan_directives_no_directive() {
        let source = r#"
// Regular comment
@"rho:metta:compile"!("(= factorial 42)")
"#;
        let tree = parse_code(source);
        let rope = Rope::from_str(source);
        let document_ir = parse_to_document_ir(&tree, &rope);

        let regions = DirectiveParser::scan_directives(source, &document_ir, &tree);

        // Should not detect anything without a directive
        assert_eq!(regions.len(), 0);
    }
}
