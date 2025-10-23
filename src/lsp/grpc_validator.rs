//! gRPC-based validator for legacy RNode server integration
//!
//! This module provides a DiagnosticProvider implementation that communicates
//! with a legacy RNode server (Scala implementation) or Docker container via gRPC.

use super::diagnostic_provider::DiagnosticProvider;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};
use tonic::transport::Channel;
use tracing::{debug, warn};

// Import the generated protobuf code
// The build.rs generates this from proto/lsp.proto
mod proto {
    tonic::include_proto!("lsp");
}

use proto::{
    lsp_client::LspClient,
    ValidateRequest,
};

/// gRPC-based diagnostic provider
///
/// Communicates with a legacy RNode server or Docker container to perform validation.
/// This backend is slower than the Rust interpreter but allows development against
/// the legacy Scala implementation.
#[derive(Debug, Clone)]
pub struct GrpcValidator {
    client: LspClient<Channel>,
    address: String,
}

impl GrpcValidator {
    /// Create a new gRPC validator
    ///
    /// The address should be in the format "host:port" (e.g., "localhost:40401")
    pub async fn new(address: String) -> anyhow::Result<Self> {
        debug!("Connecting to RNode gRPC server at {}", address);

        // Add http:// prefix if not present
        let url = if address.starts_with("http://") || address.starts_with("https://") {
            address.clone()
        } else {
            format!("http://{}", address)
        };

        let client = LspClient::connect(url).await.map_err(|e| {
            anyhow::anyhow!("Failed to connect to RNode gRPC server at {}: {}", address, e)
        })?;

        debug!("Successfully connected to RNode gRPC server");

        Ok(Self {
            client,
            address,
        })
    }

    /// Convert protobuf diagnostic to LSP diagnostic
    fn convert_diagnostic(diag: proto::Diagnostic) -> Diagnostic {
        let range = diag.range.map(|r| {
            let start = r.start.map(|p| Position {
                line: p.line as u32,
                character: p.column as u32,
            }).unwrap_or_default();

            let end = r.end.map(|p| Position {
                line: p.line as u32,
                character: p.column as u32,
            }).unwrap_or_default();

            Range { start, end }
        }).unwrap_or_default();

        let severity = match proto::DiagnosticSeverity::try_from(diag.severity) {
            Ok(proto::DiagnosticSeverity::Error) => Some(DiagnosticSeverity::ERROR),
            Ok(proto::DiagnosticSeverity::Warning) => Some(DiagnosticSeverity::WARNING),
            Ok(proto::DiagnosticSeverity::Information) => Some(DiagnosticSeverity::INFORMATION),
            Ok(proto::DiagnosticSeverity::Hint) => Some(DiagnosticSeverity::HINT),
            Err(_) => {
                warn!("Unknown diagnostic severity: {}, defaulting to ERROR", diag.severity);
                Some(DiagnosticSeverity::ERROR)
            }
        };

        Diagnostic {
            range,
            severity,
            source: if diag.source.is_empty() {
                Some("rnode-grpc".to_string())
            } else {
                Some(diag.source)
            },
            message: diag.message,
            ..Default::default()
        }
    }
}

#[async_trait::async_trait]
impl DiagnosticProvider for GrpcValidator {
    async fn validate(&self, source: &str) -> Vec<Diagnostic> {
        debug!("Sending validation request to RNode gRPC server ({} bytes)", source.len());

        let request = tonic::Request::new(ValidateRequest {
            text: source.to_string(),
        });

        // Clone the client for the request (it's cheap to clone)
        let mut client = self.client.clone();

        match client.validate(request).await {
            Ok(response) => {
                let response = response.into_inner();

                match response.result {
                    Some(proto::validate_response::Result::Success(diag_list)) => {
                        debug!("Validation succeeded with {} diagnostics", diag_list.diagnostics.len());
                        diag_list.diagnostics
                            .into_iter()
                            .map(Self::convert_diagnostic)
                            .collect()
                    }
                    Some(proto::validate_response::Result::Error(error_msg)) => {
                        warn!("Validation failed with error: {}", error_msg);
                        // Return a single diagnostic with the error
                        vec![Diagnostic {
                            range: Range::default(),
                            severity: Some(DiagnosticSeverity::ERROR),
                            source: Some("rnode-grpc".to_string()),
                            message: error_msg,
                            ..Default::default()
                        }]
                    }
                    None => {
                        warn!("Validation response had no result");
                        vec![]
                    }
                }
            }
            Err(e) => {
                warn!("gRPC validation request failed: {}", e);
                // Return a diagnostic indicating the gRPC error
                vec![Diagnostic {
                    range: Range::default(),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("rnode-grpc".to_string()),
                    message: format!("Failed to validate via gRPC: {}", e),
                    ..Default::default()
                }]
            }
        }
    }

    fn backend_name(&self) -> &'static str {
        "RNode gRPC"
    }
}
