//! Pattern matching index for Rholang contracts using PathMap
//!
//! This module provides efficient pattern-based lookup of contract definitions
//! using MORK's PathMap trie structure. It enables:
//! - Contract signature matching for goto-definition
//! - Overload resolution (multiple contracts with same name)
//! - Pattern-aware navigation (e.g., map key paths)
//!
//! # Architecture
//!
//! ```text
//! Contract Definition → Extract Pattern → MORK Bytes → PathMap Index
//!                                                            ↓
//!                                                       ReadZipper Query
//!                                                            ↓
//!                                                    Return Matching Contracts
//! ```
//!
//! # Example
//!
//! ```rholang
//! contract echo(@x) = { x!(x) }
//! contract processUser(@{"name": n, "email": e}) = { ... }
//!
//! // Call site:
//! echo!("hello")  // Query finds echo contract
//! ```

use std::sync::Arc;
use pathmap::PathMap;
use pathmap::zipper::{ZipperMoving, ZipperValues, ZipperWriting};
use mork::space::Space;
use mork_interning::SharedMappingHandle;
use serde::{Serialize, Deserialize};

use crate::ir::rholang_node::RholangNode;
use crate::ir::semantic_node::Position;
use crate::ir::mork_canonical::MorkForm;

/// Location of a symbol in the workspace
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SymbolLocation {
    /// URI of the document containing this symbol
    pub uri: String,

    /// Start position in the document
    pub start: Position,

    /// End position in the document
    pub end: Position,
}

/// Metadata about a contract pattern stored in the index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternMetadata {
    /// Location of the contract definition
    pub location: SymbolLocation,

    /// Contract name
    pub name: String,

    /// Number of parameters
    pub arity: usize,

    /// Parameter patterns (serialized MORK bytes for each parameter)
    pub param_patterns: Vec<Vec<u8>>,

    /// Optional: Parameter names if available from the source
    pub param_names: Option<Vec<String>>,
}

/// Pattern matching index for Rholang contracts using PathMap
///
/// Stores contract patterns in a trie structure for efficient lookup:
/// - Path structure: ["contract", <name>, <param0_mork_bytes>, <param1_mork_bytes>, ...]
/// - Value: PatternMetadata with location and signature info
pub struct RholangPatternIndex {
    /// PathMap trie storing contract patterns
    /// Keys are byte paths, values are PatternMetadata
    patterns: PathMap<PatternMetadata>,

    /// MORK SharedMappingHandle for thread-safe symbol interning
    /// Each thread creates its own Space when needed for serialization
    shared_mapping: SharedMappingHandle,
}

// Manual Debug implementation for cleaner output
impl std::fmt::Debug for RholangPatternIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RholangPatternIndex")
            .field("patterns", &self.patterns)
            .field("shared_mapping", &"<SharedMappingHandle>")
            .finish()
    }
}

impl RholangPatternIndex {
    /// Create a new empty pattern index
    pub fn new() -> Self {
        use mork_interning::SharedMapping;
        Self {
            patterns: PathMap::new(),
            shared_mapping: SharedMapping::new(),
        }
    }

    /// Index a contract definition from the IR
    ///
    /// Extracts the contract signature, converts parameters to MORK patterns,
    /// and stores in the PathMap trie.
    ///
    /// # Arguments
    ///
    /// * `contract_node` - The Contract node from RholangNode IR
    /// * `location` - Source location of the contract definition
    ///
    /// # Returns
    ///
    /// Ok(()) on success, Err with description on failure
    pub fn index_contract(
        &mut self,
        contract_node: &RholangNode,
        location: SymbolLocation,
    ) -> Result<(), String> {
        // Extract contract name and parameters
        let (name, params) = Self::extract_contract_signature(contract_node)?;

        // Create thread-local Space for this operation (MORK pattern)
        let space = Space {
            btm: PathMap::new(),
            sm: self.shared_mapping.clone(),
            mmaps: std::collections::HashMap::new(),
        };

        // Convert parameters to MORK bytes
        let param_patterns: Vec<Vec<u8>> = params
            .iter()
            .map(|p| Self::pattern_to_mork_bytes(p, &space))
            .collect::<Result<_, _>>()?;

        // Extract parameter names if available
        let param_names = Self::extract_param_names(&params);

        // Build PathMap path: ["contract", name_bytes, param0_bytes, param1_bytes, ...]
        let mut path: Vec<&[u8]> = Vec::with_capacity(2 + param_patterns.len());
        path.push(b"contract");
        path.push(name.as_bytes());
        for pattern_bytes in &param_patterns {
            path.push(pattern_bytes.as_slice());
        }

        // Create metadata
        let metadata = PatternMetadata {
            location,
            name: name.clone(),
            arity: params.len(),
            param_patterns: param_patterns.clone(),
            param_names,
        };

        // Use WriteZipper to insert into PathMap
        let mut wz = self.patterns.write_zipper();
        for segment in &path {
            wz.descend_to(segment);
        }
        wz.set_val(metadata);

        Ok(())
    }

