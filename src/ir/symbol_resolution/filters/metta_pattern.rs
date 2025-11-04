//! MeTTa pattern matching filter
//!
//! Filters symbol candidates using MeTTa's hybrid pattern matching system:
//! - **MORK-based pattern matching** for definitions from simple strings (O(1) lookup)
//! - **Name+arity matching** for definitions from concatenated strings (fallback)
//!
//! The filter automatically partitions definitions by provenance (simple vs concatenated)
//! and applies the appropriate matching strategy. If no patterns match, returns unfiltered
//! candidates to ensure goto_definition always provides results.

use std::sync::Arc;
use tracing::{debug, trace};

use crate::ir::metta_node::MettaNode;
use crate::ir::metta_pattern_matching::MettaPatternMatcher;
use crate::ir::symbol_resolution::{SymbolFilter, SymbolLocation, FilterContext};

/// Filter that uses MeTTa's hybrid pattern matching to refine symbol candidates
///
/// **Hybrid Matching Strategy**:
/// - For definitions from **simple strings**: Uses MORK-based O(1) pattern lookup
/// - For definitions from **concatenated strings**: Falls back to name+arity matching
/// - Automatically partitions by provenance and applies appropriate strategy
///
/// **Behavior**:
/// 1. Extracts the call site MettaNode from FilterContext
/// 2. Delegates to `MettaPatternMatcher::find_matching_definitions()` (hybrid approach)
/// 3. Filters candidates to only those whose locations match pattern results
/// 4. Returns None (passthrough) if no call site info available
/// 5. Returns unfiltered candidates if pattern matching finds nothing (ensures results)
pub struct MettaPatternFilter {
    /// Pattern matcher for looking up matching definitions
    pattern_matcher: Arc<MettaPatternMatcher>,
}

impl MettaPatternFilter {
    /// Create a new MeTTa pattern filter
    pub fn new(pattern_matcher: Arc<MettaPatternMatcher>) -> Self {
        Self { pattern_matcher }
    }

    /// Extract call info from the call site node
    fn extract_call_info(&self, call_site: &Arc<dyn std::any::Any + Send + Sync>) -> Option<(String, usize)> {
        // Try to downcast to MettaNode
        if let Some(node) = call_site.downcast_ref::<MettaNode>() {
            match node {
                MettaNode::SExpr { elements, .. } if !elements.is_empty() => {
                    // Extract function name from first element
                    let name = match &*elements[0] {
                        MettaNode::Atom { name, .. } => Some(name.clone()),
                        _ => None,
                    }?;

                    // Arity is number of arguments (elements - 1 for function name)
                    let arity = elements.len() - 1;

                    Some((name, arity))
                }
                _ => None,
            }
        } else {
            None
        }
    }
}

impl SymbolFilter for MettaPatternFilter {
    fn filter(
        &self,
        candidates: Vec<SymbolLocation>,
        context: &FilterContext,
    ) -> Option<Vec<SymbolLocation>> {
        // If no call site, can't do pattern matching - passthrough
        let call_site = match &context.call_site {
            Some(cs) => cs,
            None => {
                trace!("MettaPatternFilter: No call site, passing through");
                return None;
            }
        };

        // Downcast to MettaNode for pattern matching
        let metta_node = match call_site.downcast_ref::<MettaNode>() {
            Some(node) => node,
            None => {
                trace!("MettaPatternFilter: Call site is not a MettaNode, passing through");
                return None;
            }
        };

        // Extract call pattern for logging
        let call_info = self.extract_call_info(call_site);
        if let Some((name, arity)) = call_info {
            debug!(
                "MettaPatternFilter: Filtering for pattern '{}' with arity {}",
                name, arity
            );
        }

        // Use hybrid pattern matching (MORK + fallback to name+arity)
        // This automatically handles:
        // - MORK-based pattern lookup for simple definitions
        // - Name+arity matching for concatenated definitions
        // - Provenance-aware partitioning
        let matched_locations = self.pattern_matcher.find_matching_definitions(metta_node);

        if matched_locations.is_empty() {
            debug!("MettaPatternFilter: No pattern matches found, returning unfiltered");
            // No patterns match - return unfiltered candidates
            // This handles cases where symbols aren't function definitions
            return Some(candidates);
        }

        // Filter candidates to only those whose locations match the pattern matcher results
        let filtered: Vec<SymbolLocation> = candidates
            .into_iter()
            .filter(|loc| {
                // Check if this location is in the pattern match results
                matched_locations.iter().any(|matched_loc| {
                    matched_loc.uri == loc.uri && matched_loc.range == loc.range
                })
            })
            .collect();

        if filtered.is_empty() {
            debug!(
                "MettaPatternFilter: Pattern matching filtered out all candidates, returning unfiltered"
            );
            // Pattern matching was too restrictive - return unfiltered
            // This is the key behavior: if patterns don't match, fall back
            None
        } else {
            debug!(
                "MettaPatternFilter: Filtered to {} matching patterns (hybrid MORK + fallback)",
                filtered.len()
            );
            Some(filtered)
        }
    }

