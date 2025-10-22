//! Semantic validation using the Rholang interpreter
//!
//! This module provides semantic validation beyond what the parser can catch,
//! using the Rholang interpreter to detect runtime semantic errors.

#[cfg(feature = "interpreter")]
use rholang::rust::interpreter::{
    compiler::compiler::Compiler,
    errors::InterpreterError as RholangInterpreterError,
};

#[cfg(feature = "interpreter")]
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};
#[cfg(not(feature = "interpreter"))]
use tower_lsp::lsp_types::Diagnostic;

#[cfg(feature = "interpreter")]
use std::collections::HashMap;

#[cfg(feature = "interpreter")]
use tracing::{debug, warn};

/// Semantic validator that uses the Rholang interpreter to find semantic errors
#[cfg(feature = "interpreter")]
#[derive(Debug, Clone)]
pub struct SemanticValidator {
    _phantom: std::marker::PhantomData<()>,
}

#[cfg(feature = "interpreter")]
impl SemanticValidator {
    /// Create a new semantic validator
    pub fn new() -> anyhow::Result<Self> {
        debug!("Initializing Rholang semantic validator");
        Ok(Self {
            _phantom: std::marker::PhantomData,
        })
    }

    /// Validate Rholang source code and return semantic diagnostics
    ///
    /// This only runs if the code has no syntax errors (as determined by tree-sitter).
    /// It uses the Rholang compiler to catch semantic errors like:
    /// - Unbound variables
    /// - Type mismatches
    /// - Invalid operations
    /// - Top-level constraint violations
    pub async fn validate(&self, source: &str) -> Vec<Diagnostic> {
        debug!("Running semantic validation on source ({} bytes)", source.len());

        // Use empty normalizer environment for now
        let normalizer_env = HashMap::new();

        // Try to compile the source
        match Compiler::source_to_adt_with_normalizer_env(source, normalizer_env) {
            Ok(_par) => {
                debug!("Source compiled successfully, no semantic errors");
                vec![]
            }
            Err(e) => {
                debug!("Compilation failed with error: {:?}", e);
                self.error_to_diagnostics(e)
            }
        }
    }