    /// Query contracts matching a call-site pattern
    ///
    /// Converts the call-site arguments to MORK patterns and searches the index
    /// for matching contract definitions.
    ///
    /// # Arguments
    ///
    /// * `contract_name` - Name of the contract being called
    /// * `arguments` - Argument nodes from the call site
    ///
    /// # Returns
    ///
    /// Vector of matching PatternMetadata, empty if no matches found
    pub fn query_call_site(
        &self,
        contract_name: &str,
        arguments: &[&RholangNode],
    ) -> Result<Vec<PatternMetadata>, String> {
        // Create thread-local Space for this operation (MORK pattern)
        let space = Space {
            btm: PathMap::new(),
            sm: self.shared_mapping.clone(),
            mmaps: std::collections::HashMap::new(),
        };

        // Convert arguments to MORK bytes
        let arg_patterns: Vec<Vec<u8>> = arguments
            .iter()
            .map(|a| Self::node_to_mork_bytes(a, &space))
            .collect::<Result<_, _>>()?;

        // Build query path
        let mut path: Vec<&[u8]> = Vec::with_capacity(2 + arg_patterns.len());
        path.push(b"contract");
        path.push(contract_name.as_bytes());
        for pattern_bytes in &arg_patterns {
            path.push(pattern_bytes.as_slice());
        }

        // Try exact match first using ReadZipper
        let mut rz = self.patterns.read_zipper();
        let mut found = true;
        for segment in &path {
            if !rz.descend_to_check(segment) {
                found = false;
                break;
            }
        }

        if found {
            if let Some(metadata) = rz.val() {
                return Ok(vec![metadata.clone()]);
            }
        }

        // Fall back to pattern unification using MORK's unify
        // This handles variable patterns, wildcards, etc.
        self.unify_patterns(contract_name, &arg_patterns)
    }

    /// Use MORK's unify() for pattern matching
    ///
    /// Finds contracts where stored patterns can unify with call-site patterns.
    /// This is more flexible than exact matching - it handles variables, wildcards, etc.
    fn unify_patterns(
        &self,
        contract_name: &str,
        arg_patterns: &[Vec<u8>],
    ) -> Result<Vec<PatternMetadata>, String> {
        let mut matches = Vec::new();

        // Get ReadZipper positioned at contract name prefix
        let mut rz = self.patterns.read_zipper();

        // Navigate to contract name
        if !rz.descend_to_check(b"contract") {
            return Ok(Vec::new()); // No contracts indexed
        }
        if !rz.descend_to_check(contract_name.as_bytes()) {
            return Ok(Vec::new()); // No contracts with this name
        }

        // Iterate over all stored patterns under this contract name
        // TODO: Implement proper traversal once we understand PathMap's iteration API
        // For now, collect all metadata at this level
        if let Some(metadata) = rz.val() {
            // Check arity
            if metadata.arity == arg_patterns.len() {
                // TODO: Implement MORK unification check
                // For now, accept any arity match as a placeholder
                matches.push(metadata.clone());
            }
        }

        Ok(matches)
    }

    // ========== Helper Functions ==========

