//! Language adapter traits for unified LSP features
//!
//! This module defines the trait-based contracts that enable language-agnostic LSP feature
//! implementations. Each language provides adapters implementing these traits to customize
//! behavior while sharing the common logic.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────┐
//! │  Generic LSP Features               │
//! │  (GenericGotoDefinition, etc.)      │
//! └──────────────┬──────────────────────┘
//!                │ uses
//! ┌──────────────▼──────────────────────┐
//! │  LanguageAdapter                    │
//! │  + HoverProvider                    │
//! │  + CompletionProvider               │
//! │  + DocumentationProvider            │
//! │  + FormattingProvider (optional)    │
//! └──────────────┬──────────────────────┘
//!                │ implements
//! ┌──────────────▼──────────────────────┐
//! │  Language-Specific Adapters         │
//! │  (RholangAdapter, MettaAdapter)     │
//! └─────────────────────────────────────┘
//! ```
//!
//! # Design Principles
//!
//! 1. **Language-Agnostic Interface**: Traits work with `&dyn SemanticNode`
//! 2. **Composable**: Multiple providers can be combined
//! 3. **Extensible**: New providers can be added without breaking existing code
//! 4. **Type-Safe**: Downcasting to concrete node types when needed
//! 5. **Async-First**: All methods support async LSP handlers
//!
//! # Usage Example
//!
//! ```rust,ignore
//! use crate::lsp::features::traits::*;
//!
//! // Implement hover provider for Rholang
//! pub struct RholangHoverProvider;
//!
//! impl HoverProvider for RholangHoverProvider {
//!     fn hover_for_symbol(
//!         &self,
//!         symbol_name: &str,
//!         node: &dyn SemanticNode,
//!         context: &HoverContext,
//!     ) -> Option<HoverContents> {
//!         // Extract Rholang-specific symbol info from metadata
//!         let metadata = node.metadata()?;
//!         let symbol_info = metadata.get("symbol_info")?;
//!
//!         // Format Rholang-style hover text
//!         Some(HoverContents::Markup(MarkupContent {
//!             kind: MarkupKind::Markdown,
//!             value: format!("**{}** (Rholang channel)", symbol_name),
//!         }))
//!     }
//! }
//!
//! // Create language adapter
//! let adapter = LanguageAdapter {
//!     name: "rholang".to_string(),
//!     resolver: Arc::new(RholangResolver::new()),
//!     hover: Arc::new(RholangHoverProvider),
//!     completion: Arc::new(RholangCompletionProvider),
//!     documentation: Arc::new(RholangDocumentationProvider),
//!     formatting: None, // Optional
//! };
//! ```

use std::sync::Arc;

use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, Documentation, Hover, HoverContents,
    MarkupContent, MarkupKind, Position as LspPosition, Range, TextEdit, Url,
};

use crate::ir::semantic_node::{Position, SemanticCategory, SemanticNode};
use crate::ir::symbol_resolution::{ResolutionContext, SymbolResolver};

/// Context for hover operations
///
/// Provides information about where the hover request originated and what
/// semantic node is being hovered over.
#[derive(Clone)]
pub struct HoverContext {
    /// URI of the document
    pub uri: Url,
    /// Position in the document (LSP coordinates)
    pub lsp_position: LspPosition,
    /// Position in IR coordinates
    pub ir_position: Position,
    /// Semantic category of the node being hovered
    pub category: SemanticCategory,
    /// Language identifier (e.g., "rholang", "metta")
    pub language: String,
    /// Parent URI for virtual documents
    pub parent_uri: Option<Url>,
}

/// Context for completion operations
#[derive(Clone)]
pub struct CompletionContext {
    /// URI of the document
    pub uri: Url,
    /// Position where completion was triggered
    pub lsp_position: LspPosition,
    /// IR position
    pub ir_position: Position,
    /// Trigger character if any (e.g., ".", "::", "@")
    pub trigger_character: Option<String>,
    /// Language identifier
    pub language: String,
    /// Partial text before cursor that might be a symbol prefix
    pub prefix: String,
}

/// Context for documentation lookups
#[derive(Clone)]
pub struct DocumentationContext {
    /// Language identifier
    pub language: String,
    /// Symbol category
    pub category: SemanticCategory,
    /// Full qualified name if available (e.g., "Module::ClassName::method")
    pub qualified_name: Option<String>,
}

