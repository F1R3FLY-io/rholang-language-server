//! Pattern-aware code completion for quoted processes
//!
//! This module implements code completion for contract identifiers that use
//! quoted processes (e.g., `@"foo"`, `@{key: value}`, `@[element]`).
//!
//! It leverages the MORK+PathMap pattern matching infrastructure to suggest
//! contracts that match the partial pattern being typed at the cursor position.
//!
//! # Architecture
//!
//! 1. **Pattern Extraction**: Extract the partial pattern at cursor position
//! 2. **MORK Conversion**: Convert partial pattern to MORK canonical form
//! 3. **Pattern Index Query**: Query RholangPatternIndex for matching contracts
//! 4. **Ranking**: Rank results by pattern match quality
//!
//! # Example
//!
//! ```rholang
//! contract @"myContract"(@x) = { Nil }
//! contract @"otherContract"(@y) = { Nil }
//!
//! // User types: @"my|"  (cursor at |)
//! // → Completion suggests: @"myContract"
//! ```

use crate::ir::global_index::GlobalSymbolIndex;
use crate::ir::rholang_node::RholangNode;
use crate::ir::rholang_pattern_index::RholangPatternIndex;
use crate::ir::semantic_node::Position;
use crate::lsp::features::completion::context::{CompletionContext, CompletionContextType};
use crate::lsp::features::completion::dictionary::{CompletionSymbol, SymbolMetadata};
use std::sync::{Arc, RwLock};
use tower_lsp::lsp_types::{CompletionItemKind, Position as LspPosition};
use tracing::debug;

/// Context for extracting patterns at a specific cursor position
#[derive(Debug, Clone)]
pub struct QuotedPatternContext {
    /// The type of quoted pattern
    pub pattern_type: QuotedPatternType,

    /// Partial text at cursor (e.g., "my" in @"my|")
    pub partial_text: String,

    /// IR position of cursor
    pub ir_position: Position,

    /// Additional context (keys for maps, element count for lists/tuples/sets)
    pub metadata: PatternMetadata,
}

/// Type of quoted pattern
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuotedPatternType {
    /// Quoted string: @"text"
    String,

    /// Quoted map: @{key: value}
    Map,

    /// Quoted list: @[element]
    List,

    /// Quoted tuple: @(element)
    Tuple,

    /// Quoted set: @Set(element)
    Set,
}

/// Additional metadata about the pattern
#[derive(Debug, Clone)]
pub enum PatternMetadata {
    /// For maps: keys already present
    MapKeys(Vec<String>),

    /// For lists/tuples/sets: number of elements so far
    ElementCount(usize),

    /// No additional metadata
    None,
}

/// Extract pattern context at cursor position
///
/// # Arguments
/// * `ir` - The IR tree
/// * `position` - Cursor position (LSP coordinates)
/// * `context` - Completion context detected by context.rs
///
/// # Returns
/// QuotedPatternContext if cursor is in a quoted pattern, None otherwise
pub fn extract_pattern_at_position(
    ir: &Arc<RholangNode>,
    position: &LspPosition,
    context: &CompletionContext,
) -> Option<QuotedPatternContext> {
    use crate::lsp::features::node_finder::find_node_at_position;

    // Convert LSP position to IR position
    let ir_position = Position {
        row: position.line as usize,
        column: position.character as usize,
        byte: 0,
    };

    // Find the node at this position
    let node = find_node_at_position(ir.as_ref(), &ir_position)?;

    // Extract pattern based on context type
    match &context.context_type {
        CompletionContextType::StringLiteral => {
            // Extract partial string from StringLiteral node
            if let Some(partial) = extract_partial_string(node, &ir_position) {
                Some(QuotedPatternContext {
                    pattern_type: QuotedPatternType::String,
                    partial_text: partial,
                    ir_position,
                    metadata: PatternMetadata::None,
                })
            } else {
                None
            }
        }
        CompletionContextType::QuotedMapPattern { keys_so_far } => {
            Some(QuotedPatternContext {
                pattern_type: QuotedPatternType::Map,
                partial_text: String::new(), // TODO: Extract partial key/value
                ir_position,
                metadata: PatternMetadata::MapKeys(keys_so_far.clone()),
            })
        }
        CompletionContextType::QuotedListPattern { elements_so_far } => {
            Some(QuotedPatternContext {
                pattern_type: QuotedPatternType::List,
                partial_text: String::new(), // TODO: Extract partial element
                ir_position,
                metadata: PatternMetadata::ElementCount(*elements_so_far),
            })
        }
        CompletionContextType::QuotedTuplePattern { elements_so_far } => {
            Some(QuotedPatternContext {
                pattern_type: QuotedPatternType::Tuple,
                partial_text: String::new(), // TODO: Extract partial element
                ir_position,
                metadata: PatternMetadata::ElementCount(*elements_so_far),
            })
        }
        CompletionContextType::QuotedSetPattern { elements_so_far } => {
            Some(QuotedPatternContext {
                pattern_type: QuotedPatternType::Set,
                partial_text: String::new(), // TODO: Extract partial element
                ir_position,
                metadata: PatternMetadata::ElementCount(*elements_so_far),
            })
        }
        _ => None,
    }
}

