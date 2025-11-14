//! MeTTa language adapter
//!
//! Provides MeTTa-specific implementations of LSP features using the
//! unified LanguageAdapter architecture.

use std::sync::Arc;
use tower_lsp::lsp_types::{
    HoverContents, CompletionItem, Documentation, MarkupContent, MarkupKind,
    CompletionItemKind, Url, GotoDefinitionResponse, Location, Range,
    Position as LspPosition,
};
use tracing::debug;

use crate::lsp::features::traits::{
    LanguageAdapter, HoverProvider, CompletionProvider, DocumentationProvider,
    HoverContext, CompletionContext, DocumentationContext, GotoDefinitionProvider,
    GotoDefinitionContext,
};
use crate::ir::semantic_node::{SemanticNode, Position};
use crate::ir::symbol_resolution::{
    SymbolResolver,
    lexical_scope::LexicalScopeResolver,
    composable::ComposableSymbolResolver,
    GlobalVirtualSymbolResolver,
    filters::MettaPatternFilter,
    ResolutionContext,
};
use crate::ir::transforms::metta_symbol_table_builder::MettaSymbolTable;
use crate::ir::metta_node::MettaNode;
use crate::lsp::models::WorkspaceState;
use crate::lsp::features::node_finder::find_node_at_position_with_prev_end;

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

    fn hover_for_language_specific(
        &self,
        node: &dyn SemanticNode,
        _context: &HoverContext,
    ) -> Option<HoverContents> {
        use crate::ir::metta_node::MettaNode;

        // Try to downcast to MettaNode and extract symbol name
        if let Some(metta_node) = node.as_any().downcast_ref::<MettaNode>() {
            match metta_node {
                MettaNode::Atom { name, .. } => {
                    let content = format!("**{}**\n\n*MeTTa atom*", name);
                    return Some(HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: content,
                    }));
                }
                MettaNode::Variable { name, .. } => {
                    let content = format!("**{}**\n\n*MeTTa variable*", name);
                    return Some(HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: content,
                    }));
                }
                _ => {}
            }
        }
        None
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

/// MeTTa-specific goto-definition provider
///
/// Handles MeTTa's multi-root IR structure with proper position tracking.
/// Uses the composable resolver with lexical scoping, pattern filtering,
/// and cross-document resolution.
pub struct MettaGotoDefinitionProvider {
    resolver: Arc<dyn SymbolResolver>,
    symbol_table: Arc<MettaSymbolTable>,
}

impl MettaGotoDefinitionProvider {
    pub fn new(resolver: Arc<dyn SymbolResolver>, symbol_table: Arc<MettaSymbolTable>) -> Self {
        Self {
            resolver,
            symbol_table,
        }
    }

    /// Extract symbol name from a MeTTa node
    fn extract_symbol_name<'a>(&self, node: &'a dyn SemanticNode) -> Option<&'a str> {
        if let Some(metta_node) = node.as_any().downcast_ref::<MettaNode>() {
            match metta_node {
                MettaNode::Atom { name, .. } => {
                    debug!("MettaGotoDefinition: Extracted symbol from Atom: {}", name);
                    return Some(name.as_str());
                }
                MettaNode::Variable { name, .. } => {
                    debug!("MettaGotoDefinition: Extracted symbol from Variable: {}", name);
                    return Some(name.as_str());
                }
                _ => {
                    debug!("MettaGotoDefinition: Node is not Atom or Variable: {:?}", metta_node);
                }
            }
        }
        None
    }
}

