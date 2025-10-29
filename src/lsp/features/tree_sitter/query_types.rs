//! Query type definitions and capture metadata
//!
//! Defines the types of Tree-Sitter queries supported and structures
//! for working with query captures.

use std::sync::Arc;
use tree_sitter::{Node as TsNode, Range as TsRange};
use tower_lsp::lsp_types::Range;

use crate::ir::semantic_node::{Position, SemanticCategory};

/// Types of Tree-Sitter queries supported
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QueryType {
    /// Syntax highlighting (highlights.scm)
    Highlights,
    /// Code folding regions (folds.scm)
    Folds,
    /// Indentation rules (indents.scm)
    Indents,
    /// Embedded language detection (injections.scm)
    Injections,
    /// Local scope and symbols (locals.scm)
    Locals,
    /// Text object navigation (textobjects.scm)
    TextObjects,
}

impl QueryType {
    /// Get the standard filename for this query type
    pub fn filename(&self) -> &'static str {
        match self {
            QueryType::Highlights => "highlights.scm",
            QueryType::Folds => "folds.scm",
            QueryType::Indents => "indents.scm",
            QueryType::Injections => "injections.scm",
            QueryType::Locals => "locals.scm",
            QueryType::TextObjects => "textobjects.scm",
        }
    }

    /// Get human-readable description
    pub fn description(&self) -> &'static str {
        match self {
            QueryType::Highlights => "Syntax highlighting",
            QueryType::Folds => "Code folding",
            QueryType::Indents => "Indentation",
            QueryType::Injections => "Language injections",
            QueryType::Locals => "Local symbols and scopes",
            QueryType::TextObjects => "Text objects",
        }
    }
}

/// A captured node from a Tree-Sitter query
#[derive(Debug, Clone)]
pub struct QueryCapture<'tree> {
    /// The Tree-Sitter node that was captured
    pub node: TsNode<'tree>,
    /// The capture name (e.g., "function", "variable", "local.definition")
    pub capture_name: String,
    /// The capture type (parsed from capture_name)
    pub capture_type: CaptureType,
    /// Byte range in the source code
    pub byte_range: (usize, usize),
    /// LSP Range for this capture
    pub lsp_range: Range,
}

impl<'tree> QueryCapture<'tree> {
    /// Create a new QueryCapture from a Tree-Sitter node and capture name
    pub fn new(node: TsNode<'tree>, capture_name: String) -> Self {
        let capture_type = CaptureType::from_name(&capture_name);
        let byte_range = (node.start_byte(), node.end_byte());
        let lsp_range = ts_range_to_lsp_range(&node.range());

        Self {
            node,
            capture_name,
            capture_type,
            byte_range,
            lsp_range,
        }
    }

    /// Get the node type (e.g., "identifier", "block", "contract")
    pub fn node_type(&self) -> &str {
        self.node.kind()
    }

    /// Get the text content (if available from source)
    pub fn text<'a>(&self, source: &'a [u8]) -> &'a str {
        std::str::from_utf8(&source[self.byte_range.0..self.byte_range.1])
            .unwrap_or("")
    }

    /// Convert to IR Position (start position)
    pub fn start_position(&self) -> Position {
        let start = self.node.start_position();
        Position {
            row: start.row,
            column: start.column,
            byte: self.node.start_byte(),
        }
    }

    /// Convert to IR Position (end position)
    pub fn end_position(&self) -> Position {
        let end = self.node.end_position();
        Position {
            row: end.row,
            column: end.column,
            byte: self.node.end_byte(),
        }
    }

    /// Get semantic category inferred from capture type
    pub fn infer_semantic_category(&self) -> SemanticCategory {
        self.capture_type.to_semantic_category()
    }
}

/// Parsed capture type from capture name
///
/// Tree-Sitter captures use dotted notation like "local.definition" or "local.reference".
/// This enum categorizes captures for easier processing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureType {
    /// Highlighting capture (e.g., @function, @variable, @keyword)
    Highlight(HighlightType),
    /// Local scope-related capture (e.g., @local.scope, @local.definition)
    Local(LocalType),
    /// Fold region (@fold)
    Fold,
    /// Indent-related capture (@indent, @outdent, @align)
    Indent(IndentType),
    /// Language injection (@injection.language, @injection.content)
    Injection(InjectionType),
    /// Text object (@function.outer, @class.inner)
    TextObject { kind: String, boundary: TextObjectBoundary },
    /// Unknown/custom capture type
    Other(String),
}

impl CaptureType {
    /// Parse a capture name into a CaptureType
    pub fn from_name(name: &str) -> Self {
        // Handle dotted notation: "local.definition", "injection.language", etc.
        let parts: Vec<&str> = name.split('.').collect();

        match parts.as_slice() {
            ["local", local_type] => Self::Local(LocalType::from_str(local_type)),
            ["injection", inj_type] => Self::Injection(InjectionType::from_str(inj_type)),
            ["indent"] => Self::Indent(IndentType::Indent),
            ["outdent"] => Self::Indent(IndentType::Outdent),
            ["align"] => Self::Indent(IndentType::Align),
            ["fold"] => Self::Fold,
            [kind, "outer"] => Self::TextObject {
                kind: kind.to_string(),
                boundary: TextObjectBoundary::Outer,
            },
            [kind, "inner"] => Self::TextObject {
                kind: kind.to_string(),
                boundary: TextObjectBoundary::Inner,
            },
            _ => {
                // Try to parse as highlight type
                if let Some(hl_type) = HighlightType::from_str(name) {
                    Self::Highlight(hl_type)
                } else {
                    Self::Other(name.to_string())
                }
            }
        }
    }

