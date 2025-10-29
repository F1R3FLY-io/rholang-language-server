//! MeTTa language adapter
//!
//! Provides MeTTa-specific implementations of LSP features using the
//! unified LanguageAdapter architecture.

use std::sync::Arc;
use tower_lsp::lsp_types::{HoverContents, CompletionItem, Documentation, MarkupContent, MarkupKind, CompletionItemKind, Url};

use crate::lsp::features::traits::{
    LanguageAdapter, HoverProvider, CompletionProvider, DocumentationProvider,
    HoverContext, CompletionContext, DocumentationContext,
};
use crate::ir::semantic_node::SemanticNode;
use crate::ir::symbol_resolution::{
    SymbolResolver,
    lexical_scope::LexicalScopeResolver,
    composable::ComposableSymbolResolver,
};
use crate::ir::transforms::metta_symbol_table_builder::MettaSymbolTable;
use crate::lsp::models::WorkspaceState;

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

/// Create a MeTTa language adapter with composable symbol resolution
///
/// # Arguments
/// * `symbol_table` - MeTTa symbol table for the virtual document
/// * `workspace` - Workspace state for cross-document symbol lookup
/// * `parent_uri` - URI of the parent Rholang document
///
/// # Returns
/// Configured LanguageAdapter for MeTTa with ComposableSymbolResolver
///
/// # Architecture
/// Uses a composable resolver with:
/// 1. Base: LexicalScopeResolver for local symbols
/// 2. Fallback: AsyncGlobalVirtualSymbolResolver for cross-document symbols
/// 3. Filters: (Future) MettaPatternFilter for arity-based refinement
pub fn create_metta_adapter(
    symbol_table: Arc<MettaSymbolTable>,
    workspace: Arc<WorkspaceState>,
    _parent_uri: Url,
) -> LanguageAdapter {
    // Create base lexical scope resolver
    let base_resolver = Box::new(
        LexicalScopeResolver::new(symbol_table, "metta".to_string())
    );

    // Create composable resolver
    // Note: Pattern matching filter and global cross-document resolver will be added later
    // AsyncGlobalVirtualSymbolResolver requires async, which ComposableSymbolResolver doesn't support yet
    let resolver: Arc<dyn SymbolResolver> = Arc::new(
        ComposableSymbolResolver::new(
            base_resolver,
            vec![], // No filters for now - pattern matcher will be added later
            None,   // No global fallback for now - async support needed
        )
    );

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
    use crate::ir::transforms::metta_symbol_table_builder::MettaSymbolTable;
    use crate::lsp::models::WorkspaceState;
    use dashmap::DashMap;

    #[test]
    fn test_create_metta_adapter() {
        let symbol_table = Arc::new(MettaSymbolTable::new(Url::parse("file:///test.rho#metta:0").unwrap()));
        let workspace = Arc::new(WorkspaceState {
            documents: Arc::new(DashMap::new()),
            global_symbols: Arc::new(DashMap::new()),
            global_table: Arc::new(tokio::sync::RwLock::new(crate::ir::symbol_table::SymbolTable::new())),
            global_inverted_index: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            global_virtual_symbols: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        });
        let parent_uri = Url::parse("file:///test.rho").unwrap();

        let adapter = create_metta_adapter(symbol_table, workspace, parent_uri);

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
