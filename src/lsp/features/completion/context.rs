//! Context detection for code completion
//!
//! This module determines the completion context at a given cursor position,
//! enabling context-sensitive filtering and ranking of completion suggestions.
//!
//! Rholang has 7 primary completion contexts:
//! 1. Lexical scope contexts (New, Let, Contract, For, Match)
//! 2. Type-based contexts (List, Map, Set, String, Int methods)
//! 3. Keyword contexts (language keywords)
//! 4. Built-in/stdlib contexts (rho:io:* URIs)
//! 5. Pattern contexts (in contract formals, for bindings)
//! 6. Import/module contexts (unforgeable names)
//! 7. Virtual document contexts (embedded languages like MeTTa)

use crate::ir::rholang_node::RholangNode;
use crate::ir::semantic_node::{Position, SemanticNode};
use std::sync::Arc;
use tracing::debug;

/// Type of completion context
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionContextType {
    /// Inside a lexical scope (can access scope variables + outer scopes)
    LexicalScope {
        /// Scope ID for symbol table lookup
        scope_id: usize,
    },

    /// After a dot operator on a known type (e.g., list.length())
    TypeMethod {
        /// Type name: "List", "Map", "Set", "String", "Int", etc.
        type_name: String,
    },

    /// Top-level or general expression context (suggest keywords, contracts, etc.)
    Expression,

    /// Inside a pattern (contract formals, for bindings, match cases)
    Pattern,

    /// Inside a string literal (suggest rho:io:* URIs or nothing)
    StringLiteral,

    /// Inside a quoted map pattern (e.g., @{key: value})
    QuotedMapPattern {
        /// Keys already present in the partial map
        keys_so_far: Vec<String>,
    },

    /// Inside a quoted list pattern (e.g., @[element1, element2])
    QuotedListPattern {
        /// Number of elements already present
        elements_so_far: usize,
    },

    /// Inside a quoted tuple pattern (e.g., @(a, b, c))
    QuotedTuplePattern {
        /// Number of elements already present
        elements_so_far: usize,
    },

    /// Inside a quoted set pattern (e.g., @Set(a, b))
    QuotedSetPattern {
        /// Number of elements already present
        elements_so_far: usize,
    },

    /// Inside a virtual document (embedded language like MeTTa)
    VirtualDocument {
        /// Language name
        language: String,
    },

    /// Unknown context (fallback)
    Unknown,
}

/// Completion context at a cursor position
#[derive(Debug, Clone)]
pub struct CompletionContext {
    /// Type of context
    pub context_type: CompletionContextType,

    /// Current IR node at cursor position (if available)
    pub current_node: Option<Arc<RholangNode>>,

    /// Parent IR node (useful for determining context)
    pub parent_node: Option<Arc<RholangNode>>,

    /// Partial identifier being typed (for prefix matching)
    pub partial_identifier: Option<String>,

    /// Whether cursor is after a trigger character (., @, !, etc.)
    pub after_trigger: bool,
}

impl CompletionContext {
    /// Create a new unknown context
    pub fn unknown() -> Self {
        Self {
            context_type: CompletionContextType::Unknown,
            current_node: None,
            parent_node: None,
            partial_identifier: None,
            after_trigger: false,
        }
    }

    /// Create a lexical scope context
    pub fn lexical_scope(scope_id: usize, current_node: Option<Arc<RholangNode>>) -> Self {
        Self {
            context_type: CompletionContextType::LexicalScope { scope_id },
            current_node,
            parent_node: None,
            partial_identifier: None,
            after_trigger: false,
        }
    }

    /// Create a type method context
    pub fn type_method(type_name: String, current_node: Option<Arc<RholangNode>>) -> Self {
        Self {
            context_type: CompletionContextType::TypeMethod { type_name },
            current_node,
            parent_node: None,
            partial_identifier: None,
            after_trigger: true,
        }
    }

    /// Create an expression context
    pub fn expression(current_node: Option<Arc<RholangNode>>) -> Self {
        Self {
            context_type: CompletionContextType::Expression,
            current_node,
            parent_node: None,
            partial_identifier: None,
            after_trigger: false,
        }
    }

    /// Create a pattern context
    pub fn pattern(current_node: Option<Arc<RholangNode>>) -> Self {
        Self {
            context_type: CompletionContextType::Pattern,
            current_node,
            parent_node: None,
            partial_identifier: None,
            after_trigger: false,
        }
    }
}

