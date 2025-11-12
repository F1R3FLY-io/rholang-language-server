//! Generic goto-definition implementation
//!
//! This module provides a language-agnostic goto-definition feature that works
//! with any language implementing the SemanticNode trait and LanguageAdapter.
//!
//! # Architecture
//!
//! ```text
//! User clicks on symbol
//!       ↓
//! GenericGotoDefinition::goto_definition()
//!       ├─→ Find node at position (language-agnostic)
//!       ├─→ Determine semantic category (Variable, Invocation, etc.)
//!       ├─→ Use LanguageAdapter.resolver to find definition
//!       └─→ Return location(s)
//! ```
//!
//! # Usage
//!
//! ```rust,ignore
//! use crate::lsp::features::goto_definition::GenericGotoDefinition;
//!
//! let goto_def = GenericGotoDefinition;
//!
//! let response = goto_def.goto_definition(
//!     &root_node,
//!     &position,
//!     &uri,
//!     &language_adapter,
//! ).await?;
//! ```

use std::sync::Arc;

use tower_lsp::lsp_types::{
    GotoDefinitionResponse, Location, Position as LspPosition, Range, Url,
};
use tracing::{debug, info};

use crate::ir::semantic_node::{Position, SemanticCategory, SemanticNode};
use crate::ir::symbol_resolution::{ResolutionContext, SymbolLocation};
use crate::lsp::features::node_finder::{find_node_at_position, ir_to_lsp_position};
use crate::lsp::features::traits::LanguageAdapter;

/// Generic goto-definition feature
///
/// This struct provides language-agnostic goto-definition functionality.
/// It works with any semantic IR that implements the SemanticNode trait.
pub struct GenericGotoDefinition;

