//! Rholang language adapter
//!
//! Provides Rholang-specific implementations of LSP features using the
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

/// Rholang-specific hover provider
pub struct RholangHoverProvider;

impl HoverProvider for RholangHoverProvider {
    fn hover_for_symbol(
        &self,
        symbol_name: &str,
        _node: &dyn SemanticNode,
        _context: &HoverContext,
    ) -> Option<HoverContents> {
        // Basic hover info for Rholang symbols
        let content = format!("**{}**\n\n*Rholang symbol*", symbol_name);

        Some(HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: content,
        }))
    }
}

/// Rholang-specific completion provider
pub struct RholangCompletionProvider;

impl CompletionProvider for RholangCompletionProvider {
    fn complete_at(
        &self,
        _node: &dyn SemanticNode,
        _context: &CompletionContext,
    ) -> Vec<CompletionItem> {
        // Return Rholang keywords as completions
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
            "contract",
            "new",
            "for",
            "match",
            "select",
            "if",
            "else",
            "let",
            "true",
            "false",
            "Nil",
        ]
    }
}

/// Rholang-specific documentation provider
pub struct RholangDocumentationProvider;

impl DocumentationProvider for RholangDocumentationProvider {
    fn documentation_for(
        &self,
        symbol_name: &str,
        _context: &DocumentationContext,
    ) -> Option<Documentation> {
        // Basic documentation lookup
        let doc_text = match symbol_name {
            "contract" => "Defines a new contract that can receive messages",
            "new" => "Creates new private names",
            "for" => "Pattern matches on channels and continues execution",
            "match" => "Pattern matches a process against cases",
            _ => return None,
        };

        Some(Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value: doc_text.to_string(),
        }))
    }
}

/// Mock symbol resolver for Rholang (used until Phase 4 backend integration)
struct MockRholangResolver;

impl SymbolResolver for MockRholangResolver {
    fn resolve_symbol(
        &self,
        _symbol_name: &str,
        _position: &crate::ir::semantic_node::Position,
        _context: &crate::ir::symbol_resolution::ResolutionContext,
    ) -> Vec<crate::ir::symbol_resolution::SymbolLocation> {
        // Placeholder - real implementation will use Rholang symbol table
        Vec::new()
    }

    fn supports_language(&self, language: &str) -> bool {
        language == "rholang"
    }
}

/// Create a Rholang language adapter
///
/// # Returns
/// Configured LanguageAdapter for Rholang
///
/// # Note
/// This currently uses a mock resolver. Phase 4 will integrate with the
/// real Rholang symbol table and scoping rules.
pub fn create_rholang_adapter() -> LanguageAdapter {
    // Create mock resolver (Phase 4 will replace with real Rholang resolver)
    let resolver: Arc<dyn SymbolResolver> = Arc::new(MockRholangResolver);

    // Create providers
    let hover = Arc::new(RholangHoverProvider);
    let completion = Arc::new(RholangCompletionProvider);
    let documentation = Arc::new(RholangDocumentationProvider);

    // Bundle into adapter
    LanguageAdapter::new(
        "rholang",
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
    fn test_create_rholang_adapter() {
        let adapter = create_rholang_adapter();

        assert_eq!(adapter.language_name(), "rholang");
    }

    #[test]
    fn test_rholang_completion_provider() {
        let provider = RholangCompletionProvider;
        let keywords = provider.keywords();

        assert!(keywords.contains(&"contract"));
        assert!(keywords.contains(&"new"));
        assert!(keywords.contains(&"for"));
    }

    #[test]
    fn test_rholang_documentation_provider() {
        use crate::ir::semantic_node::SemanticCategory;

        let provider = RholangDocumentationProvider;

        let context = DocumentationContext {
            language: "rholang".to_string(),
            category: SemanticCategory::Variable,
            qualified_name: None,
        };

        let doc = provider.documentation_for("contract", &context);
        assert!(doc.is_some());

        let doc = provider.documentation_for("unknown_symbol", &context);
        assert!(doc.is_none());
    }
}