/// Determine the completion context at a given position in the IR
///
/// # Arguments
/// * `ir` - The IR tree to search
/// * `position` - Cursor position in the document (as LSP Position)
///
/// # Returns
/// CompletionContext describing what kind of completion is appropriate
pub fn determine_context(
    ir: &Arc<RholangNode>,
    position: &tower_lsp::lsp_types::Position,
) -> CompletionContext {
    use crate::lsp::features::node_finder::find_node_at_position;

    // Convert LSP Position to IR Position
    let ir_position = Position {
        row: position.line as usize,
        column: position.character as usize,
        byte: 0,  // Not needed for position lookup
    };

    // Find the node at this position
    let node = match find_node_at_position(ir.as_ref(), &ir_position) {
        Some(n) => n,
        None => return CompletionContext::expression(None),
    };

    // Get scope ID from metadata
    let scope_id = extract_scope_id(node);

    // Check if we're inside a quoted pattern (e.g., @{key: value}, @[...], @"string")
    use crate::ir::semantic_node::SemanticNodeExt;
    if let Some(rholang_node) = node.as_rholang() {
        use crate::ir::rholang_node::RholangNode;

        // Check for Quote node first (pattern-aware completion)
        if let RholangNode::Quote { quotable, .. } = rholang_node {
            if let Some(context) = extract_quoted_pattern_context(quotable.as_ref()) {
                debug!("Quoted pattern context detected: {:?}", context.context_type);
                return context;
            }
        }

        // Check if this is a Method node (for method completion after dot operator)
        if let RholangNode::Method { receiver, .. } = rholang_node {
            // We're in a method call - infer the receiver type
            if let Some(type_name) = infer_simple_type(receiver.as_ref()) {
                debug!("Method completion context detected for type: {}", type_name);
                return CompletionContext::type_method(type_name, None);
            }
        }
    }

    // Determine context type based on node category
    use crate::ir::semantic_node::SemanticCategory;
    match node.semantic_category() {
        SemanticCategory::Variable => {
            // In a variable reference - provide all visible symbols
            if let Some(scope) = scope_id {
                CompletionContext::lexical_scope(scope, None)
            } else {
                CompletionContext::expression(None)
            }
        }
        SemanticCategory::Invocation => {
            // In a contract/function call
            if let Some(scope) = scope_id {
                CompletionContext::lexical_scope(scope, None)
            } else {
                CompletionContext::expression(None)
            }
        }
        SemanticCategory::Binding | SemanticCategory::Match => {
            // In a binding or pattern matching position (contract formals, for bindings, match cases)
            CompletionContext::pattern(None)
        }
        SemanticCategory::Literal => {
            // In a literal - likely string, limited completions
            CompletionContext::string_literal(None)
        }
        _ => {
            // Default to expression context
            if let Some(scope) = scope_id {
                CompletionContext::lexical_scope(scope, None)
            } else {
                CompletionContext::expression(None)
            }
        }
    }
}

/// Infer the type of an expression for method completion
///
/// This is a simple type inference that handles literal types and known collection types.
/// For Phase 3.2, this can be enhanced with full type inference.
fn infer_simple_type(node: &RholangNode) -> Option<String> {
    use crate::ir::rholang_node::RholangNode;

    match node {
        // Literal types
        RholangNode::BoolLiteral { .. } => Some("Bool".to_string()),
        RholangNode::LongLiteral { .. } => Some("Int".to_string()),
        RholangNode::StringLiteral { .. } => Some("String".to_string()),
        RholangNode::UriLiteral { .. } => Some("Uri".to_string()),

        // Collection types
        RholangNode::List { .. } => Some("List".to_string()),
        RholangNode::Set { .. } => Some("Set".to_string()),
        RholangNode::Map { .. } => Some("Map".to_string()),
        RholangNode::Pathmap { .. } => Some("PathMap".to_string()),
        RholangNode::Tuple { .. } => Some("Tuple".to_string()),

        // For variables, we would need full type inference (Phase 3.2)
        // For now, return None for complex expressions
        _ => None,
    }
}

/// Extract scope ID from node metadata
fn extract_scope_id(node: &dyn SemanticNode) -> Option<usize> {
    use std::any::Any;

    let metadata = node.metadata()?;
    let scope_id_any = metadata.get("scope_id")?;
    scope_id_any.downcast_ref::<usize>().copied()
}

