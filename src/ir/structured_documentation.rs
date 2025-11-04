//! Structured Documentation Parser
//!
//! **Phase 7**: Parses documentation comments with support for:
//! - Multi-line doc comment aggregation
//! - Structured tags: @param, @return, @example, @throws
//! - Formatted output for LSP features
//!
//! # Example
//!
//! ```rholang
//! /// Authenticates a user with credentials
//! ///
//! /// @param username The user's login name
//! /// @param password The user's password
//! /// @return Authentication token on success
//! /// @example authenticate!("alice", "secret123")
//! contract authenticate(@username, @password) = { ... }
//! ```

use std::collections::HashMap;
use serde::{Serialize, Deserialize};

/// Structured documentation for a symbol
///
/// Stores documentation with parsed tags for richer display in IDEs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StructuredDocumentation {
    /// Main description/summary (lines before any @tags)
    pub summary: String,

    /// Parameter documentation (@param name description)
    pub params: Vec<ParamDoc>,

    /// Return value documentation (@return description)
    pub returns: Option<String>,

    /// Code examples (@example code)
    pub examples: Vec<String>,

    /// Exception/error documentation (@throws description)
    pub throws: Vec<String>,

    /// Additional custom tags (@tag content)
    pub custom_tags: HashMap<String, Vec<String>>,
}

/// Documentation for a single parameter
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ParamDoc {
    /// Parameter name (e.g., "username")
    pub name: String,

    /// Parameter description
    pub description: String,
}

impl StructuredDocumentation {
    /// Create a new empty StructuredDocumentation
    pub fn new() -> Self {
        Self {
            summary: String::new(),
            params: Vec::new(),
            returns: None,
            examples: Vec::new(),
            throws: Vec::new(),
            custom_tags: HashMap::new(),
        }
    }

    /// Parse structured documentation from raw doc comment text
    ///
    /// Supports aggregating multiple consecutive doc comments and parsing
    /// structured tags like @param, @return, @example.
    ///
    /// # Arguments
    /// * `doc_texts` - Iterator of cleaned doc comment texts (from doc_text())
    ///
    /// # Returns
    /// Parsed StructuredDocumentation with summary and tags
    ///
    /// # Example
    /// ```rust,ignore
    /// let docs = vec![
    ///     "Authenticates a user",
    ///     "",
    ///     "@param username The user's name",
    ///     "@return Authentication token",
    /// ];
    /// let structured = StructuredDocumentation::parse(docs.into_iter());
    /// assert_eq!(structured.summary, "Authenticates a user");
    /// assert_eq!(structured.params.len(), 1);
    /// ```
    pub fn parse<'a, I>(doc_texts: I) -> Self
    where
        I: Iterator<Item = &'a str>,
    {
        let mut result = Self::new();
        let mut summary_lines = Vec::new();
        let mut current_tag: Option<String> = None;
        let mut current_tag_content = String::new();

        for line in doc_texts {
            let trimmed = line.trim();

            // Check if line starts with a tag
            if let Some(tag_line) = trimmed.strip_prefix('@') {
                // Flush previous tag if any
                if let Some(tag_name) = current_tag.take() {
                    Self::add_tag(&mut result, &tag_name, &current_tag_content);
                    current_tag_content.clear();
                }

                // Parse new tag
                if let Some((tag, content)) = tag_line.split_once(char::is_whitespace) {
                    current_tag = Some(tag.to_string());
                    current_tag_content = content.trim().to_string();
                } else {
                    // Tag without content on same line
                    current_tag = Some(tag_line.to_string());
                }
            } else if let Some(_) = &current_tag {
                // Continuation of current tag
                if !current_tag_content.is_empty() {
                    current_tag_content.push(' ');
                }
                current_tag_content.push_str(trimmed);
            } else {
                // Part of summary (before any tags)
                summary_lines.push(trimmed.to_string());
            }
        }

        // Flush final tag
        if let Some(tag_name) = current_tag {
            Self::add_tag(&mut result, &tag_name, &current_tag_content);
        }

        // Join summary lines, preserving paragraph breaks
        result.summary = summary_lines.join("\n").trim().to_string();

        result
    }

