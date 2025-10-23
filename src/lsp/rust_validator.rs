//! Rust-based semantic validator using the embedded interpreter
//!
//! This module wraps the Rholang interpreter to provide semantic validation
//! as a DiagnosticProvider implementation.

#[cfg(feature = "interpreter")]
use super::diagnostic_provider::DiagnosticProvider;
#[cfg(feature = "interpreter")]
use super::semantic_validator::SemanticValidator;
#[cfg(feature = "interpreter")]
use tower_lsp::lsp_types::Diagnostic;

/// Rust-based diagnostic provider using the embedded Rholang interpreter
///
/// This is the fastest and most reliable backend since it runs locally
/// and doesn't require network communication or external processes.
#[cfg(feature = "interpreter")]
#[derive(Debug, Clone)]
pub struct RustSemanticValidator {
    validator: SemanticValidator,
}

#[cfg(feature = "interpreter")]
impl RustSemanticValidator {
    /// Create a new Rust semantic validator
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            validator: SemanticValidator::new()?,
        })
    }

    /// Get the underlying SemanticValidator for direct access
    ///
    /// This is useful for the `validate_parsed` method which isn't part
    /// of the DiagnosticProvider trait.
    pub fn validator(&self) -> &SemanticValidator {
        &self.validator
    }
}

#[cfg(feature = "interpreter")]
#[async_trait::async_trait]
impl DiagnosticProvider for RustSemanticValidator {
    async fn validate(&self, source: &str) -> Vec<Diagnostic> {
        self.validator.validate(source).await
    }

    fn backend_name(&self) -> &'static str {
        "Rust Interpreter"
    }
}
