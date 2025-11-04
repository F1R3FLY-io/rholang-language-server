//! Symbol index builder transform
//!
//! This transform traverses the semantic IR and populates the GlobalSymbolIndex
//! with contract definitions, invocations, and other symbols for cross-file navigation.
//!
//! NOTE: This is an initial implementation focused on contracts. Full support for
//! all symbol types will be added incrementally.

use std::sync::Arc;
use std::collections::HashMap;
use tower_lsp::lsp_types::{Position, Range, Url};
use crate::ir::rholang_node::{RholangNode, Position as IrPosition};
use crate::ir::global_index::{GlobalSymbolIndex, SymbolLocation, SymbolKind};

/// Transform that builds a global symbol index from semantic IR
pub struct SymbolIndexBuilder {
    /// The global index being populated
    pub index: Arc<std::sync::RwLock<GlobalSymbolIndex>>,

    /// URI of the current document being indexed
    current_uri: Url,

    /// Absolute positions for all nodes in the current document
    positions: Arc<HashMap<usize, (IrPosition, IrPosition)>>,
}

impl SymbolIndexBuilder {
    /// Create a new symbol index builder for a document
    pub fn new(
        index: Arc<std::sync::RwLock<GlobalSymbolIndex>>,
        uri: Url,
        positions: Arc<HashMap<usize, (IrPosition, IrPosition)>>,
    ) -> Self {
        Self {
            index,
            current_uri: uri,
            positions,
        }
    }

    /// Index a Rholang node tree
    pub fn index_tree(&mut self, root: &Arc<RholangNode>) {
        // Traverse the tree and index symbols
        self.visit_node(root);
    }

    /// Visit a Rholang node and index any symbols it contains
    fn visit_node(&mut self, node: &Arc<RholangNode>) {
        match node.as_ref() {
            RholangNode::Contract { name, formals, proc, .. } => {
                self.index_contract(name, formals, proc);

                // Continue traversal
                self.visit_node(name);
                for formal in formals.iter() {
                    self.visit_node(formal);
                }
                self.visit_node(proc);
            }

            RholangNode::Send { channel, inputs, .. } |
            RholangNode::SendSync { channel, inputs, ..} => {
                // Check if this is a contract invocation
                if let Some(contract_name) = Self::extract_contract_name(channel) {
                    self.index_contract_invocation(&contract_name, node);
                } else {
                    // Not a contract - might be a channel usage
                    self.index_channel_usage(channel);
                }

                // Continue traversal
                self.visit_node(channel);
                for input in inputs.iter() {
                    self.visit_node(input);
                }
            }

            RholangNode::New { decls, proc, .. } => {
                // Index channel declarations
                for decl in decls.iter() {
                    self.index_channel_declaration(decl);
                }

                // Continue traversal
                for decl in decls.iter() {
                    self.visit_node(decl);
                }
                self.visit_node(proc);
            }

            RholangNode::Let { decls, proc, .. } => {
                // Index variable declarations
                for decl in decls.iter() {
                    self.index_variable_declaration(decl);
                }

                // Continue traversal
                for decl in decls.iter() {
                    self.visit_node(decl);
                }
                self.visit_node(proc);
            }

            RholangNode::Par { left: Some(left), right: Some(right), .. } => {
                self.visit_node(left);
                self.visit_node(right);
            }

            RholangNode::Input { receipts, proc, .. } => {
                // Input receipts contain channel bindings
                // For now, just traverse the nested structure
                for receipt in receipts.iter() {
                    // Receipt is a Vector of nodes
                    for node in receipt.iter() {
                        self.visit_node(node);
                    }
                }
                self.visit_node(proc);
            }

            RholangNode::Match { expression, cases, .. } => {
                // Match cases contain pattern bindings
                self.visit_node(expression);
                for case in cases.iter() {
                    // Case is a (pattern, body) pair
                    self.visit_node(&case.0);
                    self.visit_node(&case.1);
                }
            }

            RholangNode::Block { proc, .. } => {
                self.visit_node(proc);
            }

            RholangNode::IfElse { condition, consequence, alternative, .. } => {
                self.visit_node(condition);
                self.visit_node(consequence);
                if let Some(alt) = alternative {
                    self.visit_node(alt);
                }
            }

            RholangNode::Bundle { proc, .. } => {
                self.visit_node(proc);
            }

            // Skip leaf nodes and expressions that don't contain symbols
            _ => {
                // For now, skip other node types
                // Full traversal will be added incrementally
            }
        }
    }