    /// Add a parsed tag to the structured documentation
    fn add_tag(doc: &mut StructuredDocumentation, tag: &str, content: &str) {
        match tag {
            "param" => {
                // Parse: @param name description
                if let Some((name, description)) = content.split_once(char::is_whitespace) {
                    doc.params.push(ParamDoc {
                        name: name.trim().to_string(),
                        description: description.trim().to_string(),
                    });
                } else {
                    // Just a parameter name without description
                    doc.params.push(ParamDoc {
                        name: content.trim().to_string(),
                        description: String::new(),
                    });
                }
            }
            "return" | "returns" => {
                doc.returns = Some(content.trim().to_string());
            }
            "example" => {
                doc.examples.push(content.trim().to_string());
            }
            "throws" | "throw" => {
                doc.throws.push(content.trim().to_string());
            }
            _ => {
                // Custom tag
                doc.custom_tags
                    .entry(tag.to_string())
                    .or_insert_with(Vec::new)
                    .push(content.trim().to_string());
            }
        }
    }

    /// Format as plain text for display (backwards compatible)
    ///
    /// Returns the documentation as a plain string, suitable for displaying
    /// in hover tooltips or completion item documentation.
    ///
    /// # Returns
    /// Formatted documentation string with summary, parameters, return value, and examples
    pub fn to_plain_text(&self) -> String {
        let mut result = String::new();

        // Summary
        if !self.summary.is_empty() {
            result.push_str(&self.summary);
        }

        // Parameters
        if !self.params.is_empty() {
            if !result.is_empty() {
                result.push_str("\n\n");
            }
            result.push_str("Parameters:");
            for param in &self.params {
                result.push_str(&format!("\n  @{}", param.name));
                if !param.description.is_empty() {
                    result.push_str(&format!(" - {}", param.description));
                }
            }
        }

        // Return value
        if let Some(returns) = &self.returns {
            if !result.is_empty() {
                result.push_str("\n\n");
            }
            result.push_str(&format!("Returns: {}", returns));
        }

        // Examples
        if !self.examples.is_empty() {
            if !result.is_empty() {
                result.push_str("\n\n");
            }
            result.push_str("Examples:");
            for example in &self.examples {
                result.push_str(&format!("\n  {}", example));
            }
        }

        // Throws
        if !self.throws.is_empty() {
            if !result.is_empty() {
                result.push_str("\n\n");
            }
            result.push_str("Throws:");
            for throw in &self.throws {
                result.push_str(&format!("\n  {}", throw));
            }
        }

        result
    }

    /// Format as markdown for LSP hover responses
    ///
    /// Returns documentation formatted as markdown, with proper headings,
    /// code blocks, and formatting.
    ///
    /// # Returns
    /// Markdown-formatted documentation string
    pub fn to_markdown(&self) -> String {
        let mut result = String::new();

        // Summary
        if !self.summary.is_empty() {
            result.push_str(&self.summary);
        }

        // Parameters
        if !self.params.is_empty() {
            if !result.is_empty() {
                result.push_str("\n\n");
            }
            result.push_str("## Parameters\n\n");
            for param in &self.params {
                result.push_str(&format!("- **{}**", param.name));
                if !param.description.is_empty() {
                    result.push_str(&format!(": {}", param.description));
                }
                result.push('\n');
            }
        }

        // Return value
        if let Some(returns) = &self.returns {
            if !result.is_empty() {
                result.push_str("\n");
            }
            result.push_str(&format!("## Returns\n\n{}\n", returns));
        }

        // Examples
        if !self.examples.is_empty() {
            if !result.is_empty() {
                result.push_str("\n");
            }
            result.push_str("## Examples\n\n");
            for example in &self.examples {
                result.push_str(&format!("```rholang\n{}\n```\n\n", example));
            }
        }

        // Throws
        if !self.throws.is_empty() {
            if !result.is_empty() {
                result.push_str("\n");
            }
            result.push_str("## Throws\n\n");
            for throw in &self.throws {
                result.push_str(&format!("- {}\n", throw));
            }
        }

        result.trim_end().to_string()
    }
}