/// Extract partial string from StringLiteral node at cursor position
///
/// This function extracts the text before the cursor in a string literal.
/// For example, in `@"my|Contract"` (cursor at |), it extracts "my".
fn extract_partial_string(node: &dyn crate::ir::semantic_node::SemanticNode, cursor_pos: &Position) -> Option<String> {
    use crate::ir::semantic_node::SemanticNodeExt;

    let rholang_node = node.as_rholang()?;

    if let RholangNode::StringLiteral { value, .. } = rholang_node {
        // Get the start position of the string literal content (after opening quote)
        let node_start = node.base().start();

        // Calculate character offset from start of string to cursor
        // Note: This is simplified - proper implementation needs to handle:
        // - Multi-line strings
        // - Escape sequences
        // - UTF-8 character boundaries
        let offset = if cursor_pos.row == node_start.row {
            cursor_pos.column.saturating_sub(node_start.column + 1) // +1 for opening quote
        } else {
            // Multi-line string - not yet supported
            return None;
        };

        // Extract substring from start to cursor
        if offset <= value.len() {
            Some(value.chars().take(offset).collect())
        } else {
            Some(value.clone())
        }
    } else {
        None
    }
}

/// Build partial MORK pattern from quoted pattern context
///
/// This function converts a partial pattern (e.g., user typed `@"my"`)
/// into MORK bytes that can be used to query the pattern index.
///
/// # Arguments
/// * `pattern_ctx` - The pattern context
///
/// # Returns
/// MORK bytes for querying, or None if conversion failed
pub fn build_partial_mork_pattern(pattern_ctx: &QuotedPatternContext) -> Option<Vec<u8>> {
    use crate::ir::mork_canonical::MorkForm;
    use mork::space::Space;

    let space = Space::new();

    // Build MORK form based on pattern type
    let mork_form = match pattern_ctx.pattern_type {
        QuotedPatternType::String => {
            // For partial strings, we create a String literal
            // The pattern index will need to do prefix matching
            MorkForm::Literal(crate::ir::mork_canonical::LiteralValue::String(
                pattern_ctx.partial_text.clone(),
            ))
        }
        QuotedPatternType::Map => {
            // For maps, create a Map pattern with the known keys
            if let PatternMetadata::MapKeys(keys) = &pattern_ctx.metadata {
                // Create map entries with VarPattern values (we don't know the values yet)
                let entries: Vec<(String, MorkForm)> = keys
                    .iter()
                    .map(|k| (k.clone(), MorkForm::VarPattern(format!("_{}", k))))
                    .collect();
                MorkForm::Map(entries)
            } else {
                return None;
            }
        }
        QuotedPatternType::List => {
            // For lists, create a List pattern with the known element count
            if let PatternMetadata::ElementCount(count) = &pattern_ctx.metadata {
                let elements: Vec<MorkForm> = (0..*count)
                    .map(|i| MorkForm::VarPattern(format!("_elem{}", i)))
                    .collect();
                MorkForm::List(elements)
            } else {
                return None;
            }
        }
        QuotedPatternType::Tuple => {
            // For tuples, create a Tuple pattern with the known element count
            if let PatternMetadata::ElementCount(count) = &pattern_ctx.metadata {
                let elements: Vec<MorkForm> = (0..*count)
                    .map(|i| MorkForm::VarPattern(format!("_elem{}", i)))
                    .collect();
                MorkForm::Tuple(elements)
            } else {
                return None;
            }
        }
        QuotedPatternType::Set => {
            // For sets, create a Set pattern with the known element count
            if let PatternMetadata::ElementCount(count) = &pattern_ctx.metadata {
                let elements: Vec<MorkForm> = (0..*count)
                    .map(|i| MorkForm::VarPattern(format!("_elem{}", i)))
                    .collect();
                MorkForm::Set(elements)
            } else {
                return None;
            }
        }
    };

    // Convert to MORK bytes
    match mork_form.to_mork_bytes(&space) {
        Ok(bytes) => {
            debug!(
                "Built MORK pattern for {:?}: {} bytes",
                pattern_ctx.pattern_type,
                bytes.len()
            );
            Some(bytes)
        }
        Err(e) => {
            debug!("Failed to convert pattern to MORK bytes: {}", e);
            None
        }
    }
}

