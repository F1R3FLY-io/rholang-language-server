//! Global symbol index for cross-file navigation and reference finding
//!
//! This module provides a workspace-wide index of symbols using MORK pattern matching
//! for efficient O(k) lookups. The index is incrementally updated on document changes.

use std::sync::Arc;
use std::collections::HashMap;
use tower_lsp::lsp_types::{Location, Range, Position, Url};
use crate::ir::pattern_matching::RholangPatternMatcher;
use crate::ir::rholang_node::{RholangNode, NodeBase, RelativePosition};

/// Unique identifier for a symbol in the workspace
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct SymbolId {
    /// URI of the document containing the symbol
    pub uri: Url,
    /// Qualified name of the symbol (e.g., "MyContract" for a contract)
    pub name: String,
    /// Position of the symbol definition (line, character)
    pub position: (u32, u32),
}

/// Kind of symbol in Rholang code
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    /// Contract definition: `contract Foo(...) = { ... }`
    Contract,
    /// Channel name from `new` binding: `new x in { ... }`
    Channel,
    /// Bundle restriction: `bundle+ { ... }`
    Bundle,
    /// Variable binding from various contexts
    Variable,
    /// Let binding: `let x = ... in { ... }`
    LetBinding,
}

/// Location information for a symbol
#[derive(Debug, Clone)]
pub struct SymbolLocation {
    pub uri: Url,
    pub range: Range,
    pub kind: SymbolKind,
    /// Optional documentation/hover text
    pub documentation: Option<String>,
    /// Signature for contracts (e.g., "contract Foo(@x, @y)")
    pub signature: Option<String>,
}

impl SymbolLocation {
    /// Convert to LSP Location
    pub fn to_lsp_location(&self) -> Location {
        Location {
            uri: self.uri.clone(),
            range: self.range,
        }
    }

    /// Create a RholangNode representing this location (for pattern matcher storage)
    pub fn to_rholang_node(&self) -> Arc<RholangNode> {
        // Store as a string literal containing serialized location data
        // Use | as delimiter since URIs can contain :
        let location_data = format!(
            "{}|{}|{}|{}|{}",
            self.uri.as_str(),
            self.range.start.line,
            self.range.start.character,
            self.range.end.line,
            self.range.end.character
        );

        let data_len = location_data.len();

        Arc::new(RholangNode::StringLiteral {
            value: location_data,
            base: NodeBase::new(
                RelativePosition {
                    delta_lines: 0,
                    delta_columns: 0,
                    delta_bytes: 0,
                },
                0, 0, data_len
            ),
            metadata: None,
        })
    }

    /// Parse a SymbolLocation from a RholangNode
    pub fn from_rholang_node(node: &RholangNode) -> Result<Self, String> {
        match node {
            RholangNode::StringLiteral { value, .. } => {
                let parts: Vec<&str> = value.split('|').collect();
                if parts.len() != 5 {
                    return Err(format!("Invalid location data format: expected 5 parts, got {}", parts.len()));
                }

                let uri = Url::parse(parts[0])
                    .map_err(|e| format!("Invalid URI: {}", e))?;

                let start_line = parts[1].parse::<u32>()
                    .map_err(|e| format!("Invalid start line: {}", e))?;
                let start_char = parts[2].parse::<u32>()
                    .map_err(|e| format!("Invalid start character: {}", e))?;
                let end_line = parts[3].parse::<u32>()
                    .map_err(|e| format!("Invalid end line: {}", e))?;
                let end_char = parts[4].parse::<u32>()
                    .map_err(|e| format!("Invalid end character: {}", e))?;

                Ok(SymbolLocation {
                    uri,
                    range: Range {
                        start: Position { line: start_line, character: start_char },
                        end: Position { line: end_line, character: end_char },
                    },
                    kind: SymbolKind::Contract, // Default, should be stored separately
                    documentation: None,
                    signature: None,
                })
            }
            _ => Err("Expected StringLiteral node".to_string())
        }
    }
}

/// Global workspace symbol index using pattern matching
#[derive(Debug)]
pub struct GlobalSymbolIndex {
    /// Index of contract definitions
    /// Pattern: (contract "<name>" ...) -> SymbolLocation
    pub contract_definitions: RholangPatternMatcher,

    /// Index of contract invocations (sends to contract channels)
    /// Pattern: (send (contract "<name>") ...) -> SymbolLocation
    pub contract_invocations: RholangPatternMatcher,

    /// Index of channel definitions (from `new` bindings)
    /// Pattern: (new "<channel_name>" ...) -> SymbolLocation
    pub channel_definitions: RholangPatternMatcher,

    /// Inverted index: SymbolId -> [reference locations]
    /// Used for find-references
    pub references: HashMap<SymbolId, Vec<SymbolLocation>>,

