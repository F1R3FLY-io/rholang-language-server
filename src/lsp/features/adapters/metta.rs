//! MeTTa language adapter
//!
//! Provides MeTTa-specific implementations of LSP features using the
//! unified LanguageAdapter architecture.

use std::sync::Arc;
use tower_lsp::lsp_types::{HoverContents, CompletionItem, Documentation, MarkupContent, MarkupKind, CompletionItemKind};

use crate::lsp::features::traits::{
    LanguageAdapter, HoverProvider, CompletionProvider, DocumentationProvider,
    HoverContext, CompletionContext, DocumentationContext,
};
use crate::ir::semantic_node::SemanticNode;
use crate::ir::symbol_resolution::{SymbolResolver, lexical_scope::LexicalScopeResolver};
use crate::ir::symbol_table::SymbolTable;

/// MeTTa-specific hover provider
pub struct MettaHoverProvider;

impl HoverProvider for MettaHoverProvider {
    fn hover_for_symbol(
        &self,
        symbol_name: &str,
        _node: &dyn SemanticNode,
        _context: &HoverContext,
    ) -> Option<HoverContents> {
        // Basic hover info for MeTTa symbols
        let content = format!("**{}**\n\n*MeTTa symbol*", symbol_name);

        Some(HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: content,
        }))
    }
}

/// MeTTa-specific completion provider
pub struct MettaCompletionProvider;

impl CompletionProvider for MettaCompletionProvider {
    fn complete_at(
        &self,
        _node: &dyn SemanticNode,
        _context: &CompletionContext,
    ) -> Vec<CompletionItem> {
        // Return MeTTa keywords and built-ins as completions
        self.keywords()
            .iter()
            .map(|&kw| CompletionItem {
                label: kw.to_string(),
                kind: Some(CompletionItemKind::KEYWORD),
                ..Default::default()
            })
            .collect()
    }

    fn keywords(&self) -> &[&str] {
        &[
            "=",
            "if",
            "case",
            "let",
            "import",
            "pragma",
            // Built-in functions
            "match",
            "car",
            "cdr",
            "cons",
            "get-type",
            "get-metatype",
        ]
    }
}

/// MeTTa-specific documentation provider
pub struct MettaDocumentationProvider;

impl DocumentationProvider for MettaDocumentationProvider {
    fn documentation_for(
        &self,
        symbol_name: &str,
        _context: &DocumentationContext,
    ) -> Option<Documentation> {
        // Basic documentation lookup for MeTTa
        let doc_text = match symbol_name {
            "=" => "Defines an equality relationship or function",
            "if" => "Conditional expression",
            "case" => "Pattern matching construct",
            "let" => "Local variable binding",
            "match" => "Pattern matching function",
            _ => return None,
        };

        Some(Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value: doc_text.to_string(),
        }))
    }
}

/// Mock symbol resolver for MeTTa (used until Phase 4 backend integration)
struct MockMettaResolver;

impl SymbolResolver for MockMettaResolver {
    fn resolve_symbol(
        &self,
        _symbol_name: &str,
        _position: &crate::ir::semantic_node::Position,
        _context: &crate::ir::symbol_resolution::ResolutionContext,
    ) -> Vec<crate::ir::symbol_resolution::SymbolLocation> {
        // Placeholder - real implementation will use MettaSymbolTable
        Vec::new()
    }

    fn supports_language(&self, language: &str) -> bool {
        language == "metta"
    }
}

/// Create a MeTTa language adapter
///
/// # Returns
/// Configured LanguageAdapter for MeTTa
///
/// # Note
/// This currently uses a mock resolver. Phase 4 will integrate with the
/// real MettaSymbolTable and LexicalScopeResolver.
pub fn create_metta_adapter() -> LanguageAdapter {
    // Create mock resolver (Phase 4 will replace with LexicalScopeResolver)
    let resolver: Arc<dyn SymbolResolver> = Arc::new(MockMettaResolver);

    // Create providers
    let hover = Arc::new(MettaHoverProvider);
    let completion = Arc::new(MettaCompletionProvider);
    let documentation = Arc::new(MettaDocumentationProvider);

    // Bundle into adapter
    LanguageAdapter::new(
        "metta",
        resolver,
        hover,
        completion,
        documentation,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::symbol_table::SymbolTable;

    #[test]
    fn test_create_metta_adapter() {
        let adapter = create_metta_adapter();

        assert_eq!(adapter.language_name(), "metta");
    }

    #[test]
    fn test_metta_completion_provider() {
        let provider = MettaCompletionProvider;
        let keywords = provider.keywords();

        assert!(keywords.contains(&"="));
        assert!(keywords.contains(&"if"));
        assert!(keywords.contains(&"match"));
    }

    #[test]
    fn test_metta_documentation_provider() {
        use crate::ir::semantic_node::SemanticCategory;

        let provider = MettaDocumentationProvider;

        let context = DocumentationContext {
            language: "metta".to_string(),
            category: SemanticCategory::Variable,
            qualified_name: None,
        };

        let doc = provider.documentation_for("=", &context);
        assert!(doc.is_some());

        let doc = provider.documentation_for("unknown_symbol", &context);
        assert!(doc.is_none());
    }
}