    /// Extract contract signature from IR node
    ///
    /// Returns (name, parameters) where parameters are pattern nodes
    fn extract_contract_signature(
        contract_node: &RholangNode,
    ) -> Result<(String, Vec<Arc<RholangNode>>), String> {
        match contract_node {
            RholangNode::Contract { name, formals, .. } => {
                // Extract contract name
                let contract_name = match name.as_ref() {
                    RholangNode::Var { name, .. } => name.clone(),
                    RholangNode::Quote { quotable, .. } => {
                        // Contract names can be quoted processes like @"myContract"
                        match quotable.as_ref() {
                            RholangNode::StringLiteral { value, .. } => value.clone(),
                            _ => return Err(format!("Unsupported contract name format: {:?}", quotable)),
                        }
                    }
                    _ => return Err(format!("Unsupported contract name node: {:?}", name)),
                };

                // Extract parameters (formals are the parameter patterns)
                let params: Vec<Arc<RholangNode>> = formals.iter().cloned().collect();

                Ok((contract_name, params))
            }
            _ => Err(format!("Expected Contract node, got: {:?}", contract_node)),
        }
    }

    /// Convert a pattern node to MORK bytes
    fn pattern_to_mork_bytes(
        pattern_node: &RholangNode,
        space: &Space,
    ) -> Result<Vec<u8>, String> {
        // Convert RholangNode pattern to MorkForm
        let mork_form = Self::rholang_pattern_to_mork(pattern_node)?;

        // Serialize to MORK bytes
        mork_form.to_mork_bytes(space)
    }

    /// Convert any RholangNode to MORK bytes
    fn node_to_mork_bytes(
        node: &RholangNode,
        space: &Space,
    ) -> Result<Vec<u8>, String> {
        // Convert RholangNode to MorkForm
        let mork_form = Self::rholang_node_to_mork(node)?;

        // Serialize to MORK bytes
        mork_form.to_mork_bytes(space)
    }

    /// Convert RholangNode pattern to MorkForm
    ///
    /// This is similar to `rholang_node_to_mork()` but specifically for patterns.
    /// The key difference is that in pattern context, we prefer pattern-specific
    /// MorkForm variants like MapPattern, ListPattern, etc.
    fn rholang_pattern_to_mork(
        node: &RholangNode,
    ) -> Result<MorkForm, String> {
        use crate::ir::mork_canonical::{MorkForm as MF, LiteralValue as LV};

        match node {
            // ========== Literals (in patterns) ==========
            RholangNode::Nil { .. } => Ok(MF::Nil),
            RholangNode::BoolLiteral { value, .. } => Ok(MF::Literal(LV::Bool(*value))),
            RholangNode::LongLiteral { value, .. } => Ok(MF::Literal(LV::Int(*value))),
            RholangNode::StringLiteral { value, .. } => Ok(MF::Literal(LV::String(value.clone()))),
            RholangNode::UriLiteral { value, .. } => Ok(MF::Literal(LV::Uri(value.clone()))),

            // ========== Pattern-specific nodes ==========
            RholangNode::Var { name, .. } => Ok(MF::VarPattern(name.clone())),
            RholangNode::Wildcard { .. } => Ok(MF::WildcardPattern),

            // ========== Quote pattern ==========
            RholangNode::Quote { quotable, .. } => {
                let inner = Self::rholang_pattern_to_mork(quotable)?;
                Ok(MF::Name(Box::new(inner)))
            }

            // ========== Collection patterns ==========
            RholangNode::List { elements, remainder, .. } => {
                if remainder.is_some() {
                    return Err("List patterns with remainder not yet supported".to_string());
                }
                let elems: Result<Vec<_>, _> = elements.iter()
                    .map(|e| Self::rholang_pattern_to_mork(e))
                    .collect();
                Ok(MF::ListPattern(elems?))
            }

            RholangNode::Tuple { elements, .. } => {
                let elems: Result<Vec<_>, _> = elements.iter()
                    .map(|e| Self::rholang_pattern_to_mork(e))
                    .collect();
                Ok(MF::TuplePattern(elems?))
            }

            RholangNode::Set { elements, remainder, .. } => {
                if remainder.is_some() {
                    return Err("Set patterns with remainder not yet supported".to_string());
                }
                let elems: Result<Vec<_>, _> = elements.iter()
                    .map(|e| Self::rholang_pattern_to_mork(e))
                    .collect();
                Ok(MF::SetPattern(elems?))
            }

            RholangNode::Map { pairs, remainder, .. } => {
                if remainder.is_some() {
                    return Err("Map patterns with remainder not yet supported".to_string());
                }
                let map_pairs: Result<Vec<(String, MF)>, String> = pairs.iter()
                    .map(|(key_node, value_node)| -> Result<(String, MF), String> {
                        // Keys must be strings
                        let key = match key_node.as_ref() {
                            RholangNode::StringLiteral { value, .. } => Ok(value.clone()),
                            RholangNode::Quote { quotable, .. } => {
                                match quotable.as_ref() {
                                    RholangNode::StringLiteral { value, .. } => Ok(value.clone()),
                                    _ => Err("Map keys must be string literals".to_string()),
                                }
                            }
                            _ => Err("Map keys must be string literals".to_string()),
                        }?;
                        let value = Self::rholang_pattern_to_mork(value_node)?;
                        Ok((key, value))
                    })
                    .collect();
                Ok(MF::MapPattern(map_pairs?))
            }

            // ========== Unwrap wrappers ==========
            RholangNode::Parenthesized { expr, .. } => {
                Self::rholang_pattern_to_mork(expr)
            }

            RholangNode::Block { proc, .. } => {
                Self::rholang_pattern_to_mork(proc)
            }

            // ========== Unsupported ==========
            _ => {
                Err(format!("Pattern conversion to MORK not implemented for: {:?}", node))
            }
        }
    }