    /// Validate a pre-parsed AST, avoiding redundant parsing.
    ///
    /// This is more efficient when the AST has already been parsed for syntax validation.
    /// Returns semantic diagnostics for any issues found.
    ///
    /// Note: This is synchronous (not async) because the underlying validation is synchronous
    /// and the parser reference cannot be held across await points.
    pub fn validate_parsed<'a>(
        &self,
        ast: rholang_parser::ast::AnnProc<'a>,
        parser: &'a rholang_parser::RholangParser<'a>,
    ) -> Vec<Diagnostic> {
        debug!("Running semantic validation on pre-parsed AST");

        // Use empty normalizer environment for now
        let normalizer_env = HashMap::new();

        // Validate the pre-parsed AST
        match Compiler::validate_parsed(ast, normalizer_env, parser) {
            Ok(_par) => {
                debug!("AST validated successfully, no semantic errors");
                vec![]
            }
            Err(e) => {
                debug!("Validation failed with error: {:?}", e);
                self.error_to_diagnostics(e)
            }
        }
    }

    /// Convert a Rholang interpreter error to one or more LSP diagnostics
    ///
    /// Some errors contain multiple source spans (e.g., duplicate declarations showing both locations).
    /// For these cases, we create one diagnostic per span as requested by the user.
    fn error_to_diagnostics(&self, error: RholangInterpreterError) -> Vec<Diagnostic> {
        use RholangInterpreterError::*;

        match &error {
            // Errors with source span information
            UnboundVariableRefSpan { var_name, source_span } => {
                debug!("Unbound variable '{}' at {:?}", var_name, source_span);
                vec![Diagnostic {
                    range: source_span_to_range(source_span),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("rholang-semantic".to_string()),
                    message: format!("Unbound variable: {}", var_name),
                    ..Default::default()
                }]
            }

            UnboundVariableRefPos { var_name, source_pos } => {
                debug!("Unbound variable '{}' at {:?}", var_name, source_pos);
                vec![Diagnostic {
                    range: source_pos_to_range(source_pos),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("rholang-semantic".to_string()),
                    message: format!("Unbound variable: {}", var_name),
                    ..Default::default()
                }]
            }

            // Errors with multiple source spans - create one diagnostic per span
            UnexpectedProcContext {
                var_name,
                name_var_source_span,
                process_source_span,
            } => {
                debug!("Name variable '{}' used in process context", var_name);
                vec![
                    Diagnostic {
                        range: source_span_to_range(name_var_source_span),
                        severity: Some(DiagnosticSeverity::ERROR),
                        source: Some("rholang-semantic".to_string()),
                        message: format!(
                            "Name variable '{}' declared here",
                            var_name
                        ),
                        ..Default::default()
                    },
                    Diagnostic {
                        range: source_span_to_range(process_source_span),
                        severity: Some(DiagnosticSeverity::ERROR),
                        source: Some("rholang-semantic".to_string()),
                        message: format!(
                            "Name variable '{}' used in process context here",
                            var_name
                        ),
                        ..Default::default()
                    },
                ]
            }

            UnexpectedNameContext {
                var_name,
                proc_var_source_span,
                name_source_span,
            } => {
                debug!("Process variable '{}' used in name context", var_name);
                vec![
                    Diagnostic {
                        range: source_span_to_range(proc_var_source_span),
                        severity: Some(DiagnosticSeverity::ERROR),
                        source: Some("rholang-semantic".to_string()),
                        message: format!("Process variable '{}' declared here", var_name),
                        ..Default::default()
                    },
                    Diagnostic {
                        range: source_span_to_range(name_source_span),
                        severity: Some(DiagnosticSeverity::ERROR),
                        source: Some("rholang-semantic".to_string()),
                        message: format!("Process variable '{}' used in name context here", var_name),
                        ..Default::default()
                    },
                ]
            }

            UnexpectedReuseOfProcContextFree {
                var_name,
                first_use,
                second_use,
            } => {
                debug!("Variable '{}' used twice as binder", var_name);
                vec![
                    Diagnostic {
                        range: source_span_to_range(first_use),
                        severity: Some(DiagnosticSeverity::ERROR),
                        source: Some("rholang-semantic".to_string()),
                        message: format!(
                            "Process variable '{}' first used as binder here",
                            var_name
                        ),
                        ..Default::default()
                    },
                    Diagnostic {
                        range: source_span_to_range(second_use),
                        severity: Some(DiagnosticSeverity::ERROR),
                        source: Some("rholang-semantic".to_string()),
                        message: format!(
                            "Process variable '{}' used again as binder here (duplicate)",
                            var_name
                        ),
                        ..Default::default()
                    },
                ]
            }

            UnexpectedReuseOfNameContextFree {
                var_name,
                first_use,
                second_use,
            } => {
                debug!("Variable '{}' used twice as binder in name context", var_name);
                vec![
                    Diagnostic {
                        range: source_span_to_range(first_use),
                        severity: Some(DiagnosticSeverity::ERROR),
                        source: Some("rholang-semantic".to_string()),
                        message: format!(
                            "Name variable '{}' first used as binder here",
                            var_name
                        ),
                        ..Default::default()
                    },
                    Diagnostic {
                        range: source_span_to_range(second_use),
                        severity: Some(DiagnosticSeverity::ERROR),
                        source: Some("rholang-semantic".to_string()),
                        message: format!(
                            "Name variable '{}' used again as binder here (duplicate)",
                            var_name
                        ),
                        ..Default::default()
                    },
                ]
            }

            ReceiveOnSameChannelsError { source_span } => {
                debug!("Receive on same channels at {:?}", source_span);
                vec![Diagnostic {
                    range: source_span_to_range(source_span),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("rholang-semantic".to_string()),
                    message: "Receiving on the same channels is not allowed".to_string(),
                    ..Default::default()
                }]
            }

            // Errors without precise position information
            TopLevelFreeVariablesNotAllowedError(vars) => {
                warn!("Top-level free variables: {}", vars);
                vec![Diagnostic {
                    range: Range::default(),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("rholang-semantic".to_string()),
                    message: format!("Top-level free variables are not allowed: {}", vars),
                    ..Default::default()
                }]
            }

            TopLevelWildcardsNotAllowedError(wildcards) => {
                warn!("Top-level wildcards: {}", wildcards);
                vec![Diagnostic {
                    range: Range::default(),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("rholang-semantic".to_string()),
                    message: format!("Top-level wildcards are not allowed: {}", wildcards),
                    ..Default::default()
                }]
            }

            TopLevelLogicalConnectivesNotAllowedError(connectives) => {
                warn!("Top-level logical connectives: {}", connectives);
                vec![Diagnostic {
                    range: Range::default(),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("rholang-semantic".to_string()),
                    message: format!(
                        "Top-level logical connectives are not allowed: {}",
                        connectives
                    ),
                    ..Default::default()
                }]
            }

            MethodNotDefined { method, other_type } => {
                vec![Diagnostic {
                    range: Range::default(),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("rholang-semantic".to_string()),
                    message: format!("Method '{}' is not defined on {}", method, other_type),
                    ..Default::default()
                }]
            }

            MethodArgumentNumberMismatch {
                method,
                expected,
                actual,
            } => {
                vec![Diagnostic {
                    range: Range::default(),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("rholang-semantic".to_string()),
                    message: format!(
                        "Method '{}' expects {} argument(s), but got {}",
                        method, expected, actual
                    ),
                    ..Default::default()
                }]
            }

            OperatorNotDefined { op, other_type } => {
                vec![Diagnostic {
                    range: Range::default(),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("rholang-semantic".to_string()),
                    message: format!("Operator '{}' is not defined on {}", op, other_type),
                    ..Default::default()
                }]
            }

            OperatorExpectedError {
                op,
                expected,
                other_type,
            } => {
                vec![Diagnostic {
                    range: Range::default(),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("rholang-semantic".to_string()),
                    message: format!(
                        "Operator '{}' expected {}, got {}",
                        op, expected, other_type
                    ),
                    ..Default::default()
                }]
            }

            AggregateError { interpreter_errors } => {
                // For aggregate errors, convert all sub-errors to diagnostics
                if interpreter_errors.is_empty() {
                    return vec![Diagnostic {
                        range: Range::default(),
                        severity: Some(DiagnosticSeverity::ERROR),
                        source: Some("rholang-semantic".to_string()),
                        message: "Multiple interpreter errors occurred".to_string(),
                        ..Default::default()
                    }];
                }
                // Recursively convert all errors and flatten into single vec
                interpreter_errors
                    .iter()
                    .flat_map(|e| self.error_to_diagnostics(e.clone()))
                    .collect()
            }

            // Runtime and resource errors
            RSpaceError(err) => {
                warn!("RSpace error: {}", err);
                vec![Diagnostic {
                    range: Range::default(),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("rholang-runtime".to_string()),
                    message: format!("Runtime storage error: {}", err),
                    ..Default::default()
                }]
            }

            OutOfPhlogistonsError => {
                vec![Diagnostic {
                    range: Range::default(),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("rholang-runtime".to_string()),
                    message: "Computation ran out of phlogistons (gas limit exceeded)".to_string(),
                    ..Default::default()
                }]
            }

            // Parser/Lexer errors (should be caught earlier by tree-sitter, but handle anyway)
            SyntaxError(msg) | LexerError(msg) | ParserError(msg) => {
                vec![Diagnostic {
                    range: Range::default(),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("rholang-parser".to_string()),
                    message: msg.clone(),
                    ..Default::default()
                }]
            }

            // Normalization errors
            NormalizerError(msg) | UnrecognizedNormalizerError(msg) => {
                vec![Diagnostic {
                    range: Range::default(),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("rholang-normalizer".to_string()),
                    message: format!("Normalization error: {}", msg),
                    ..Default::default()
                }]
            }

            // Pattern matching errors
            PatternReceiveError(msg) => {
                vec![Diagnostic {
                    range: Range::default(),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("rholang-semantic".to_string()),
                    message: format!("Invalid pattern in receive: {}. Only logical AND is allowed", msg),
                    ..Default::default()
                }]
            }

            SortMatchError(msg) => {
                vec![Diagnostic {
                    range: Range::default(),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("rholang-semantic".to_string()),
                    message: format!("Pattern matching error: {}", msg),
                    ..Default::default()
                }]
            }

            // Reduction and substitution errors
            ReduceError(msg) => {
                vec![Diagnostic {
                    range: Range::default(),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("rholang-runtime".to_string()),
                    message: format!("Reduction error: {}", msg),
                    ..Default::default()
                }]
            }

            SubstituteError(msg) => {
                vec![Diagnostic {
                    range: Range::default(),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("rholang-semantic".to_string()),
                    message: format!("Substitution error: {}", msg),
                    ..Default::default()
                }]
            }

            // Encoding/Decoding errors
            EncodeError(msg) => {
                vec![Diagnostic {
                    range: Range::default(),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("rholang-serialization".to_string()),
                    message: format!("Encoding error: {}", msg),
                    ..Default::default()
                }]
            }

            DecodeError(msg) => {
                vec![Diagnostic {
                    range: Range::default(),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("rholang-serialization".to_string()),
                    message: format!("Decoding error: {}", msg),
                    ..Default::default()
                }]
            }

            // Bundle errors
            UnexpectedBundleContent(msg) => {
                vec![Diagnostic {
                    range: Range::default(),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("rholang-semantic".to_string()),
                    message: format!("Unexpected bundle content: {}", msg),
                    ..Default::default()
                }]
            }

            // Internal/system errors
            BugFoundError(msg) => {
                warn!("Internal bug detected: {}", msg);
                vec![Diagnostic {
                    range: Range::default(),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("rholang-internal".to_string()),
                    message: format!("Internal error (please report): {}", msg),
                    ..Default::default()
                }]
            }

            UndefinedRequiredProtobufFieldError(field) => {
                warn!("Protobuf field missing: {}", field);
                vec![Diagnostic {
                    range: Range::default(),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("rholang-internal".to_string()),
                    message: format!("Internal serialization error: missing field {}", field),
                    ..Default::default()
                }]
            }

            SetupError(msg) => {
                vec![Diagnostic {
                    range: Range::default(),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("rholang-setup".to_string()),
                    message: format!("Setup error: {}", msg),
                    ..Default::default()
                }]
            }

            UnrecognizedInterpreterError(msg) => {
                warn!("Unrecognized interpreter error: {}", msg);
                vec![Diagnostic {
                    range: Range::default(),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("rholang-runtime".to_string()),
                    message: format!("Unrecognized error: {}", msg),
                    ..Default::default()
                }]
            }

            // External service errors (typically not from user code)
            OpenAIError(msg) => {
                warn!("OpenAI service error: {}", msg);
                vec![Diagnostic {
                    range: Range::default(),
                    severity: Some(DiagnosticSeverity::WARNING),
                    source: Some("rholang-external".to_string()),
                    message: format!("External service error: {}", msg),
                    ..Default::default()
                }]
            }

            // Argument errors
            IllegalArgumentError(msg) => {
                vec![Diagnostic {
                    range: Range::default(),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("rholang-semantic".to_string()),
                    message: format!("Illegal argument: {}", msg),
                    ..Default::default()
                }]
            }

            // IO errors
            IoError(msg) => {
                vec![Diagnostic {
                    range: Range::default(),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("rholang-io".to_string()),
                    message: format!("IO error: {}", msg),
                    ..Default::default()
                }]
            }
        }
    }
}