impl Default for StructuredDocumentation {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_summary() {
        let docs = vec!["This is a simple summary"];
        let structured = StructuredDocumentation::parse(docs.into_iter());

        assert_eq!(structured.summary, "This is a simple summary");
        assert!(structured.params.is_empty());
        assert!(structured.returns.is_none());
    }

    #[test]
    fn test_parse_multi_line_summary() {
        let docs = vec![
            "This is line 1",
            "This is line 2",
            "This is line 3",
        ];
        let structured = StructuredDocumentation::parse(docs.into_iter());

        assert_eq!(structured.summary, "This is line 1\nThis is line 2\nThis is line 3");
    }

    #[test]
    fn test_parse_with_params() {
        let docs = vec![
            "Authenticates a user",
            "@param username The user's name",
            "@param password The user's password",
        ];
        let structured = StructuredDocumentation::parse(docs.into_iter());

        assert_eq!(structured.summary, "Authenticates a user");
        assert_eq!(structured.params.len(), 2);
        assert_eq!(structured.params[0].name, "username");
        assert_eq!(structured.params[0].description, "The user's name");
        assert_eq!(structured.params[1].name, "password");
        assert_eq!(structured.params[1].description, "The user's password");
    }

    #[test]
    fn test_parse_with_return() {
        let docs = vec![
            "Gets user info",
            "@return User information object",
        ];
        let structured = StructuredDocumentation::parse(docs.into_iter());

        assert_eq!(structured.summary, "Gets user info");
        assert_eq!(structured.returns, Some("User information object".to_string()));
    }

    #[test]
    fn test_parse_with_example() {
        let docs = vec![
            "Adds two numbers",
            "@example add!(1, 2)",
        ];
        let structured = StructuredDocumentation::parse(docs.into_iter());

        assert_eq!(structured.summary, "Adds two numbers");
        assert_eq!(structured.examples.len(), 1);
        assert_eq!(structured.examples[0], "add!(1, 2)");
    }

    #[test]
    fn test_parse_complete_documentation() {
        let docs = vec![
            "Authenticates a user with credentials",
            "",
            "This contract validates the provided username and password",
            "against the authentication service.",
            "",
            "@param username The user's login name",
            "@param password The user's password",
            "@return Authentication token on success",
            "@throws AuthenticationError if credentials are invalid",
            "@example authenticate!(\"alice\", \"secret123\")",
        ];
        let structured = StructuredDocumentation::parse(docs.into_iter());

        assert!(structured.summary.contains("Authenticates a user"));
        assert!(structured.summary.contains("authentication service"));
        assert_eq!(structured.params.len(), 2);
        assert_eq!(structured.params[0].name, "username");
        assert_eq!(structured.returns, Some("Authentication token on success".to_string()));
        assert_eq!(structured.throws.len(), 1);
        assert_eq!(structured.examples.len(), 1);
    }

    #[test]
    fn test_to_plain_text() {
        let docs = vec![
            "Authenticates a user",
            "@param username The user's name",
            "@return Authentication token",
        ];
        let structured = StructuredDocumentation::parse(docs.into_iter());
        let plain_text = structured.to_plain_text();

        assert!(plain_text.contains("Authenticates a user"));
        assert!(plain_text.contains("Parameters:"));
        assert!(plain_text.contains("@username"));
        assert!(plain_text.contains("Returns:"));
    }

    #[test]
    fn test_to_markdown() {
        let docs = vec![
            "Authenticates a user",
            "@param username The user's name",
            "@return Authentication token",
            "@example authenticate!(\"alice\", \"pass\")",
        ];
        let structured = StructuredDocumentation::parse(docs.into_iter());
        let markdown = structured.to_markdown();

        assert!(markdown.contains("Authenticates a user"));
        assert!(markdown.contains("## Parameters"));
        assert!(markdown.contains("**username**"));
        assert!(markdown.contains("## Returns"));
        assert!(markdown.contains("## Examples"));
        assert!(markdown.contains("```rholang"));
    }
}
