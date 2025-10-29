//! Symbol filters for refining resolution results
//!
//! Filters apply language-specific logic to refine symbol candidates
//! without replacing the entire resolution strategy.

pub mod metta_pattern;
pub mod chained;

pub use metta_pattern::MettaPatternFilter;
pub use chained::ChainedFilter;