    /// Forward index: SymbolId -> definition location
    /// Used for go-to-definition
    pub definitions: HashMap<SymbolId, SymbolLocation>,
}

impl Default for GlobalSymbolIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl GlobalSymbolIndex {
    /// Create a new empty global symbol index
    pub fn new() -> Self {
        Self {
            contract_definitions: RholangPatternMatcher::new(),
            contract_invocations: RholangPatternMatcher::new(),
            channel_definitions: RholangPatternMatcher::new(),
            references: HashMap::new(),
            definitions: HashMap::new(),
        }
    }

    /// Add a contract definition to the index
    pub fn add_contract_definition(
        &mut self,
        name: &str,
        location: SymbolLocation,
    ) -> Result<(), String> {
        // Create pattern: (contract "<name>" <formals> <body>)
        let pattern = Self::create_contract_pattern(name);

        // Store in pattern matcher
        let location_node = location.to_rholang_node();
        self.contract_definitions.add_pattern(&pattern, &location_node)?;

        // Add to definitions map
        let symbol_id = SymbolId {
            uri: location.uri.clone(),
            name: name.to_string(),
            position: (location.range.start.line, location.range.start.character),
        };
        self.definitions.insert(symbol_id, location);

        Ok(())
    }

    /// Add a contract invocation to the index
    pub fn add_contract_invocation(
        &mut self,
        name: &str,
        location: SymbolLocation,
    ) -> Result<(), String> {
        // Create pattern: (send (contract "<name>") <args>)
        let pattern = Self::create_invocation_pattern(name);

        // Store in pattern matcher
        let location_node = location.to_rholang_node();
        self.contract_invocations.add_pattern(&pattern, &location_node)?;

        // Add to references map
        let symbol_id = SymbolId {
            uri: location.uri.clone(),
            name: name.to_string(),
            position: (location.range.start.line, location.range.start.character),
        };
        self.references.entry(symbol_id)
            .or_insert_with(Vec::new)
            .push(location);

        Ok(())
    }

    /// Find all references to a contract
    pub fn find_contract_references(&self, name: &str) -> Result<Vec<SymbolLocation>, String> {
        // Query pattern: (send (contract "<name>") $args)
        let query = Self::create_invocation_pattern(name);

        let matches = self.contract_invocations.match_query(&query)?;

        // Convert results to SymbolLocations
        let mut locations = Vec::new();
        for (node, _bindings) in matches {
            match SymbolLocation::from_rholang_node(&node) {
                Ok(loc) => locations.push(loc),
                Err(e) => eprintln!("Warning: Failed to parse location: {}", e),
            }
        }

        Ok(locations)
    }

    /// Find the definition of a contract
    pub fn find_contract_definition(&self, name: &str) -> Result<Option<SymbolLocation>, String> {
        // Query pattern: (contract "<name>" $formals $body)
        let query = Self::create_contract_pattern(name);

        let matches = self.contract_definitions.match_query(&query)?;

        // Return first match (should only be one definition)
        if let Some((node, _bindings)) = matches.first() {
            Ok(Some(SymbolLocation::from_rholang_node(node)?))
        } else {
            Ok(None)
        }
    }

    /// Add a channel definition to the index
    pub fn add_channel_definition(
        &mut self,
        name: &str,
        location: SymbolLocation,
    ) -> Result<(), String> {
        // Create pattern for channel definition
        let pattern = Self::create_channel_pattern(name);

        // Store in pattern matcher
        let location_node = location.to_rholang_node();
        self.channel_definitions.add_pattern(&pattern, &location_node)?;

        // Add to definitions map
        let symbol_id = SymbolId {
            uri: location.uri.clone(),
            name: name.to_string(),
            position: (location.range.start.line, location.range.start.character),
        };
        self.definitions.insert(symbol_id, location);

        Ok(())
    }

    /// Add a channel usage/reference to the index
    pub fn add_channel_reference(
        &mut self,
        name: &str,
        location: SymbolLocation,
    ) -> Result<(), String> {
        // Add to references map
        let symbol_id = SymbolId {
            uri: location.uri.clone(),
            name: name.to_string(),
            position: (location.range.start.line, location.range.start.character),
        };
        self.references.entry(symbol_id)
            .or_insert_with(Vec::new)
            .push(location);

        Ok(())
    }

    /// Find the definition of a channel
    pub fn find_channel_definition(&self, name: &str) -> Result<Option<SymbolLocation>, String> {
        let query = Self::create_channel_pattern(name);
        let matches = self.channel_definitions.match_query(&query)?;

        if let Some((node, _bindings)) = matches.first() {
            Ok(Some(SymbolLocation::from_rholang_node(node)?))
        } else {
            Ok(None)
        }
    }