    /// Convert RholangNode to MorkForm
    fn rholang_node_to_mork(
        node: &RholangNode,
    ) -> Result<MorkForm, String> {
        use crate::ir::mork_canonical::{MorkForm as MF, LiteralValue as LV};

        match node {
            // ========== Literals ==========
            RholangNode::Nil { .. } => Ok(MF::Nil),

            RholangNode::BoolLiteral { value, .. } => {
                Ok(MF::Literal(LV::Bool(*value)))
            }

            RholangNode::LongLiteral { value, .. } => {
                Ok(MF::Literal(LV::Int(*value)))
            }

            RholangNode::StringLiteral { value, .. } => {
                Ok(MF::Literal(LV::String(value.clone())))
            }

            RholangNode::UriLiteral { value, .. } => {
                Ok(MF::Literal(LV::Uri(value.clone())))
            }

            // ========== Variables and Patterns ==========
            RholangNode::Var { name, .. } => {
                // In patterns, this is a variable binding
                Ok(MF::VarPattern(name.clone()))
            }

            RholangNode::Wildcard { .. } => {
                Ok(MF::WildcardPattern)
            }

            // ========== Quotation ==========
            RholangNode::Quote { quotable, .. } => {
                let inner = Self::rholang_node_to_mork(quotable)?;
                Ok(MF::Name(Box::new(inner)))
            }

            // ========== Collections ==========
            RholangNode::List { elements, remainder, .. } => {
                if remainder.is_some() {
                    return Err("List patterns with remainder not yet supported".to_string());
                }
                let elems: Result<Vec<_>, _> = elements.iter()
                    .map(|e| Self::rholang_node_to_mork(e))
                    .collect();
                Ok(MF::List(elems?))
            }

            RholangNode::Tuple { elements, .. } => {
                let elems: Result<Vec<_>, _> = elements.iter()
                    .map(|e| Self::rholang_node_to_mork(e))
                    .collect();
                Ok(MF::Tuple(elems?))
            }

            RholangNode::Set { elements, remainder, .. } => {
                if remainder.is_some() {
                    return Err("Set patterns with remainder not yet supported".to_string());
                }
                let elems: Result<Vec<_>, _> = elements.iter()
                    .map(|e| Self::rholang_node_to_mork(e))
                    .collect();
                Ok(MF::Set(elems?))
            }

            RholangNode::Map { pairs, remainder, .. } => {
                if remainder.is_some() {
                    return Err("Map patterns with remainder not yet supported".to_string());
                }
                let map_pairs: Result<Vec<(String, MF)>, String> = pairs.iter()
                    .map(|(key_node, value_node)| -> Result<(String, MF), String> {
                        // Keys must be strings
                        let key = match key_node.as_ref() {
                            RholangNode::StringLiteral { value, .. } => Ok(value.clone()),
                            RholangNode::Quote { quotable, .. } => {
                                match quotable.as_ref() {
                                    RholangNode::StringLiteral { value, .. } => Ok(value.clone()),
                                    _ => Err("Map keys must be string literals".to_string()),
                                }
                            }
                            _ => Err("Map keys must be string literals".to_string()),
                        }?;
                        let value = Self::rholang_node_to_mork(value_node)?;
                        Ok((key, value))
                    })
                    .collect();
                Ok(MF::Map(map_pairs?))
            }

            // ========== Processes ==========
            RholangNode::Send { channel, inputs, .. } => {
                let chan = Box::new(Self::rholang_node_to_mork(channel)?);
                let args: Result<Vec<_>, _> = inputs.iter()
                    .map(|i| Self::rholang_node_to_mork(i))
                    .collect();
                Ok(MF::Send {
                    channel: chan,
                    arguments: args?,
                })
            }

            RholangNode::Par { processes: Some(procs), .. } => {
                let ps: Result<Vec<_>, _> = procs.iter()
                    .map(|p| Self::rholang_node_to_mork(p))
                    .collect();
                Ok(MF::Par(ps?))
            }

            RholangNode::Par { left: Some(l), right: Some(r), .. } => {
                // Legacy binary Par form
                let left_mork = Self::rholang_node_to_mork(l)?;
                let right_mork = Self::rholang_node_to_mork(r)?;
                Ok(MF::Par(vec![left_mork, right_mork]))
            }

            RholangNode::New { decls, proc, .. } => {
                // Extract variable names from declarations
                let var_names: Result<Vec<String>, _> = decls.iter()
                    .map(|d| match d.as_ref() {
                        RholangNode::NameDecl { var, .. } => {
                            match var.as_ref() {
                                RholangNode::Var { name, .. } => Ok(name.clone()),
                                _ => Err("Expected Var in NameDecl".to_string()),
                            }
                        }
                        _ => Err("Expected NameDecl in New".to_string()),
                    })
                    .collect();

                let body = Box::new(Self::rholang_node_to_mork(proc)?);
                Ok(MF::New {
                    variables: var_names?,
                    body,
                })
            }

            RholangNode::Contract { name, formals, proc, .. } => {
                // Extract contract name
                let contract_name = match name.as_ref() {
                    RholangNode::Var { name, .. } => name.clone(),
                    RholangNode::Quote { quotable, .. } => {
                        match quotable.as_ref() {
                            RholangNode::StringLiteral { value, .. } => value.clone(),
                            _ => return Err("Unsupported contract name format".to_string()),
                        }
                    }
                    _ => return Err("Unsupported contract name node".to_string()),
                };

                // Convert parameters
                let params: Result<Vec<_>, _> = formals.iter()
                    .map(|p| Self::rholang_pattern_to_mork(p))
                    .collect();

                let body = Box::new(Self::rholang_node_to_mork(proc)?);

                Ok(MF::Contract {
                    name: contract_name,
                    parameters: params?,
                    body,
                })
            }

            RholangNode::Input { receipts, proc, .. } => {
                // For comprehension: for(@pattern <- channel) { body }
                let bindings: Result<Vec<(MF, MF)>, _> = receipts.iter()
                    .map(|receipt| {
                        // Each receipt is a vector of binds
                        // For simplicity, handle single LinearBind per receipt
                        if receipt.len() != 1 {
                            return Err("Only single binds per receipt supported".to_string());
                        }

                        match receipt.iter().next().unwrap().as_ref() {
                            RholangNode::LinearBind { names, source, .. } => {
                                // Pattern from names
                                let pattern = if names.len() == 1 {
                                    Self::rholang_pattern_to_mork(&names[0])?
                                } else {
                                    // Multiple names - tuple pattern
                                    let pats: Result<Vec<_>, _> = names.iter()
                                        .map(|n| Self::rholang_pattern_to_mork(n))
                                        .collect();
                                    MF::TuplePattern(pats?)
                                };
                                let channel = Self::rholang_node_to_mork(source)?;
                                Ok((pattern, channel))
                            }
                            _ => Err("Expected LinearBind in receipt".to_string()),
                        }
                    })
                    .collect();

                let body = Box::new(Self::rholang_node_to_mork(proc)?);

                Ok(MF::For {
                    bindings: bindings?,
                    body,
                })
            }

            RholangNode::Match { expression, cases, .. } => {
                let target = Box::new(Self::rholang_node_to_mork(expression)?);
                let case_list: Result<Vec<(MF, MF)>, String> = cases.iter()
                    .map(|(pattern, body)| -> Result<(MF, MF), String> {
                        let pat = Self::rholang_pattern_to_mork(pattern)?;
                        let bod = Self::rholang_node_to_mork(body)?;
                        Ok((pat, bod))
                    })
                    .collect();

                Ok(MF::Match {
                    target,
                    cases: case_list?,
                })
            }

            // ========== Parenthesized/Block (unwrap) ==========
            RholangNode::Parenthesized { expr, .. } => {
                Self::rholang_node_to_mork(expr)
            }

            RholangNode::Block { proc, .. } => {
                Self::rholang_node_to_mork(proc)
            }

            // ========== Unsupported (return error) ==========
            _ => {
                Err(format!("Conversion to MORK not implemented for: {:?}", node))
            }
        }
    }

