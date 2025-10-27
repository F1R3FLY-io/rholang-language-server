//! Comment directive parser for detecting embedded language regions
//!
//! Scans Rholang source files for comment directives like `// @metta` or `// @language: metta`
//! that indicate embedded language regions within string literals.

use tree_sitter::{Node as TSNode, Tree};
use ropey::Rope;
use tracing::{debug, trace};

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
}

/// Parser for detecting embedded language regions via comment directives
pub struct DirectiveParser;

impl DirectiveParser {
    /// Scans a document for comment directives above string literals
    ///
    /// Looks for patterns like:
    /// - `// @metta`
    /// - `// @language: metta`
    /// - `// @language:meta`
    /// - `/* @metta */`
    ///
    /// # Arguments
    /// * `source` - The source text to scan
    /// * `tree` - The Tree-Sitter parse tree
    /// * `rope` - The rope representation of the source
    ///
    /// # Returns
    /// Vector of detected language regions
    pub fn scan_directives(source: &str, tree: &Tree, _rope: &Rope) -> Vec<LanguageRegion> {
        let mut regions = Vec::new();
        let root = tree.root_node();

        // Collect all comments with their positions
        let comments = Self::collect_comments(&root, source);
        debug!("Found {} comment nodes", comments.len());

        // Collect all string literals with their positions
        let string_literals = Self::collect_string_literals(&root, source);
        debug!("Found {} string literal nodes", string_literals.len());

        // Match comments to string literals
        for (string_node, string_text) in &string_literals {
            // Look for a comment immediately before this string literal
            if let Some((directive_lang, _comment_node)) = Self::find_directive_before(
                string_node.start_byte(),
                string_node.start_position().row,
                &comments,
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
                });
            }
        }

        regions
    }

    /// Collects all comment nodes from the tree
    fn collect_comments<'a>(node: &TSNode<'a>, source: &'a str) -> Vec<(TSNode<'a>, String)> {
        let mut comments = Vec::new();
        let mut cursor = node.walk();

        Self::walk_tree_for_comments(&mut cursor, source, &mut comments);

        comments
    }

    /// Recursively walks the tree to find all comment nodes
    fn walk_tree_for_comments<'a>(
        cursor: &mut tree_sitter::TreeCursor<'a>,
        source: &'a str,
        comments: &mut Vec<(TSNode<'a>, String)>,
    ) {
        let node = cursor.node();

        // Check if this is a comment node
        if node.kind() == "line_comment" || node.kind() == "block_comment" {
            if let Ok(text) = node.utf8_text(source.as_bytes()) {
                trace!("Found comment: {:?}", text);
                comments.push((node, text.to_string()));
            }
        }

        // Recurse into children
        if cursor.goto_first_child() {
            loop {
                Self::walk_tree_for_comments(cursor, source, comments);
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
            cursor.goto_parent();
        }
    }

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

    /// Finds a language directive comment before a string literal
    ///
    /// Returns the language name if a directive is found
    fn find_directive_before<'a>(
        string_start_byte: usize,
        string_line: usize,
        comments: &[(TSNode<'a>, String)],
    ) -> Option<(String, TSNode<'a>)> {
        // Look for a comment on the line immediately before the string literal
        // or on the same line before the string
        for (comment_node, comment_text) in comments {
            let comment_line = comment_node.start_position().row;
            let comment_end_byte = comment_node.end_byte();

            // Comment should be before the string (same line or previous line)
            let is_before = comment_end_byte < string_start_byte
                && (comment_line == string_line || comment_line + 1 == string_line);

            if is_before {
                // Check if comment contains a directive
                if let Some(lang) = Self::parse_directive(comment_text) {
                    return Some((lang, *comment_node));
                }
            }
        }

        None
    }

    /// Parses a comment to extract a language directive
    ///
    /// Matches patterns:
    /// - `@metta`
    /// - `@language: metta` (whitespace irrelevant)
    /// - `@language:meta` (alias for metta, whitespace irrelevant)
    ///
    /// Whitespace between `@language`, `:`, and the language name is completely ignored.
    /// Examples: `@language:metta`, `@language: metta`, `@language :metta`, `@language : metta`
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree_sitter::parse_code;

    #[test]
    fn test_parse_directive_metta() {
        assert_eq!(
            DirectiveParser::parse_directive("// @metta"),
            Some("metta".to_string())
        );
    }

    #[test]
    fn test_parse_directive_language_metta() {
        assert_eq!(
            DirectiveParser::parse_directive("// @language: metta"),
            Some("metta".to_string())
        );
    }

    #[test]
    fn test_parse_directive_language_meta() {
        assert_eq!(
            DirectiveParser::parse_directive("// @language:meta"),
            Some("metta".to_string())
        );
    }

    #[test]
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
    fn test_parse_directive_block_comment() {
        assert_eq!(
            DirectiveParser::parse_directive("/* @metta */"),
            Some("metta".to_string())
        );
    }

    #[test]
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

        let regions = DirectiveParser::scan_directives(source, &tree, &rope);

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

        let regions = DirectiveParser::scan_directives(source, &tree, &rope);

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

        let regions = DirectiveParser::scan_directives(source, &tree, &rope);

        // Should not detect anything without a directive
        assert_eq!(regions.len(), 0);
    }
}
