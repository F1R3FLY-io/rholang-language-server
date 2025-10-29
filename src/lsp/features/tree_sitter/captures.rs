//! Capture processors for converting Tree-Sitter query results to LSP features
//!
//! This module provides processors that convert Tree-Sitter query captures into
//! LSP responses for semantic tokens, folding ranges, and formatting.

use std::collections::HashMap;
use tower_lsp::lsp_types::{
    FoldingRange, FoldingRangeKind, Position, Range,
    SemanticToken, SemanticTokenType, SemanticTokensLegend,
    TextEdit,
};
use tracing::{debug, trace};

use super::query_types::{QueryCapture, CaptureType, HighlightType, IndentType, LocalType};

/// Processor for converting query captures to LSP features
pub struct CaptureProcessor;

impl CaptureProcessor {
    /// Convert highlight captures to LSP semantic tokens
    ///
    /// Takes the results of a highlights.scm query and converts them to
    /// LSP semantic tokens for syntax highlighting.
    ///
    /// # Arguments
    /// * `captures` - Captures from highlights.scm query
    ///
    /// # Returns
    /// Vector of semantic tokens, delta-encoded as per LSP spec
    pub fn to_semantic_tokens(captures: &[QueryCapture]) -> Vec<SemanticToken> {
        debug!("Converting {} captures to semantic tokens", captures.len());

        // Filter to only highlight captures and sort by position
        let mut highlights: Vec<_> = captures
            .iter()
            .filter_map(|c| {
                if let CaptureType::Highlight(hl_type) = c.capture_type {
                    Some((c, hl_type))
                } else {
                    None
                }
            })
            .collect();

        // Sort by start position (line, then column)
        highlights.sort_by(|a, b| {
            let a_start = a.0.lsp_range.start;
            let b_start = b.0.lsp_range.start;
            a_start.line.cmp(&b_start.line)
                .then(a_start.character.cmp(&b_start.character))
        });

        // Convert to delta-encoded semantic tokens
        let mut tokens = Vec::new();
        let mut prev_line = 0u32;
        let mut prev_char = 0u32;

        for (capture, hl_type) in highlights {
            let start = capture.lsp_range.start;
            let length = capture.byte_range.1 - capture.byte_range.0;

            // Delta encoding (as per LSP spec)
            let delta_line = start.line - prev_line;
            let delta_start = if delta_line == 0 {
                start.character - prev_char
            } else {
                start.character
            };

            let token_type = Self::highlight_to_token_type_index(hl_type);

            tokens.push(SemanticToken {
                delta_line,
                delta_start,
                length: length as u32,
                token_type,
                token_modifiers_bitset: 0,
            });

            prev_line = start.line;
            prev_char = start.character;
        }

        trace!("Generated {} semantic tokens", tokens.len());
        tokens
    }

    /// Convert fold captures to LSP folding ranges
    ///
    /// Takes the results of a folds.scm query and converts them to
    /// LSP folding ranges for code folding.
    ///
    /// # Arguments
    /// * `captures` - Captures from folds.scm query
    ///
    /// # Returns
    /// Vector of folding ranges
    pub fn to_folding_ranges(captures: &[QueryCapture]) -> Vec<FoldingRange> {
        debug!("Converting {} captures to folding ranges", captures.len());

        let mut ranges: Vec<FoldingRange> = captures
            .iter()
            .filter(|c| c.capture_type == CaptureType::Fold)
            .map(|c| {
                let range = c.lsp_range;
                FoldingRange {
                    start_line: range.start.line,
                    start_character: Some(range.start.character),
                    end_line: range.end.line,
                    end_character: Some(range.end.character),
                    kind: Self::infer_folding_kind(c),
                    collapsed_text: None,
                }
            })
            .collect();

        // Sort by start line
        ranges.sort_by_key(|r| r.start_line);

        trace!("Generated {} folding ranges", ranges.len());
        ranges
    }