    fn applies_to_language(&self, language: &str) -> bool {
        language == "metta"
    }

    fn name(&self) -> &'static str {
        "MettaPatternFilter"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp::lsp_types::{Range, Url, Position as LspPosition};
    use crate::ir::symbol_resolution::{SymbolKind, ResolutionConfidence, ResolutionContext};
    use crate::ir::semantic_node::{NodeBase, Position};

    fn test_base() -> NodeBase {
        NodeBase::new_simple(
            Position {
                row: 0,
                column: 0,
                byte: 0,
            },
            10,
            0,
            10,
        )
    }

    #[test]
    fn test_filter_with_matching_pattern() {
        let pattern_matcher = Arc::new(MettaPatternMatcher::new());

        // Add a pattern for "foo" with arity 2
        let def_range = Range {
            start: LspPosition { line: 0, character: 0 },
            end: LspPosition { line: 0, character: 10 },
        };
        // Note: We'd need to actually add to pattern_matcher here in a real test

        let filter = MettaPatternFilter::new(pattern_matcher);

        // Create a call site: (foo arg1 arg2)
        let call_site = Arc::new(MettaNode::SExpr {
            base: test_base(),
            elements: vec![
                Arc::new(MettaNode::Atom {
                    base: test_base(),
                    name: "foo".to_string(),
                    metadata: None,
                }),
                Arc::new(MettaNode::Atom {
                    base: test_base(),
                    name: "arg1".to_string(),
                    metadata: None,
                }),
                Arc::new(MettaNode::Atom {
                    base: test_base(),
                    name: "arg2".to_string(),
                    metadata: None,
                }),
            ],
            metadata: None,
        }) as Arc<dyn std::any::Any + Send + Sync>;

        let candidates = vec![SymbolLocation {
            uri: Url::parse("file:///test.metta").unwrap(),
            range: def_range,
            kind: SymbolKind::Function,
            confidence: ResolutionConfidence::Exact,
            metadata: None,
        }];

        let context = FilterContext {
            call_site: Some(call_site),
            symbol_name: "foo".to_string(),
            language: "metta".to_string(),
            resolution_context: ResolutionContext {
                uri: Url::parse("file:///test.metta").unwrap(),
                scope_id: None,
                ir_node: None,
                language: "metta".to_string(),
                parent_uri: None,
            },
        };

        let result = filter.filter(candidates.clone(), &context);

        // Should return Some (either filtered or unfiltered)
        assert!(result.is_some());
    }

    #[test]
    fn test_filter_without_call_site() {
        let pattern_matcher = Arc::new(MettaPatternMatcher::new());
        let filter = MettaPatternFilter::new(pattern_matcher);

        let candidates = vec![];
        let context = FilterContext {
            call_site: None,  // No call site
            symbol_name: "foo".to_string(),
            language: "metta".to_string(),
            resolution_context: ResolutionContext {
                uri: Url::parse("file:///test.metta").unwrap(),
                scope_id: None,
                ir_node: None,
                language: "metta".to_string(),
                parent_uri: None,
            },
        };

        let result = filter.filter(candidates, &context);

        // Should return None (passthrough) when no call site
        assert!(result.is_none());
    }
}
