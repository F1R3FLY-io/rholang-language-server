//! Generic global scope symbol resolver
//!
//! Provides language-agnostic symbol resolution using a single global scope.
//! Supports multiple declarations and definitions per symbol, with cross-document linking.

use std::sync::Arc;

use tower_lsp::lsp_types::Range;
use tracing::debug;

use crate::ir::semantic_node::Position;
use crate::lsp::models::WorkspaceState;

use super::{
    SymbolResolver, SymbolLocation, ResolutionContext, ResolutionConfidence, SymbolKind,
};

/// Generic symbol resolver using global scope
///
/// This resolver implements a flat, global scope model suitable for generic
/// language support. Key features:
///
/// - **Single Global Scope**: No lexical hierarchy - all symbols in one namespace
/// - **Multiple Locations**: Symbols can have many declarations and definitions
/// - **Cross-Document**: Automatically links symbols across document boundaries
/// - **Language-Agnostic**: Works for any language without language-specific logic
///
/// # Resolution Strategy
///
/// 1. Query `workspace.global_virtual_symbols[language][symbol_name]`
/// 2. Return ALL matching locations (no filtering)
/// 3. Each location has `ResolutionConfidence::Exact`
///
/// # Usage
///
/// This resolver is used as the default for `LanguageContext::Other` and can
/// be composed with language-specific resolvers via `ComposableSymbolResolver`.
///
/// ```ignore
/// let resolver = GenericSymbolResolver::new(workspace.clone(), "python".to_string());
/// let locations = resolver.resolve_symbol(&symbol_name, &position, &context);
/// ```
pub struct GenericSymbolResolver {
    /// Workspace containing global symbol index
    workspace: Arc<WorkspaceState>,
    /// Language this resolver handles
    language: String,
}

impl GenericSymbolResolver {
    /// Create a new generic symbol resolver
    ///
    /// # Arguments
    /// * `workspace` - Workspace state with global_virtual_symbols index
    /// * `language` - Language identifier (e.g., "python", "javascript")
    pub fn new(workspace: Arc<WorkspaceState>, language: String) -> Self {
        Self {
            workspace,
            language,
        }
    }

    /// Query global symbol index for all matching locations
    fn lookup_global_symbols(&self, symbol_name: &str) -> Vec<SymbolLocation> {
        let mut locations = Vec::new();

        // Query global_virtual_symbols[language][symbol_name]
        if let Some(language_symbols) = self.workspace.global_virtual_symbols.get(&self.language) {
            if let Some(symbol_locations) = language_symbols.get(symbol_name) {
                debug!(
                    "GenericSymbolResolver: Found {} locations for '{}' in language '{}'",
                    symbol_locations.len(),
                    symbol_name,
                    self.language
                );

                // Return all locations - no filtering in global scope
                for (uri, range) in symbol_locations.iter() {
                    locations.push(SymbolLocation {
                        uri: uri.clone(),
                        range: *range,
                        kind: SymbolKind::Variable, // Generic - can't determine specific kind
                        confidence: ResolutionConfidence::Exact,
                        metadata: None,
                    });
                }
            } else {
                debug!(
                    "GenericSymbolResolver: Symbol '{}' not found in language '{}'",
                    symbol_name, self.language
                );
            }
        } else {
            debug!(
                "GenericSymbolResolver: Language '{}' not found in global_virtual_symbols",
                self.language
            );
        }

        locations
    }
}

impl SymbolResolver for GenericSymbolResolver {
    fn resolve_symbol(
        &self,
        symbol_name: &str,
        _position: &Position,
        context: &ResolutionContext,
    ) -> Vec<SymbolLocation> {
        debug!(
            "GenericSymbolResolver: Resolving '{}' (language={}, uri={})",
            symbol_name, context.language, context.uri
        );

        // Global scope lookup - position doesn't matter
        self.lookup_global_symbols(symbol_name)
    }

    fn supports_language(&self, language: &str) -> bool {
        self.language == language
    }

    fn name(&self) -> &'static str {
        "GenericSymbolResolver"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashmap::DashMap;
    use tower_lsp::lsp_types::{Position as LspPosition, Url};

