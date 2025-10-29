//! Composable symbol resolver
//!
//! Combines a base resolver with optional filters and fallback resolvers.
//! This is the main entry point for symbol resolution in the LSP.

use tracing::{debug, trace};

use crate::ir::semantic_node::Position;

use super::{
    SymbolResolver, SymbolFilter, SymbolLocation, ResolutionContext, FilterContext,
};

/// Composable symbol resolver that combines multiple resolution strategies
///
/// Resolution flow:
/// 1. Base resolver finds initial candidates (e.g., lexical scope lookup)
/// 2. Each filter refines the candidates (e.g., pattern matching)
/// 3. If filters produce empty result, fall back to unfiltered candidates
/// 4. If base resolver produces empty result, try fallback resolver (e.g., global symbols)
///
/// # Example
/// ```ignore
/// let resolver = ComposableSymbolResolver::new(
///     Box::new(LexicalScopeResolver::new(symbol_table, "metta".to_string())),
///     vec![Box::new(MettaPatternFilter::new(pattern_matcher))],
///     Some(Box::new(GlobalVirtualSymbolResolver::new(workspace))),
/// );
/// ```
pub struct ComposableSymbolResolver {
    /// Base resolver (e.g., lexical scope)
    base_resolver: Box<dyn SymbolResolver>,
    /// Filters to apply (e.g., pattern matching)
    filters: Vec<Box<dyn SymbolFilter>>,
    /// Fallback resolver if base returns empty (e.g., global symbols)
    fallback_resolver: Option<Box<dyn SymbolResolver>>,
}

impl ComposableSymbolResolver {
    /// Create a new composable resolver
    ///
    /// # Arguments
    /// * `base_resolver` - Primary resolver (usually lexical scope)
    /// * `filters` - Optional filters to refine results
    /// * `fallback_resolver` - Optional fallback if base returns nothing
    pub fn new(
        base_resolver: Box<dyn SymbolResolver>,
        filters: Vec<Box<dyn SymbolFilter>>,
        fallback_resolver: Option<Box<dyn SymbolResolver>>,
    ) -> Self {
        Self {
            base_resolver,
            filters,
            fallback_resolver,
        }
    }

    /// Apply filters to candidates, with fallback to unfiltered on empty result
    fn apply_filters(
        &self,
        candidates: Vec<SymbolLocation>,
        filter_context: &FilterContext,
    ) -> Vec<SymbolLocation> {
        if candidates.is_empty() {
            return candidates;
        }

        let mut current = candidates.clone();

        for filter in &self.filters {
            // Skip filter if it doesn't apply to this language
            if !filter.applies_to_language(&filter_context.language) {
                trace!(
                    "Skipping filter '{}' (doesn't apply to language '{}')",
                    filter.name(),
                    filter_context.language
                );
                continue;
            }

            match filter.filter(current.clone(), filter_context) {
                Some(filtered) if !filtered.is_empty() => {
                    debug!(
                        "Filter '{}' refined {} candidates to {}",
                        filter.name(),
                        current.len(),
                        filtered.len()
                    );
                    current = filtered;
                }
                Some(filtered) if filtered.is_empty() => {
                    debug!(
                        "Filter '{}' returned empty, falling back to unfiltered ({} candidates)",
                        filter.name(),
                        candidates.len()
                    );
                    // Filter returned empty - fall back to unfiltered
                    return candidates;
                }
                None => {
                    trace!(
                        "Filter '{}' passed through (returned None)",
                        filter.name()
                    );
                    // Filter chose not to apply - continue with current
                }
                _ => unreachable!(),
            }
        }

        current
    }
}

impl SymbolResolver for ComposableSymbolResolver {
    fn resolve_symbol(
        &self,
        symbol_name: &str,
        position: &Position,
        context: &ResolutionContext,
    ) -> Vec<SymbolLocation> {
        debug!(
            "ComposableSymbolResolver: Resolving '{}' at {:?} in {}",
            symbol_name, position, context.language
        );

        // Try base resolver
        let base_candidates = self.base_resolver.resolve_symbol(symbol_name, position, context);

        debug!(
            "Base resolver '{}' found {} candidates",
            self.base_resolver.name(),
            base_candidates.len()
        );

        if !base_candidates.is_empty() {
            // Apply filters
            let filter_context = FilterContext {
                call_site: context.ir_node.clone(),
                symbol_name: symbol_name.to_string(),
                language: context.language.clone(),
                resolution_context: context.clone(),
            };

            let filtered = self.apply_filters(base_candidates, &filter_context);
            debug!("After filtering: {} candidates", filtered.len());
            return filtered;
        }

        // Base resolver returned nothing - try fallback
        if let Some(ref fallback) = self.fallback_resolver {
            debug!("Base resolver empty, trying fallback '{}'", fallback.name());
            let fallback_candidates = fallback.resolve_symbol(symbol_name, position, context);
            debug!("Fallback found {} candidates", fallback_candidates.len());
            return fallback_candidates;
        }

        debug!("No candidates found (no fallback configured)");
        Vec::new()
    }

