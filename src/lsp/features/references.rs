//! Generic find-references implementation
//!
//! Provides language-agnostic reference finding using the SymbolResolver trait.

use std::sync::Arc;
use tower_lsp::lsp_types::{Location, Position as LspPosition, Range, Url};
use tracing::debug;

use crate::ir::semantic_node::{Position, SemanticNode};
use crate::ir::symbol_resolution::ResolutionContext;
use crate::lsp::features::node_finder::{find_node_at_position, ir_to_lsp_position};
use crate::lsp::features::traits::LanguageAdapter;
use crate::lsp::rholang_contracts::RholangContracts;

/// Generic find-references feature
pub struct GenericReferences;

impl GenericReferences {
    /// Find the nearest ancestor node that has a symbol_table in its metadata
    ///
    /// Walks down the tree from root following the position path, tracking the last node
    /// that had symbol_table metadata. This gives us the correct scope for symbol lookup.
    fn find_ancestor_symbol_table(root: &dyn SemanticNode, target_pos: &Position) -> Option<Arc<crate::ir::symbol_table::SymbolTable>> {
        Self::find_ancestor_table_recursive(root, target_pos, &Position { row: 0, column: 0, byte: 0 })
    }

    /// Recursive helper for find_ancestor_symbol_table
    fn find_ancestor_table_recursive(
        node: &dyn SemanticNode,
        target: &Position,
        prev_end: &Position,
    ) -> Option<Arc<crate::ir::symbol_table::SymbolTable>> {
        use super::node_finder::position_in_range;

        let start = node.base().start();
        let end = node.base().end();

        // Check if target position is within this node's span
        if !position_in_range(target, &start, &end) {
            return None;
        }

        // Check if this node has a symbol table - remember it
        let current_table = node.metadata()
            .and_then(|m| m.get("symbol_table"))
            .and_then(|st| st.downcast_ref::<Arc<crate::ir::symbol_table::SymbolTable>>())
            .cloned();

        // Recursively search children
        for i in 0..node.children_count() {
            if let Some(child) = node.child_at(i) {
                let child_start = child.base().start();
                if let Some(child_table) = Self::find_ancestor_table_recursive(child, target, &start) {
                    // Child or descendant has a symbol table - use it (it's more specific)
                    return Some(child_table);
                }
            }
        }

        // No child had a symbol table, use ours if we have one
        current_table
    }

    /// Resolve the position for symbol lookup when dealing with quoted parameters
    ///
    /// When a user clicks inside a quoted parameter like `@fromRoom`, the node finder
    /// might return the Var node (for `fromRoom`) instead of the Quote node (for `@fromRoom`).
    /// However, the symbol table stores the position of the Quote node.
    ///
    /// This function walks up the tree to find if there's a Quote parent at the same position,
    /// and if so, returns the Quote's position for symbol lookup.
    ///
    /// # Arguments
    /// * `root` - Root node of the semantic tree
    /// * `node` - The node found at the position (might be Var inside Quote)
    /// * `position` - The original position where the user clicked
    ///
    /// # Returns
    /// The position to use for symbol table lookup (Quote position if applicable, else original)
    fn resolve_symbol_position(
        root: &dyn SemanticNode,
        node: &dyn SemanticNode,
        position: &Position,
    ) -> Position {
        use crate::ir::rholang_node::RholangNode;

        // Check if this is a Var node
        if let Some(rholang_node) = node.as_any().downcast_ref::<RholangNode>() {
            if let RholangNode::Var { .. } = rholang_node {
                // This is a Var - check if there's a Quote parent
                if let Some(quote_pos) = Self::find_quote_parent_position(root, position) {
                    debug!(
                        "Resolved Var position {:?} to Quote position {:?}",
                        position, quote_pos
                    );
                    return quote_pos;
                }
            }
        }

        // Not a Var, or no Quote parent - use original position
        *position
    }

    /// Find if there's a Quote node that contains the given position
    ///
    /// This is a helper for `resolve_symbol_position` that walks the tree to find
    /// a Quote node whose child (a Var) is at the target position.
    fn find_quote_parent_position(root: &dyn SemanticNode, target: &Position) -> Option<Position> {
        Self::find_quote_parent_recursive(root, target, &Position { row: 0, column: 0, byte: 0 })
    }

