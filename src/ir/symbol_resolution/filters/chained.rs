//! Chained filter combinator
//!
//! Combines multiple filters into a single filter that applies them in sequence.

use tracing::debug;

use crate::ir::symbol_resolution::{SymbolFilter, SymbolLocation, FilterContext};

/// Combines multiple filters into a single filter
///
/// Filters are applied in order. Each filter receives the output of the previous filter.
/// If any filter returns None (passthrough), it's skipped.
/// If any filter returns an empty Vec, the chain returns the input (fallback to unfiltered).
pub struct ChainedFilter {
    filters: Vec<Box<dyn SymbolFilter>>,
}

impl ChainedFilter {
    /// Create a new chained filter
    pub fn new(filters: Vec<Box<dyn SymbolFilter>>) -> Self {
        Self { filters }
    }
}

impl SymbolFilter for ChainedFilter {
    fn filter(
        &self,
        mut candidates: Vec<SymbolLocation>,
        context: &FilterContext,
    ) -> Option<Vec<SymbolLocation>> {
        if self.filters.is_empty() {
            return None; // No filters - passthrough
        }

        let original_count = candidates.len();
        let mut applied_any = false;

        for filter in &self.filters {
            if !filter.applies_to_language(&context.language) {
                continue;
            }

            match filter.filter(candidates.clone(), context) {
                Some(filtered) if !filtered.is_empty() => {
                    debug!(
                        "ChainedFilter: '{}' refined {} -> {}",
                        filter.name(),
                        candidates.len(),
                        filtered.len()
                    );
                    candidates = filtered;
                    applied_any = true;
                }
                Some(_empty) => {
                    // Filter returned empty - abort chain and return original
                    debug!(
                        "ChainedFilter: '{}' returned empty, aborting chain",
                        filter.name()
                    );
                    return Some(candidates); // Return pre-filter candidates
                }
                None => {
                    // Passthrough - continue with current candidates
                    debug!("ChainedFilter: '{}' passed through", filter.name());
                }
            }
        }

        if applied_any {
            debug!(
                "ChainedFilter: Applied filters, {} -> {} candidates",
                original_count,
                candidates.len()
            );
            Some(candidates)
        } else {
            None // No filters were applied - passthrough
        }
    }

    fn applies_to_language(&self, language: &str) -> bool {
        // Chain applies if any of its filters apply
        self.filters.iter().any(|f| f.applies_to_language(language))
    }

    fn name(&self) -> &'static str {
        "ChainedFilter"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp::lsp_types::{Range, Url};
    use crate::ir::symbol_resolution::{SymbolKind, ResolutionConfidence, ResolutionContext};

    // Mock filter that removes first element
    struct RemoveFirstFilter;
    impl SymbolFilter for RemoveFirstFilter {
        fn filter(&self, mut candidates: Vec<SymbolLocation>, _: &FilterContext) -> Option<Vec<SymbolLocation>> {
            if !candidates.is_empty() {
                candidates.remove(0);
            }
            Some(candidates)
        }
        fn applies_to_language(&self, _: &str) -> bool { true }
    }

    // Mock filter that passes through
    struct PassthroughFilter;
    impl SymbolFilter for PassthroughFilter {
        fn filter(&self, _: Vec<SymbolLocation>, _: &FilterContext) -> Option<Vec<SymbolLocation>> {
            None
        }
        fn applies_to_language(&self, _: &str) -> bool { true }
    }

    #[test]
    fn test_chained_filter_sequence() {
        let filters: Vec<Box<dyn SymbolFilter>> = vec![
            Box::new(RemoveFirstFilter),
            Box::new(RemoveFirstFilter),
        ];

        let chain = ChainedFilter::new(filters);

        let locs = vec![
            SymbolLocation {
                uri: Url::parse("file:///a").unwrap(),
                range: Range::default(),
                kind: SymbolKind::Function,
                confidence: ResolutionConfidence::Exact,
                metadata: None,
            },
            SymbolLocation {
                uri: Url::parse("file:///b").unwrap(),
                range: Range::default(),
                kind: SymbolKind::Function,
                confidence: ResolutionConfidence::Exact,
                metadata: None,
            },
            SymbolLocation {
                uri: Url::parse("file:///c").unwrap(),
                range: Range::default(),
                kind: SymbolKind::Function,
                confidence: ResolutionConfidence::Exact,
                metadata: None,
            },
        ];

        let context = FilterContext {
            call_site: None,
            symbol_name: "test".to_string(),
            language: "test".to_string(),
            resolution_context: ResolutionContext {
                uri: Url::parse("file:///test").unwrap(),
                scope_id: None,
                ir_node: None,
                language: "test".to_string(),
                parent_uri: None,
            },
        };

        let result = chain.filter(locs, &context);

        // Should remove 2 elements, leaving 1
        assert_eq!(result.unwrap().len(), 1);
    }

    #[test]
    fn test_chained_filter_with_passthrough() {
        let filters: Vec<Box<dyn SymbolFilter>> = vec![
            Box::new(PassthroughFilter),
            Box::new(RemoveFirstFilter),
        ];

        let chain = ChainedFilter::new(filters);

        let locs = vec![
            SymbolLocation {
                uri: Url::parse("file:///a").unwrap(),
                range: Range::default(),
                kind: SymbolKind::Function,
                confidence: ResolutionConfidence::Exact,
                metadata: None,
            },
            SymbolLocation {
                uri: Url::parse("file:///b").unwrap(),
                range: Range::default(),
                kind: SymbolKind::Function,
                confidence: ResolutionConfidence::Exact,
                metadata: None,
            },
        ];

        let context = FilterContext {
            call_site: None,
            symbol_name: "test".to_string(),
            language: "test".to_string(),
            resolution_context: ResolutionContext {
                uri: Url::parse("file:///test").unwrap(),
                scope_id: None,
                ir_node: None,
                language: "test".to_string(),
                parent_uri: None,
            },
        };

        let result = chain.filter(locs, &context);

        // Passthrough is ignored, second filter removes one element
        assert_eq!(result.unwrap().len(), 1);
    }
}