    fn supports_language(&self, language: &str) -> bool {
        self.base_resolver.supports_language(language)
            || self.fallback_resolver.as_ref().map_or(false, |f| f.supports_language(language))
    }

    fn name(&self) -> &'static str {
        "ComposableSymbolResolver"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tower_lsp::lsp_types::{Range, Url, Position as LspPosition};

    use crate::ir::symbol_resolution::{SymbolKind, ResolutionConfidence};

    // Mock resolver for testing
    struct MockResolver {
        results: Vec<SymbolLocation>,
        language: String,
    }

    impl SymbolResolver for MockResolver {
        fn resolve_symbol(&self, _: &str, _: &Position, _: &ResolutionContext) -> Vec<SymbolLocation> {
            self.results.clone()
        }

        fn supports_language(&self, language: &str) -> bool {
            self.language == language
        }
    }

    // Mock filter for testing
    struct MockFilter {
        should_filter: bool,
    }

    impl SymbolFilter for MockFilter {
        fn filter(&self, candidates: Vec<SymbolLocation>, _: &FilterContext) -> Option<Vec<SymbolLocation>> {
            if self.should_filter {
                // Return first candidate only
                Some(candidates.into_iter().take(1).collect())
            } else {
                None // Passthrough
            }
        }

        fn applies_to_language(&self, _: &str) -> bool {
            true
        }
    }

    #[test]
    fn test_base_resolver_with_results() {
        let loc = SymbolLocation {
            uri: Url::parse("file:///test.metta").unwrap(),
            range: Range::default(),
            kind: SymbolKind::Function,
            confidence: ResolutionConfidence::Exact,
            metadata: None,
        };

        let base = Box::new(MockResolver {
            results: vec![loc.clone()],
            language: "metta".to_string(),
        });

        let resolver = ComposableSymbolResolver::new(base, vec![], None);

        let context = ResolutionContext {
            uri: Url::parse("file:///test.metta").unwrap(),
            scope_id: Some(0),
            ir_node: None,
            language: "metta".to_string(),
            parent_uri: None,
        };

        let pos = Position { row: 0, column: 0, byte: 0 };
        let results = resolver.resolve_symbol("test", &pos, &context);

        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_filter_refinement() {
        let loc1 = SymbolLocation {
            uri: Url::parse("file:///test.metta").unwrap(),
            range: Range::default(),
            kind: SymbolKind::Function,
            confidence: ResolutionConfidence::Exact,
            metadata: None,
        };
        let loc2 = loc1.clone();

        let base = Box::new(MockResolver {
            results: vec![loc1, loc2],
            language: "metta".to_string(),
        });

        let filter = Box::new(MockFilter { should_filter: true });

        let resolver = ComposableSymbolResolver::new(base, vec![filter], None);

        let context = ResolutionContext {
            uri: Url::parse("file:///test.metta").unwrap(),
            scope_id: Some(0),
            ir_node: None,
            language: "metta".to_string(),
            parent_uri: None,
        };

        let pos = Position { row: 0, column: 0, byte: 0 };
        let results = resolver.resolve_symbol("test", &pos, &context);

        // Filter should reduce 2 candidates to 1
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_fallback_when_base_empty() {
        let base = Box::new(MockResolver {
            results: vec![],
            language: "metta".to_string(),
        });

        let loc = SymbolLocation {
            uri: Url::parse("file:///fallback.metta").unwrap(),
            range: Range::default(),
            kind: SymbolKind::Function,
            confidence: ResolutionConfidence::Fuzzy,
            metadata: None,
        };

        let fallback = Box::new(MockResolver {
            results: vec![loc],
            language: "metta".to_string(),
        });

        let resolver = ComposableSymbolResolver::new(base, vec![], Some(fallback));

        let context = ResolutionContext {
            uri: Url::parse("file:///test.metta").unwrap(),
            scope_id: Some(0),
            ir_node: None,
            language: "metta".to_string(),
            parent_uri: None,
        };

        let pos = Position { row: 0, column: 0, byte: 0 };
        let results = resolver.resolve_symbol("test", &pos, &context);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].uri.path(), "/fallback.metta");
    }
}