    /// Index a contract definition
    fn index_contract(
        &mut self,
        name: &Arc<RholangNode>,
        formals: &rpds::Vector<Arc<RholangNode>, archery::ArcK>,
        _proc: &Arc<RholangNode>,
    ) {
        // Extract contract name (handle both Var and StringLiteral)
        let contract_name = match name.as_ref() {
            RholangNode::Var { name, .. } => name.clone(),
            RholangNode::StringLiteral { value, .. } => value.clone(),
            _ => {
                // Can't extract name, skip
                return;
            }
        };

        // Look up actual position from positions HashMap
        let key = &**name as *const RholangNode as usize;
        let (start_pos, _end_pos) = match self.positions.get(&key) {
            Some(pos) => pos,
            None => {
                eprintln!("Warning: No position found for contract name '{}'", contract_name);
                return;
            }
        };

        // Create range with actual positions from IR
        let location = SymbolLocation {
            uri: self.current_uri.clone(),
            range: Range {
                start: Position {
                    line: start_pos.row as u32,
                    character: start_pos.column as u32,
                },
                end: Position {
                    line: start_pos.row as u32,
                    character: (start_pos.column + contract_name.len()) as u32,
                },
            },
            kind: SymbolKind::Contract,
            documentation: None,
            signature: Some(Self::build_contract_signature(&contract_name, formals)),
        };

        // Add to index
        if let Ok(mut index) = self.index.write() {
            if let Err(e) = index.add_contract_definition(&contract_name, location) {
                eprintln!("Warning: Failed to index contract '{}': {}", contract_name, e);
            }

            // Extract and index map keys from formal parameters
            self.extract_and_index_map_keys(&contract_name, formals, &mut index);
        }
    }

    /// Index a contract invocation (send to a contract channel)
    fn index_contract_invocation(
        &mut self,
        contract_name: &str,
        node: &Arc<RholangNode>,
    ) {
        // Try to get the actual position of the invocation node
        let key = &**node as *const RholangNode as usize;
        let range = if let Some((start_pos, _end_pos)) = self.positions.get(&key) {
            // Use actual position from IR
            Range {
                start: Position {
                    line: start_pos.row as u32,
                    character: start_pos.column as u32,
                },
                end: Position {
                    line: start_pos.row as u32,
                    character: (start_pos.column + contract_name.len()) as u32,
                },
            }
        } else {
            // Fallback to placeholder if position not found
            Range {
                start: Position { line: 0, character: 0 },
                end: Position { line: 0, character: contract_name.len() as u32 },
            }
        };

        let location = SymbolLocation {
            uri: self.current_uri.clone(),
            range,
            kind: SymbolKind::Contract,
            documentation: None,
            signature: None,
        };

        // Add to index
        if let Ok(mut index) = self.index.write() {
            if let Err(e) = index.add_contract_invocation(contract_name, location) {
                eprintln!("Warning: Failed to index contract invocation '{}': {}", contract_name, e);
            }
        }
    }