/// Extract quoted pattern context from a Quote node
///
/// This function determines if we're inside a quoted pattern (like @{key: value})
/// and returns the appropriate context type with metadata about the pattern.
///
/// # Arguments
/// * `quoted_node` - The process being quoted (inside the @ operator)
///
/// # Returns
/// CompletionContext for the quoted pattern, or None if not a recognized pattern
fn extract_quoted_pattern_context(quoted_node: &RholangNode) -> Option<CompletionContext> {
    match quoted_node {
        RholangNode::StringLiteral { .. } => {
            // @"string" - already handled by string_literal context
            Some(CompletionContext::string_literal(None))
        }
        RholangNode::Map { pairs, .. } => {
            // @{key: value, ...} - extract existing keys
            let keys: Vec<String> = pairs
                .iter()
                .filter_map(|(k, _)| {
                    if let RholangNode::StringLiteral { value, .. } = k.as_ref() {
                        Some(value.clone())
                    } else {
                        None
                    }
                })
                .collect();
            Some(CompletionContext::quoted_map_pattern(keys, None))
        }
        RholangNode::List { elements, .. } => {
            // @[element1, element2, ...]
            Some(CompletionContext::quoted_list_pattern(elements.len(), None))
        }
        RholangNode::Tuple { elements, .. } => {
            // @(element1, element2, ...)
            Some(CompletionContext::quoted_tuple_pattern(elements.len(), None))
        }
        RholangNode::Set { elements, .. } => {
            // @Set(element1, element2, ...)
            Some(CompletionContext::quoted_set_pattern(elements.len(), None))
        }
        _ => None,
    }
}

/// Extract partial identifier at cursor position from document text
///
/// This function walks backward and forward from the cursor position to extract
/// the identifier being typed. Rholang identifiers can contain:
/// - Letters (a-z, A-Z)
/// - Digits (0-9)
/// - Underscores (_)
///
/// # Arguments
/// * `text` - Document text as a Rope
/// * `position` - Cursor position (LSP coordinates)
///
/// # Returns
/// The partial identifier at the cursor, or empty string if not in an identifier
pub fn extract_partial_identifier(
    text: &ropey::Rope,
    position: &tower_lsp::lsp_types::Position,
) -> String {
    let line_idx = position.line as usize;
    let char_idx = position.character as usize;

    // Get the line text
    let line = match text.get_line(line_idx) {
        Some(line) => line,
        None => return String::new(),
    };

    let line_text: String = line.chars().collect();

    // Handle empty lines or cursor beyond line end
    if line_text.is_empty() || char_idx > line_text.len() {
        return String::new();
    }

    // Find start of identifier (walk backward)
    let mut start = char_idx;
    while start > 0 {
        let ch = line_text.chars().nth(start - 1);
        match ch {
            Some(c) if is_identifier_char(c) => start -= 1,
            _ => break,
        }
    }

    // Find end of identifier (walk forward)
    let mut end = char_idx;
    while end < line_text.len() {
        let ch = line_text.chars().nth(end);
        match ch {
            Some(c) if is_identifier_char(c) => end += 1,
            _ => break,
        }
    }

    // Extract the identifier
    if start < end {
        line_text.chars().skip(start).take(end - start).collect()
    } else {
        String::new()
    }
}

/// Check if a character is valid in a Rholang identifier
fn is_identifier_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

// Helper methods for CompletionContext
impl CompletionContext {
    /// Create a string literal context
    pub fn string_literal(current_node: Option<Arc<RholangNode>>) -> Self {
        Self {
            context_type: CompletionContextType::StringLiteral,
            current_node,
            parent_node: None,
            partial_identifier: None,
            after_trigger: false,
        }
    }

    /// Create a quoted map pattern context
    pub fn quoted_map_pattern(keys_so_far: Vec<String>, current_node: Option<Arc<RholangNode>>) -> Self {
        Self {
            context_type: CompletionContextType::QuotedMapPattern { keys_so_far },
            current_node,
            parent_node: None,
            partial_identifier: None,
            after_trigger: false,
        }
    }

    /// Create a quoted list pattern context
    pub fn quoted_list_pattern(elements_so_far: usize, current_node: Option<Arc<RholangNode>>) -> Self {
        Self {
            context_type: CompletionContextType::QuotedListPattern { elements_so_far },
            current_node,
            parent_node: None,
            partial_identifier: None,
            after_trigger: false,
        }
    }

    /// Create a quoted tuple pattern context
    pub fn quoted_tuple_pattern(elements_so_far: usize, current_node: Option<Arc<RholangNode>>) -> Self {
        Self {
            context_type: CompletionContextType::QuotedTuplePattern { elements_so_far },
            current_node,
            parent_node: None,
            partial_identifier: None,
            after_trigger: false,
        }
    }

    /// Create a quoted set pattern context
    pub fn quoted_set_pattern(elements_so_far: usize, current_node: Option<Arc<RholangNode>>) -> Self {
        Self {
            context_type: CompletionContextType::QuotedSetPattern { elements_so_far },
            current_node,
            parent_node: None,
            partial_identifier: None,
            after_trigger: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_creation() {
        // Test basic context construction
        let ctx = CompletionContext::unknown();
        assert_eq!(ctx.context_type, CompletionContextType::Unknown);

        let ctx2 = CompletionContext::lexical_scope(0, None);
        match ctx2.context_type {
            CompletionContextType::LexicalScope { scope_id } => {
                assert_eq!(scope_id, 0);
            }
            _ => panic!("Expected LexicalScope"),
        }
    }
}
