//! Composable symbol resolution system
//!
//! This module provides a trait-based architecture for symbol resolution that supports:
//! - Default lexical scoping with optional complete override
//! - Composable filters (e.g., pattern matching)
//! - Language-agnostic base with language-specific specialization
//!
//! # Architecture
//!
//! The system is built around three main traits:
//!
//! 1. **SymbolResolver**: Core trait for finding symbol definitions
//! 2. **SymbolFilter**: Trait for refining/filtering symbol candidates
//! 3. **CustomScopeResolver**: Optional trait for non-lexical scoping
//!
//! # Example Usage
//!
//! ```ignore
//! // Create a composable resolver with lexical scoping + pattern matching
//! let resolver = ComposableSymbolResolver::new(
//!     Box::new(LexicalScopeResolver::new(symbol_table)),
//!     vec![Box::new(MettaPatternFilter::new(pattern_matcher))],
//!     Some(Box::new(GlobalSymbolResolver::new(workspace))),
//! );
//!
//! // Resolve a symbol
//! let locations = resolver.resolve_symbol("get_neighbors", &position, &context);
//! ```

use std::any::Any;
use std::sync::Arc;

use tower_lsp::lsp_types::{Range, Url};

use crate::ir::semantic_node::Position;

pub mod lexical_scope;
pub mod composable;
pub mod filters;
pub mod global;
pub mod generic;
pub mod pattern_aware_resolver;

pub use lexical_scope::LexicalScopeResolver;
pub use composable::ComposableSymbolResolver;
pub use filters::{MettaPatternFilter, ChainedFilter};
pub use global::GlobalVirtualSymbolResolver;
pub use generic::GenericSymbolResolver;
pub use pattern_aware_resolver::PatternAwareContractResolver;

/// Resolution confidence level for symbol locations
///
/// Variants are ordered from lowest to highest confidence.
/// With `#[derive(Ord)]`, this means: Ambiguous < Fuzzy < Exact
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResolutionConfidence {
    /// Ambiguous match (multiple candidates with same confidence)
    Ambiguous,
    /// Fuzzy match (e.g., name match but pattern signature unknown)
    Fuzzy,
    /// Exact match (e.g., same name and matching pattern signature)
    Exact,
}

/// Kind of symbol for categorization
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    /// Function or procedure definition
    Function,
    /// Variable binding
    Variable,
    /// Parameter in function/lambda
    Parameter,
    /// Type definition
    Type,
    /// Module or namespace
    Module,
    /// Other/unknown
    Other,
}

/// Location of a symbol definition or reference
#[derive(Debug, Clone)]
pub struct SymbolLocation {
    /// URI of the document containing the symbol
    pub uri: Url,
    /// Range of the symbol in the document
    pub range: Range,
    /// Kind of symbol
    pub kind: SymbolKind,
    /// Confidence level of this resolution
    pub confidence: ResolutionConfidence,
    /// Optional metadata (language-specific)
    pub metadata: Option<Arc<dyn Any + Send + Sync>>,
}

/// Context for symbol resolution
#[derive(Clone)]
pub struct ResolutionContext {
    /// URI of the document being queried
    pub uri: Url,
    /// Optional scope ID if known
    pub scope_id: Option<usize>,
    /// Optional IR node at the query position (language-agnostic)
    pub ir_node: Option<Arc<dyn Any + Send + Sync>>,
    /// Language being resolved
    pub language: String,
    /// Optional parent URI for virtual documents
    pub parent_uri: Option<Url>,
}

/// Context for symbol filtering
#[derive(Clone)]
pub struct FilterContext {
    /// Optional call site node (e.g., SExpr for MeTTa)
    pub call_site: Option<Arc<dyn Any + Send + Sync>>,
    /// Name of symbol being filtered
    pub symbol_name: String,
    /// Language being filtered
    pub language: String,
    /// Original resolution context
    pub resolution_context: ResolutionContext,
}

/// Core trait for symbol resolution
///
/// Implementors provide logic to find symbol definitions given a name and position.
/// The trait is designed to be composable - multiple resolvers can be chained together.
pub trait SymbolResolver: Send + Sync {
    /// Find symbol definitions by name at a given position
    ///
    /// # Arguments
    /// * `symbol_name` - The name of the symbol to resolve
    /// * `position` - The position in the document where the symbol is referenced
    /// * `context` - Additional context for resolution (URI, scope, language, etc.)
    ///
    /// # Returns
    /// A vector of possible symbol locations, ordered by confidence (highest first)
    fn resolve_symbol(
        &self,
        symbol_name: &str,
        position: &Position,
        context: &ResolutionContext,
    ) -> Vec<SymbolLocation>;

    /// Check if this resolver can handle the given language
    ///
    /// # Arguments
    /// * `language` - The language identifier (e.g., "metta", "rholang")
    ///
    /// # Returns
    /// True if this resolver supports the language
    fn supports_language(&self, language: &str) -> bool;

    /// Optional: Get a display name for this resolver (for debugging)
    fn name(&self) -> &'static str {
        "SymbolResolver"
    }
}

/// Trait for filtering/refining symbol resolution results
///
/// Filters can be used to apply language-specific logic (e.g., pattern matching)
/// without replacing the entire resolution strategy.
pub trait SymbolFilter: Send + Sync {
    /// Filter or refine symbol locations based on language-specific rules
    ///
    /// # Arguments
    /// * `candidates` - Unfiltered symbol locations from base resolver
    /// * `context` - Context for filtering (call site, symbol name, etc.)
    ///
    /// # Returns
    /// - `Some(filtered)` - Apply filter, use filtered results
    /// - `None` - Skip filter, use original candidates (passthrough)
    ///
    /// If the filter returns an empty vector, the ComposableSymbolResolver
    /// will fall back to the unfiltered candidates.
    fn filter(
        &self,
        candidates: Vec<SymbolLocation>,
        context: &FilterContext,
    ) -> Option<Vec<SymbolLocation>>;

    /// Check if this filter applies to the given language
    fn applies_to_language(&self, language: &str) -> bool;

    /// Optional: Get a display name for this filter (for debugging)
    fn name(&self) -> &'static str {
        "SymbolFilter"
    }
}

/// Optional trait for languages with non-standard scoping
///
/// Languages with dynamic scoping, effect handlers, or other non-lexical
/// scoping mechanisms can implement this trait to completely override
/// the default scoping behavior.
pub trait CustomScopeResolver: SymbolResolver {
    /// Resolve symbol with custom scoping logic
    ///
    /// This method is called instead of the default lexical scope traversal.
    ///
    /// # Arguments
    /// * `symbol_name` - The name of the symbol to resolve
    /// * `context` - Resolution context
    ///
    /// # Returns
    /// Resolved symbol locations
    fn resolve_with_custom_scope(
        &self,
        symbol_name: &str,
        context: &ResolutionContext,
    ) -> Vec<SymbolLocation>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_confidence_ordering() {
        assert!(ResolutionConfidence::Exact > ResolutionConfidence::Fuzzy);
        assert!(ResolutionConfidence::Fuzzy > ResolutionConfidence::Ambiguous);
    }

    #[test]
    fn test_symbol_kind_equality() {
        assert_eq!(SymbolKind::Function, SymbolKind::Function);
        assert_ne!(SymbolKind::Function, SymbolKind::Variable);
    }
}