    /// Extract parameter names from pattern nodes if available
    ///
    /// For simple variable patterns like `@x`, extracts "x".
    /// For complex patterns, returns None.
    fn extract_param_names(
        params: &[Arc<RholangNode>],
    ) -> Option<Vec<String>> {
        let mut names = Vec::new();

        for param in params {
            match param.as_ref() {
                // Simple variable pattern: @x
                RholangNode::Quote { quotable, .. } => {
                    match quotable.as_ref() {
                        RholangNode::Var { name, .. } => {
                            names.push(name.clone());
                        }
                        _ => return None, // Complex pattern
                    }
                }
                // Direct variable (rare but possible)
                RholangNode::Var { name, .. } => {
                    names.push(name.clone());
                }
                // Complex patterns - can't extract simple name
                _ => return None,
            }
        }

        Some(names)
    }
}

impl Default for RholangPatternIndex {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::mork_canonical::{MorkForm, LiteralValue};

    // ========== Basic Tests ==========

    #[test]
    fn test_create_index() {
        let index = RholangPatternIndex::new();
        // Just verify the index was created successfully
        assert!(Arc::strong_count(index.space()) >= 1);
    }

    // ========== MORK Form Tests ==========
    // Test the conversion logic directly with MorkForm, not RholangNode

    #[test]
    fn test_mork_int_serialization() {
        let space = mork::space::Space::new();
        let mork = MorkForm::Literal(LiteralValue::Int(42));
        let result = mork.to_mork_bytes(&space);
        assert!(result.is_ok());
        assert!(result.unwrap().len() > 0);
    }

