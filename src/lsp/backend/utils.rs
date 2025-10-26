//! Utility types and functions for the LSP backend

use tower_lsp::lsp_types::SemanticToken;

/// Helper for building semantic tokens using delta encoding
///
/// LSP semantic tokens use delta encoding where each token's position
/// is relative to the previous token, reducing payload size.
pub(super) struct SemanticTokensBuilder {
    tokens: Vec<SemanticToken>,
    prev_line: u32,
    prev_start: u32,
}

impl SemanticTokensBuilder {
    pub(super) fn new() -> Self {
        Self {
            tokens: Vec::new(),
            prev_line: 0,
            prev_start: 0,
        }
    }

    /// Add a semantic token with absolute position
    ///
    /// The builder automatically converts to delta encoding
    pub(super) fn push(&mut self, line: u32, start: u32, length: u32, token_type: u32) {
        let delta_line = if line >= self.prev_line {
            line - self.prev_line
        } else {
            // Should not happen in well-formed code
            0
        };

        let delta_start = if delta_line == 0 && start >= self.prev_start {
            start - self.prev_start
        } else if delta_line == 0 {
            // Should not happen - tokens on same line should be in order
            0
        } else {
            start
        };

        self.tokens.push(SemanticToken {
            delta_line,
            delta_start,
            length,
            token_type,
            token_modifiers_bitset: 0,
        });

        self.prev_line = line;
        self.prev_start = start;
    }

    /// Build the final vector of semantic tokens
    pub(super) fn build(self) -> Vec<SemanticToken> {
        self.tokens
    }
}