    /// Index a channel declaration from a `new` binding
    fn index_channel_declaration(&mut self, decl: &Arc<RholangNode>) {
        // Extract channel name
        let channel_name = match decl.as_ref() {
            RholangNode::Var { name, .. } => name.clone(),
            RholangNode::NameDecl { var, .. } => {
                // NameDecl wraps a Var node
                if let RholangNode::Var { name, .. } = var.as_ref() {
                    name.clone()
                } else {
                    return;
                }
            }
            _ => return, // Skip non-variable declarations
        };

        // Look up actual position from positions HashMap
        let key = &**decl as *const RholangNode as usize;
        let (start_pos, _end_pos) = match self.positions.get(&key) {
            Some(pos) => pos,
            None => {
                eprintln!("Warning: No position found for channel '{}'", channel_name);
                return;
            }
        };

        // Create location with actual positions from IR
        let location = SymbolLocation {
            uri: self.current_uri.clone(),
            range: Range {
                start: Position {
                    line: start_pos.row as u32,
                    character: start_pos.column as u32,
                },
                end: Position {
                    line: start_pos.row as u32,
                    character: (start_pos.column + channel_name.len()) as u32,
                },
            },
            kind: SymbolKind::Channel,
            documentation: Some(format!("Channel declared via `new {}`", channel_name)),
            signature: None,
        };

        // Add to global index using proper API
        if let Ok(mut index) = self.index.write() {
            if let Err(e) = index.add_channel_definition(&channel_name, location) {
                eprintln!("Warning: Failed to index channel '{}': {}", channel_name, e);
            }
        }
    }

    /// Index a variable declaration from a `let` binding
    fn index_variable_declaration(&mut self, decl: &Arc<RholangNode>) {
        // Let declarations are typically of the form `x = expr`
        // For now, we'll extract just the variable name
        let var_name = match decl.as_ref() {
            RholangNode::Var { name, .. } => name.clone(),
            _ => return, // Skip non-variable declarations
        };

        // Look up actual position from positions HashMap
        let key = &**decl as *const RholangNode as usize;
        let (start_pos, _end_pos) = match self.positions.get(&key) {
            Some(pos) => pos,
            None => {
                eprintln!("Warning: No position found for variable '{}'", var_name);
                return;
            }
        };

        // Create location with actual positions from IR
        let location = SymbolLocation {
            uri: self.current_uri.clone(),
            range: Range {
                start: Position {
                    line: start_pos.row as u32,
                    character: start_pos.column as u32,
                },
                end: Position {
                    line: start_pos.row as u32,
                    character: (start_pos.column + var_name.len()) as u32,
                },
            },
            kind: SymbolKind::LetBinding,
            documentation: Some(format!("Variable declared via `let {}`", var_name)),
            signature: None,
        };

        // Add to global index using proper API
        if let Ok(mut index) = self.index.write() {
            if let Err(e) = index.add_variable_definition(&var_name, location) {
                eprintln!("Warning: Failed to index variable '{}': {}", var_name, e);
            }
        }
    }

    /// Index a channel usage (send or receive)
    fn index_channel_usage(&mut self, channel_node: &Arc<RholangNode>) {
        // Extract channel name from Var node
        let channel_name = match channel_node.as_ref() {
            RholangNode::Var { name, .. } => name.clone(),
            _ => return, // Only handle simple Var references for now
        };

        // Look up actual position
        let key = &**channel_node as *const RholangNode as usize;
        let (start_pos, _end_pos) = match self.positions.get(&key) {
            Some(pos) => pos,
            None => return, // Skip if no position found
        };

        // Create location for the usage
        let location = SymbolLocation {
            uri: self.current_uri.clone(),
            range: Range {
                start: Position {
                    line: start_pos.row as u32,
                    character: start_pos.column as u32,
                },
                end: Position {
                    line: start_pos.row as u32,
                    character: (start_pos.column + channel_name.len()) as u32,
                },
            },
            kind: SymbolKind::Channel,
            documentation: None,
            signature: None,
        };

        // Add to global index
        if let Ok(mut index) = self.index.write() {
            if let Err(e) = index.add_channel_reference(&channel_name, location) {
                eprintln!("Warning: Failed to index channel usage '{}': {}", channel_name, e);
            }
        }
    }

