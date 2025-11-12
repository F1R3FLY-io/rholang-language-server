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
/// # Implementation Notes
/// - String patterns: Use prefix matching on contract names via GlobalSymbolIndex.definitions
/// - Complex patterns (Map/List/Tuple/Set): Deferred to Phase 2 (requires full MORK unification)
///
/// # Performance
/// Current implementation uses O(n) HashMap iteration where n = total symbols.
/// Future optimization: Use PrefixZipper trait from liblevenshtein for O(k+m) complexity.
/// See docs/completion/prefix_zipper_integration.md for details.
pub fn query_contracts_by_pattern(
    global_index: &Arc<RwLock<GlobalSymbolIndex>>,
    pattern_ctx: &QuotedPatternContext,
) -> Vec<CompletionSymbol> {
    match pattern_ctx.pattern_type {
        QuotedPatternType::String => {
            // String literal pattern: @"prefix|"
            // Use prefix matching on contract names
            debug!(
                "Querying contracts by string prefix: '{}'",
                pattern_ctx.partial_text
            );
            query_contracts_by_name_prefix(global_index, &pattern_ctx.partial_text)
        }

        // Complex patterns deferred to Phase 2 (require full MORK unification)
        QuotedPatternType::Map => {
            debug!("Map pattern completion deferred to Phase 2 (requires MORK unification)");
            vec![]
        }
        QuotedPatternType::List => {
            debug!("List pattern completion deferred to Phase 2 (requires MORK unification)");
            vec![]
        }
        QuotedPatternType::Tuple => {
            debug!("Tuple pattern completion deferred to Phase 2 (requires MORK unification)");
            vec![]
        }
        QuotedPatternType::Set => {
            debug!("Set pattern completion deferred to Phase 2 (requires MORK unification)");
            vec![]
        }
    }
}

/// Query contracts by name prefix using GlobalSymbolIndex
///
/// This helper function iterates the global definitions HashMap
/// and filters for contracts whose names start with the given prefix.
///
/// # Arguments
/// * `global_index` - Global symbol index containing all workspace contracts
/// * `prefix` - String prefix to match against contract names
///
/// # Returns
/// Vector of matching contracts as CompletionSymbols, sorted by name length
///
/// # Performance
/// **Phase A-1 Optimization**: O(m) where m = number of contracts in workspace.
/// Uses lazy subtrie extraction instead of O(n) full workspace iteration.
///
/// **Previous**: O(n) where n = total symbols (~500-1000), ~20-50µs
/// **Current**: O(m) where m = contracts only (~50-100), ~5-10µs for typical workspaces
/// **Speedup**: 2-5x for typical workspaces, up to 100x for large workspaces (>100K symbols)
///
/// # Implementation Details (Phase A-1)
///
/// Uses `GlobalSymbolIndex.query_all_contracts()` which leverages PathMap's
/// `.restrict()` method to extract a contract-only subtrie in O(1) time (cached).
/// This avoids iterating through all workspace symbols to find contracts.
///
/// **Key Benefits**:
/// 1. Constant-time cache lookup (~41ns) instead of O(n) iteration
/// 2. Only traverses contracts (m), not all symbols (n)
/// 3. Scales well with workspace size (performance independent of non-contract symbols)
///
/// See `docs/optimization/ledger/phase-a-1-lazy-subtrie.md` for full analysis.
fn query_contracts_by_name_prefix(
    global_index: &Arc<RwLock<GlobalSymbolIndex>>,
    prefix: &str,
) -> Vec<CompletionSymbol> {
    let index = match global_index.read() {
        Ok(guard) => guard,
        Err(e) => {
            debug!("Failed to acquire read lock on global index: {}", e);
            return vec![];
        }
    };

    // Phase A-1: Use lazy subtrie extraction to get all contracts in O(m) time
    // instead of O(n) iteration through all workspace symbols
    let contracts = match index.query_all_contracts() {
        Ok(locations) => locations,
        Err(e) => {
            debug!("Failed to query contracts: {}", e);
            return vec![];
        }
    };

    let mut results = Vec::new();

    // Filter contracts by name prefix (O(m) where m = number of contracts)
    for location in contracts {
        // Extract contract name from signature
        // Signature format: "contract ContractName(...)"
        let name = if let Some(sig) = &location.signature {
            sig.split_whitespace()
                .nth(1) // Get the second word (contract name)
                .and_then(|s| s.split('(').next()) // Remove parameter list
                .unwrap_or("")
                .to_string()
        } else {
            continue; // Skip contracts without signatures
        };

        // Filter by name prefix
        if !name.starts_with(prefix) {
            continue;
        }

        // Convert to CompletionSymbol
        results.push(CompletionSymbol {
            metadata: SymbolMetadata {
                name,
                kind: CompletionItemKind::FUNCTION, // Contracts complete as functions
                documentation: location.documentation.clone(),
                signature: location.signature.clone(),
                reference_count: 0, // Could be enriched from references index in future
            },
            distance: 0,            // Exact prefix match
            scope_depth: usize::MAX, // Global scope
        });
    }

    // Sort by name length (shorter names more likely to be relevant)
    results.sort_by_key(|s| s.metadata.name.len());

    debug!(
        "Found {} contracts matching prefix '{}' (Phase A-1 optimized)",
        results.len(),
        prefix
    );

    results
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