/// Provider trait for hover information
///
/// Languages implement this to customize what information is shown when hovering
/// over different kinds of symbols and constructs.
pub trait HoverProvider: Send + Sync {
    /// Generate hover content for a symbol
    ///
    /// This is called when hovering over variables, function names, etc.
    ///
    /// # Arguments
    /// * `symbol_name` - The name of the symbol being hovered
    /// * `node` - The semantic node at the hover position
    /// * `context` - Additional context about the hover request
    ///
    /// # Returns
    /// `Some(HoverContents)` with formatted hover information, or `None` if no hover available
    fn hover_for_symbol(
        &self,
        symbol_name: &str,
        node: &dyn SemanticNode,
        context: &HoverContext,
    ) -> Option<HoverContents>;

    /// Generate hover content for a literal value
    ///
    /// This is called when hovering over numbers, strings, booleans, etc.
    ///
    /// # Arguments
    /// * `node` - The literal node being hovered
    /// * `context` - Hover context
    ///
    /// # Returns
    /// `Some(HoverContents)` with literal info, or `None` for default behavior
    fn hover_for_literal(
        &self,
        node: &dyn SemanticNode,
        context: &HoverContext,
    ) -> Option<HoverContents> {
        // Default: no special hover for literals
        let _ = (node, context);
        None
    }

    /// Generate hover content for language-specific constructs
    ///
    /// This is called for nodes with `SemanticCategory::LanguageSpecific`.
    ///
    /// # Arguments
    /// * `node` - The language-specific node
    /// * `context` - Hover context
    ///
    /// # Returns
    /// `Some(HoverContents)` with construct info, or `None` for default behavior
    fn hover_for_language_specific(
        &self,
        node: &dyn SemanticNode,
        context: &HoverContext,
    ) -> Option<HoverContents> {
        // Default: show node type name
        let _ = (node, context);
        Some(HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: format!("**{}**", node.type_name()),
        }))
    }
}

/// Provider trait for code completion
///
/// Languages implement this to provide context-sensitive completion suggestions.
pub trait CompletionProvider: Send + Sync {
    /// Generate completion items for the given context
    ///
    /// # Arguments
    /// * `node` - The semantic node at the completion position
    /// * `context` - Completion context with trigger info
    ///
    /// # Returns
    /// Vector of completion items, sorted by relevance (most relevant first)
    fn complete_at(
        &self,
        node: &dyn SemanticNode,
        context: &CompletionContext,
    ) -> Vec<CompletionItem>;

    /// Get language keywords for completion
    ///
    /// Returns a list of language keywords that should be offered as completions
    /// in appropriate contexts.
    ///
    /// # Returns
    /// Slice of keyword strings
    fn keywords(&self) -> &[&str];

    /// Get snippet templates for common patterns
    ///
    /// Returns snippet-style completion items for common code patterns.
    ///
    /// # Example Return
    /// ```rust,ignore
    /// vec![
    ///     CompletionItem {
    ///         label: "for".to_string(),
    ///         kind: Some(CompletionItemKind::SNIPPET),
    ///         insert_text: Some("for ($var <- $chan) { $0 }".to_string()),
    ///         ..Default::default()
    ///     }
    /// ]
    /// ```
    fn snippets(&self) -> Vec<CompletionItem> {
        // Default: no snippets
        vec![]
    }
}

/// Provider trait for documentation lookups
///
/// Languages implement this to provide rich documentation for symbols and constructs.
pub trait DocumentationProvider: Send + Sync {
    /// Get documentation for a symbol
    ///
    /// # Arguments
    /// * `symbol_name` - The name of the symbol
    /// * `context` - Documentation context
    ///
    /// # Returns
    /// `Some(Documentation)` with formatted docs, or `None` if no docs available
    fn documentation_for(
        &self,
        symbol_name: &str,
        context: &DocumentationContext,
    ) -> Option<Documentation>;

    /// Get documentation for a language keyword
    ///
    /// # Arguments
    /// * `keyword` - The keyword (e.g., "contract", "for", "match")
    ///
    /// # Returns
    /// `Some(Documentation)` with keyword explanation, or `None` if unknown
    fn documentation_for_keyword(&self, keyword: &str) -> Option<Documentation> {
        let _ = keyword;
        None
    }
}

/// Provider trait for code formatting (optional)
///
/// Languages can optionally implement this to provide custom formatting.
pub trait FormattingProvider: Send + Sync {
    /// Format a document or range
    ///
    /// # Arguments
    /// * `node` - The root node to format (or subtree if range formatting)
    /// * `range` - Optional range to format (None = full document)
    /// * `options` - Formatting options (tab size, insert spaces, etc.)
    ///
    /// # Returns
    /// Vector of text edits to apply
    fn format(
        &self,
        node: &dyn SemanticNode,
        range: Option<Range>,
        options: &FormattingOptions,
    ) -> Vec<TextEdit> {
        // Default: no formatting
        let _ = (node, range, options);
        vec![]
    }
}

