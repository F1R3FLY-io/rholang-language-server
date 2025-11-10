//! Ranking and sorting of completion results
//!
//! This module implements ranking criteria for completion suggestions to ensure
//! the most relevant results appear first.
//!
//! Ranking algorithm (in order of priority):
//! 1. Scope depth (lexical scope proximity) - lower is better (HIGHEST PRIORITY)
//! 2. Distance (Levenshtein distance from query) - lower is better
//! 3. Reference count (frequency of usage) - higher is better
//! 4. Length (shorter names preferred) - lower is better
//! 5. Lexicographic order (alphabetical) - as tie-breaker

use super::dictionary::CompletionSymbol;
use std::cmp::Ordering;

/// Criteria for ranking completion results
#[derive(Debug, Clone)]
pub struct RankingCriteria {
    /// Weight for scope depth (default: 10.0)
    /// Must dominate distance to ensure local symbols always rank first
    pub scope_depth_weight: f64,

    /// Weight for distance (default: 1.0)
    pub distance_weight: f64,

    /// Weight for reference count (default: 0.1)
    pub reference_count_weight: f64,

    /// Weight for length (default: 0.01)
    pub length_weight: f64,

    /// Maximum results to return (default: 50)
    pub max_results: usize,
}

impl RankingCriteria {
    /// Create default ranking criteria
    pub fn default() -> Self {
        Self {
            scope_depth_weight: 10.0,  // Highest priority - local symbols rank first
            distance_weight: 1.0,
            reference_count_weight: 0.1,
            length_weight: 0.01,
            max_results: 50,
        }
    }

    /// Create ranking criteria optimized for exact prefix matches
    pub fn exact_prefix() -> Self {
        Self {
            scope_depth_weight: 10.0,  // Always prioritize scope proximity
            distance_weight: 0.0,  // Distance is always 0 for exact matches
            reference_count_weight: 0.5,  // Prioritize frequently used symbols
            length_weight: 0.5,  // Prefer shorter names
            max_results: 50,
        }
    }

    /// Create ranking criteria optimized for fuzzy matches
    pub fn fuzzy() -> Self {
        Self {
            scope_depth_weight: 10.0,  // Always prioritize scope proximity
            distance_weight: 2.0,  // Distance is most important after scope
            reference_count_weight: 0.1,  // Secondary: usage frequency
            length_weight: 0.01,  // Tertiary: prefer shorter names
            max_results: 50,
        }
    }
}

/// Rank completion symbols according to the given criteria
///
/// # Arguments
/// * `symbols` - Vector of completion symbols with distances
/// * `criteria` - Ranking criteria
///
/// # Returns
/// Sorted vector of completion symbols (best matches first), limited to max_results
pub fn rank_completions(
    mut symbols: Vec<CompletionSymbol>,
    criteria: &RankingCriteria,
) -> Vec<CompletionSymbol> {
    // Sort by composite score
    symbols.sort_by(|a, b| {
        // Calculate scores
        let score_a = calculate_score(a, criteria);
        let score_b = calculate_score(b, criteria);

        // Compare scores (lower is better)
        match score_a.partial_cmp(&score_b) {
            Some(Ordering::Equal) => {
                // Tie-breaker: lexicographic order
                a.metadata.name.cmp(&b.metadata.name)
            }
            Some(ord) => ord,
            None => Ordering::Equal,
        }
    });

    // Limit to max_results
    symbols.truncate(criteria.max_results);

    symbols
}

