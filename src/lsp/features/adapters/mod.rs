//! Language-specific LSP feature adapters
//!
//! This module provides language-specific implementations of LSP features
//! using the unified LanguageAdapter architecture. Each language (Rholang,
//! MeTTa, etc.) has its own adapter that bundles:
//! - HoverProvider: Symbol hover information
//! - CompletionProvider: Code completion and keywords
//! - DocumentationProvider: Symbol documentation
//! - SymbolResolver: Symbol resolution and scoping rules

pub mod rholang;
pub mod metta;

pub use rholang::{
    RholangHoverProvider,
    RholangCompletionProvider,
    RholangDocumentationProvider,
    create_rholang_adapter,
};

pub use metta::{
    MettaHoverProvider,
    MettaCompletionProvider,
    MettaDocumentationProvider,
    create_metta_adapter,
};