#[cfg(feature = "interpreter")]
fn source_span_to_range(span: &rholang_parser::SourceSpan) -> Range {
    Range {
        start: Position {
            line: (span.start.line as u32).saturating_sub(1),
            character: (span.start.col as u32).saturating_sub(1),
        },
        end: Position {
            line: (span.end.line as u32).saturating_sub(1),
            character: (span.end.col as u32).saturating_sub(1),
        },
    }
}

#[cfg(feature = "interpreter")]
fn source_pos_to_range(pos: &rholang_parser::SourcePos) -> Range {
    let lsp_pos = Position {
        line: (pos.line as u32).saturating_sub(1),
        character: (pos.col as u32).saturating_sub(1),
    };
    Range {
        start: lsp_pos,
        end: lsp_pos,
    }
}

// Stub implementation when interpreter feature is not enabled
#[cfg(not(feature = "interpreter"))]
#[derive(Debug, Clone)]
pub struct SemanticValidator {
    _phantom: std::marker::PhantomData<()>,
}

#[cfg(not(feature = "interpreter"))]
impl SemanticValidator {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            _phantom: std::marker::PhantomData,
        })
    }

    pub async fn validate(&self, _source: &str) -> Vec<Diagnostic> {
        // No-op when interpreter feature is not enabled
        vec![]
    }

    pub fn validate_parsed<'a>(
        &self,
        _ast: rholang_parser::ast::AnnProc<'a>,
        _parser: &'a rholang_parser::RholangParser<'a>,
    ) -> Vec<Diagnostic> {
        // No-op when interpreter feature is not enabled
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "interpreter")]
    #[tokio::test]
    async fn test_unbound_variable() {
        let validator = SemanticValidator::new().unwrap();
        let source = "new x in { y!(42) }"; // 'y' is unbound
        let diagnostics = validator.validate(source).await;

        assert!(!diagnostics.is_empty(), "Should detect unbound variable");
        assert!(
            diagnostics[0].message.contains("free variable")
                || diagnostics[0].message.contains("Unbound")
                || diagnostics[0].message.contains("unbound"),
            "Error message should mention free/unbound variable, got: {:?}",
            diagnostics[0].message
        );
    }

    #[cfg(feature = "interpreter")]
    #[tokio::test]
    async fn test_valid_code() {
        let validator = SemanticValidator::new().unwrap();
        let source = "new x in { x!(42) }"; // Valid code
        let diagnostics = validator.validate(source).await;

        assert_eq!(diagnostics.len(), 0, "Valid code should have no errors");
    }

    #[cfg(not(feature = "interpreter"))]
    #[tokio::test]
    async fn test_no_validation_without_feature() {
        let validator = SemanticValidator::new().unwrap();
        let source = "new x in { y!(42) }"; // Would be invalid
        let diagnostics = validator.validate(source).await;

        assert_eq!(
            diagnostics.len(),
            0,
            "Should not validate when feature is disabled"
        );
    }

    #[cfg(feature = "interpreter")]
    #[tokio::test]
    async fn test_top_level_free_variables() {
        let validator = SemanticValidator::new().unwrap();
        // Top-level free variables are not allowed
        let source = "x!(42)"; // 'x' is a free variable at top level
        let diagnostics = validator.validate(source).await;

        assert!(!diagnostics.is_empty(), "Should detect top-level free variable");
        assert!(
            diagnostics.iter().any(|d| d.message.contains("free variable")
                || d.message.contains("Unbound")),
            "Error message should mention free variable or unbound, got: {:?}",
            diagnostics
        );
    }

    #[cfg(feature = "interpreter")]
    #[tokio::test]
    async fn test_multiple_contracts() {
        let validator = SemanticValidator::new().unwrap();
        // Multiple contracts should be valid when wrapped in new
        let source = r#"
            new foo, bar in {
                contract foo(x) = { x!(42) } |
                contract bar(y) = { y!(100) }
            }
        "#;
        let diagnostics = validator.validate(source).await;

        assert_eq!(diagnostics.len(), 0, "Multiple contracts should be valid, got: {:?}", diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>());
    }

    #[cfg(feature = "interpreter")]
    #[tokio::test]
    async fn test_nested_scopes() {
        let validator = SemanticValidator::new().unwrap();
        // Nested scopes should work correctly
        let source = r#"
            new outer in {
                outer!(42) |
                new inner in {
                    inner!(100) |
                    outer!(200)
                }
            }
        "#;
        let diagnostics = validator.validate(source).await;

        assert_eq!(diagnostics.len(), 0, "Nested scopes should be valid");
    }

    #[cfg(feature = "interpreter")]
    #[tokio::test]
    async fn test_contract_definition() {
        let validator = SemanticValidator::new().unwrap();
        // Contract definition should be valid when wrapped in new
        // Contract parameters are names, so use * to dereference them
        let source = r#"
            new myContract in {
                contract myContract(input, output) = {
                    output!(*input)
                }
            }
        "#;
        let diagnostics = validator.validate(source).await;

        assert_eq!(diagnostics.len(), 0, "Contract definition should be valid, got: {:?}", diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>());
    }

    #[cfg(feature = "interpreter")]
    #[tokio::test]
    async fn test_receive_pattern() {
        let validator = SemanticValidator::new().unwrap();
        // Receive with pattern should be valid
        let source = r#"
            new ch in {
                for (x <- ch) { x!(42) }
            }
        "#;
        let diagnostics = validator.validate(source).await;

        assert_eq!(diagnostics.len(), 0, "Receive pattern should be valid");
    }

    #[cfg(feature = "interpreter")]
    #[tokio::test]
    async fn test_complex_valid_program() {
        let validator = SemanticValidator::new().unwrap();
        // More complex but valid program - contract name must be bound with new
        let source = r#"
            new stdout(`rho:io:stdout`), helloWorld in {
                contract helloWorld(input) = {
                    for (msg <- input) {
                        stdout!(*msg)
                    }
                } |
                new ch in {
                    helloWorld!(*ch) |
                    ch!("Hello, World!")
                }
            }
        "#;
        let diagnostics = validator.validate(source).await;

        assert_eq!(diagnostics.len(), 0, "Complex valid program should have no errors, got: {:?}", diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>());
    }

    #[cfg(feature = "interpreter")]
    #[test]
    fn test_position_mapping() {
        // Test that source positions are correctly mapped to LSP positions
        use rholang_parser::SourceSpan;

        let span = SourceSpan {
            start: rholang_parser::SourcePos { line: 5, col: 10 },
            end: rholang_parser::SourcePos { line: 5, col: 20 },
        };

        let range = source_span_to_range(&span);

        // LSP uses 0-based indexing, rholang-parser uses 1-based
        assert_eq!(range.start.line, 4, "Line should be converted to 0-based");
        assert_eq!(range.start.character, 9, "Column should be converted to 0-based");
        assert_eq!(range.end.line, 4);
        assert_eq!(range.end.character, 19);
    }
}