    /// Add a variable (let binding) definition to the index
    pub fn add_variable_definition(
        &mut self,
        name: &str,
        location: SymbolLocation,
    ) -> Result<(), String> {
        // Add to definitions map (variables don't use pattern matcher for now)
        let symbol_id = SymbolId {
            uri: location.uri.clone(),
            name: name.to_string(),
            position: (location.range.start.line, location.range.start.character),
        };
        self.definitions.insert(symbol_id, location);

        Ok(())
    }

    /// Add a variable usage/reference to the index
    pub fn add_variable_reference(
        &mut self,
        name: &str,
        location: SymbolLocation,
    ) -> Result<(), String> {
        let symbol_id = SymbolId {
            uri: location.uri.clone(),
            name: name.to_string(),
            position: (location.range.start.line, location.range.start.character),
        };
        self.references.entry(symbol_id)
            .or_insert_with(Vec::new)
            .push(location);

        Ok(())
    }

    /// Create a contract definition pattern
    fn create_contract_pattern(name: &str) -> Arc<RholangNode> {
        // Pattern: (contract "<name>" <formals> <body>)
        // Use StringLiteral (not Var) to create a constant pattern, not a variable
        // Var nodes are converted to "$name" (variables) which unify with anything!
        Arc::new(RholangNode::StringLiteral {
            value: format!("contract:{}", name),
            base: NodeBase::new(
                RelativePosition {
                    delta_lines: 0,
                    delta_columns: 0,
                    delta_bytes: 0,
                },
                0, 0, name.len()
            ),
            metadata: None,
        })
    }

    /// Create a contract invocation pattern
    fn create_invocation_pattern(name: &str) -> Arc<RholangNode> {
        // Pattern: (send (contract "<name>") <args>)
        // Use StringLiteral (not Var) to create a constant pattern, not a variable
        // Var nodes are converted to "$name" (variables) which unify with anything!
        Arc::new(RholangNode::StringLiteral {
            value: format!("send:contract:{}", name),
            base: NodeBase::new(
                RelativePosition {
                    delta_lines: 0,
                    delta_columns: 0,
                    delta_bytes: 0,
                },
                0, 0, name.len()
            ),
            metadata: None,
        })
    }

    /// Create a channel definition pattern
    fn create_channel_pattern(name: &str) -> Arc<RholangNode> {
        // Pattern: (new "<channel_name>" ...)
        // Use StringLiteral for constant pattern matching
        Arc::new(RholangNode::StringLiteral {
            value: format!("channel:{}", name),
            base: NodeBase::new(
                RelativePosition {
                    delta_lines: 0,
                    delta_columns: 0,
                    delta_bytes: 0,
                },
                0, 0, name.len()
            ),
            metadata: None,
        })
    }

    /// Clear all indices (useful for workspace refresh)
    pub fn clear(&mut self) {
        self.contract_definitions = RholangPatternMatcher::new();
        self.contract_invocations = RholangPatternMatcher::new();
        self.channel_definitions = RholangPatternMatcher::new();
        self.references.clear();
        self.definitions.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_location(uri_str: &str, line: u32, character: u32) -> SymbolLocation {
        SymbolLocation {
            uri: Url::parse(uri_str).unwrap(),
            range: Range {
                start: Position { line, character },
                end: Position { line, character: character + 10 },
            },
            kind: SymbolKind::Contract,
            documentation: None,
            signature: None,
        }
    }

    #[test]
    fn test_global_index_creation() {
        let index = GlobalSymbolIndex::new();
        assert_eq!(index.definitions.len(), 0);
        assert_eq!(index.references.len(), 0);
    }

    #[test]
    fn test_add_contract_definition() {
        let mut index = GlobalSymbolIndex::new();
        let location = create_test_location("file:///test.rho", 0, 0);

        let result = index.add_contract_definition("MyContract", location);
        assert!(result.is_ok(), "Should add contract definition successfully");
        assert_eq!(index.definitions.len(), 1);
    }

    #[test]
    fn test_symbol_location_serialization() {
        let location = create_test_location("file:///test.rho", 5, 10);

        // Convert to RholangNode
        let node = location.to_rholang_node();

        // Convert back
        let parsed = SymbolLocation::from_rholang_node(&node).unwrap();

        assert_eq!(parsed.uri, location.uri);
        assert_eq!(parsed.range.start.line, location.range.start.line);
        assert_eq!(parsed.range.start.character, location.range.start.character);
    }

    #[test]
    fn test_clear_index() {
        let mut index = GlobalSymbolIndex::new();
        let location = create_test_location("file:///test.rho", 0, 0);

        index.add_contract_definition("MyContract", location).unwrap();
        assert_eq!(index.definitions.len(), 1);

        index.clear();
        assert_eq!(index.definitions.len(), 0);
        assert_eq!(index.references.len(), 0);
    }
}
