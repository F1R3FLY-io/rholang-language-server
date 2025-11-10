//! Code completion module for fuzzy, context-aware completion
//!
//! This module provides:
//! - Fuzzy string matching using liblevenshtein DynamicDawg
//! - Context-sensitive filtering based on Rholang's hierarchical lexical scoping
//! - Type-aware method completion for List, Map, Set, String, Int
//! - Relevance ranking based on edit distance, symbol frequency, and context
//! - Incremental completion state caching (Phase 9) for 10-50x faster responses

pub mod dictionary;
pub mod context;
pub mod ranking;
pub mod indexing;
pub mod type_methods;
pub mod parameter_hints;
pub mod incremental;

pub use dictionary::{WorkspaceCompletionIndex, SymbolMetadata, CompletionSymbol};
pub use context::{CompletionContext, CompletionContextType, determine_context, extract_partial_identifier};
pub use ranking::{rank_completions, RankingCriteria};
pub use indexing::{populate_from_symbol_table, populate_from_symbol_table_with_tracking, add_keywords, filter_keywords_by_context};
pub use type_methods::{get_type_methods, all_type_methods};
pub use parameter_hints::{ParameterContext, ExpectedPatternType, get_parameter_context};
pub use incremental::{DocumentCompletionState, SharedCompletionState};