/// Formatting options
#[derive(Debug, Clone)]
pub struct FormattingOptions {
    /// Size of a tab in spaces
    pub tab_size: u32,
    /// Prefer spaces over tabs
    pub insert_spaces: bool,
    /// Trim trailing whitespace on a line
    pub trim_trailing_whitespace: bool,
    /// Insert a newline at the end of the file if not present
    pub insert_final_newline: bool,
    /// Trim all newlines after the final newline at the end of the file
    pub trim_final_newlines: bool,
}

/// Language adapter - bundles all language-specific providers
///
/// This struct acts as the main integration point between generic LSP features
/// and language-specific behavior. Each language creates one adapter instance
/// that provides all necessary customization.
///
/// # Example
/// ```rust,ignore
/// let rholang_adapter = LanguageAdapter {
///     name: "rholang".to_string(),
///     resolver: Arc::new(LexicalScopeResolver::new(symbol_table, "rholang".to_string())),
///     hover: Arc::new(RholangHoverProvider),
///     completion: Arc::new(RholangCompletionProvider),
///     documentation: Arc::new(RholangDocumentationProvider),
///     formatting: None, // Optional
/// };
/// ```
pub struct LanguageAdapter {
    /// Language name (e.g., "rholang", "metta")
    pub name: String,

    /// Symbol resolver for goto-definition, references, rename
    pub resolver: Arc<dyn SymbolResolver>,

    /// Hover information provider
    pub hover: Arc<dyn HoverProvider>,

    /// Code completion provider
    pub completion: Arc<dyn CompletionProvider>,

    /// Documentation lookup provider
    pub documentation: Arc<dyn DocumentationProvider>,

    /// Optional formatting provider
    pub formatting: Option<Arc<dyn FormattingProvider>>,
}

impl LanguageAdapter {
    /// Create a new language adapter
    ///
    /// # Arguments
    /// * `name` - Language identifier
    /// * `resolver` - Symbol resolution implementation
    /// * `hover` - Hover provider implementation
    /// * `completion` - Completion provider implementation
    /// * `documentation` - Documentation provider implementation
    ///
    /// # Returns
    /// New `LanguageAdapter` instance
    pub fn new(
        name: impl Into<String>,
        resolver: Arc<dyn SymbolResolver>,
        hover: Arc<dyn HoverProvider>,
        completion: Arc<dyn CompletionProvider>,
        documentation: Arc<dyn DocumentationProvider>,
    ) -> Self {
        Self {
            name: name.into(),
            resolver,
            hover,
            completion,
            documentation,
            formatting: None,
        }
    }

    /// Create a language adapter with formatting support
    ///
    /// # Arguments
    /// * `name` - Language identifier
    /// * `resolver` - Symbol resolution implementation
    /// * `hover` - Hover provider implementation
    /// * `completion` - Completion provider implementation
    /// * `documentation` - Documentation provider implementation
    /// * `formatting` - Formatting provider implementation
    ///
    /// # Returns
    /// New `LanguageAdapter` instance with formatting
    pub fn with_formatting(
        name: impl Into<String>,
        resolver: Arc<dyn SymbolResolver>,
        hover: Arc<dyn HoverProvider>,
        completion: Arc<dyn CompletionProvider>,
        documentation: Arc<dyn DocumentationProvider>,
        formatting: Arc<dyn FormattingProvider>,
    ) -> Self {
        Self {
            name: name.into(),
            resolver,
            hover,
            completion,
            documentation,
            formatting: Some(formatting),
        }
    }

    /// Get the language name
    pub fn language_name(&self) -> &str {
        &self.name
    }

