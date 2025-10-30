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
    GlobalVirtualSymbolResolver,
    filters::MettaPatternFilter,
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
/// Configured LanguageAdapter for MeTTa with full symbol resolution
///
/// # Architecture
/// Uses a composable resolver with:
/// 1. **Base**: LexicalScopeResolver for local symbols within the virtual document
/// 2. **Filters**: MettaPatternFilter for arity-based pattern matching refinement
/// 3. **Fallback**: GlobalVirtualSymbolResolver for cross-document symbol resolution
///
/// ## Resolution Flow
/// 1. First tries local scope lookup via MettaSymbolTable
/// 2. If found, applies MettaPatternFilter to refine by name + arity matching
/// 3. If not found locally (or filter returns empty), falls back to global_virtual_symbols index
/// 4. Returns all matching locations with appropriate confidence levels
pub fn create_metta_adapter(
    symbol_table: Arc<MettaSymbolTable>,
    workspace: Arc<WorkspaceState>,
    _parent_uri: Url,
) -> LanguageAdapter {
    // Create base lexical scope resolver for local symbols
    let base_resolver = Box::new(
        LexicalScopeResolver::new(symbol_table.clone(), "metta".to_string())
    );

    // Create pattern matching filter for arity-based refinement
    // The pattern_matcher is already populated by MettaSymbolTableBuilder
    // It's wrapped in Arc in MettaSymbolTable, so we just clone the Arc (cheap)
    let pattern_filter = Box::new(
        MettaPatternFilter::new(symbol_table.pattern_matcher.clone())
    );

    // Create global cross-document resolver as fallback
    let global_resolver = Box::new(
        GlobalVirtualSymbolResolver::new(workspace)
    );

    // Create composable resolver with local + pattern filter + global resolution
    let resolver: Arc<dyn SymbolResolver> = Arc::new(
        ComposableSymbolResolver::new(
            base_resolver,
            vec![pattern_filter], // Pattern filter refines results by arity matching
            Some(global_resolver), // Global fallback enables cross-document goto_definition
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
        let uri = Url::parse("file:///test.rho#metta:0").unwrap();
        let symbol_table = Arc::new(MettaSymbolTable {
            scopes: vec![],
            all_occurrences: vec![],
            pattern_matcher: Arc::new(crate::ir::metta_pattern_matching::MettaPatternMatcher::new()),
            uri: uri.clone(),
            ir_nodes: vec![],
        });
        let workspace = Arc::new(WorkspaceState {
            documents: Arc::new(DashMap::new()),
            global_symbols: Arc::new(DashMap::new()),
            global_table: Arc::new(tokio::sync::RwLock::new(crate::ir::symbol_table::SymbolTable::new(None))),
            global_inverted_index: Arc::new(DashMap::new()),
            global_contracts: Arc::new(DashMap::new()),
            global_calls: Arc::new(DashMap::new()),
            global_index: Arc::new(std::sync::RwLock::new(crate::ir::global_index::GlobalSymbolIndex::new())),
            global_virtual_symbols: Arc::new(DashMap::new()),
            indexing_state: Arc::new(tokio::sync::RwLock::new(crate::lsp::models::IndexingState::Idle)),
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