#[async_trait::async_trait]
impl GotoDefinitionProvider for MettaGotoDefinitionProvider {
    async fn goto_definition(
        &self,
        context: &GotoDefinitionContext,
    ) -> Option<GotoDefinitionResponse> {
        debug!(
            "MettaGotoDefinition: Starting goto_definition at position {:?}",
            context.ir_position
        );

        // Iterate through all top-level nodes to find one containing the position
        let mut prev_end = Position { row: 0, column: 0, byte: 0 };

        for (i, root) in context.all_roots.iter().enumerate() {
            let node = find_node_at_position_with_prev_end(
                root.as_ref(),
                &context.ir_position,
                &prev_end,
            );

            if let Some(node) = node {
                debug!(
                    "MettaGotoDefinition: Found node in root {} at position {:?}",
                    i, context.ir_position
                );

                // Extract symbol name from the node
                let symbol_name = match self.extract_symbol_name(node) {
                    Some(name) => {
                        debug!("MettaGotoDefinition: Extracted symbol name: '{}'", name);
                        name
                    }
                    None => {
                        debug!(
                            "MettaGotoDefinition: Failed to extract symbol name from node type={}",
                            node.type_name()
                        );
                        // Update prev_end and continue to next root
                        prev_end = root.base().end();
                        continue;
                    }
                };

                // Find the symbol at position to get scope_id
                let lsp_pos = LspPosition {
                    line: context.ir_position.row as u32,
                    character: context.ir_position.column as u32,
                };

                let symbol = self.symbol_table.find_symbol_at_position(&lsp_pos);
                let scope_id = symbol.map(|s| s.scope_id);

                debug!(
                    "MettaGotoDefinition: Symbol '{}' has scope_id={:?}",
                    symbol_name, scope_id
                );

                // Build resolution context
                let res_context = ResolutionContext {
                    uri: context.uri.clone(),
                    scope_id,
                    ir_node: None, // We can't pass the node due to Send + Sync constraints
                    language: "metta".to_string(),
                    parent_uri: context.parent_uri.clone(),
                };

                // Use the resolver to find definitions
                let locations = self.resolver.resolve_symbol(
                    symbol_name,
                    &context.ir_position,
                    &res_context,
                );

                debug!(
                    "MettaGotoDefinition: Resolver found {} location(s) for '{}'",
                    locations.len(),
                    symbol_name
                );

                if !locations.is_empty() {
                    // Convert SymbolLocation to LSP Location
                    let lsp_locations: Vec<Location> = locations
                        .into_iter()
                        .map(|loc| Location {
                            uri: loc.uri,
                            range: loc.range,
                        })
                        .collect();

                    return Some(GotoDefinitionResponse::Array(lsp_locations));
                }
            }

            // Update prev_end for next root
            prev_end = root.base().end();
        }

        debug!(
            "MettaGotoDefinition: No definition found at position {:?}",
            context.ir_position
        );
        None
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

    // Create specialized goto-definition provider for multi-root support
    let goto_definition = Arc::new(
        MettaGotoDefinitionProvider::new(resolver.clone(), symbol_table.clone())
    );

    // Bundle into adapter
    let mut adapter = LanguageAdapter::new(
        "metta",
        resolver,
        hover,
        completion,
        documentation,
    );

    // Set the specialized goto-definition provider
    adapter.goto_definition = Some(goto_definition);

    adapter
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
            global_table: Arc::new(tokio::sync::RwLock::new(crate::ir::symbol_table::SymbolTable::new(None))),
            // REMOVED (Priority 2b): global_inverted_index
            global_contracts: Arc::new(DashMap::new()),
            global_calls: Arc::new(DashMap::new()),
            global_index: Arc::new(std::sync::RwLock::new(crate::ir::global_index::GlobalSymbolIndex::new())),
            global_virtual_symbols: Arc::new(DashMap::new()),
            rholang_symbols: Arc::new(crate::lsp::rholang_contracts::RholangContracts::new()),
            indexing_state: Arc::new(tokio::sync::RwLock::new(crate::lsp::models::IndexingState::Idle)),
            completion_index: Arc::new(crate::lsp::features::completion::WorkspaceCompletionIndex::new()),
            file_modification_tracker: Arc::new(crate::lsp::backend::file_modification_tracker::FileModificationTracker::new_for_testing()),
            dependency_graph: Arc::new(crate::lsp::backend::dependency_graph::DependencyGraph::new()),
            document_cache: Arc::new(crate::lsp::backend::document_cache::DocumentCache::new()),
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
