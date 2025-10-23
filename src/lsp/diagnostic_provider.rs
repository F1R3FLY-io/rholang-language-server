//! Diagnostic provider abstraction for pluggable validation backends
//!
//! This module defines the core trait for validation backends and provides
//! factory functions for creating the appropriate backend based on configuration.

use tower_lsp::lsp_types::Diagnostic;

/// Common interface for all diagnostic/validation backends
///
/// This trait allows the LSP backend to work with different validation implementations:
/// - Rust interpreter (fast, local, embedded)
/// - gRPC to legacy RNode server (network-based, Scala implementation)
/// - gRPC to Docker container (network-based, containerized)
#[async_trait::async_trait]
pub trait DiagnosticProvider: Send + Sync {
    /// Validate Rholang source code and return diagnostics
    ///
    /// This method should:
    /// 1. Parse and validate the source code
    /// 2. Return any syntax or semantic errors as LSP diagnostics
    /// 3. Return an empty vec if the code is valid
    async fn validate(&self, source: &str) -> Vec<Diagnostic>;

    /// Get a human-readable name for this backend (for logging/debugging)
    fn backend_name(&self) -> &'static str;
}

/// Configuration for selecting a diagnostic backend
#[derive(Debug, Clone)]
pub enum BackendConfig {
    /// Use the embedded Rust interpreter
    Rust,

    /// Use gRPC to connect to a legacy RNode server
    ///
    /// The string should be the server address (e.g., "localhost:40401")
    Grpc(String),
}

impl BackendConfig {
    /// Parse backend configuration from environment or initialization options
    ///
    /// Checks in order:
    /// 1. Environment variable RHOLANG_VALIDATOR_BACKEND
    /// 2. Explicit initialization parameter
    /// 3. Falls back to Rust backend if interpreter feature is enabled
    pub fn from_env_or_default(init_option: Option<&str>) -> Self {
        // Check environment variable first
        if let Ok(backend) = std::env::var("RHOLANG_VALIDATOR_BACKEND") {
            return Self::parse(&backend);
        }

        // Check initialization option
        if let Some(backend) = init_option {
            return Self::parse(backend);
        }

        // Default to Rust backend if interpreter feature is enabled
        #[cfg(feature = "interpreter")]
        {
            Self::Rust
        }

        // If no interpreter feature, must use gRPC (default RNode port)
        #[cfg(not(feature = "interpreter"))]
        {
            Self::Grpc("localhost:40402".to_string())
        }
    }

    /// Parse backend configuration from a string
    ///
    /// Format:
    /// - "rust" -> Rust backend
    /// - "grpc:<address>" -> gRPC backend (e.g., "grpc:localhost:40401")
    fn parse(s: &str) -> Self {
        let s = s.trim().to_lowercase();

        if s == "rust" {
            Self::Rust
        } else if let Some(addr) = s.strip_prefix("grpc:") {
            Self::Grpc(addr.to_string())
        } else {
            // Default to Rust if we can't parse
            tracing::warn!("Unknown backend config '{}', defaulting to Rust", s);
            Self::Rust
        }
    }
}

/// Create a diagnostic provider based on the configuration
pub async fn create_provider(config: BackendConfig) -> anyhow::Result<Box<dyn DiagnosticProvider>> {
    use tracing::info;

    match config {
        #[cfg(feature = "interpreter")]
        BackendConfig::Rust => {
            info!("Creating Rust interpreter diagnostic provider");
            let provider = super::rust_validator::RustSemanticValidator::new()?;
            Ok(Box::new(provider))
        }

        #[cfg(not(feature = "interpreter"))]
        BackendConfig::Rust => {
            anyhow::bail!("Rust backend requested but 'interpreter' feature is not enabled")
        }

        BackendConfig::Grpc(address) => {
            info!("Creating gRPC diagnostic provider for address: {}", address);
            let provider = super::grpc_validator::GrpcValidator::new(address).await?;
            Ok(Box::new(provider))
        }
    }
}
