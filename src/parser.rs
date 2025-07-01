use anyhow::Result;
use rholang_parser::{RholangParser, errors::{ParseResult, ParserError}};
use tracing::{debug, error};

/// A wrapper around `RholangParser` for syntax validation and tree generation.
pub struct Parser {
    inner: RholangParser,
}

impl std::fmt::Debug for Parser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Minimal Debug implementation that avoids requiring `RholangParser` to implement Debug
        f.debug_struct("Parser").finish()
    }
}

impl Parser {
    /// Creates a new `Parser` instance with an initialized `RholangParser`.
    pub fn new() -> Result<Self> {
        let inner = RholangParser::new()?;
        Ok(Parser { inner })
    }

    /// Validates the syntax of the provided Rholang code.
    /// Returns `Ok(())` if valid, or an error with diagnostic information if invalid.
    pub fn validate(&mut self, code: &str) -> Result<(), ParserError> {
        match self.inner.parse(code) {
            ParseResult::Success(()) => {
                debug!("Syntax validation successful for code snippet");
                Ok(())
            }
            ParseResult::Error(err) => {
                error!("Syntax validation failed: {}", err);
                Err(err)
            }
        }
    }

    /// Generates a pretty-printed string representation of the parse tree.
    pub fn get_pretty_tree(&mut self, code: &str) -> Result<String, String> {
        match self.inner.get_pretty_tree(code) {
            ParseResult::Success(tree) => {
                debug!("Generated pretty tree for code snippet");
                Ok(tree)
            }
            ParseResult::Error(err) => {
                error!("Failed to generate pretty tree: {}", err);
                Err(err.message)
            }
        }
    }
}