    /// Index a variable usage
    fn index_variable_usage(&mut self, var_node: &Arc<RholangNode>) {
        // Extract variable name
        let var_name = match var_node.as_ref() {
            RholangNode::Var { name, .. } => name.clone(),
            _ => return,
        };

        // Look up actual position
        let key = &**var_node as *const RholangNode as usize;
        let (start_pos, _end_pos) = match self.positions.get(&key) {
            Some(pos) => pos,
            None => return, // Skip if no position found
        };

        // Create location for the usage
        let location = SymbolLocation {
            uri: self.current_uri.clone(),
            range: Range {
                start: Position {
                    line: start_pos.row as u32,
                    character: start_pos.column as u32,
                },
                end: Position {
                    line: start_pos.row as u32,
                    character: (start_pos.column + var_name.len()) as u32,
                },
            },
            kind: SymbolKind::Variable,
            documentation: None,
            signature: None,
        };

        // Add to global index
        if let Ok(mut index) = self.index.write() {
            if let Err(e) = index.add_variable_reference(&var_name, location) {
                eprintln!("Warning: Failed to index variable usage '{}': {}", var_name, e);
            }
        }
    }

    /// Extract and index map keys from contract formal parameters
    ///
    /// Recursively traverses formal parameters looking for map patterns and indexes
    /// each map key with its location.
    ///
    /// # Arguments
    /// * `contract_name` - Name of the contract
    /// * `formals` - The formal parameters to traverse
    /// * `index` - Mutable reference to the GlobalSymbolIndex
    fn extract_and_index_map_keys(
        &self,
        contract_name: &str,
        formals: &rpds::Vector<Arc<RholangNode>, archery::ArcK>,
        index: &mut GlobalSymbolIndex,
    ) {
        for formal in formals.iter() {
            // Each formal is typically a Quote node containing the pattern
            if let RholangNode::Quote { quotable, .. } = formal.as_ref() {
                // Extract map keys from the quotable content
                self.extract_map_keys_from_node(contract_name, quotable, "", index);
            } else {
                // If not a Quote, traverse it anyway (might be a direct pattern)
                self.extract_map_keys_from_node(contract_name, formal, "", index);
            }
        }
    }

    /// Recursively extract map keys from a pattern node
    ///
    /// # Arguments
    /// * `contract_name` - Name of the contract
    /// * `node` - The pattern node to traverse
    /// * `key_prefix` - Dot-separated prefix for nested keys (e.g., "user" for top level, "user.address" for nested)
    /// * `index` - Mutable reference to the GlobalSymbolIndex
    fn extract_map_keys_from_node(
        &self,
        contract_name: &str,
        node: &Arc<RholangNode>,
        key_prefix: &str,
        index: &mut GlobalSymbolIndex,
    ) {
        match node.as_ref() {
            // Map patterns: @{key1: val1, key2: val2, ...}
            RholangNode::Map { pairs, .. } => {
                for (key_node, value_node) in pairs.iter() {
                    // Extract the key string
                    if let Some(key_str) = Self::extract_string_from_node(key_node) {
                        // Build the full key path
                        let full_key_path = if key_prefix.is_empty() {
                            key_str.clone()
                        } else {
                            format!("{}.{}", key_prefix, key_str)
                        };

                        // Get position for this key node
                        let key_ptr = &**key_node as *const RholangNode as usize;
                        if let Some((start_pos, _end_pos)) = self.positions.get(&key_ptr) {
                            let location = SymbolLocation {
                                uri: self.current_uri.clone(),
                                range: Range {
                                    start: Position {
                                        line: start_pos.row as u32,
                                        character: start_pos.column as u32,
                                    },
                                    end: Position {
                                        line: start_pos.row as u32,
                                        character: (start_pos.column + key_str.len()) as u32,
                                    },
                                },
                                kind: SymbolKind::Variable, // Map keys are like variables
                                documentation: None,
                                signature: None,
                            };

                            // Add to index
                            if let Err(e) = index.add_map_key_pattern(contract_name, &full_key_path, location) {
                                eprintln!("Warning: Failed to index map key '{}': {}", full_key_path, e);
                            }
                        }

                        // Recursively extract from nested patterns in the value
                        self.extract_map_keys_from_node(contract_name, value_node, &full_key_path, index);
                    }
                }
            }

            // Other pattern types that might contain nested maps
            RholangNode::List { elements, .. } => {
                for element in elements.iter() {
                    self.extract_map_keys_from_node(contract_name, element, key_prefix, index);
                }
            }

            RholangNode::Tuple { elements, .. } => {
                for element in elements.iter() {
                    self.extract_map_keys_from_node(contract_name, element, key_prefix, index);
                }
            }

            // Skip other node types (variables, wildcards, etc.)
            _ => {}
        }
    }

