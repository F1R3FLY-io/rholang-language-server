//! Pattern-aware contract resolver using MORK+PathMap matching
//!
//! This resolver enhances contract goto-definition by matching call-site arguments
//! against contract parameter patterns. It enables overload resolution and parameter-
//! aware navigation.

use std::sync::Arc;
use tracing::{debug, warn};
use rpds::Vector;

use crate::ir::global_index::GlobalSymbolIndex;
use crate::ir::rholang_node::{RholangNode, RholangSendType};
use crate::ir::semantic_node::Position;
use crate::ir::symbol_resolution::{ResolutionContext, SymbolLocation, SymbolResolver};

/// Pattern-aware resolver for contract invocations
///
/// This resolver:
/// 1. Detects if the symbol is a contract invocation (Send node)
/// 2. Extracts contract name and arguments from the call site
/// 3. Queries the pattern index using MORK serialization
/// 4. Falls back to empty (letting other resolvers handle it) if pattern matching fails
pub struct PatternAwareContractResolver {
    global_index: Arc<std::sync::RwLock<GlobalSymbolIndex>>,
}

impl PatternAwareContractResolver {
    /// Create a new pattern-aware contract resolver
    pub fn new(global_index: Arc<std::sync::RwLock<GlobalSymbolIndex>>) -> Self {
        Self { global_index }
    }

    /// Convert global_index::SymbolLocation to symbol_resolution::SymbolLocation
    ///
    /// Maps between the two SymbolLocation types which have compatible fields
    /// but different metadata structures.
    fn convert_location(
        global_loc: crate::ir::global_index::SymbolLocation,
    ) -> SymbolLocation {
        use crate::ir::symbol_resolution::{SymbolKind as ResSymbolKind, ResolutionConfidence};

        // Map SymbolKind from global_index to symbol_resolution
        let kind = match global_loc.kind {
            crate::ir::global_index::SymbolKind::Contract => ResSymbolKind::Function,
            crate::ir::global_index::SymbolKind::Channel => ResSymbolKind::Variable,
            crate::ir::global_index::SymbolKind::Variable => ResSymbolKind::Variable,
            crate::ir::global_index::SymbolKind::LetBinding => ResSymbolKind::Variable,
            crate::ir::global_index::SymbolKind::Bundle => ResSymbolKind::Other,
        };

        SymbolLocation {
            uri: global_loc.uri,
            range: global_loc.range,
            kind,
            confidence: ResolutionConfidence::Exact, // Pattern match is exact
            metadata: None,
        }
    }

