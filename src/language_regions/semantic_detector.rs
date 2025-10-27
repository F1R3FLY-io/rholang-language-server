//! Semantic detection for embedded language regions
//!
//! Detects embedded languages by analyzing the semantic structure of the code,
//! without requiring comment directives. For example, strings sent to the
//! MeTTa compiler are automatically detected as MeTTa code.

use tree_sitter::{Node as TSNode, Tree};
use ropey::Rope;
use tracing::{debug, trace};

use super::{LanguageRegion, RegionSource};

/// Semantic analyzer for detecting embedded language regions
pub struct SemanticDetector;

impl SemanticDetector {
    /// Detects embedded language regions by analyzing semantic patterns
    ///
    /// Currently detects:
    /// - Strings sent to `@"rho:metta:compile"` channel
    /// - Strings sent to `@"rho:metta:eval"` channel
    ///
    /// # Arguments
    /// * `source` - The source text
    /// * `tree` - The Tree-Sitter parse tree
    /// * `rope` - The rope representation
    ///
    /// # Returns
    /// Vector of detected language regions
    pub fn detect_regions(source: &str, tree: &Tree, _rope: &Rope) -> Vec<LanguageRegion> {
        let mut regions = Vec::new();
        let root = tree.root_node();

        // Find all send operations
        Self::find_metta_sends(&root, source, &mut regions);

        debug!("Semantic detector found {} regions", regions.len());
        regions
    }

