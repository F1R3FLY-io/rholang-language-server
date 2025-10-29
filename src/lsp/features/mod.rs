//! Unified LSP features with language-agnostic implementations
//!
//! This module provides a composable architecture for LSP features that work across
//! multiple embedded languages (Rholang, MeTTa, etc.) without duplicating logic.
//!
//! # Architecture Overview
//!
//! The module is organized into three layers:
//!
//! ## 1. Traits Layer (`traits.rs`)
//! Defines the contracts that languages must implement:
//! - `HoverProvider` - Customizes hover tooltips
//! - `CompletionProvider` - Provides code completions
//! - `DocumentationProvider` - Looks up symbol documentation
//! - `FormattingProvider` - Optional code formatting
//! - `LanguageAdapter` - Bundles all providers for a language
//!
//! ## 2. Generic Features Layer (future modules)
//! Language-agnostic implementations of LSP features:
//! - `goto_definition.rs` - Generic goto-definition using SemanticNode
//! - `hover.rs` - Generic hover using HoverProvider
//! - `references.rs` - Generic find-references
//! - `rename.rs` - Generic symbol renaming
//! - `completion.rs` - Generic code completion
//! - `document_symbols.rs` - Generic symbol extraction
//!
//! ## 3. Language Adapters Layer (future modules)
//! Language-specific implementations of provider traits:
//! - `adapters/rholang.rs` - Rholang language adapter
//! - `adapters/metta.rs` - MeTTa language adapter
//! - `adapters/mod.rs` - Adapter registry
//!
//! # Design Principles
//!
//! 1. **DRY (Don't Repeat Yourself)**: Write LSP logic once, reuse across languages
//! 2. **Language-Agnostic**: Core features work with `&dyn SemanticNode`
//! 3. **Composable**: Mix and match providers as needed
//! 4. **Type-Safe**: Rust's type system ensures correct usage
//! 5. **Testable**: Each layer can be tested independently
//!
//! # Usage Example
//!
//! ```rust,ignore
//! use crate::lsp::features::traits::*;
//! use crate::lsp::features::goto_definition::GenericGotoDefinition;
//! use crate::lsp::features::adapters::rholang::create_rholang_adapter;
//!
//! // Create language adapter
//! let rholang_adapter = create_rholang_adapter(symbol_table);
//!
//! // Create generic goto-definition feature
//! let goto_def = GenericGotoDefinition::new();
//!
//! // Use it with any SemanticNode and the adapter
//! let result = goto_def.goto_definition(
//!     root_node,
//!     &position,
//!     &uri,
//!     &rholang_adapter,
//! ).await?;
//! ```
//!
//! # Migration Strategy
//!
//! This module is being built incrementally:
//!
//! ## Phase 1: Traits ✅ (Current)
//! - Define provider traits
//! - Define LanguageAdapter struct
//! - Write comprehensive tests
//!
//! ## Phase 2: Generic Features (In Progress)
//! - Implement GenericGotoDefinition
//! - Implement GenericHover
//! - Implement GenericReferences
//! - Implement GenericRename
//! - Test with mock adapters
//!
//! ## Phase 3: Language Adapters
//! - Extract Rholang-specific logic into RholangAdapter
//! - Extract MeTTa-specific logic into MettaAdapter
//! - Test adapters work with generic features
//!
//! ## Phase 4: Integration
//! - Wire up adapters in RholangBackend
//! - Create unified_handlers.rs dispatch logic
//! - Gradually replace old handlers
//! - Run full integration tests
//!
//! ## Phase 5: Cleanup
//! - Remove duplicated code
//! - Update documentation
//! - Measure code reduction (target: 50%+)

pub mod traits;
pub mod node_finder;
pub mod goto_definition;
pub mod hover;
pub mod references;
pub mod rename;
pub mod tree_sitter;

// Phase 2 modules (in progress):
// pub mod completion;

// Future modules:
// pub mod document_symbols;
// pub mod adapters;

// Re-export main types for convenience
pub use traits::{
    CompletionContext,
    CompletionProvider,
    DocumentationContext,
    DocumentationProvider,
    FormattingOptions,
    FormattingProvider,
    HoverContext,
    HoverProvider,
    LanguageAdapter,
};