    /// Convert indent captures to formatting edits
    ///
    /// Takes the results of an indents.scm query and generates text edits
    /// to apply proper indentation.
    ///
    /// # Arguments
    /// * `captures` - Captures from indents.scm query
    /// * `source_lines` - Source code split by lines
    /// * `tab_size` - Number of spaces per indentation level
    ///
    /// # Returns
    /// Vector of text edits for formatting
    pub fn to_formatting_edits(
        captures: &[QueryCapture],
        source_lines: &[&str],
        tab_size: usize,
    ) -> Vec<TextEdit> {
        debug!("Converting {} indent captures to formatting edits", captures.len());

        // Build indentation map: line number → indentation level
        let mut indent_map: HashMap<usize, isize> = HashMap::new();
        let mut current_indent: isize = 0;

        for capture in captures {
            let line = capture.lsp_range.start.line as usize;

            match capture.capture_type {
                CaptureType::Indent(IndentType::Indent) => {
                    current_indent += 1;
                    indent_map.insert(line, current_indent);
                }
                CaptureType::Indent(IndentType::Outdent) => {
                    current_indent = current_indent.saturating_sub(1);
                    indent_map.insert(line, current_indent);
                }
                CaptureType::Indent(IndentType::Align) => {
                    // Alignment not implemented yet (requires column tracking)
                    indent_map.insert(line, current_indent);
                }
                _ => {}
            }
        }

        // Generate text edits
        let mut edits = Vec::new();

        for (line_idx, line_text) in source_lines.iter().enumerate() {
            if let Some(&indent_level) = indent_map.get(&line_idx) {
                let expected_spaces = (indent_level as usize) * tab_size;
                let current_spaces = line_text.chars().take_while(|c| *c == ' ').count();

                if current_spaces != expected_spaces {
                    // Create edit to fix indentation
                    let range = Range {
                        start: Position {
                            line: line_idx as u32,
                            character: 0,
                        },
                        end: Position {
                            line: line_idx as u32,
                            character: current_spaces as u32,
                        },
                    };

                    let new_indent = " ".repeat(expected_spaces);
                    edits.push(TextEdit {
                        range,
                        new_text: new_indent,
                    });
                }
            }
        }

        trace!("Generated {} formatting edits", edits.len());
        edits
    }

    /// Build scope tree from locals.scm captures
    ///
    /// Creates a hierarchical scope structure for symbol resolution.
    ///
    /// # Arguments
    /// * `captures` - Captures from locals.scm query
    ///
    /// # Returns
    /// Root scope node
    pub fn build_scope_tree(captures: &[QueryCapture]) -> ScopeNode {
        debug!("Building scope tree from {} captures", captures.len());

        let mut root = ScopeNode {
            range: Range {
                start: Position { line: 0, character: 0 },
                end: Position { line: u32::MAX, character: u32::MAX },
            },
            definitions: Vec::new(),
            references: Vec::new(),
            children: Vec::new(),
        };

        // Collect scopes, definitions, and references
        let mut scopes = vec![&mut root as *mut ScopeNode];
        let mut scope_stack: Vec<Range> = vec![root.range];

        for capture in captures {
            match &capture.capture_type {
                CaptureType::Local(LocalType::Scope) => {
                    // Create new scope
                    let scope_node = ScopeNode {
                        range: capture.lsp_range,
                        definitions: Vec::new(),
                        references: Vec::new(),
                        children: Vec::new(),
                    };

                    // Add to current scope
                    unsafe {
                        if let Some(current_scope) = scopes.last_mut() {
                            (**current_scope).children.push(scope_node);
                        }
                    }

                    scope_stack.push(capture.lsp_range);
                }
                CaptureType::Local(LocalType::Definition) => {
                    // Add definition to current scope
                    unsafe {
                        if let Some(current_scope) = scopes.last_mut() {
                            (**current_scope).definitions.push(capture.lsp_range);
                        }
                    }
                }
                CaptureType::Local(LocalType::Reference) => {
                    // Add reference to current scope
                    unsafe {
                        if let Some(current_scope) = scopes.last_mut() {
                            (**current_scope).references.push(capture.lsp_range);
                        }
                    }
                }
                _ => {}
            }
        }

        trace!("Built scope tree with {} scopes", root.count_scopes());
        root
    }