/// Calculate composite score for a completion symbol
///
/// Lower scores are better (closer scope, closer to query, more frequently used, shorter name)
fn calculate_score(symbol: &CompletionSymbol, criteria: &RankingCriteria) -> f64 {
    // Scope depth is highest priority (local symbols rank first)
    let scope_score = symbol.scope_depth as f64 * criteria.scope_depth_weight;

    let distance_score = symbol.distance as f64 * criteria.distance_weight;

    // Invert reference count (higher count = lower score)
    let reference_score = if symbol.metadata.reference_count > 0 {
        -1.0 * (symbol.metadata.reference_count as f64) * criteria.reference_count_weight
    } else {
        0.0
    };

    let length_score = symbol.metadata.name.len() as f64 * criteria.length_weight;

    scope_score + distance_score + reference_score + length_score
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::features::completion::dictionary::SymbolMetadata;
    use tower_lsp::lsp_types::CompletionItemKind;

    #[test]
    fn test_rank_by_distance() {
        let symbols = vec![
            CompletionSymbol {
                metadata: SymbolMetadata {
                    name: "far".to_string(),
                    kind: CompletionItemKind::VARIABLE,
                    documentation: None,
                    signature: None,
                    reference_count: 0,
                },
                distance: 3,
                scope_depth: usize::MAX,
            },
            CompletionSymbol {
                metadata: SymbolMetadata {
                    name: "close".to_string(),
                    kind: CompletionItemKind::VARIABLE,
                    documentation: None,
                    signature: None,
                    reference_count: 0,
                },
                distance: 1,
                scope_depth: usize::MAX,
            },
            CompletionSymbol {
                metadata: SymbolMetadata {
                    name: "medium".to_string(),
                    kind: CompletionItemKind::VARIABLE,
                    documentation: None,
                    signature: None,
                    reference_count: 0,
                },
                distance: 2,
                scope_depth: usize::MAX,
            },
        ];

        let criteria = RankingCriteria::fuzzy();
        let ranked = rank_completions(symbols, &criteria);

        assert_eq!(ranked[0].metadata.name, "close");
        assert_eq!(ranked[1].metadata.name, "medium");
        assert_eq!(ranked[2].metadata.name, "far");
    }

    #[test]
    fn test_rank_by_reference_count() {
        let symbols = vec![
            CompletionSymbol {
                metadata: SymbolMetadata {
                    name: "rarely_used".to_string(),
                    kind: CompletionItemKind::VARIABLE,
                    documentation: None,
                    signature: None,
                    reference_count: 1,
                },
                distance: 0,
                scope_depth: usize::MAX,
            },
            CompletionSymbol {
                metadata: SymbolMetadata {
                    name: "frequently_used".to_string(),
                    kind: CompletionItemKind::VARIABLE,
                    documentation: None,
                    signature: None,
                    reference_count: 100,
                },
                distance: 0,
                scope_depth: usize::MAX,
            },
            CompletionSymbol {
                metadata: SymbolMetadata {
                    name: "moderately_used".to_string(),
                    kind: CompletionItemKind::VARIABLE,
                    documentation: None,
                    signature: None,
                    reference_count: 10,
                },
                distance: 0,
                scope_depth: usize::MAX,
            },
        ];

        let criteria = RankingCriteria::exact_prefix();
        let ranked = rank_completions(symbols, &criteria);

        // Higher reference count should come first (when distance is equal)
        assert_eq!(ranked[0].metadata.name, "frequently_used");
        assert_eq!(ranked[1].metadata.name, "moderately_used");
        assert_eq!(ranked[2].metadata.name, "rarely_used");
    }

    #[test]
    fn test_rank_by_length() {
        let symbols = vec![
            CompletionSymbol {
                metadata: SymbolMetadata {
                    name: "verylongname".to_string(),
                    kind: CompletionItemKind::VARIABLE,
                    documentation: None,
                    signature: None,
                    reference_count: 0,
                },
                distance: 0,
                scope_depth: usize::MAX,
            },
            CompletionSymbol {
                metadata: SymbolMetadata {
                    name: "short".to_string(),
                    kind: CompletionItemKind::VARIABLE,
                    documentation: None,
                    signature: None,
                    reference_count: 0,
                },
                distance: 0,
                scope_depth: usize::MAX,
            },
            CompletionSymbol {
                metadata: SymbolMetadata {
                    name: "medium".to_string(),
                    kind: CompletionItemKind::VARIABLE,
                    documentation: None,
                    signature: None,
                    reference_count: 0,
                },
                distance: 0,
                scope_depth: usize::MAX,
            },
        ];

        let criteria = RankingCriteria {
            scope_depth_weight: 0.0,
            distance_weight: 0.0,
            reference_count_weight: 0.0,
            length_weight: 1.0,  // Only consider length
            max_results: 50,
        };
        let ranked = rank_completions(symbols, &criteria);

        // Shorter names should come first
        assert_eq!(ranked[0].metadata.name, "short");
        assert_eq!(ranked[1].metadata.name, "medium");
        assert_eq!(ranked[2].metadata.name, "verylongname");
    }

    #[test]
    fn test_max_results_limit() {
        let symbols: Vec<CompletionSymbol> = (0..100)
            .map(|i| CompletionSymbol {
                metadata: SymbolMetadata {
                    name: format!("symbol{}", i),
                    kind: CompletionItemKind::VARIABLE,
                    documentation: None,
                    signature: None,
                    reference_count: 0,
                },
                distance: i,
                scope_depth: usize::MAX,
            })
            .collect();

        let criteria = RankingCriteria {
            scope_depth_weight: 0.0,
            distance_weight: 1.0,
            reference_count_weight: 0.0,
            length_weight: 0.0,
            max_results: 10,
        };
        let ranked = rank_completions(symbols, &criteria);

        assert_eq!(ranked.len(), 10);
        assert_eq!(ranked[0].metadata.name, "symbol0");
        assert_eq!(ranked[9].metadata.name, "symbol9");
    }
}
