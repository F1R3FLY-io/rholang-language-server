//! Generic language adapter
//!
//! Provides default language-agnostic implementations of LSP features using the
//! unified LanguageAdapter architecture. This adapter uses a single global scope
//! and supports multiple declarations/definitions with cross-document linking.

use std::sync::Arc;
use tower_lsp::lsp_types::{
    HoverContents, CompletionItem, Documentation, MarkupContent, MarkupKind, CompletionItemKind,
};

use crate::lsp::features::traits::{
    LanguageAdapter, HoverProvider, CompletionProvider, DocumentationProvider,
    HoverContext, CompletionContext, DocumentationContext,
};
use crate::ir::semantic_node::SemanticNode;
use crate::ir::symbol_resolution::{SymbolResolver, GenericSymbolResolver};
use crate::lsp::models::WorkspaceState;

/// Generic hover provider for language-agnostic symbol information
pub struct GenericHoverProvider;

impl HoverProvider for GenericHoverProvider {
    fn hover_for_symbol(
        &self,
        symbol_name: &str,
        _node: &dyn SemanticNode,
        _context: &HoverContext,
    ) -> Option<HoverContents> {
        // Basic hover info - no language-specific details
        let content = format!("**{}**\n\n*Symbol*", symbol_name);

        Some(HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: content,
        }))
    }
}

/// Generic completion provider with no built-in keywords
pub struct GenericCompletionProvider;

impl CompletionProvider for GenericCompletionProvider {
    fn complete_at(
        &self,
        _node: &dyn SemanticNode,
        _context: &CompletionContext,
    ) -> Vec<CompletionItem> {
        // Return empty - no generic keywords
        // Language-specific adapters should override this
        Vec::new()
    }

    fn keywords(&self) -> &[&str] {
        // No keywords for generic adapter
        &[]
    }
}

/// Generic documentation provider with no built-in docs
pub struct GenericDocumentationProvider;

impl DocumentationProvider for GenericDocumentationProvider {
    fn documentation_for(
        &self,
        _symbol_name: &str,
        _context: &DocumentationContext,
    ) -> Option<Documentation> {
        // No documentation for generic symbols
        // Language-specific adapters should override this
        None
    }
}

/// Create a generic language adapter with global scope resolution
///
/// # Arguments
/// * `workspace` - Workspace state for accessing global_virtual_symbols
/// * `language_name` - Language identifier (e.g., "python", "javascript")
///
/// # Returns
/// Configured LanguageAdapter for the specified language
///
/// # Architecture
///
/// This adapter uses `GenericSymbolResolver` which provides:
/// - **Single global scope** - no lexical hierarchy
/// - **Multiple locations** - symbols can have many declarations/definitions
/// - **Cross-document linking** - uses `global_virtual_symbols` index
/// - **Language-agnostic** - works for any language
///
/// ## Resolution Strategy
///
/// The resolver queries `workspace.global_virtual_symbols[language][symbol_name]`
/// and returns ALL matching locations without filtering. This provides a simple,
/// flat namespace model suitable for:
/// - Prototyping new language support
/// - Languages with global-only scoping
/// - Fallback when language-specific logic isn't available
///
/// ## Composability
///
/// Language-specific adapters can:
/// 1. **Override**: Replace GenericSymbolResolver entirely
/// 2. **Compose**: Use `ComposableSymbolResolver` to layer generic + specific logic
/// 3. **Extend**: Subclass providers to add language-specific behavior
///
/// # Example
///
/// ```ignore
/// // Create generic adapter for Python
/// let adapter = create_generic_adapter(workspace.clone(), "python".to_string());
///
/// // Use in unified LSP handlers
/// let locations = adapter.resolver.resolve_symbol(&symbol_name, &position, &context);
/// ```
pub fn create_generic_adapter(
    workspace: Arc<WorkspaceState>,
    language_name: String,
) -> LanguageAdapter {
    // Create generic global scope resolver
    let resolver: Arc<dyn SymbolResolver> = Arc::new(
        GenericSymbolResolver::new(workspace, language_name.clone())
    );

    // Create generic providers (can be overridden by language-specific adapters)
    let hover = Arc::new(GenericHoverProvider);
    let completion = Arc::new(GenericCompletionProvider);
    let documentation = Arc::new(GenericDocumentationProvider);

    // Bundle into adapter
    LanguageAdapter::new(
        &language_name,
        resolver,
        hover,
        completion,
        documentation,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashmap::DashMap;

    #[test]
    fn test_create_generic_adapter() {
        let workspace = Arc::new(WorkspaceState {
            documents: Arc::new(DashMap::new()),
            global_table: Arc::new(tokio::sync::RwLock::new(
                crate::ir::symbol_table::SymbolTable::new(None),
            )),
            // REMOVED (Priority 2b): global_inverted_index
            global_contracts: Arc::new(DashMap::new()),
            global_calls: Arc::new(DashMap::new()),
            global_index: Arc::new(std::sync::RwLock::new(
                crate::ir::global_index::GlobalSymbolIndex::new(),
            )),
            global_virtual_symbols: Arc::new(DashMap::new()),
            rholang_symbols: Arc::new(crate::lsp::rholang_contracts::RholangContracts::new()),
            indexing_state: Arc::new(tokio::sync::RwLock::new(
                crate::lsp::models::IndexingState::Idle,
            )),
            completion_index: Arc::new(crate::lsp::features::completion::WorkspaceCompletionIndex::new()),
        });

        let adapter = create_generic_adapter(workspace, "python".to_string());

        assert_eq!(adapter.language_name(), "python");
        assert!(adapter.resolver.supports_language("python"));
    }

    #[test]
    fn test_generic_completion_provider_empty() {
        let provider = GenericCompletionProvider;
        let keywords = provider.keywords();

        assert_eq!(keywords.len(), 0);
    }

    #[test]
    fn test_generic_documentation_provider_none() {
        use crate::ir::semantic_node::SemanticCategory;

        let provider = GenericDocumentationProvider;

        let context = DocumentationContext {
            language: "python".to_string(),
            category: SemanticCategory::Variable,
            qualified_name: None,
        };

        let doc = provider.documentation_for("some_symbol", &context);
        assert!(doc.is_none());
    }
}
