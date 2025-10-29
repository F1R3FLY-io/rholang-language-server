//! Global symbol resolver
//!
//! Resolves symbols using the workspace-wide global_virtual_symbols index.
//! This is typically used as a fallback when local/lexical scope resolution fails.

use std::sync::Arc;
use tracing::debug;

use tower_lsp::lsp_types::{Range, Url};

use crate::ir::semantic_node::Position;
use crate::lsp::models::WorkspaceState;

use super::{
    SymbolResolver, SymbolLocation, ResolutionContext, ResolutionConfidence, SymbolKind,
};

/// Resolves symbols using global_virtual_symbols from WorkspaceState
///
/// This resolver searches the workspace-wide symbol index for cross-document references.
/// It's typically used as a fallback when local scope resolution doesn't find anything.
///
/// Phase 1 Optimization: Now uses lock-free DashMap access
pub struct GlobalVirtualSymbolResolver {
    workspace: Arc<WorkspaceState>,
}

impl GlobalVirtualSymbolResolver {
    /// Create a new global symbol resolver
    pub fn new(workspace: Arc<WorkspaceState>) -> Self {
        Self { workspace }
    }
}

impl SymbolResolver for GlobalVirtualSymbolResolver {
    fn resolve_symbol(
        &self,
        symbol_name: &str,
        _position: &Position,
        context: &ResolutionContext,
    ) -> Vec<SymbolLocation> {
        debug!(
            "GlobalVirtualSymbolResolver: Looking up '{}' in language '{}'",
            symbol_name, context.language
        );

        // Lock-free access via DashMap
        let locations: Vec<SymbolLocation> = self.workspace
            .global_virtual_symbols
            .get(&context.language)
            .and_then(|lang_symbols_entry| {
                let lang_symbols = lang_symbols_entry.value();
                lang_symbols.get(symbol_name).map(|locs_entry| {
                    locs_entry.value().iter()
                        .map(|(uri, range)| SymbolLocation {
                            uri: uri.clone(),
                            range: *range,
                            kind: SymbolKind::Function,
                            confidence: ResolutionConfidence::Fuzzy,
                            metadata: None,
                        })
                        .collect()
                })
            })
            .unwrap_or_default();

        debug!(
            "GlobalVirtualSymbolResolver: Found {} locations for '{}'",
            locations.len(),
            symbol_name
        );

        locations
    }

    fn supports_language(&self, _language: &str) -> bool {
        // Global resolver supports all languages
        true
    }

    fn name(&self) -> &'static str {
        "GlobalVirtualSymbolResolver"
    }
}

/// Async-friendly version of global symbol resolver
///
/// This resolver can be used directly from async LSP handlers
///
/// Phase 1 Optimization: No longer needs RwLock wrapper as WorkspaceState
/// uses lock-free DashMap internally for concurrent access
pub struct AsyncGlobalVirtualSymbolResolver {
    workspace: Arc<WorkspaceState>,
}

impl AsyncGlobalVirtualSymbolResolver {
    /// Create a new async global resolver
    pub fn new(workspace: Arc<WorkspaceState>) -> Self {
        Self { workspace }
    }

    /// Resolve symbol asynchronously using lock-free lookups
    pub async fn resolve_symbol_async(
        &self,
        symbol_name: &str,
        context: &ResolutionContext,
    ) -> Vec<SymbolLocation> {
        debug!(
            "AsyncGlobalVirtualSymbolResolver: Looking up '{}' in language '{}'",
            symbol_name, context.language
        );

        // No lock needed - DashMap provides lock-free access
        let locations: Vec<SymbolLocation> = self.workspace
            .global_virtual_symbols
            .get(&context.language)
            .and_then(|lang_symbols_entry| {
                let lang_symbols = lang_symbols_entry.value();
                lang_symbols.get(symbol_name).map(|locs_entry| {
                    locs_entry.value().iter()
                        .map(|(uri, range)| SymbolLocation {
                            uri: uri.clone(),
                            range: *range,
                            kind: SymbolKind::Function, // Assume function for now
                            confidence: ResolutionConfidence::Fuzzy, // Cross-document is less certain
                            metadata: None,
                        })
                        .collect()
                })
            })
            .unwrap_or_default();

        debug!(
            "AsyncGlobalVirtualSymbolResolver: Found {} locations for '{}'",
            locations.len(),
            symbol_name
        );

        locations
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tower_lsp::lsp_types::Position as LspPosition;

    use crate::ir::symbol_table::SymbolTable;
    use crate::ir::global_index::GlobalSymbolIndex;

    #[tokio::test]
    async fn test_async_global_resolver() {
        use dashmap::DashMap;

        // Create a workspace with some global symbols using new DashMap structure
        let range = Range {
            start: LspPosition { line: 0, character: 0 },
            end: LspPosition { line: 0, character: 10 },
        };

        // Build the nested DashMap structure
        let global_virtual_symbols = Arc::new(DashMap::new());
        let metta_symbols = Arc::new(DashMap::new());
        metta_symbols.insert(
            "test_symbol".to_string(),
            vec![(Url::parse("file:///test.metta#vdoc:0").unwrap(), range)],
        );
        global_virtual_symbols.insert("metta".to_string(), metta_symbols);

        let workspace = Arc::new(WorkspaceState {
            documents: Arc::new(DashMap::new()),
            global_symbols: Arc::new(DashMap::new()),
            global_table: Arc::new(tokio::sync::RwLock::new(SymbolTable::new(None))),
            global_inverted_index: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            global_contracts: Arc::new(DashMap::new()),
            global_calls: Arc::new(DashMap::new()),
            global_index: Arc::new(std::sync::RwLock::new(GlobalSymbolIndex::new())),
            global_virtual_symbols,
            indexing_state: Arc::new(tokio::sync::RwLock::new(crate::lsp::models::IndexingState::Idle)),
        });

        let resolver = AsyncGlobalVirtualSymbolResolver::new(workspace);

        let context = ResolutionContext {
            uri: Url::parse("file:///query.metta").unwrap(),
            scope_id: None,
            ir_node: None,
            language: "metta".to_string(),
            parent_uri: None,
        };

        let results = resolver.resolve_symbol_async("test_symbol", &context).await;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].uri.path(), "/test.metta");
    }
}
