//! Global symbol resolver
//!
//! Resolves symbols using the workspace-wide global_virtual_symbols index.
//! This is typically used as a fallback when local/lexical scope resolution fails.

use std::sync::Arc;
use tokio::sync::RwLock;
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
pub struct GlobalVirtualSymbolResolver {
    workspace: Arc<RwLock<WorkspaceState>>,
}

impl GlobalVirtualSymbolResolver {
    /// Create a new global symbol resolver
    pub fn new(workspace: Arc<RwLock<WorkspaceState>>) -> Self {
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

        // This is an async function being called from sync context
        // We need to use block_on or similar - for now, return empty
        // In practice, this will be called from async LSP handlers
        // TODO: Make SymbolResolver trait async or use blocking read

        // For now, we'll document that this should be called from async context
        // and the caller should handle the async read
        debug!("GlobalVirtualSymbolResolver: Requires async context (placeholder)");

        Vec::new()
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
pub struct AsyncGlobalVirtualSymbolResolver {
    workspace: Arc<RwLock<WorkspaceState>>,
}

impl AsyncGlobalVirtualSymbolResolver {
    /// Create a new async global resolver
    pub fn new(workspace: Arc<RwLock<WorkspaceState>>) -> Self {
        Self { workspace }
    }

    /// Resolve symbol asynchronously
    pub async fn resolve_symbol_async(
        &self,
        symbol_name: &str,
        context: &ResolutionContext,
    ) -> Vec<SymbolLocation> {
        debug!(
            "AsyncGlobalVirtualSymbolResolver: Looking up '{}' in language '{}'",
            symbol_name, context.language
        );

        let workspace = self.workspace.read().await;

        // Look up in global_virtual_symbols
        let locations: Vec<SymbolLocation> = workspace
            .global_virtual_symbols
            .get(&context.language)
            .and_then(|lang_symbols| lang_symbols.get(symbol_name))
            .map(|locs| {
                locs.iter()
                    .map(|(uri, range)| SymbolLocation {
                        uri: uri.clone(),
                        range: *range,
                        kind: SymbolKind::Function, // Assume function for now
                        confidence: ResolutionConfidence::Fuzzy, // Cross-document is less certain
                        metadata: None,
                    })
                    .collect()
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
        // Create a workspace with some global symbols
        let mut global_virtual_symbols = HashMap::new();
        let mut metta_symbols = HashMap::new();

        let range = Range {
            start: LspPosition { line: 0, character: 0 },
            end: LspPosition { line: 0, character: 10 },
        };

        metta_symbols.insert(
            "test_symbol".to_string(),
            vec![(Url::parse("file:///test.metta#vdoc:0").unwrap(), range)],
        );

        global_virtual_symbols.insert("metta".to_string(), metta_symbols);

        let workspace = Arc::new(RwLock::new(WorkspaceState {
            documents: HashMap::new(),
            global_symbols: HashMap::new(),
            global_table: Arc::new(SymbolTable::new(None)),
            global_inverted_index: HashMap::new(),
            global_contracts: Vec::new(),
            global_calls: Vec::new(),
            global_index: Arc::new(std::sync::RwLock::new(GlobalSymbolIndex::new())),
            global_virtual_symbols,
        }));

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