    /// Extract contract name from a channel expression
    ///
    /// Handles:
    /// - `contract_name!(...)` → Var node
    /// - `@"contractName"!(...)` → Quote(StringLiteral)
    fn extract_contract_name(channel: &Arc<RholangNode>) -> Option<String> {
        match channel.as_ref() {
            RholangNode::Var { name, .. } => Some(name.clone()),
            RholangNode::Quote { quotable, .. } => {
                // Handle @"contractName" pattern
                if let RholangNode::StringLiteral { value, .. } = quotable.as_ref() {
                    Some(value.clone())
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Extract arguments from a Send node
    fn extract_arguments(send_node: &RholangNode) -> Option<Vec<Arc<RholangNode>>> {
        match send_node {
            RholangNode::Send { inputs, .. } => Some(inputs.iter().cloned().collect()),
            _ => None,
        }
    }
}

impl SymbolResolver for PatternAwareContractResolver {
    fn resolve_symbol(
        &self,
        symbol_name: &str,
        _position: &Position,
        context: &ResolutionContext,
    ) -> Vec<SymbolLocation> {
        // Check if we have an IR node in context (for pattern matching)
        if let Some(node_any) = &context.ir_node {
            // Try to downcast to RholangNode
            if let Some(rholang_node) = node_any.downcast_ref::<RholangNode>() {
                // Check if this is a Send node (contract invocation)
                if let RholangNode::Send { channel, .. } = rholang_node {
                    // Extract contract name
                    if let Some(contract_name) = Self::extract_contract_name(channel) {
                        // Only proceed if the contract name matches the symbol we're looking for
                        if contract_name == symbol_name {
                            // Extract arguments
                            if let Some(arguments) = Self::extract_arguments(rholang_node) {
                                // Query pattern index
                                let arg_refs: Vec<&RholangNode> =
                                    arguments.iter().map(|a| a.as_ref()).collect();

                                debug!(
                                    "PatternAwareContractResolver: Querying pattern index for contract '{}' with {} arguments",
                                    contract_name,
                                    arg_refs.len()
                                );

                                // Lock global_index for reading
                                if let Ok(index) = self.global_index.read() {
                                    match index.query_contract_by_pattern(&contract_name, &arg_refs) {
                                        Ok(locations) if !locations.is_empty() => {
                                            debug!(
                                                "PatternAwareContractResolver: Found {} matches via pattern index for contract '{}'",
                                                locations.len(),
                                                contract_name
                                            );
                                            return locations
                                                .into_iter()
                                                .map(Self::convert_location)
                                                .collect();
                                        }
                                        Ok(_) => {
                                            debug!(
                                                "PatternAwareContractResolver: No pattern matches for contract '{}', will fall back",
                                                contract_name
                                            );
                                        }
                                        Err(e) => {
                                            warn!(
                                                "PatternAwareContractResolver: Pattern query failed: {}",
                                                e
                                            );
                                        }
                                    }
                                } else {
                                    warn!("PatternAwareContractResolver: Failed to acquire read lock on global_index");
                                }
                            }
                        }
                    }
                }
            }
        }

        // If we reach here, either:
        // - Not a Send node
        // - Pattern matching failed
        // - No matches found
        // Return empty to let other resolvers handle it
        vec![]
    }

    fn supports_language(&self, language: &str) -> bool {
        language == "rholang"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::semantic_node::{NodeBase, Position};

    fn test_base() -> NodeBase {
        NodeBase::new_simple(Position { row: 0, column: 0, byte: 0 }, 0, 0, 0)
    }

    #[test]
    fn test_extract_contract_name_from_var() {
        let chan = Arc::new(RholangNode::Var {
            base: test_base(),
            name: "echo".to_string(),
            metadata: None,
        });

        let name = PatternAwareContractResolver::extract_contract_name(&chan);
        assert_eq!(name, Some("echo".to_string()));
    }

    #[test]
    fn test_extract_contract_name_from_quoted_string() {
        let quotable = Arc::new(RholangNode::StringLiteral {
            base: test_base(),
            value: "myContract".to_string(),
            metadata: None,
        });

        let chan = Arc::new(RholangNode::Quote {
            base: test_base(),
            quotable,
            metadata: None,
        });

        let name = PatternAwareContractResolver::extract_contract_name(&chan);
        assert_eq!(name, Some("myContract".to_string()));
    }

    #[test]
    fn test_extract_contract_name_returns_none_for_non_var_non_quote() {
        let chan = Arc::new(RholangNode::Nil {
            base: test_base(),
            metadata: None,
        });

        let name = PatternAwareContractResolver::extract_contract_name(&chan);
        assert_eq!(name, None);
    }

    #[test]
    fn test_extract_arguments_from_send() {
        let arg1 = Arc::new(RholangNode::LongLiteral {
            base: test_base(),
            value: 42,
            metadata: None,
        });

        let arg2 = Arc::new(RholangNode::StringLiteral {
            base: test_base(),
            value: "hello".to_string(),
            metadata: None,
        });

        let channel = Arc::new(RholangNode::Var {
            base: test_base(),
            name: "test".to_string(),
            metadata: None,
        });

        let inputs = Vector::new_with_ptr_kind()
            .push_back(arg1)
            .push_back(arg2);

        let send_node = RholangNode::Send {
            base: test_base(),
            channel,
            send_type: RholangSendType::Single,
            send_type_pos: Position { row: 0, column: 0, byte: 0 },
            inputs,
            metadata: None,
        };

        let args = PatternAwareContractResolver::extract_arguments(&send_node);
        assert!(args.is_some());
        assert_eq!(args.unwrap().len(), 2);
    }

    #[test]
    fn test_extract_arguments_returns_none_for_non_send() {
        let node = RholangNode::Nil {
            base: test_base(),
            metadata: None,
        };

        let args = PatternAwareContractResolver::extract_arguments(&node);
        assert_eq!(args, None);
    }

    #[test]
    fn test_supports_language() {
        let global_index = Arc::new(std::sync::RwLock::new(GlobalSymbolIndex::new()));
        let resolver = PatternAwareContractResolver::new(global_index);

        assert!(resolver.supports_language("rholang"));
        assert!(!resolver.supports_language("metta"));
        assert!(!resolver.supports_language("python"));
    }
}
