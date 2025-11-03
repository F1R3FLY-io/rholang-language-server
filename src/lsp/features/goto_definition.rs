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
        let symbol_name = match self.extract_symbol_name(node) {
            Some(name) => {
                debug!("GenericGotoDefinition: Extracted symbol name: '{}'", name);
                name
            }
            None => {
                debug!("GenericGotoDefinition: FAILED to extract symbol name from node type={}", node.type_name());
                return None;
            }
        };

        // Check if node has referenced_symbol metadata (set by SymbolTableBuilder)
        // If so, use it directly to get the definition location, bypassing resolver
        if let Some(metadata) = node.metadata() {
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
        let context = ResolutionContext {
            uri: uri.clone(),
            scope_id: None, // LexicalScopeResolver will extract from symbol table via position
            ir_node: None,  // Not needed - resolver uses position lookup
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

    /// Extract symbol name from a node
    ///
    /// This examines the node's type and metadata to extract a symbol name.
    /// The exact extraction logic depends on the semantic category.
    fn extract_symbol_name<'a>(&self, node: &'a dyn SemanticNode) -> Option<&'a str> {
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
                    // For Quote nodes (e.g., @fromRoom), extract from the inner node
                    if let RholangNode::Var { name, .. } = &**quotable {
                        debug!("Extracted symbol name from RholangNode::Quote->Var: {}", name);
                        return Some(name.as_str());
                    }
                }
                RholangNode::Block { proc, .. } => {
                    // For Block nodes (e.g., { x!() }), recursively extract from the inner proc
                    debug!("Found Block node, recursively extracting from inner proc");
                    return self.extract_symbol_name(&**proc);
                }
                RholangNode::Par { processes: Some(procs), .. } => {
                    // For Par nodes, recursively extract from the first process
                    // This handles cases like tuple/list usages where variables are wrapped in Par
                    debug!("Found Par node with {} processes", procs.len());
                    if procs.len() >= 1 {
                        if let Some(first_proc) = procs.get(0) {
                            debug!("Recursively extracting from first process in Par");
                            return self.extract_symbol_name(&**first_proc);
                        }
                    }
                }
                RholangNode::Par { left: Some(left), right: Some(right), .. } => {
                    // Binary Par node - try left first, then right
                    debug!("Found binary Par node, trying left child");
                    if let Some(name) = self.extract_symbol_name(&**left) {
                        return Some(name);
                    }
                    debug!("Left child didn't yield a symbol, trying right child");
                    return self.extract_symbol_name(&**right);
                }
                RholangNode::Par { left: Some(left), .. } => {
                    // Par node with only left child
                    debug!("Found Par node with only left child");
                    return self.extract_symbol_name(&**left);
                }
                RholangNode::Par { right: Some(right), .. } => {
                    // Par node with only right child
                    debug!("Found Par node with only right child");
                    return self.extract_symbol_name(&**right);
                }
                RholangNode::Tuple { elements, .. } if elements.len() > 0 => {
                    // For Tuple nodes, try to extract from each element
                    // This handles cases where find_node_at_position returns Tuple instead of descending to Var
                    // Note: This is a fallback - ideally find_node_at_position should return the Var directly
                    debug!("Tuple fallback: trying to extract symbol from tuple elements");

                    for (i, elem) in elements.iter().enumerate() {
                        // Try to extract from this element
                        if let Some(symbol) = self.extract_symbol_name(&**elem) {
                            debug!("  Found symbol '{}' in tuple element[{}]", symbol, i);
                            return Some(symbol);
                        }
                    }

                    debug!("No symbol found in any tuple element");
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::Any;
    use std::collections::HashMap;
    use crate::ir::semantic_node::{NodeBase, RelativePosition, Metadata};
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
                    RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 },
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
