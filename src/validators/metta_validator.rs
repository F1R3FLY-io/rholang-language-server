//! MeTTa validator using MeTTaTron's compile_safe API
//!
//! This module provides validation for MeTTa source code by using
//! MeTTaTron's safe compilation API that never panics.

use mettatron::rholang_integration::compile_safe;
use mettatron::backend::models::MettaValue;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};

/// Validator for MeTTa source code
pub struct MettaValidator;

impl MettaValidator {
    /// Create a new MeTTa validator
    pub fn new() -> Self {
        Self
    }

    /// Validate MeTTa source code and return diagnostics
    ///
    /// Uses MeTTaTron's `compile_safe()` which never panics.
    /// Returns error diagnostics if the code has syntax errors.
    pub fn validate(&self, source: &str) -> Vec<Diagnostic> {
        let state = compile_safe(source);

        // Check if the state contains error s-expressions
        let mut diagnostics = Vec::new();

        for value in &state.source {
            if let Some(diag) = self.extract_error_diagnostic(value, source) {
                diagnostics.push(diag);
            }
        }

        diagnostics
    }

    /// Extract a diagnostic from an error s-expression
    ///
    /// Error s-expressions have the form: (error "message")
    fn extract_error_diagnostic(&self, value: &MettaValue, source: &str) -> Option<Diagnostic> {
        match value {
            MettaValue::SExpr(items) if items.len() == 2 => {
                // Check if this is (error "message")
                if let (MettaValue::Atom(op), MettaValue::String(msg)) = (&items[0], &items[1]) {
                    if op == "error" {
                        return Some(self.create_diagnostic_from_error(msg, source));
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Create a diagnostic from an error message
    ///
    /// Attempts to extract line/column information from the error message.
    /// If no position info is found, reports the error at the start of the file.
    fn create_diagnostic_from_error(&self, message: &str, source: &str) -> Diagnostic {
        // Try to extract position information from the error message
        // Common patterns: "line X, column Y", "at line X", "column Y"
        let (line, column) = self.extract_position_from_error(message);

        // Validate position is within source bounds
        let (line, column) = self.validate_position(line, column, source);

        // Create a range for the error (single character or line)
        let range = self.create_error_range(line, column, source);

        Diagnostic {
            range,
            severity: Some(DiagnosticSeverity::ERROR),
            source: Some("metta-parser".to_string()),
            message: message.to_string(),
            ..Default::default()
        }
    }

    /// Extract line and column from error message
    ///
    /// Returns (line, column) as 0-based indices.
    /// Returns (0, 0) if position cannot be extracted.
    fn extract_position_from_error(&self, message: &str) -> (u32, u32) {
        // Pattern 1: "line X, column Y"
        if let Some(line_col) = self.extract_line_column_pattern(message) {
            return line_col;
        }

        // Pattern 2: "at line X"
        if let Some(line) = self.extract_line_pattern(message) {
            return (line, 0);
        }

        // Default to start of file
        (0, 0)
    }

    /// Extract "line X, column Y" pattern
    fn extract_line_column_pattern(&self, message: &str) -> Option<(u32, u32)> {
        // Look for "line X, column Y"
        let line_idx = message.find("line ")?;
        let col_idx = message.find(", column ")?;

        let line_start = line_idx + 5; // "line ".len()
        let line_end = col_idx;
        let col_start = col_idx + 9; // ", column ".len()

        // Extract line number
        let line_str = &message[line_start..line_end];
        let line: u32 = line_str.trim().parse().ok()?;

        // Extract column number (look for next space or end)
        let col_end = message[col_start..]
            .find(|c: char| !c.is_ascii_digit())
            .map(|i| col_start + i)
            .unwrap_or(message.len());
        let col_str = &message[col_start..col_end];
        let col: u32 = col_str.trim().parse().ok()?;

        // Convert to 0-based indices (LSP uses 0-based)
        Some((line.saturating_sub(1), col.saturating_sub(1)))
    }

    /// Extract "at line X" pattern
    fn extract_line_pattern(&self, message: &str) -> Option<u32> {
        let line_idx = message.find("line ")?;
        let line_start = line_idx + 5;

        // Find end of line number
        let line_end = message[line_start..]
            .find(|c: char| !c.is_ascii_digit())
            .map(|i| line_start + i)
            .unwrap_or(message.len());

        let line_str = &message[line_start..line_end];
        let line: u32 = line_str.trim().parse().ok()?;

        // Convert to 0-based
        Some(line.saturating_sub(1))
    }

    /// Validate that position is within source bounds
    fn validate_position(&self, line: u32, column: u32, source: &str) -> (u32, u32) {
        let lines: Vec<&str> = source.lines().collect();
        let max_line = lines.len().saturating_sub(1) as u32;

        let line = line.min(max_line);
        let max_column = if let Some(line_text) = lines.get(line as usize) {
            line_text.len() as u32
        } else {
            0
        };

        let column = column.min(max_column);

        (line, column)
    }

    /// Create an error range from position
    ///
    /// Creates a range that highlights either:
    /// - A single character at the error position
    /// - The entire line if at line start
    fn create_error_range(&self, line: u32, column: u32, source: &str) -> Range {
        let lines: Vec<&str> = source.lines().collect();

        let end_column = if let Some(line_text) = lines.get(line as usize) {
            if column == 0 {
                // Highlight entire line
                line_text.len() as u32
            } else {
                // Highlight single character
                (column + 1).min(line_text.len() as u32)
            }
        } else {
            column
        };

        Range {
            start: Position { line, character: column },
            end: Position { line, character: end_column },
        }
    }
}

/// Default implementation
impl Default for MettaValidator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_valid_code() {
        let validator = MettaValidator::new();
        let diagnostics = validator.validate("(+ 1 2)");
        assert_eq!(diagnostics.len(), 0);
    }

    #[test]
    fn test_validate_multiple_expressions() {
        let validator = MettaValidator::new();
        let diagnostics = validator.validate("(+ 1 2)\n(* 3 4)");
        assert_eq!(diagnostics.len(), 0);
    }

    #[test]
    fn test_validate_invalid_syntax() {
        let validator = MettaValidator::new();
        let diagnostics = validator.validate("(+ 1 2"); // Unclosed parenthesis
        assert!(!diagnostics.is_empty(), "Expected error diagnostic for unclosed parenthesis");

        let diag = &diagnostics[0];
        assert_eq!(diag.severity, Some(DiagnosticSeverity::ERROR));
        assert_eq!(diag.source, Some("metta-parser".to_string()));
    }

    #[test]
    fn test_validate_empty_source() {
        let validator = MettaValidator::new();
        let diagnostics = validator.validate("");
        assert_eq!(diagnostics.len(), 0);
    }

    #[test]
    fn test_extract_position_line_column() {
        let validator = MettaValidator::new();
        let (line, col) = validator.extract_position_from_error(
            "Syntax error at line 5, column 12: unexpected token"
        );
        assert_eq!(line, 4); // 0-based
        assert_eq!(col, 11); // 0-based
    }

    #[test]
    fn test_extract_position_line_only() {
        let validator = MettaValidator::new();
        let (line, col) = validator.extract_position_from_error(
            "Error at line 3: something went wrong"
        );
        assert_eq!(line, 2); // 0-based
        assert_eq!(col, 0);
    }

    #[test]
    fn test_extract_position_no_info() {
        let validator = MettaValidator::new();
        let (line, col) = validator.extract_position_from_error(
            "Generic error message without position"
        );
        assert_eq!(line, 0);
        assert_eq!(col, 0);
    }

    #[test]
    fn test_validate_position_bounds() {
        let validator = MettaValidator::new();
        let source = "line 1\nline 2\nline 3";

        // Valid position
        let (line, col) = validator.validate_position(1, 3, source);
        assert_eq!(line, 1);
        assert_eq!(col, 3);

        // Line out of bounds
        let (line, col) = validator.validate_position(10, 0, source);
        assert_eq!(line, 2); // Last line (0-based)

        // Column out of bounds
        let (line, col) = validator.validate_position(1, 100, source);
        assert_eq!(line, 1);
        assert_eq!(col, 6); // "line 2".len()
    }

    #[test]
    fn test_create_error_range() {
        let validator = MettaValidator::new();
        let source = "hello world";

        // Single character at position
        let range = validator.create_error_range(0, 5, source);
        assert_eq!(range.start.line, 0);
        assert_eq!(range.start.character, 5);
        assert_eq!(range.end.line, 0);
        assert_eq!(range.end.character, 6);

        // Entire line at start
        let range = validator.create_error_range(0, 0, source);
        assert_eq!(range.start.line, 0);
        assert_eq!(range.start.character, 0);
        assert_eq!(range.end.line, 0);
        assert_eq!(range.end.character, 11); // "hello world".len()
    }
}