    /// Check if this adapter supports formatting
    pub fn supports_formatting(&self) -> bool {
        self.formatting.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::Any;
    use crate::ir::semantic_node::{NodeBase, RelativePosition, Metadata};

    // Mock semantic node for testing
    #[derive(Debug)]
    struct MockNode {
        base: NodeBase,
        category: SemanticCategory,
        type_name: &'static str,
    }

    impl SemanticNode for MockNode {
        fn base(&self) -> &NodeBase {
            &self.base
        }

        fn metadata(&self) -> Option<&Metadata> {
            None
        }

        fn metadata_mut(&mut self) -> Option<&mut Metadata> {
            None
        }

        fn semantic_category(&self) -> SemanticCategory {
            self.category
        }

        fn type_name(&self) -> &'static str {
            self.type_name
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    // Mock hover provider for testing
    struct MockHoverProvider;

    impl HoverProvider for MockHoverProvider {
        fn hover_for_symbol(
            &self,
            symbol_name: &str,
            _node: &dyn SemanticNode,
            _context: &HoverContext,
        ) -> Option<HoverContents> {
            use tower_lsp::lsp_types::MarkedString;
            Some(HoverContents::Scalar(
                MarkedString::String(format!("Hover: {}", symbol_name))
            ))
        }
    }

    // Mock completion provider for testing
    struct MockCompletionProvider;

    impl CompletionProvider for MockCompletionProvider {
        fn complete_at(
            &self,
            _node: &dyn SemanticNode,
            _context: &CompletionContext,
        ) -> Vec<CompletionItem> {
            vec![CompletionItem {
                label: "test_completion".to_string(),
                kind: Some(CompletionItemKind::FUNCTION),
                ..Default::default()
            }]
        }

        fn keywords(&self) -> &[&str] {
            &["if", "else", "for"]
        }
    }

    // Mock documentation provider for testing
    struct MockDocumentationProvider;

    impl DocumentationProvider for MockDocumentationProvider {
        fn documentation_for(
            &self,
            symbol_name: &str,
            _context: &DocumentationContext,
        ) -> Option<Documentation> {
            Some(Documentation::String(
                format!("Documentation for {}", symbol_name)
            ))
        }
    }

    // Mock resolver for testing
    struct MockResolver;

    impl SymbolResolver for MockResolver {
        fn resolve_symbol(
            &self,
            _symbol_name: &str,
            _position: &Position,
            _context: &ResolutionContext,
        ) -> Vec<crate::ir::symbol_resolution::SymbolLocation> {
            vec![]
        }

        fn supports_language(&self, _language: &str) -> bool {
            true
        }

        fn name(&self) -> &'static str {
            "MockResolver"
        }
    }

    #[test]
    fn test_language_adapter_creation() {
        let adapter = LanguageAdapter::new(
            "test",
            Arc::new(MockResolver),
            Arc::new(MockHoverProvider),
            Arc::new(MockCompletionProvider),
            Arc::new(MockDocumentationProvider),
        );

        assert_eq!(adapter.language_name(), "test");
        assert!(!adapter.supports_formatting());
    }

    #[test]
    fn test_language_adapter_with_formatting() {
        struct MockFormatter;
        impl FormattingProvider for MockFormatter {}

        let adapter = LanguageAdapter::with_formatting(
            "test",
            Arc::new(MockResolver),
            Arc::new(MockHoverProvider),
            Arc::new(MockCompletionProvider),
            Arc::new(MockDocumentationProvider),
            Arc::new(MockFormatter),
        );

        assert_eq!(adapter.language_name(), "test");
        assert!(adapter.supports_formatting());
    }

    #[test]
    fn test_hover_provider() {
        let provider = MockHoverProvider;
        let node = MockNode {
            base: NodeBase::new_simple(
                RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 },
                10,
                0,
                10,
            ),
            category: SemanticCategory::Variable,
            type_name: "TestNode",
        };

        let context = HoverContext {
            uri: Url::parse("file:///test.rho").unwrap(),
            lsp_position: LspPosition { line: 0, character: 0 },
            ir_position: Position { row: 0, column: 0, byte: 0 },
            category: SemanticCategory::Variable,
            language: "test".to_string(),
            parent_uri: None,
        };

        let result = provider.hover_for_symbol("test_var", &node, &context);
        assert!(result.is_some());
    }

    #[test]
    fn test_completion_provider() {
        let provider = MockCompletionProvider;
        let node = MockNode {
            base: NodeBase::new_simple(
                RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 },
                10,
                0,
                10,
            ),
            category: SemanticCategory::Variable,
            type_name: "TestNode",
        };

        let context = CompletionContext {
            uri: Url::parse("file:///test.rho").unwrap(),
            lsp_position: LspPosition { line: 0, character: 0 },
            ir_position: Position { row: 0, column: 0, byte: 0 },
            trigger_character: None,
            language: "test".to_string(),
            prefix: "test".to_string(),
        };

        let items = provider.complete_at(&node, &context);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "test_completion");

        let keywords = provider.keywords();
        assert_eq!(keywords.len(), 3);
    }
}