    /// Finds all send operations to MeTTa compiler channels
    fn find_metta_sends<'a>(
        node: &TSNode<'a>,
        source: &'a str,
        regions: &mut Vec<LanguageRegion>,
    ) {
        // Check if this is a send node
        if node.kind() == "send" {
            Self::check_send_for_metta(node, source, regions);
        }

        // Recurse into children
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            Self::find_metta_sends(&child, source, regions);
        }
    }

    /// Checks if a send operation is to a MeTTa compiler channel
    fn check_send_for_metta<'a>(
        send_node: &TSNode<'a>,
        source: &'a str,
        regions: &mut Vec<LanguageRegion>,
    ) {
        // Get the channel (first child of send)
        let mut cursor = send_node.walk();
        if !cursor.goto_first_child() {
            return;
        }

        let channel_node = cursor.node();

        // Check if channel is a quote of a string literal (e.g., @"rho:metta:compile")
        if channel_node.kind() != "quote" {
            return;
        }

        // Get the quoted string
        if !cursor.goto_first_child() {
            cursor.goto_parent();
            return;
        }

        // Skip the '@' token
        if cursor.node().kind() == "@" {
            if !cursor.goto_next_sibling() {
                cursor.goto_parent();
                cursor.goto_parent();
                return;
            }
        }

        let string_node = cursor.node();
        if string_node.kind() != "string_literal" {
            cursor.goto_parent();
            cursor.goto_parent();
            return;
        }

        // Extract channel name
        let channel_text = match string_node.utf8_text(source.as_bytes()) {
            Ok(text) => text,
            Err(_) => {
                cursor.goto_parent();
                cursor.goto_parent();
                return;
            }
        };
        let channel_name = Self::extract_string_content(channel_text);

        trace!("Found send to channel: {:?}", channel_name);

        // Check if this is a MeTTa compiler channel
        if !Self::is_metta_compiler_channel(&channel_name) {
            cursor.goto_parent();
            cursor.goto_parent();
            return;
        }

        debug!("Detected send to MeTTa compiler channel: {}", channel_name);

        // Go back to send node to find the inputs
        cursor.goto_parent(); // Back to quote
        cursor.goto_parent(); // Back to send

        // Find the inputs node
        for child in send_node.children(&mut send_node.walk()) {
            if child.kind() == "inputs" {
                Self::extract_metta_strings(&child, source, regions);
                break;
            }
        }
    }

    /// Extracts string literals from inputs node
    fn extract_metta_strings<'a>(
        inputs_node: &TSNode<'a>,
        source: &'a str,
        regions: &mut Vec<LanguageRegion>,
    ) {
        for child in inputs_node.children(&mut inputs_node.walk()) {
            if child.kind() == "string_literal" {
                // Extract the string content
                if let Ok(text) = child.utf8_text(source.as_bytes()) {
                    let content = Self::extract_string_content(text);

                    // Create a language region
                    regions.push(LanguageRegion {
                        language: "metta".to_string(),
                        start_byte: child.start_byte() + 1, // Skip opening quote
                        end_byte: child.end_byte() - 1,     // Skip closing quote
                        start_line: child.start_position().row,
                        start_column: child.start_position().column,
                        source: RegionSource::SemanticAnalysis,
                        content,
                    });
                }
            }
        }
    }

    /// Checks if a channel name is a MeTTa compiler channel
    fn is_metta_compiler_channel(channel_name: &str) -> bool {
        matches!(
            channel_name,
            "rho:metta:compile" | "rho:metta:eval" | "rho:metta:repl"
        )
    }

    /// Extracts content from a string literal (removes quotes)
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
    fn test_detect_direct_metta_send() {
        let source = r#"
@"rho:metta:compile"!("(= factorial (lambda (n) 42))")
"#;
        let tree = parse_code(source);
        let rope = Rope::from_str(source);

        let regions = SemanticDetector::detect_regions(source, &tree, &rope);

        assert_eq!(regions.len(), 1, "Should detect one MeTTa region");
        assert_eq!(regions[0].language, "metta");
        assert_eq!(regions[0].source, RegionSource::SemanticAnalysis);
        assert!(regions[0].content.contains("factorial"));
    }

    #[test]
    fn test_detect_multiple_metta_sends() {
        let source = r#"
@"rho:metta:compile"!("(= foo 42)") |
@"rho:metta:compile"!("(= bar 24)")
"#;
        let tree = parse_code(source);
        let rope = Rope::from_str(source);

        let regions = SemanticDetector::detect_regions(source, &tree, &rope);

        assert_eq!(regions.len(), 2, "Should detect two MeTTa regions");
        assert_eq!(regions[0].language, "metta");
        assert_eq!(regions[1].language, "metta");
    }

    #[test]
    fn test_no_detection_for_non_metta_channels() {
        let source = r#"
@"rho:io:stdout"!("hello world")
"#;
        let tree = parse_code(source);
        let rope = Rope::from_str(source);

        let regions = SemanticDetector::detect_regions(source, &tree, &rope);

        assert_eq!(
            regions.len(),
            0,
            "Should not detect regions for non-MeTTa channels"
        );
    }

    #[test]
    fn test_detect_metta_eval_channel() {
        let source = r#"
@"rho:metta:eval"!("(+ 1 2)")
"#;
        let tree = parse_code(source);
        let rope = Rope::from_str(source);

        let regions = SemanticDetector::detect_regions(source, &tree, &rope);

        assert_eq!(
            regions.len(),
            1,
            "Should detect MeTTa region for eval channel"
        );
        assert_eq!(regions[0].content, "(+ 1 2)");
    }

    #[test]
    fn test_extract_string_content() {
        assert_eq!(
            SemanticDetector::extract_string_content(r#""hello world""#),
            "hello world"
        );
        assert_eq!(
            SemanticDetector::extract_string_content(r#""test \"quote\"""#),
            r#"test "quote""#
        );
    }

    #[test]
    fn test_is_metta_compiler_channel() {
        assert!(SemanticDetector::is_metta_compiler_channel(
            "rho:metta:compile"
        ));
        assert!(SemanticDetector::is_metta_compiler_channel(
            "rho:metta:eval"
        ));
        assert!(SemanticDetector::is_metta_compiler_channel(
            "rho:metta:repl"
        ));
        assert!(!SemanticDetector::is_metta_compiler_channel(
            "rho:io:stdout"
        ));
    }
}