    /// Recursive helper for find_quote_parent_position
    fn find_quote_parent_recursive(
        node: &dyn SemanticNode,
        target: &Position,
        prev_end: &Position,
    ) -> Option<Position> {
        use super::node_finder::position_in_range;
        use crate::ir::rholang_node::RholangNode;

        let start = node.base().start();
        let end = node.base().end();

        // Check if target position is within this node's span
        if !position_in_range(target, &start, &end) {
            return None;
        }

        // Check if this is a Quote node
        if let Some(rholang_node) = node.as_any().downcast_ref::<RholangNode>() {
            if let RholangNode::Quote { quotable, .. } = rholang_node {
                // Check if the quotable child (Var or StringLiteral) contains our target
                if let Some(child_node) = quotable.as_ref().as_any().downcast_ref::<RholangNode>() {
                    match child_node {
                        RholangNode::Var { .. } | RholangNode::StringLiteral { .. } => {
                            let child_start = quotable.base().start();
                            let child_end = quotable.base().end();

                            // If target is inside the Var/StringLiteral, return Quote's position
                            if position_in_range(target, &child_start, &child_end) {
                                return Some(start); // Return Quote's position
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        // Recursively search children
        for i in 0..node.children_count() {
            if let Some(child) = node.child_at(i) {
                if let Some(quote_pos) = Self::find_quote_parent_recursive(child, target, &start) {
                    return Some(quote_pos);
                }
            }
        }

        None
    }

    /// Find all references to a symbol at the given position
    ///
    /// # Arguments
    /// * `root` - Root node of the semantic tree
    /// * `position` - Position where find-references was requested
    /// * `uri` - URI of the document
    /// * `adapter` - Language adapter
    /// * `include_declaration` - Whether to include the declaration in results
    /// * `symbol_table` - Per-document symbol table for local variable resolution
    /// * `inverted_index` - Per-document inverted index for local references
    /// * `rholang_symbols` - Global contract storage for cross-document resolution
    ///
    /// # Returns
    /// `Some(Vec<Location>)` with all reference locations, or `None` if symbol not found
    pub async fn find_references(
        &self,
        root: &dyn SemanticNode,
        position: &Position,
        uri: &Url,
        adapter: &LanguageAdapter,
        include_declaration: bool,
        symbol_table: &Arc<crate::ir::symbol_table::SymbolTable>,
        inverted_index: &crate::ir::transforms::symbol_table_builder::InvertedIndex,
        rholang_symbols: &Arc<RholangContracts>,
    ) -> Option<Vec<Location>> {
        debug!(
            "GenericReferences::find_references at {:?} in {} (include_decl: {})",
            position, uri, include_declaration
        );

        // Find node at position
        let mut node = find_node_at_position(root, position)?;

        // Special handling for Par nodes: if we get a Par node, look at its first child
        // This handles cases where position tracking doesn't drill down into Par children
        use crate::ir::rholang_node::RholangNode;
        if let Some(rholang_node) = node.as_any().downcast_ref::<RholangNode>() {
            if let RholangNode::Par { processes, left, .. } = rholang_node {
                debug!("Found Par node, checking first child");
                // Try n-ary Par first
                if let Some(procs) = processes {
                    if let Some(first_child) = procs.first() {
                        node = first_child.as_ref();
                        debug!("Using first child from n-ary Par");
                    }
                }
                // Try binary Par
                else if let Some(left_child) = left {
                    node = left_child.as_ref();
                    debug!("Using left child from binary Par");
                }
            }
        }

        // Resolve the position for symbol lookup
        // If this is a Var inside a Quote (e.g., user clicked inside @fromRoom),
        // use the Quote's position since that's where the symbol is stored
        let lookup_position = Self::resolve_symbol_position(root, node, position);

        // Extract symbol name
        let symbol_name = self.extract_symbol_name(node)?;
        debug!("Finding references for symbol '{}'", symbol_name);

        // Try to get the scope-specific symbol table from the node's metadata
        // If the node doesn't have one, find the nearest ancestor that does by walking the tree
        let scope_table = node.metadata()
            .and_then(|m| m.get("symbol_table"))
            .and_then(|st| st.downcast_ref::<Arc<crate::ir::symbol_table::SymbolTable>>())
            .cloned()
            .or_else(|| {
                // Node doesn't have symbol table, search for nearest ancestor with one
                Self::find_ancestor_symbol_table(root, &lookup_position)
            })
            .unwrap_or_else(|| symbol_table.clone());

        // Two-tier resolution: Check contracts first, then local variables

        // Tier 1: Try to find as a global contract
        if let Some(contract) = rholang_symbols.lookup(symbol_name) {
            debug!("Found contract '{}' with {} reference(s)", symbol_name, contract.references.len());

            let mut locations: Vec<Location> = Vec::new();

            // Include declaration if requested
            if include_declaration {
                let decl_lsp_pos = ir_to_lsp_position(&contract.declaration.position);
                locations.push(Location {
                    uri: contract.declaration.uri.clone(),
                    range: Range {
                        start: decl_lsp_pos,
                        end: LspPosition {
                            line: decl_lsp_pos.line,
                            character: decl_lsp_pos.character + symbol_name.len() as u32,
                        },
                    },
                });
            }

            // Always include definition if it's different from declaration
            // (definition is considered a "reference" to the declared symbol)
            if let Some(ref definition) = contract.definition {
                if definition.position != contract.declaration.position {
                    let def_lsp_pos = ir_to_lsp_position(&definition.position);
                    locations.push(Location {
                        uri: definition.uri.clone(),
                        range: Range {
                            start: def_lsp_pos,
                            end: LspPosition {
                                line: def_lsp_pos.line,
                                character: def_lsp_pos.character + symbol_name.len() as u32,
                            },
                        },
                    });
                }
            }

            // Add all contract references
            for ref_loc in &contract.references {
                let ref_lsp_pos = ir_to_lsp_position(&ref_loc.position);
                locations.push(Location {
                    uri: ref_loc.uri.clone(),
                    range: Range {
                        start: ref_lsp_pos,
                        end: LspPosition {
                            line: ref_lsp_pos.line,
                            character: ref_lsp_pos.character + symbol_name.len() as u32,
                        },
                    },
                });
            }

            return if locations.is_empty() { None } else { Some(locations) };
        }

        // Tier 2: Look up as local variable in symbol table
        if let Some(symbol) = scope_table.lookup(symbol_name) {
            debug!("Found local variable '{}' declared at {:?}", symbol_name, symbol.declaration_location);

            let mut locations: Vec<Location> = Vec::new();

            // Include declaration if requested
            if include_declaration {
                let decl_lsp_pos = ir_to_lsp_position(&symbol.declaration_location);
                locations.push(Location {
                    uri: symbol.declaration_uri.clone(),
                    range: Range {
                        start: decl_lsp_pos,
                        end: LspPosition {
                            line: decl_lsp_pos.line,
                            character: decl_lsp_pos.character + symbol_name.len() as u32,
                        },
                    },
                });
            }

            // Look up references in inverted_index
            if let Some(refs) = inverted_index.get(&symbol.declaration_location) {
                debug!("Found {} reference(s) in inverted_index", refs.len());
                for ref_pos in refs {
                    let ref_lsp_pos = ir_to_lsp_position(ref_pos);
                    locations.push(Location {
                        uri: uri.clone(),
                        range: Range {
                            start: ref_lsp_pos,
                            end: LspPosition {
                                line: ref_lsp_pos.line,
                                character: ref_lsp_pos.character + symbol_name.len() as u32,
                            },
                        },
                    });
                }
            }

            return if locations.is_empty() { None } else { Some(locations) };
        }

        debug!("Symbol '{}' not found in contracts or local variables", symbol_name);
        None
    }

    /// Extract symbol name from node metadata or structure
    fn extract_symbol_name<'a>(&self, node: &'a dyn SemanticNode) -> Option<&'a str> {
        use crate::ir::rholang_node::RholangNode;

        // First try metadata keys (set by symbol table builder for references)
        if let Some(metadata) = node.metadata() {
            if let Some(name_any) = metadata.get("symbol_name") {
                if let Some(name_ref) = name_any.downcast_ref::<String>() {
                    return Some(name_ref.as_str());
                }
                if let Some(name_ref) = name_any.downcast_ref::<std::sync::Arc<String>>() {
                    return Some(name_ref.as_str());
                }
            }

            if let Some(name_any) = metadata.get("name") {
                if let Some(name_ref) = name_any.downcast_ref::<String>() {
                    return Some(name_ref.as_str());
                }
                if let Some(name_ref) = name_any.downcast_ref::<std::sync::Arc<String>>() {
                    return Some(name_ref.as_str());
                }
            }
        }

        // Then try to downcast to RholangNode and match on variants
        if let Some(rholang_node) = node.as_any().downcast_ref::<RholangNode>() {
            match rholang_node {
                // For NameDecl nodes, extract name from the Var child
                RholangNode::NameDecl { var, .. } => {
                    if let RholangNode::Var { name, .. } = var.as_ref() {
                        return Some(name.as_str());
                    }
                }
                // For Var nodes, extract name directly
                RholangNode::Var { name, .. } => {
                    return Some(name.as_str());
                }
                // For Send/SendSync nodes, extract from the channel (which should be a Var)
                RholangNode::Send { channel, .. } | RholangNode::SendSync { channel, .. } => {
                    if let RholangNode::Var { name, .. } = channel.as_ref() {
                        return Some(name.as_str());
                    }
                }
                // For StringLiteral nodes directly (quoted strings like @"init")
                RholangNode::StringLiteral { value, .. } => {
                    return Some(value.as_str());
                }
                // For Quote nodes (e.g., @fromRoom or @"string"), extract from the inner node
                RholangNode::Quote { quotable, .. } => {
                    match quotable.as_ref() {
                        // Quoted variable: @x
                        RholangNode::Var { name, .. } => {
                            return Some(name.as_str());
                        }
                        // Quoted string: @"foo"
                        RholangNode::StringLiteral { value, .. } => {
                            return Some(value.as_str());
                        }
                        _ => {}
                    }
                }
                // For LinearBind/RepeatedBind/PeekBind nodes (e.g., for (@x <- ch) or for (@x <= ch))
                RholangNode::LinearBind { names, .. }
                | RholangNode::RepeatedBind { names, .. }
                | RholangNode::PeekBind { names, .. } => {
                    // Extract from first name (most common case)
                    if let Some(first_name) = names.first() {
                        match &**first_name {
                            // Handle Quote wrapper (e.g., @fromRoom)
                            RholangNode::Quote { quotable, .. } => {
                                match quotable.as_ref() {
                                    RholangNode::Var { name, .. } => {
                                        return Some(name.as_str());
                                    }
                                    RholangNode::StringLiteral { value, .. } => {
                                        return Some(value.as_str());
                                    }
                                    _ => {}
                                }
                            }
                            // Direct Var (no Quote)
                            RholangNode::Var { name, .. } => {
                                return Some(name.as_str());
                            }
                            _ => {}
                        }
                    }
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
    use std::sync::Arc;
    use crate::ir::semantic_node::{NodeBase, Position, SemanticCategory, Metadata};
    use crate::ir::symbol_resolution::{SymbolResolver, SymbolLocation, ResolutionConfidence, SymbolKind};
    use crate::lsp::features::traits::{HoverProvider, CompletionProvider, DocumentationProvider, LanguageAdapter};

    #[derive(Debug)]
    struct MockNode {
        base: NodeBase,
        category: SemanticCategory,
        metadata: Metadata,
    }

    impl MockNode {
        fn new_with_name(name: String) -> Self {
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
                category: SemanticCategory::Variable,
                metadata,
            }
        }
    }

    impl SemanticNode for MockNode {
        fn base(&self) -> &NodeBase { &self.base }
        fn metadata(&self) -> Option<&Metadata> { Some(&self.metadata) }
        fn metadata_mut(&mut self) -> Option<&mut Metadata> { Some(&mut self.metadata) }
        fn semantic_category(&self) -> SemanticCategory { self.category }
        fn type_name(&self) -> &'static str { "MockNode" }
        fn as_any(&self) -> &dyn Any { self }
    }

    struct MockResolver { has_refs: bool }
    impl SymbolResolver for MockResolver {
        fn resolve_symbol(&self, _: &str, _: &Position, _: &ResolutionContext) -> Vec<SymbolLocation> {
            if self.has_refs {
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
        fn supports_language(&self, _: &str) -> bool { true }
        fn name(&self) -> &'static str { "MockResolver" }
    }

    struct MockHover;
    impl HoverProvider for MockHover {
        fn hover_for_symbol(&self, _: &str, _: &dyn SemanticNode, _: &crate::lsp::features::traits::HoverContext) -> Option<tower_lsp::lsp_types::HoverContents> { None }
    }

    struct MockCompletion;
    impl CompletionProvider for MockCompletion {
        fn complete_at(&self, _: &dyn SemanticNode, _: &crate::lsp::features::traits::CompletionContext) -> Vec<tower_lsp::lsp_types::CompletionItem> { vec![] }
        fn keywords(&self) -> &[&str] { &[] }
    }

    struct MockDoc;
    impl DocumentationProvider for MockDoc {
        fn documentation_for(&self, _: &str, _: &crate::lsp::features::traits::DocumentationContext) -> Option<tower_lsp::lsp_types::Documentation> { None }
    }

    #[tokio::test]
    async fn test_find_references_found() {
        use crate::ir::symbol_table::SymbolType;

        let adapter = LanguageAdapter::new(
            "test",
            Arc::new(MockResolver { has_refs: true }),
            Arc::new(MockHover),
            Arc::new(MockCompletion),
            Arc::new(MockDoc),
        );

        let refs = GenericReferences;
        let node = MockNode::new_with_name("test_var".to_string());
        let pos = Position { row: 0, column: 5, byte: 5 };
        let uri = Url::parse("file:///test.rho").unwrap();

        // Create mock symbol_table and inverted_index for local variables
        let symbol_table = Arc::new(crate::ir::symbol_table::SymbolTable::new(None));
        let mut inverted_index = std::collections::HashMap::new();

        // Insert a test symbol
        use crate::ir::symbol_table::Symbol;
        symbol_table.insert(Arc::new(Symbol::new(
            "test_var".to_string(),
            SymbolType::Variable,
            uri.clone(),
            pos.clone(),
        )));

        // Add a reference in inverted_index
        inverted_index.insert(pos.clone(), vec![Position { row: 5, column: 10, byte: 50 }]);

        // Empty rholang_symbols (no contracts)
        let rholang_symbols = Arc::new(RholangContracts::new());

        let result = refs.find_references(&node, &pos, &uri, &adapter, true, &symbol_table, &inverted_index, &rholang_symbols).await;

        assert!(result.is_some());
        let locs = result.unwrap();
        assert_eq!(locs.len(), 2); // Declaration + 1 reference from inverted_index
        assert_eq!(locs[0].uri.as_str(), "file:///test.rho");
    }

    #[tokio::test]
    async fn test_find_references_not_found() {
        let adapter = LanguageAdapter::new(
            "test",
            Arc::new(MockResolver { has_refs: false }),
            Arc::new(MockHover),
            Arc::new(MockCompletion),
            Arc::new(MockDoc),
        );

        let refs = GenericReferences;
        let node = MockNode::new_with_name("unknown_var".to_string());
        let pos = Position { row: 0, column: 5, byte: 5 };
        let uri = Url::parse("file:///test.rho").unwrap();

        // Create empty symbol_table, inverted_index, and rholang_symbols
        let symbol_table = Arc::new(crate::ir::symbol_table::SymbolTable::new(None));
        let inverted_index = std::collections::HashMap::new();
        let rholang_symbols = Arc::new(RholangContracts::new());

        let result = refs.find_references(&node, &pos, &uri, &adapter, true, &symbol_table, &inverted_index, &rholang_symbols).await;

        assert!(result.is_none());
    }
}