    /// Get LSP semantic token type legend
    pub fn semantic_token_legend() -> SemanticTokensLegend {
        SemanticTokensLegend {
            token_types: vec![
                SemanticTokenType::FUNCTION,
                SemanticTokenType::VARIABLE,
                SemanticTokenType::KEYWORD,
                SemanticTokenType::STRING,
                SemanticTokenType::NUMBER,
                SemanticTokenType::COMMENT,
                SemanticTokenType::OPERATOR,
                SemanticTokenType::TYPE,
                SemanticTokenType::ENUM_MEMBER, // Used for constants
                SemanticTokenType::PARAMETER,
                SemanticTokenType::PROPERTY,
            ],
            token_modifiers: vec![],
        }
    }

    // Helper: Convert HighlightType to token type index
    fn highlight_to_token_type_index(hl_type: HighlightType) -> u32 {
        match hl_type {
            HighlightType::Function => 0,
            HighlightType::Variable => 1,
            HighlightType::Keyword => 2,
            HighlightType::String => 3,
            HighlightType::Number => 4,
            HighlightType::Comment => 5,
            HighlightType::Operator => 6,
            HighlightType::Type => 7,
            HighlightType::Constant => 8,
            HighlightType::Parameter => 9,
            HighlightType::Property => 10,
        }
    }

    // Helper: Infer folding kind from node type
    fn infer_folding_kind(capture: &QueryCapture) -> Option<FoldingRangeKind> {
        match capture.node_type() {
            "block_comment" | "line_comment" => Some(FoldingRangeKind::Comment),
            "import" | "use" => Some(FoldingRangeKind::Imports),
            _ => None, // Region (default)
        }
    }
}

/// Scope node for tracking lexical scopes and symbols
#[derive(Debug, Clone)]
pub struct ScopeNode {
    /// Range of this scope in the document
    pub range: Range,
    /// Symbol definitions in this scope
    pub definitions: Vec<Range>,
    /// Symbol references in this scope
    pub references: Vec<Range>,
    /// Child scopes
    pub children: Vec<ScopeNode>,
}

impl ScopeNode {
    /// Count total number of scopes (including self)
    pub fn count_scopes(&self) -> usize {
        1 + self.children.iter().map(|c| c.count_scopes()).sum::<usize>()
    }

    /// Find the innermost scope containing a position
    pub fn find_scope_at(&self, position: Position) -> Option<&ScopeNode> {
        if !self.contains(position) {
            return None;
        }

        // Check children first (innermost scope)
        for child in &self.children {
            if let Some(scope) = child.find_scope_at(position) {
                return Some(scope);
            }
        }

        // Position is in this scope but not in any child
        Some(self)
    }

    /// Check if this scope contains a position
    fn contains(&self, position: Position) -> bool {
        position >= self.range.start && position <= self.range.end
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::features::tree_sitter::query_types::HighlightType;

    #[test]
    fn test_semantic_token_legend() {
        let legend = CaptureProcessor::semantic_token_legend();
        assert_eq!(legend.token_types.len(), 11);
        assert!(legend.token_types.contains(&SemanticTokenType::FUNCTION));
        assert!(legend.token_types.contains(&SemanticTokenType::VARIABLE));
    }

    #[test]
    fn test_highlight_to_token_index() {
        assert_eq!(
            CaptureProcessor::highlight_to_token_type_index(HighlightType::Function),
            0
        );
        assert_eq!(
            CaptureProcessor::highlight_to_token_type_index(HighlightType::Variable),
            1
        );
        assert_eq!(
            CaptureProcessor::highlight_to_token_type_index(HighlightType::Keyword),
            2
        );
    }

    #[test]
    fn test_scope_contains() {
        let scope = ScopeNode {
            range: Range {
                start: Position { line: 0, character: 0 },
                end: Position { line: 10, character: 0 },
            },
            definitions: vec![],
            references: vec![],
            children: vec![],
        };

        assert!(scope.contains(Position { line: 5, character: 0 }));
        assert!(!scope.contains(Position { line: 15, character: 0 }));
    }
}