    /// Convert to SemanticCategory for use with SemanticNode
    pub fn to_semantic_category(&self) -> SemanticCategory {
        match self {
            CaptureType::Local(LocalType::Definition) => SemanticCategory::Binding,
            CaptureType::Local(LocalType::Reference) => SemanticCategory::Variable,
            CaptureType::Local(LocalType::Scope) => SemanticCategory::Block,
            CaptureType::Highlight(HighlightType::Function) => SemanticCategory::Invocation,
            CaptureType::Highlight(HighlightType::Variable) => SemanticCategory::Variable,
            CaptureType::Highlight(HighlightType::String) => SemanticCategory::Literal,
            CaptureType::Highlight(HighlightType::Number) => SemanticCategory::Literal,
            CaptureType::Fold => SemanticCategory::Block,
            _ => SemanticCategory::LanguageSpecific,
        }
    }
}

/// Local scope capture types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalType {
    /// @local.scope - defines a lexical scope
    Scope,
    /// @local.definition - defines a symbol
    Definition,
    /// @local.reference - references a symbol
    Reference,
}

impl LocalType {
    fn from_str(s: &str) -> Self {
        match s {
            "scope" => Self::Scope,
            "definition" => Self::Definition,
            "reference" => Self::Reference,
            _ => Self::Reference, // Default to reference
        }
    }
}

/// Highlight capture types (subset of common ones)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HighlightType {
    Function,
    Variable,
    Keyword,
    String,
    Number,
    Comment,
    Operator,
    Type,
    Constant,
    Parameter,
    Property,
}

impl HighlightType {
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "function" => Some(Self::Function),
            "variable" => Some(Self::Variable),
            "keyword" => Some(Self::Keyword),
            "string" => Some(Self::String),
            "number" | "number.float" | "number.integer" => Some(Self::Number),
            "comment" => Some(Self::Comment),
            "operator" => Some(Self::Operator),
            "type" => Some(Self::Type),
            "constant" | "boolean" => Some(Self::Constant),
            "parameter" => Some(Self::Parameter),
            "property" => Some(Self::Property),
            _ => None,
        }
    }

    /// Convert to LSP semantic token type
    pub fn to_lsp_token_type(&self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Variable => "variable",
            Self::Keyword => "keyword",
            Self::String => "string",
            Self::Number => "number",
            Self::Comment => "comment",
            Self::Operator => "operator",
            Self::Type => "type",
            Self::Constant => "enumMember", // LSP doesn't have "constant"
            Self::Parameter => "parameter",
            Self::Property => "property",
        }
    }
}

/// Indent capture types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndentType {
    /// @indent - increase indentation
    Indent,
    /// @outdent - decrease indentation
    Outdent,
    /// @align - align with specific column
    Align,
}

/// Injection capture types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectionType {
    /// @injection.language - identifies the language name
    Language,
    /// @injection.content - the content to inject
    Content,
}

impl InjectionType {
    fn from_str(s: &str) -> Self {
        match s {
            "language" => Self::Language,
            "content" => Self::Content,
            _ => Self::Content, // Default to content
        }
    }
}

/// Text object boundary
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextObjectBoundary {
    /// Outer boundary (includes surrounding delimiters)
    Outer,
    /// Inner boundary (excludes surrounding delimiters)
    Inner,
}

/// Convert Tree-Sitter Range to LSP Range
fn ts_range_to_lsp_range(ts_range: &TsRange) -> Range {
    Range {
        start: tower_lsp::lsp_types::Position {
            line: ts_range.start_point.row as u32,
            character: ts_range.start_point.column as u32,
        },
        end: tower_lsp::lsp_types::Position {
            line: ts_range.end_point.row as u32,
            character: ts_range.end_point.column as u32,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capture_type_parsing() {
        assert_eq!(
            CaptureType::from_name("local.definition"),
            CaptureType::Local(LocalType::Definition)
        );
        assert_eq!(
            CaptureType::from_name("local.reference"),
            CaptureType::Local(LocalType::Reference)
        );
        assert_eq!(
            CaptureType::from_name("local.scope"),
            CaptureType::Local(LocalType::Scope)
        );
        assert_eq!(
            CaptureType::from_name("function"),
            CaptureType::Highlight(HighlightType::Function)
        );
        assert_eq!(CaptureType::from_name("fold"), CaptureType::Fold);
    }

    #[test]
    fn test_text_object_parsing() {
        match CaptureType::from_name("function.outer") {
            CaptureType::TextObject { kind, boundary } => {
                assert_eq!(kind, "function");
                assert_eq!(boundary, TextObjectBoundary::Outer);
            }
            _ => panic!("Expected TextObject"),
        }
    }

    #[test]
    fn test_semantic_category_inference() {
        assert_eq!(
            CaptureType::Local(LocalType::Definition).to_semantic_category(),
            SemanticCategory::Binding
        );
        assert_eq!(
            CaptureType::Local(LocalType::Reference).to_semantic_category(),
            SemanticCategory::Variable
        );
        assert_eq!(
            CaptureType::Highlight(HighlightType::Function).to_semantic_category(),
            SemanticCategory::Invocation
        );
    }
}