    #[test]
    fn test_generic_resolver_single_location() {
        // Setup workspace with global_virtual_symbols
        let workspace = Arc::new(WorkspaceState {
            documents: Arc::new(DashMap::new()),
            // REMOVED (Priority 2b): global_symbols, global_inverted_index
            global_table: Arc::new(tokio::sync::RwLock::new(
                crate::ir::symbol_table::SymbolTable::new(None),
            )),
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
            file_modification_tracker: Arc::new(crate::lsp::backend::file_modification_tracker::FileModificationTracker::new_for_testing()),
            dependency_graph: Arc::new(crate::lsp::backend::dependency_graph::DependencyGraph::new()),
            document_cache: Arc::new(crate::lsp::backend::document_cache::DocumentCache::new()),
        });

        // Add test symbol
        let python_symbols = Arc::new(DashMap::new());
        let uri = Url::parse("file:///test.py").unwrap();
        let range = Range {
            start: LspPosition {
                line: 5,
                character: 10,
            },
            end: LspPosition {
                line: 5,
                character: 15,
            },
        };
        python_symbols.insert("my_var".to_string(), vec![(uri.clone(), range)]);
        workspace
            .global_virtual_symbols
            .insert("python".to_string(), python_symbols);

        // Create resolver and test
        let resolver = GenericSymbolResolver::new(workspace, "python".to_string());

        let context = ResolutionContext {
            uri: uri.clone(),
            scope_id: None,
            ir_node: None,
            language: "python".to_string(),
            parent_uri: None,
        };

        let position = Position {
            row: 5,
            column: 10,
            byte: 0,
        };

        let locations = resolver.resolve_symbol("my_var", &position, &context);

        assert_eq!(locations.len(), 1);
        assert_eq!(locations[0].uri, uri);
        assert_eq!(locations[0].range, range);
    }

    #[test]
    fn test_generic_resolver_multiple_locations() {
        let workspace = Arc::new(WorkspaceState {
            documents: Arc::new(DashMap::new()),
            // REMOVED (Priority 2b): global_symbols, global_inverted_index
            global_table: Arc::new(tokio::sync::RwLock::new(
                crate::ir::symbol_table::SymbolTable::new(None),
            )),
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
            file_modification_tracker: Arc::new(crate::lsp::backend::file_modification_tracker::FileModificationTracker::new_for_testing()),
            dependency_graph: Arc::new(crate::lsp::backend::dependency_graph::DependencyGraph::new()),
            document_cache: Arc::new(crate::lsp::backend::document_cache::DocumentCache::new()),
        });

        // Add multiple locations for same symbol
        let js_symbols = Arc::new(DashMap::new());
        let uri1 = Url::parse("file:///module1.js").unwrap();
        let uri2 = Url::parse("file:///module2.js").unwrap();
        let range1 = Range {
            start: LspPosition {
                line: 1,
                character: 0,
            },
            end: LspPosition {
                line: 1,
                character: 5,
            },
        };
        let range2 = Range {
            start: LspPosition {
                line: 10,
                character: 5,
            },
            end: LspPosition {
                line: 10,
                character: 10,
            },
        };
        js_symbols.insert(
            "myFunc".to_string(),
            vec![(uri1.clone(), range1), (uri2.clone(), range2)],
        );
        workspace
            .global_virtual_symbols
            .insert("javascript".to_string(), js_symbols);

        let resolver = GenericSymbolResolver::new(workspace, "javascript".to_string());

        let context = ResolutionContext {
            uri: uri1.clone(),
            scope_id: None,
            ir_node: None,
            language: "javascript".to_string(),
            parent_uri: None,
        };

        let position = Position {
            row: 1,
            column: 0,
            byte: 0,
        };

        let locations = resolver.resolve_symbol("myFunc", &position, &context);

        // Should return ALL locations (multiple declarations/definitions)
        assert_eq!(locations.len(), 2);
        assert!(locations.iter().any(|loc| loc.uri == uri1 && loc.range == range1));
        assert!(locations.iter().any(|loc| loc.uri == uri2 && loc.range == range2));
    }

    #[test]
    fn test_generic_resolver_symbol_not_found() {
        let workspace = Arc::new(WorkspaceState {
            documents: Arc::new(DashMap::new()),
            // REMOVED (Priority 2b): global_symbols, global_inverted_index
            global_table: Arc::new(tokio::sync::RwLock::new(
                crate::ir::symbol_table::SymbolTable::new(None),
            )),
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
            file_modification_tracker: Arc::new(crate::lsp::backend::file_modification_tracker::FileModificationTracker::new_for_testing()),
            dependency_graph: Arc::new(crate::lsp::backend::dependency_graph::DependencyGraph::new()),
            document_cache: Arc::new(crate::lsp::backend::document_cache::DocumentCache::new()),
        });

        let resolver = GenericSymbolResolver::new(workspace, "ruby".to_string());

        let context = ResolutionContext {
            uri: Url::parse("file:///test.rb").unwrap(),
            scope_id: None,
            ir_node: None,
            language: "ruby".to_string(),
            parent_uri: None,
        };

        let position = Position {
            row: 1,
            column: 0,
            byte: 0,
        };

        let locations = resolver.resolve_symbol("undefined_var", &position, &context);

        assert_eq!(locations.len(), 0);
    }
}