    /// Extract a string value from a node (handles StringLiteral and Var)
    fn extract_string_from_node(node: &Arc<RholangNode>) -> Option<String> {
        match node.as_ref() {
            RholangNode::StringLiteral { value, .. } => Some(value.clone()),
            RholangNode::Var { name, .. } => Some(name.clone()),
            _ => None,
        }
    }

    /// Extract contract name from a channel expression
    /// Returns Some(name) if this is a contract channel reference
    fn extract_contract_name(channel: &Arc<RholangNode>) -> Option<String> {
        match channel.as_ref() {
            RholangNode::Var { name, .. } => {
                // Simple variable reference - could be a contract name
                Some(name.clone())
            }

            RholangNode::StringLiteral { value, .. } => {
                // Quoted contract names: @"MyContract"!()
                Some(value.clone())
            }

            // TODO: Handle other cases like:
            // - Unforgeable names: @contract!()

            _ => None
        }
    }

    /// Build a contract signature string for display
    fn build_contract_signature(
        name: &str,
        formals: &rpds::Vector<Arc<RholangNode>, archery::ArcK>,
    ) -> String {
        let formal_strs: Vec<String> = formals.iter()
            .map(|formal| Self::format_formal(formal))
            .collect();

        format!("contract {}({})", name, formal_strs.join(", "))
    }

    /// Format a formal parameter for signature display
    fn format_formal(formal: &Arc<RholangNode>) -> String {
        match formal.as_ref() {
            RholangNode::Var { name, .. } => format!("@{}", name),
            _ => "@_".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::rholang_node::NodeBase;

    fn create_test_contract_name(name: &str) -> Arc<RholangNode> {
        Arc::new(RholangNode::Var {
            name: name.to_string(),
            base: NodeBase::new_simple(
                crate::ir::rholang_node::Position {
                    row: 0,
                    column: 0,
                    byte: 0,
                },
                0, 0, name.len()
            ),
            metadata: None,
        })
    }

    #[test]
    fn test_extract_contract_name() {
        let var_node = create_test_contract_name("MyContract");

        let name = SymbolIndexBuilder::extract_contract_name(&var_node);
        assert_eq!(name, Some("MyContract".to_string()));
    }

    #[test]
    fn test_build_contract_signature_empty() {
        let signature = SymbolIndexBuilder::build_contract_signature(
            "MyContract",
            &rpds::Vector::new_with_ptr_kind()
        );
        assert_eq!(signature, "contract MyContract()");
    }

    #[test]
    fn test_symbol_index_builder_creation() {
        let index = Arc::new(std::sync::RwLock::new(GlobalSymbolIndex::new()));
        let uri = Url::parse("file:///test.rho").unwrap();
        let positions = Arc::new(HashMap::new());

        let builder = SymbolIndexBuilder::new(index, uri, positions);
        assert_eq!(builder.current_uri.as_str(), "file:///test.rho");
    }
}