impl GenericGotoDefinition {
    /// Perform goto-definition at a given position
    ///
    /// # Arguments
    /// * `root` - Root node of the semantic tree
    /// * `position` - Position where goto-definition was requested (IR coordinates)
    /// * `uri` - URI of the document
    /// * `adapter` - Language adapter for this document's language
    ///
    /// # Returns
    /// `Some(GotoDefinitionResponse)` with definition location(s), or `None` if not found
    ///
    /// # Algorithm
    /// 1. Find the node at the requested position
    /// 2. Check the node's semantic category
    /// 3. Use language adapter's resolver to find definition
    /// 4. Convert symbol locations to LSP locations
    pub async fn goto_definition(
        &self,
        root: &dyn SemanticNode,
        position: &Position,
        uri: &Url,
        adapter: &LanguageAdapter,
    ) -> Option<GotoDefinitionResponse> {
        debug!(
            "GenericGotoDefinition::goto_definition at {:?} in {} (language: {})",
            position, uri, adapter.language_name()
        );

        // Find node at position
        let node = find_node_at_position(root, position)?;
        debug!(
            "GenericGotoDefinition: Found node at position: type={}, category={:?}",
            node.type_name(),
            node.semantic_category()
        );

        // Get symbol name based on semantic category
        let symbol_name = match self.extract_symbol_name(node, position) {
            Some(name) => {
                debug!("GenericGotoDefinition: Extracted symbol name: '{}'", name);
                name
            }
            None => {
                debug!("GenericGotoDefinition: FAILED to extract symbol name from node type={}", node.type_name());
                return None;
            }
        };

        // Try to get the actual Var node from wrapped structures like LinearBind
        let var_node = self.find_var_node_in_tree(node).unwrap_or(node);

        // Check if node has referenced_symbol metadata (set by SymbolTableBuilder)
        // If so, use it directly to get the definition location, bypassing resolver
        if let Some(metadata) = var_node.metadata() {
            if let Some(sym_any) = metadata.get("referenced_symbol") {
                if let Some(symbol) = sym_any.downcast_ref::<Arc<crate::ir::symbol_table::Symbol>>() {
                    debug!("Using referenced_symbol metadata for '{}'", symbol.name);

                    // Use definition location if available, otherwise declaration
                    let target_location = symbol.definition_location.as_ref().unwrap_or(&symbol.declaration_location);

                    use tower_lsp::lsp_types::{Position as LspPosition, Range};
                    let lsp_pos = LspPosition {
                        line: target_location.row as u32,
                        character: target_location.column as u32,
                    };
                    let range = Range {
                        start: lsp_pos,
                        end: LspPosition {
                            line: lsp_pos.line,
                            character: lsp_pos.character + symbol.name.len() as u32,
                        },
                    };

                    let location = tower_lsp::lsp_types::Location {
                        uri: symbol.declaration_uri.clone(),
                        range,
                    };

                    debug!("Referenced symbol resolved to {:?}", location);
                    return Some(GotoDefinitionResponse::Scalar(location));
                }
            }
        }

        // Fallback: Use language adapter's resolver to find definition
        // The LexicalScopeResolver will query the symbol table at this position
        // to find the symbol and its scope_id, so we don't need to extract it here
        //
        // For pattern-aware resolution (e.g., Rholang contracts), we need to pass the Send node
        // so the resolver can extract arguments for pattern matching.
        let ir_node_any: Option<Arc<dyn std::any::Any + Send + Sync>> = if adapter.language_name() == "rholang" {
            // For Rholang, find the containing Send node for contract invocations
            use crate::ir::rholang_node::RholangNode;
            self.find_containing_send_node(root, position)
                .map(|send_node| Arc::new(send_node) as Arc<dyn std::any::Any + Send + Sync>)
        } else {
            None
        };

        let context = ResolutionContext {
            uri: uri.clone(),
            scope_id: None, // LexicalScopeResolver will extract from symbol table via position
            ir_node: ir_node_any,  // Pass Send node for pattern-aware resolution
            language: adapter.language_name().to_string(),
            parent_uri: None, // Set by caller if this is a virtual document
        };

        let locations = adapter.resolver.resolve_symbol(
            symbol_name,
            position,
            &context,
        );

        if locations.is_empty() {
            debug!("No definitions found for symbol '{}'", symbol_name);
            return None;
        }

        debug!(
            "Found {} definition location(s) for '{}'",
            locations.len(),
            symbol_name
        );

        // Convert to LSP response
        Some(self.symbol_locations_to_response(locations))
    }

    /// Check if a position is within a range using line/column comparison
    ///
    /// Position must be >= start and < end (half-open interval: [start, end))
    fn position_in_range(pos: &Position, start: &Position, end: &Position) -> bool {
        // Position must be >= start
        if pos.row < start.row {
            return false;
        }
        if pos.row == start.row && pos.column < start.column {
            return false;
        }

        // Position must be < end (exclusive)
        if pos.row > end.row {
            return false;
        }
        if pos.row == end.row && pos.column >= end.column {
            return false;
        }

        true
    }