/// Query pattern index for contracts matching partial pattern
///
/// # Arguments
/// * `global_index` - The global symbol index containing pattern index
/// * `pattern_ctx` - The pattern context
///
/// # Returns
/// Vector of completion symbols matching the pattern
///
/// # TODO (Phase 3)
/// This function needs to be fully implemented in Phase 3.
/// For now, it's a placeholder that returns empty results.
/// The full implementation will:
/// 1. Build MORK pattern from context
/// 2. Query RholangPatternIndex with the pattern
/// 3. Convert PatternMetadata results to CompletionSymbols
/// 4. Apply prefix matching for string patterns
pub fn query_contracts_by_pattern(
    _global_index: &Arc<RwLock<GlobalSymbolIndex>>,
    _pattern_ctx: &QuotedPatternContext,
) -> Vec<CompletionSymbol> {
    // TODO: Implement in Phase 3
    debug!("query_contracts_by_pattern called - implementation pending (Phase 3)");
    vec![]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quoted_pattern_context_creation() {
        let ctx = QuotedPatternContext {
            pattern_type: QuotedPatternType::String,
            partial_text: "my".to_string(),
            ir_position: Position {
                row: 0,
                column: 3,
                byte: 0,
            },
            metadata: PatternMetadata::None,
        };

        assert_eq!(ctx.pattern_type, QuotedPatternType::String);
        assert_eq!(ctx.partial_text, "my");
    }

    #[test]
    fn test_build_partial_mork_pattern_string() {
        let ctx = QuotedPatternContext {
            pattern_type: QuotedPatternType::String,
            partial_text: "test".to_string(),
            ir_position: Position {
                row: 0,
                column: 0,
                byte: 0,
            },
            metadata: PatternMetadata::None,
        };

        let mork_bytes = build_partial_mork_pattern(&ctx);
        assert!(mork_bytes.is_some());
    }

    #[test]
    fn test_build_partial_mork_pattern_map() {
        let ctx = QuotedPatternContext {
            pattern_type: QuotedPatternType::Map,
            partial_text: String::new(),
            ir_position: Position {
                row: 0,
                column: 0,
                byte: 0,
            },
            metadata: PatternMetadata::MapKeys(vec!["key1".to_string(), "key2".to_string()]),
        };

        let mork_bytes = build_partial_mork_pattern(&ctx);
        assert!(mork_bytes.is_some());
    }
}
