//! Global symbol index for cross-file navigation and reference finding
//!
//! This module provides a workspace-wide index of symbols using MORK pattern matching
//! for efficient O(k) lookups. The index is incrementally updated on document changes.

use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use tower_lsp::lsp_types::{Location, Range, Position, Url};
use crate::ir::pattern_matching::RholangPatternMatcher;
use crate::ir::rholang_node::{RholangNode, NodeBase, Position as IrPosition};
use crate::ir::rholang_pattern_index::{RholangPatternIndex, PatternMetadata};
use pathmap::PathMap;

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
            base: NodeBase::new_simple(
                IrPosition {
                    row: 0,
                    column: 0,
                    byte: 0,
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
    /// NEW: MORK+PathMap-based pattern index for contract parameter matching
    /// Enables goto-definition with pattern unification and overload resolution
    /// Path structure: ["contract", <name>, <param0_mork>, <param1_mork>, ...]
    pub pattern_index: RholangPatternIndex,

    /// LEGACY: Index of contract definitions (to be replaced by pattern_index)
    /// Pattern: (contract "<name>" ...) -> SymbolLocation
    pub contract_definitions: RholangPatternMatcher,

    /// LEGACY: Index of contract invocations (to be replaced by pattern_index)
    /// Pattern: (send (contract "<name>") ...) -> SymbolLocation
    pub contract_invocations: RholangPatternMatcher,

    /// Index of channel definitions (from `new` bindings)
    /// Pattern: (new "<channel_name>" ...) -> SymbolLocation
    pub channel_definitions: RholangPatternMatcher,

    /// Index of map key patterns in contract parameters
    /// Pattern: (map-key-pattern "<contract_name>" "<key_path>") -> SymbolLocation
    /// Example: For contract processComplex(@{user: {name: n, email: e}}, ret)
    ///   - (map-key-pattern "processComplex" "user") -> location of "user:" key
    ///   - (map-key-pattern "processComplex" "user.name") -> location of "name:" key
    ///   - (map-key-pattern "processComplex" "user.email") -> location of "email:" key
    pub map_key_patterns: RholangPatternMatcher,

    /// Inverted index: SymbolId -> [reference locations]
    /// Used for find-references
    pub references: HashMap<SymbolId, Vec<SymbolLocation>>,

    /// Forward index: SymbolId -> definition location
    /// Used for go-to-definition
    pub definitions: HashMap<SymbolId, SymbolLocation>,

    /// Phase A Quick Win #1: Lazy contract-only subtrie
    /// Cached extraction of contract definitions from pattern_index
    /// Path structure: All paths starting with ["contract", ...]
    /// Speedup: 100-551x for workspace symbol queries (from MeTTaTron Phase 1)
    /// Invalidated on: contract indexing/removal
    contract_subtrie: Arc<Mutex<Option<PathMap<crate::ir::rholang_pattern_index::PatternMetadata>>>>,

    /// Tracks whether contract_subtrie needs regeneration
    /// Set to true on: add_contract_with_pattern_index, clear
    /// Set to false on: ensure_contract_subtrie
    contract_subtrie_dirty: Arc<Mutex<bool>>,
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
            pattern_index: RholangPatternIndex::new(),
            contract_definitions: RholangPatternMatcher::new(),
            contract_invocations: RholangPatternMatcher::new(),
            channel_definitions: RholangPatternMatcher::new(),
            map_key_patterns: RholangPatternMatcher::new(),
            references: HashMap::new(),
            definitions: HashMap::new(),
            contract_subtrie: Arc::new(Mutex::new(None)),
            contract_subtrie_dirty: Arc::new(Mutex::new(true)),
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
            base: NodeBase::new_simple(
                IrPosition {
                    row: 0,
                    column: 0,
                    byte: 0,
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
            base: NodeBase::new_simple(
                IrPosition {
                    row: 0,
                    column: 0,
                    byte: 0,
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
            base: NodeBase::new_simple(
                IrPosition {
                    row: 0,
                    column: 0,
                    byte: 0,
                },
                0, 0, name.len()
            ),
            metadata: None,
        })
    }

    /// Add a map key pattern to the index
    ///
    /// # Arguments
    /// * `contract_name` - Name of the contract containing the map pattern
    /// * `key_path` - Dot-separated path to the key (e.g., "user.email")
    /// * `location` - Location of the key in the pattern definition
    ///
    /// # Example
    /// For `contract processComplex(@{user: {name: n, email: e}}, ret)`:
    /// - add_map_key_pattern("processComplex", "user", location_of_user_key)
    /// - add_map_key_pattern("processComplex", "user.name", location_of_name_key)
    /// - add_map_key_pattern("processComplex", "user.email", location_of_email_key)
    pub fn add_map_key_pattern(
        &mut self,
        contract_name: &str,
        key_path: &str,
        location: SymbolLocation,
    ) -> Result<(), String> {
        // Create pattern: (map-key-pattern "<contract_name>" "<key_path>")
        let pattern = Self::create_map_key_pattern(contract_name, key_path);

        // Store in pattern matcher
        let location_node = location.to_rholang_node();
        self.map_key_patterns.add_pattern(&pattern, &location_node)?;

        Ok(())
    }

    /// Query map key patterns for a specific contract and key path
    ///
    /// # Arguments
    /// * `contract_name` - Name of the contract
    /// * `key_path` - Dot-separated path to the key
    ///
    /// # Returns
    /// Vector of matching symbol locations
    ///
    /// # Example
    /// ```
    /// let locations = index.query_map_key_pattern("processComplex", "user.email")?;
    /// ```
    pub fn query_map_key_pattern(
        &self,
        contract_name: &str,
        key_path: &str,
    ) -> Result<Vec<SymbolLocation>, String> {
        // Query pattern: (map-key-pattern "<contract_name>" "<key_path>")
        let query = Self::create_map_key_pattern(contract_name, key_path);

        let matches = self.map_key_patterns.match_query(&query)?;

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

    /// Create a map key pattern for MORK matching
    ///
    /// Pattern format: "map-key:<contract_name>:<key_path>"
    ///
    /// # Arguments
    /// * `contract_name` - Name of the contract
    /// * `key_path` - Dot-separated key path (e.g., "user.email")
    fn create_map_key_pattern(contract_name: &str, key_path: &str) -> Arc<RholangNode> {
        // Pattern: (map-key-pattern "<contract_name>" "<key_path>")
        // Use StringLiteral for constant pattern matching
        let pattern_value = format!("map-key:{}:{}", contract_name, key_path);
        Arc::new(RholangNode::StringLiteral {
            value: pattern_value.clone(),
            base: NodeBase::new_simple(
                IrPosition {
                    row: 0,
                    column: 0,
                    byte: 0,
                },
                0, 0, pattern_value.len()
            ),
            metadata: None,
        })
    }

    /// Index a contract using the new MORK+PathMap pattern index
    ///
    /// This method uses pattern-based matching and supports:
    /// - Exact pattern matching
    /// - Overload resolution by arity
    /// - Parameter pattern unification
    ///
    /// # Arguments
    ///
    /// * `contract_node` - The Contract RholangNode from IR
    /// * `location` - LSP location of the contract definition
    ///
    /// # Returns
    ///
    /// Ok(()) on success, Err with description on failure
    ///
    /// # Example
    ///
    /// ```ignore
    /// let location = SymbolLocation {
    ///     uri: Url::parse("file:///test.rho").unwrap(),
    ///     range: Range { ... },
    ///     kind: SymbolKind::Contract,
    ///     documentation: None,
    ///     signature: Some("contract echo(@x)".to_string()),
    /// };
    /// index.add_contract_with_pattern_index(&contract_node, location)?;
    /// ```
    pub fn add_contract_with_pattern_index(
        &mut self,
        contract_node: &RholangNode,
        location: SymbolLocation,
    ) -> Result<(), String> {
        // Convert LSP SymbolLocation to pattern index SymbolLocation
        let pattern_location = crate::ir::rholang_pattern_index::SymbolLocation {
            uri: location.uri.to_string(),
            start: IrPosition {
                row: location.range.start.line as usize,
                column: location.range.start.character as usize,
                byte: 0, // Not used in this context
            },
            end: IrPosition {
                row: location.range.end.line as usize,
                column: location.range.end.character as usize,
                byte: 0, // Not used in this context
            },
        };

        // Index using the pattern index
        self.pattern_index.index_contract(contract_node, pattern_location)?;

        // Invalidate contract subtrie cache
        self.invalidate_contract_index();

        Ok(())
    }

    /// Query contracts by call-site pattern using the pattern index
    ///
    /// Converts call-site arguments to MORK patterns and queries the index
    /// for matching contract definitions.
    ///
    /// # Arguments
    ///
    /// * `contract_name` - Name of the contract being called
    /// * `arguments` - Argument nodes from the call site
    ///
    /// # Returns
    ///
    /// Vector of matching SymbolLocations (converted from PatternMetadata)
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Query: echo!(42)
    /// let matches = index.query_contract_by_pattern("echo", &[&int_node])?;
    /// ```
    pub fn query_contract_by_pattern(
        &self,
        contract_name: &str,
        arguments: &[&RholangNode],
    ) -> Result<Vec<SymbolLocation>, String> {
        // Query the pattern index
        let matches = self.pattern_index.query_call_site(contract_name, arguments)?;

        // Convert PatternMetadata to SymbolLocation
        let mut locations = Vec::new();
        for metadata in matches {
            let uri = Url::parse(&metadata.location.uri)
                .map_err(|e| format!("Invalid URI in pattern metadata: {}", e))?;

            let location = SymbolLocation {
                uri,
                range: Range {
                    start: Position {
                        line: metadata.location.start.row as u32,
                        character: metadata.location.start.column as u32,
                    },
                    end: Position {
                        line: metadata.location.end.row as u32,
                        character: metadata.location.end.column as u32,
                    },
                },
                kind: SymbolKind::Contract,
                documentation: None,
                signature: Some(Self::format_contract_signature(&metadata)),
            };

            locations.push(location);
        }

        Ok(locations)
    }

    /// Format a contract signature from pattern metadata for display
    fn format_contract_signature(
        metadata: &crate::ir::rholang_pattern_index::PatternMetadata,
    ) -> String {
        if let Some(ref param_names) = metadata.param_names {
            // Use actual parameter names if available
            format!("contract {}({})", metadata.name, param_names.join(", "))
        } else {
            // Use generic parameter names
            let params = (0..metadata.arity)
                .map(|i| format!("@param{}", i))
                .collect::<Vec<_>>()
                .join(", ");
            format!("contract {}({})", metadata.name, params)
        }
    }

    /// Clear all indices (useful for workspace refresh)
    pub fn clear(&mut self) {
        self.pattern_index = RholangPatternIndex::new();
        self.contract_definitions = RholangPatternMatcher::new();
        self.contract_invocations = RholangPatternMatcher::new();
        self.channel_definitions = RholangPatternMatcher::new();
        self.map_key_patterns = RholangPatternMatcher::new();
        self.references.clear();
        self.definitions.clear();

        // Invalidate contract subtrie cache
        *self.contract_subtrie_dirty.lock().unwrap() = true;
    }

    /// Ensure the contract subtrie is initialized and up-to-date
    ///
    /// Phase A Quick Win #1: Lazy subtrie extraction
    /// - Uses PathMap's `.restrict()` to extract contract-only paths without copying
    /// - 100-551x faster than full PathMap traversal (from MeTTaTron Phase 1)
    /// - O(1) cached access after first call
    ///
    /// # Returns
    ///
    /// Ok(()) on success, Err if subtrie extraction fails
    fn ensure_contract_subtrie(&self) -> Result<(), String> {
        let mut dirty = self.contract_subtrie_dirty.lock().unwrap();
        if !*dirty {
            // Subtrie is already up-to-date
            return Ok(());
        }

        // Extract contract-only subtrie using PathMap's restrict() method
        // All contracts are indexed with paths starting with b"contract"
        // This follows MeTTaTron's Phase 1 optimization pattern
        let all_patterns = self.pattern_index.patterns();

        // Create a PathMap containing only the "contract" prefix
        // restrict() will return all paths in all_patterns that have matching prefixes
        // NOTE: The type must match the original PathMap type (PathMap<PatternMetadata>)
        let mut contract_prefix_map: PathMap<PatternMetadata> = PathMap::new();
        let contract_bytes = b"contract";

        // Insert a single path with just "contract" to match all contract definitions
        // IMPORTANT: Must use descend_to() not descend_to_byte() to match pattern_index insertion
        {
            use pathmap::zipper::{ZipperMoving, ZipperWriting};
            use crate::ir::rholang_pattern_index::PatternMetadata;

            let mut wz = contract_prefix_map.write_zipper();
            wz.descend_to(contract_bytes);

            // CRITICAL: Must set a value for restrict() to work!
            //
            // PathMap::restrict() only matches paths that LEAD TO VALUES in the prefix map.
            // Without this set_val() call, restrict() returns empty because the prefix path
            // has no value in PathMap's internal structure.
            //
            // The actual metadata value doesn't matter - only the path prefix matters for
            // prefix matching. We use a dummy PatternMetadata since restrict() just needs
            // the path to have *some* value.
            //
            // See: PathMap documentation on restrict() - "paths leading to values"
            // Reference: MeTTaTron's type_index uses the same pattern (environment.rs:388-395)
            wz.set_val(PatternMetadata::dummy());
        }

        // Extract the subtrie - this is O(prefix_length) not O(total_patterns)!
        let contract_subtrie = all_patterns.restrict(&contract_prefix_map);

        // Update cache
        *self.contract_subtrie.lock().unwrap() = Some(contract_subtrie);
        *dirty = false;

        Ok(())
    }

    /// Query all contracts in the workspace
    ///
    /// Phase A Quick Win #1: Uses lazy subtrie extraction for 100-551x speedup
    /// over full PathMap traversal.
    ///
    /// # Returns
    ///
    /// Vector of all contract locations in the workspace
    ///
    /// # Example
    ///
    /// ```ignore
    /// let contracts = index.query_all_contracts()?;
    /// println!("Found {} contracts in workspace", contracts.len());
    /// ```
    pub fn query_all_contracts(&self) -> Result<Vec<SymbolLocation>, String> {
        // Ensure subtrie is initialized
        self.ensure_contract_subtrie()?;

        // Access cached subtrie
        let subtrie_guard = self.contract_subtrie.lock().unwrap();
        let subtrie = subtrie_guard
            .as_ref()
            .ok_or("Contract subtrie not initialized")?;

        // Collect all PatternMetadata from subtrie
        let mut locations = Vec::new();
        let rz = subtrie.read_zipper();

        // Traverse the subtrie to collect all values
        // Note: This traversal is O(n) where n = number of contracts,
        // NOT O(total_workspace_symbols) which is the key speedup
        Self::collect_all_metadata_from_zipper(rz, &mut locations)?;

        Ok(locations)
    }

    /// Recursively collect all PatternMetadata from a PathMap
    ///
    /// Helper function for query_all_contracts()
    ///
    /// Note: This is a simplified implementation that navigates the PathMap structure.
    /// A more efficient implementation would use PathMap's iterator API when available.
    fn collect_all_metadata_from_zipper(
        mut rz: pathmap::zipper::ReadZipperUntracked<PatternMetadata>,
        locations: &mut Vec<SymbolLocation>,
    ) -> Result<(), String> {
        use pathmap::zipper::{ZipperValues, ZipperIteration};

        // Phase A+: Full subtrie traversal using PathMap's depth-first iteration API
        //
        // Strategy: Use to_next_val() to systematically traverse all values in the subtrie
        // in depth-first order. This is O(n) where n = number of contracts, which is
        // optimal since we must visit every contract to collect all locations.

        // Process current node if it has a value
        if let Some(metadata) = rz.val() {
            let uri = Url::parse(&metadata.location.uri)
                .map_err(|e| format!("Invalid URI in pattern metadata: {}", e))?;

            let location = SymbolLocation {
                uri,
                range: Range {
                    start: Position {
                        line: metadata.location.start.row as u32,
                        character: metadata.location.start.column as u32,
                    },
                    end: Position {
                        line: metadata.location.end.row as u32,
                        character: metadata.location.end.column as u32,
                    },
                },
                kind: SymbolKind::Contract,
                documentation: None,
                signature: Some(Self::format_contract_signature(&metadata)),
            };

            locations.push(location);
        }

        // Traverse all remaining values in depth-first order
        while rz.to_next_val() {
            if let Some(metadata) = rz.val() {
                let uri = Url::parse(&metadata.location.uri)
                    .map_err(|e| format!("Invalid URI in pattern metadata: {}", e))?;

                let location = SymbolLocation {
                    uri,
                    range: Range {
                        start: Position {
                            line: metadata.location.start.row as u32,
                            character: metadata.location.start.column as u32,
                        },
                        end: Position {
                            line: metadata.location.end.row as u32,
                            character: metadata.location.end.column as u32,
                        },
                    },
                    kind: SymbolKind::Contract,
                    documentation: None,
                    signature: Some(Self::format_contract_signature(&metadata)),
                };

                locations.push(location);
            }
        }

        Ok(())
    }

    /// Invalidate the contract subtrie cache
    ///
    /// Call this after adding or removing contracts to force regeneration
    /// on next query_all_contracts() call.
    ///
    /// # Note
    ///
    /// This is automatically called by add_contract_with_pattern_index() and clear()
    pub fn invalidate_contract_index(&self) {
        *self.contract_subtrie_dirty.lock().unwrap() = true;
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

    #[test]
    fn test_add_map_key_pattern() {
        let mut index = GlobalSymbolIndex::new();

        // Create location for "user" key
        let user_location = create_test_location("file:///test.rho", 5, 10);

        // Add map key pattern
        let result = index.add_map_key_pattern("processComplex", "user", user_location);
        assert!(result.is_ok(), "Should add map key pattern successfully");
    }

    #[test]
    fn test_query_map_key_pattern() {
        let mut index = GlobalSymbolIndex::new();

        // Add patterns for nested map keys
        let user_location = create_test_location("file:///test.rho", 5, 10);
        let email_location = create_test_location("file:///test.rho", 5, 20);

        index.add_map_key_pattern("processComplex", "user", user_location.clone()).unwrap();
        index.add_map_key_pattern("processComplex", "user.email", email_location.clone()).unwrap();

        // Query for "user" key
        let results = index.query_map_key_pattern("processComplex", "user").unwrap();
        assert_eq!(results.len(), 1, "Should find one match for 'user' key");
        assert_eq!(results[0].range.start.line, 5);
        assert_eq!(results[0].range.start.character, 10);

        // Query for "user.email" key
        let results = index.query_map_key_pattern("processComplex", "user.email").unwrap();
        assert_eq!(results.len(), 1, "Should find one match for 'user.email' key");
        assert_eq!(results[0].range.start.line, 5);
        assert_eq!(results[0].range.start.character, 20);

        // Query for non-existent key
        let results = index.query_map_key_pattern("processComplex", "nonexistent").unwrap();
        assert_eq!(results.len(), 0, "Should find no matches for non-existent key");
    }

    #[test]
    fn test_map_key_pattern_multiple_contracts() {
        let mut index = GlobalSymbolIndex::new();

        // Add same key path for different contracts
        let location1 = create_test_location("file:///test1.rho", 5, 10);
        let location2 = create_test_location("file:///test2.rho", 10, 20);

        index.add_map_key_pattern("contractA", "user.email", location1).unwrap();
        index.add_map_key_pattern("contractB", "user.email", location2).unwrap();

        // Query for contractA's pattern
        let results = index.query_map_key_pattern("contractA", "user.email").unwrap();
        assert_eq!(results.len(), 1, "Should find contractA's pattern");
        assert_eq!(results[0].uri.as_str(), "file:///test1.rho");

        // Query for contractB's pattern
        let results = index.query_map_key_pattern("contractB", "user.email").unwrap();
        assert_eq!(results.len(), 1, "Should find contractB's pattern");
        assert_eq!(results[0].uri.as_str(), "file:///test2.rho");
    }

    #[test]
    fn test_clear_index_includes_map_patterns() {
        let mut index = GlobalSymbolIndex::new();
        let location = create_test_location("file:///test.rho", 5, 10);

        // Add map key pattern
        index.add_map_key_pattern("processComplex", "user", location).unwrap();

        // Verify it's added (query should succeed)
        let results = index.query_map_key_pattern("processComplex", "user").unwrap();
        assert_eq!(results.len(), 1, "Pattern should be added");

        // Clear the index
        index.clear();

        // Verify it's cleared (query should return empty)
        let results = index.query_map_key_pattern("processComplex", "user").unwrap();
        assert_eq!(results.len(), 0, "Pattern should be cleared");
    }
}