    /// Extract symbol name from a node
    ///
    /// This examines the node's type and metadata to extract a symbol name.
    /// The exact extraction logic depends on the semantic category.
    ///
    /// # Arguments
    /// * `node` - The node to extract a symbol name from
    /// * `position` - The target position (used for position-aware traversal of collections)
    fn extract_symbol_name<'a>(&self, node: &'a dyn SemanticNode, position: &Position) -> Option<&'a str> {
        // Try to get symbol name from metadata
        if let Some(metadata) = node.metadata() {
            debug!(
                "Node has metadata with keys: {:?}",
                metadata.keys().collect::<Vec<_>>()
            );

            // Priority 1: Check for "referenced_symbol" key (most authoritative - used by SymbolTableBuilder)
            if let Some(sym_any) = metadata.get("referenced_symbol") {
                if let Some(symbol) = sym_any.downcast_ref::<Arc<crate::ir::symbol_table::Symbol>>() {
                    debug!("Found referenced_symbol in metadata: {}", symbol.name);
                    return Some(&symbol.name);
                }
            }

            // Priority 2: Check for "symbol_name" key (common convention)
            if let Some(name_any) = metadata.get("symbol_name") {
                if let Some(name_ref) = name_any.downcast_ref::<String>() {
                    debug!("Found symbol_name in metadata: {}", name_ref);
                    return Some(name_ref.as_str());
                }
                if let Some(name_ref) = name_any.downcast_ref::<Arc<String>>() {
                    debug!("Found symbol_name (Arc) in metadata: {}", name_ref);
                    return Some(name_ref.as_str());
                }
                if let Some(name_ref) = name_any.downcast_ref::<&str>() {
                    debug!("Found symbol_name (&str) in metadata: {}", name_ref);
                    return Some(*name_ref);
                }
            }

            // Priority 3: Check for "name" key (alternative convention)
            if let Some(name_any) = metadata.get("name") {
                if let Some(name_ref) = name_any.downcast_ref::<String>() {
                    debug!("Found name in metadata: {}", name_ref);
                    return Some(name_ref.as_str());
                }
                if let Some(name_ref) = name_any.downcast_ref::<Arc<String>>() {
                    debug!("Found name (Arc) in metadata: {}", name_ref);
                    return Some(name_ref.as_str());
                }
            }
        }

        // Priority 4: Extract directly from node structure based on type
        // Try to downcast to RholangNode
        if let Some(rho_node) = node.as_any().downcast_ref::<crate::ir::rholang_node::RholangNode>() {
            use crate::ir::rholang_node::RholangNode;
            match rho_node {
                RholangNode::Var { name, .. } => {
                    debug!("Extracted symbol name from RholangNode::Var: {}", name);
                    return Some(name.as_str());
                }
                RholangNode::VarRef { var, .. } => {
                    // Recursively extract from the var inside VarRef
                    if let RholangNode::Var { name, .. } = &**var {
                        debug!("Extracted symbol name from RholangNode::VarRef->Var: {}", name);
                        return Some(name.as_str());
                    }
                }
                RholangNode::Quote { quotable, .. } => {
                    // For Quote nodes (e.g., @fromRoom or @"myContract"), extract from the inner node
                    if let RholangNode::Var { name, .. } = &**quotable {
                        debug!("Extracted symbol name from RholangNode::Quote->Var: {}", name);
                        return Some(name.as_str());
                    }
                    // Also handle quoted string literals (e.g., @"myContract")
                    if let RholangNode::StringLiteral { value, .. } = &**quotable {
                        debug!("Extracted symbol name from RholangNode::Quote->StringLiteral: {}", value);
                        return Some(value.as_str());
                    }
                }
                RholangNode::LinearBind { source, .. } => {
                    // For LinearBind nodes (e.g., @result <- queryResult), extract from the source
                    debug!("Found LinearBind node, recursively extracting from source (type={})", source.type_name());
                    let result = self.extract_symbol_name(&**source, position);
                    if result.is_none() {
                        debug!("LinearBind source extraction failed for type={}", source.type_name());
                    }
                    return result;
                }
                RholangNode::RepeatedBind { source, .. } => {
                    // For RepeatedBind nodes (e.g., x <= ch), extract from the source
                    debug!("Found RepeatedBind node, recursively extracting from source");
                    return self.extract_symbol_name(&**source, position);
                }
                RholangNode::Send { channel, .. } => {
                    // For Send nodes (e.g., contract!(...), name!?(msg), channel!!(data)),
                    // recursively extract from the channel
                    debug!("Found Send node, recursively extracting from channel");
                    return self.extract_symbol_name(&**channel, position);
                }
                RholangNode::SendSync { channel, .. } => {
                    // For SendSync nodes (e.g., contract!(args); continuation),
                    // recursively extract from the channel
                    debug!("Found SendSync node, recursively extracting from channel");
                    return self.extract_symbol_name(&**channel, position);
                }
                RholangNode::SendReceiveSource { name, .. } => {
                    // For SendReceiveSource nodes (peek send, e.g., channel!?(args)),
                    // recursively extract from the name
                    debug!("Found SendReceiveSource node, recursively extracting from name");
                    return self.extract_symbol_name(&**name, position);
                }
                RholangNode::Block { proc, .. } => {
                    // For Block nodes (e.g., { x!() }), recursively extract from the inner proc
                    debug!("Found Block node, recursively extracting from inner proc");
                    return self.extract_symbol_name(&**proc, position);
                }
                RholangNode::Par { processes: Some(procs), .. } => {
                    // For Par nodes, recursively extract from the first process
                    // This handles cases like tuple/list usages where variables are wrapped in Par
                    debug!("Found Par node with {} processes", procs.len());
                    if procs.len() >= 1 {
                        if let Some(first_proc) = procs.get(0) {
                            debug!("Recursively extracting from first process in Par");
                            return self.extract_symbol_name(&**first_proc, position);
                        }
                    }
                }
                RholangNode::Par { left: Some(left), right: Some(right), base, .. } => {
                    // Binary Par node - use position-aware traversal to determine which child
                    debug!("Found binary Par node, using position-aware traversal");

                    let left_start = left.base().start();
                    let left_end = left.base().end();
                    let right_start = right.base().start();
                    let right_end = right.base().end();

                    debug!("  Left: start=({}, {}), end=({}, {})", left_start.row, left_start.column, left_end.row, left_end.column);
                    debug!("  Right: start=({}, {}), end=({}, {})", right_start.row, right_start.column, right_end.row, right_end.column);
                    debug!("  Target: ({}, {})", position.row, position.column);

                    // Check if position is in left child
                    if Self::position_in_range(position, &left_start, &left_end) {
                        debug!("  Position is in left child, recursing");
                        return self.extract_symbol_name(&**left, position);
                    }

                    // Check if position is in right child
                    if Self::position_in_range(position, &right_start, &right_end) {
                        debug!("  Position is in right child, recursing");
                        return self.extract_symbol_name(&**right, position);
                    }

                    // Fallback: position doesn't match either child, try both
                    debug!("  Position not in either child's range, trying left then right");
                    if let Some(name) = self.extract_symbol_name(&**left, position) {
                        return Some(name);
                    }
                    return self.extract_symbol_name(&**right, position);
                }
                RholangNode::Par { left: Some(left), .. } => {
                    // Par node with only left child
                    debug!("Found Par node with only left child");
                    return self.extract_symbol_name(&**left, position);
                }
                RholangNode::Par { right: Some(right), .. } => {
                    // Par node with only right child
                    debug!("Found Par node with only right child");
                    return self.extract_symbol_name(&**right, position);
                }
                RholangNode::Tuple { elements, base, .. } if elements.len() > 0 => {
                    // For Tuple nodes, use position-aware traversal to find the correct element
                    debug!("Tuple with {} elements: using position-aware traversal", elements.len());

                    // Try to find element containing the position
                    for (i, elem) in elements.iter().enumerate() {
                        let elem_start = elem.base().start();
                        let elem_end = elem.base().end();

                        // Check if position is within this element's span
                        if Self::position_in_range(position, &elem_start, &elem_end) {
                            debug!("  Position is in tuple element[{}], recursing", i);
                            return self.extract_symbol_name(&**elem, position);
                        }
                    }

                    debug!("Position not found in any tuple element, using fallback");
                    // Fallback: try each element if position comparison failed
                    for (i, elem) in elements.iter().enumerate() {
                        if let Some(symbol) = self.extract_symbol_name(&**elem, position) {
                            debug!("  Fallback found symbol '{}' in tuple element[{}]", symbol, i);
                            return Some(symbol);
                        }
                    }
                }
                RholangNode::Map { pairs, base, .. } if pairs.len() > 0 => {
                    // For Map nodes, use position-aware traversal to find the correct pair element
                    debug!("Map with {} pairs: using position-aware traversal", pairs.len());

                    // Try to find key/value containing the position
                    for (i, (key, value)) in pairs.iter().enumerate() {
                        let key_start = key.base().start();
                        let key_end = key.base().end();
                        let value_start = value.base().start();
                        let value_end = value.base().end();

                        // Check if position is within the key
                        if Self::position_in_range(position, &key_start, &key_end) {
                            debug!("  Position is in map pair[{}] key, recursing", i);
                            return self.extract_symbol_name(&**key, position);
                        }

                        // Check if position is within the value
                        if Self::position_in_range(position, &value_start, &value_end) {
                            debug!("  Position is in map pair[{}] value, recursing", i);
                            return self.extract_symbol_name(&**value, position);
                        }
                    }

                    debug!("Position not found in any map pair, using fallback");
                    // Fallback: try each key and value if position comparison failed
                    for (i, (key, value)) in pairs.iter().enumerate() {
                        if let Some(symbol) = self.extract_symbol_name(&**key, position) {
                            debug!("  Fallback found symbol '{}' in map pair[{}] key", symbol, i);
                            return Some(symbol);
                        }
                        if let Some(symbol) = self.extract_symbol_name(&**value, position) {
                            debug!("  Fallback found symbol '{}' in map pair[{}] value", symbol, i);
                            return Some(symbol);
                        }
                    }
                }
                RholangNode::List { elements, base, .. } if elements.len() > 0 => {
                    // For List nodes, use position-aware traversal to find the correct element
                    debug!("List with {} elements: using position-aware traversal", elements.len());

                    // Try to find element containing the position
                    for (i, elem) in elements.iter().enumerate() {
                        let elem_start = elem.base().start();
                        let elem_end = elem.base().end();

                        // Check if position is within this element's span
                        if Self::position_in_range(position, &elem_start, &elem_end) {
                            debug!("  Position is in list element[{}], recursing", i);
                            return self.extract_symbol_name(&**elem, position);
                        }
                    }

                    debug!("Position not found in any list element, using fallback");
                    // Fallback: try each element if position comparison failed
                    for (i, elem) in elements.iter().enumerate() {
                        if let Some(symbol) = self.extract_symbol_name(&**elem, position) {
                            debug!("  Fallback found symbol '{}' in list element[{}]", symbol, i);
                            return Some(symbol);
                        }
                    }
                }
                RholangNode::Par { processes: None, left: None, right: None, .. } => {
                    debug!("Found empty Par node (no processes, no left, no right)");
                }
                _ => {
                    // Log unhandled RholangNode types for debugging
                    use crate::ir::rholang_node::RholangNode;
                    match rho_node {
                        RholangNode::Par { .. } => {
                            debug!("Par node didn't match any of our patterns - this shouldn't happen!");
                        }
                        _ => {}
                    }
                }
            }
        }

        // Try to downcast to MettaNode (for virtual documents)
        if let Some(metta_node) = node.as_any().downcast_ref::<crate::ir::metta_node::MettaNode>() {
            use crate::ir::metta_node::MettaNode;
            match metta_node {
                MettaNode::Atom { name, .. } => {
                    debug!("Extracted symbol name from MettaNode::Atom: {}", name);
                    return Some(name.as_str());
                }
                MettaNode::Variable { name, .. } => {
                    debug!("Extracted symbol name from MettaNode::Variable: {}", name);
                    return Some(name.as_str());
                }
                _ => {}
            }
        }

        // Try to extract from RholangNode again for StringLiteral special case
        // (quoted contract invocations: @"contractName"!(...))
        if let Some(rholang_node) = node.as_any().downcast_ref::<crate::ir::rholang_node::RholangNode>() {
            use crate::ir::rholang_node::RholangNode;
            if let RholangNode::StringLiteral { value, .. } = rholang_node {
                debug!("Extracted contract name from StringLiteral: {}", value);
                return Some(value.as_str());
            }
        }

        debug!(
            "Could not extract symbol name from node type={}, category={:?}",
            node.type_name(),
            node.semantic_category()
        );
        None
    }

    /// Convert SymbolLocations to LSP GotoDefinitionResponse
    fn symbol_locations_to_response(
        &self,
        mut locations: Vec<SymbolLocation>,
    ) -> GotoDefinitionResponse {
        // Sort by confidence (highest first)
        locations.sort_by(|a, b| b.confidence.cmp(&a.confidence));

        // Convert to LSP Locations
        let lsp_locations: Vec<Location> = locations
            .into_iter()
            .map(|sym_loc| Location {
                uri: sym_loc.uri,
                range: sym_loc.range,
            })
            .collect();

        // Return single location or array based on count
        if lsp_locations.len() == 1 {
            GotoDefinitionResponse::Scalar(lsp_locations.into_iter().next().unwrap())
        } else {
            GotoDefinitionResponse::Array(lsp_locations)
        }
    }

    /// Helper: Try goto-definition one character to the left
    ///
    /// This handles the common IDE pattern where the cursor is positioned
    /// at the right edge of a symbol (after the last character).
    pub async fn goto_definition_with_fallback(
        &self,
        root: &dyn SemanticNode,
        position: &Position,
        uri: &Url,
        adapter: &LanguageAdapter,
    ) -> Option<GotoDefinitionResponse> {
        // Try at the requested position
        if let Some(response) = self.goto_definition(root, position, uri, adapter).await {
            return Some(response);
        }

        // Try one column to the left (right word boundary)
        if position.column > 0 {
            debug!("No definition found, trying one column left for right word boundary");
            let left_pos = Position {
                row: position.row,
                column: position.column - 1,
                byte: position.byte.saturating_sub(1),
            };
            self.goto_definition(root, &left_pos, uri, adapter).await
        } else {
            None
        }
    }

    /// Recursively search for a Var node within the given node tree.
    /// This is useful for finding the actual Var node when the cursor is on a LinearBind or other wrapper.
    fn find_var_node_in_tree<'a>(&self, node: &'a dyn SemanticNode) -> Option<&'a dyn SemanticNode> {
        use crate::ir::rholang_node::RholangNode;

        // If this is already a Var node with referenced_symbol, return it
        if let Some(metadata) = node.metadata() {
            if metadata.contains_key("referenced_symbol") {
                debug!("Found Var node with referenced_symbol");
                return Some(node);
            }
        }

        // Otherwise, recursively search children
        if let Some(rholang_node) = node.as_any().downcast_ref::<RholangNode>() {
            match rholang_node {
                RholangNode::LinearBind { source, .. } => {
                    return self.find_var_node_in_tree(&**source);
                }
                RholangNode::RepeatedBind { source, .. } => {
                    return self.find_var_node_in_tree(&**source);
                }
                RholangNode::Block { proc, .. } => {
                    return self.find_var_node_in_tree(&**proc);
                }
                RholangNode::Send { channel, .. } => {
                    debug!("find_var_node_in_tree: Recursing into Send channel");
                    return self.find_var_node_in_tree(&**channel);
                }
                RholangNode::SendSync { channel, .. } => {
                    debug!("find_var_node_in_tree: Recursing into SendSync channel");
                    return self.find_var_node_in_tree(&**channel);
                }
                RholangNode::SendReceiveSource { name, .. } => {
                    debug!("find_var_node_in_tree: Recursing into SendReceiveSource name");
                    return self.find_var_node_in_tree(&**name);
                }
                _ => {}
            }
        }

        None
    }

    /// Find the containing Send node for a given position
    ///
    /// This is used for pattern-aware contract resolution. When clicking on a contract name
    /// in an invocation like `echo!("hello")`, we need to find the Send node that contains
    /// both the contract name and its arguments.
    ///
    /// # Arguments
    /// * `root` - The root node to search from
    /// * `position` - The position to search at
    ///
    /// # Returns
    /// The Send node if found, None otherwise
    fn find_containing_send_node(
        &self,
        root: &dyn SemanticNode,
        position: &Position,
    ) -> Option<crate::ir::rholang_node::RholangNode> {
        use crate::ir::rholang_node::RholangNode;
        use crate::lsp::features::node_finder::find_node_at_position;

        // Strategy: Search from root to find Send nodes that contain this position
        // We need to traverse the tree and check each Send node
        self.find_send_recursive(root, position)
    }

    /// Recursive helper to find Send node containing position
    fn find_send_recursive(
        &self,
        node: &dyn SemanticNode,
        position: &Position,
    ) -> Option<crate::ir::rholang_node::RholangNode> {
        use crate::ir::rholang_node::RholangNode;

        // Check if this node contains the position by using base positions
        let node_base = node.base();
        let node_start = node_base.start();
        let node_end = node_base.end();

        if !Self::position_in_range(position, &node_start, &node_end) {
            return None;
        }

        // If this is a Send node, check if position is in the channel (contract name)
        if let Some(rholang_node) = node.as_any().downcast_ref::<RholangNode>() {
            if let RholangNode::Send { channel, .. } = rholang_node {
                // Check if position is within the channel part using base positions
                let chan_base = channel.base();
                let chan_start = chan_base.start();
                let chan_end = chan_base.end();

                if Self::position_in_range(position, &chan_start, &chan_end) {
                    // Position is in the channel name - return this Send node
                    debug!("Found containing Send node for position {:?}", position);
                    return Some(rholang_node.clone());
                }
            }

            // Recursively search children
            match rholang_node {
                RholangNode::Par { left: Some(left), right: Some(right), .. } => {
                    if let Some(send) = self.find_send_recursive(&**left, position) {
                        return Some(send);
                    }
                    return self.find_send_recursive(&**right, position);
                }
                RholangNode::New { proc, .. } => {
                    return self.find_send_recursive(&**proc, position);
                }
                RholangNode::Block { proc, .. } => {
                    return self.find_send_recursive(&**proc, position);
                }
                _ => {}
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::Any;
    use std::collections::HashMap;
    use crate::ir::semantic_node::{NodeBase, Position, Metadata};
    use crate::ir::symbol_resolution::{ResolutionConfidence, SymbolKind, SymbolResolver};

    // Mock node with symbol name in metadata
    #[derive(Debug)]
    struct MockSymbolNode {
        base: NodeBase,
        category: SemanticCategory,
        metadata: Metadata,
    }

    impl MockSymbolNode {
        fn new_with_name(name: String, category: SemanticCategory) -> Self {
            let name_len = name.len();
            let mut metadata = HashMap::new();
            metadata.insert("symbol_name".to_string(), Arc::new(name) as Arc<dyn Any + Send + Sync>);

            Self {
                base: NodeBase::new_simple(
                    Position { row: 0, column: 0, byte: 0 },
                    name_len,
                    0,
                    name_len,
                ),
                category,
                metadata,
            }
        }
    }

    impl SemanticNode for MockSymbolNode {
        fn base(&self) -> &NodeBase {
            &self.base
        }

        fn metadata(&self) -> Option<&Metadata> {
            Some(&self.metadata)
        }

        fn metadata_mut(&mut self) -> Option<&mut Metadata> {
            Some(&mut self.metadata)
        }

        fn semantic_category(&self) -> SemanticCategory {
            self.category
        }

        fn type_name(&self) -> &'static str {
            "MockSymbolNode"
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    // Mock resolver that returns a fixed location
    struct MockResolver {
        return_location: bool,
    }

    impl SymbolResolver for MockResolver {
        fn resolve_symbol(
            &self,
            _symbol_name: &str,
            _position: &Position,
            _context: &ResolutionContext,
        ) -> Vec<SymbolLocation> {
            if self.return_location {
                vec![SymbolLocation {
                    uri: Url::parse("file:///test.rho").unwrap(),
                    range: Range {
                        start: LspPosition { line: 0, character: 0 },
                        end: LspPosition { line: 0, character: 10 },
                    },
                    kind: SymbolKind::Variable,
                    confidence: ResolutionConfidence::Exact,
                    metadata: None,
                }]
            } else {
                vec![]
            }
        }

        fn supports_language(&self, _language: &str) -> bool {
            true
        }

        fn name(&self) -> &'static str {
            "MockResolver"
        }
    }

    #[tokio::test]
    async fn test_goto_definition_basic() {
        use crate::lsp::features::traits::{HoverProvider, CompletionProvider, DocumentationProvider};

        // Mock providers (minimal implementations)
        struct MockHover;
        impl HoverProvider for MockHover {
            fn hover_for_symbol(&self, _: &str, _: &dyn SemanticNode, _: &crate::lsp::features::traits::HoverContext) -> Option<tower_lsp::lsp_types::HoverContents> {
                None
            }
        }

        struct MockCompletion;
        impl CompletionProvider for MockCompletion {
            fn complete_at(&self, _: &dyn SemanticNode, _: &crate::lsp::features::traits::CompletionContext) -> Vec<tower_lsp::lsp_types::CompletionItem> {
                vec![]
            }
            fn keywords(&self) -> &[&str] {
                &[]
            }
        }

        struct MockDoc;
        impl DocumentationProvider for MockDoc {
            fn documentation_for(&self, _: &str, _: &crate::lsp::features::traits::DocumentationContext) -> Option<tower_lsp::lsp_types::Documentation> {
                None
            }
        }

        // Create adapter with mock resolver that returns a location
        let adapter = LanguageAdapter::new(
            "test",
            Arc::new(MockResolver { return_location: true }),
            Arc::new(MockHover),
            Arc::new(MockCompletion),
            Arc::new(MockDoc),
        );

        let goto_def = GenericGotoDefinition;
        let node = MockSymbolNode::new_with_name("test_var".to_string(), SemanticCategory::Variable);
        let position = Position { row: 0, column: 5, byte: 5 };
        let uri = Url::parse("file:///test.rho").unwrap();

        let result = goto_def.goto_definition(&node, &position, &uri, &adapter).await;

        assert!(result.is_some());
        match result.unwrap() {
            GotoDefinitionResponse::Scalar(loc) => {
                assert_eq!(loc.uri.as_str(), "file:///test.rho");
            }
            _ => panic!("Expected scalar response"),
        }
    }

    #[tokio::test]
    async fn test_goto_definition_no_result() {
        use crate::lsp::features::traits::{HoverProvider, CompletionProvider, DocumentationProvider};

        struct MockHover;
        impl HoverProvider for MockHover {
            fn hover_for_symbol(&self, _: &str, _: &dyn SemanticNode, _: &crate::lsp::features::traits::HoverContext) -> Option<tower_lsp::lsp_types::HoverContents> {
                None
            }
        }

        struct MockCompletion;
        impl CompletionProvider for MockCompletion {
            fn complete_at(&self, _: &dyn SemanticNode, _: &crate::lsp::features::traits::CompletionContext) -> Vec<tower_lsp::lsp_types::CompletionItem> {
                vec![]
            }
            fn keywords(&self) -> &[&str] {
                &[]
            }
        }

        struct MockDoc;
        impl DocumentationProvider for MockDoc {
            fn documentation_for(&self, _: &str, _: &crate::lsp::features::traits::DocumentationContext) -> Option<tower_lsp::lsp_types::Documentation> {
                None
            }
        }

        // Create adapter with mock resolver that returns no locations
        let adapter = LanguageAdapter::new(
            "test",
            Arc::new(MockResolver { return_location: false }),
            Arc::new(MockHover),
            Arc::new(MockCompletion),
            Arc::new(MockDoc),
        );

        let goto_def = GenericGotoDefinition;
        let node = MockSymbolNode::new_with_name("unknown_var".to_string(), SemanticCategory::Variable);
        let position = Position { row: 0, column: 5, byte: 5 };
        let uri = Url::parse("file:///test.rho").unwrap();

        let result = goto_def.goto_definition(&node, &position, &uri, &adapter).await;

        assert!(result.is_none());
    }
}