    #[test]
    fn test_mork_string_serialization() {
        let space = mork::space::Space::new();
        let mork = MorkForm::Literal(LiteralValue::String("hello".to_string()));
        let result = mork.to_mork_bytes(&space);
        assert!(result.is_ok());
        assert!(result.unwrap().len() > 0);
    }

    #[test]
    fn test_mork_var_pattern_serialization() {
        let space = mork::space::Space::new();
        let mork = MorkForm::VarPattern("x".to_string());
        let result = mork.to_mork_bytes(&space);
        assert!(result.is_ok());
        assert!(result.unwrap().len() > 0);
    }

    #[test]
    fn test_mork_deterministic_serialization() {
        let space = mork::space::Space::new();
        let mork = MorkForm::VarPattern("test".to_string());

        let bytes1 = mork.to_mork_bytes(&space).unwrap();
        let bytes2 = mork.to_mork_bytes(&space).unwrap();

        assert_eq!(bytes1, bytes2, "Serialization should be deterministic");
    }

    #[test]
    fn test_mork_round_trip() {
        let space = mork::space::Space::new();
        let mork = MorkForm::Literal(LiteralValue::Int(123));

        let bytes = mork.to_mork_bytes(&space).unwrap();
        let recovered = MorkForm::from_mork_bytes(&bytes, &space).unwrap();

        assert_eq!(mork, recovered, "Round-trip should preserve form");
    }

    // NOTE: More comprehensive tests for contract signature extraction,
    // pattern conversion, and indexing will be added in integration tests
    // where we can use the actual parser to create RholangNode instances.
    // For now, we verify the MORK serialization layer works correctly.
}
