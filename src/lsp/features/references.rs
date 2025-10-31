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
        let node = find_node_at_position(root, position)?;

        // Extract symbol name
        let symbol_name = self.extract_symbol_name(node)?;
        debug!("Finding references for symbol '{}'", symbol_name);

        // Try to get the scope-specific symbol table from the node's metadata
        // This allows us to access variables in nested scopes (new, let, for, etc.)
        let scope_table = node.metadata()
            .and_then(|m| m.get("symbol_table"))
            .and_then(|st| st.downcast_ref::<Arc<crate::ir::symbol_table::SymbolTable>>())
            .cloned()
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

        // Try to downcast to RholangNode and match on variants
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
                // For Quote nodes (e.g., @fromRoom), extract from the inner node
                RholangNode::Quote { quotable, .. } => {
                    if let RholangNode::Var { name, .. } = quotable.as_ref() {
                        return Some(name.as_str());
                    }
                }
                _ => {}
            }
        }

        // Try metadata keys
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
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::Any;
    use std::collections::HashMap;
    use std::sync::Arc;
    use crate::ir::semantic_node::{NodeBase, RelativePosition, SemanticCategory, Metadata};
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
                    RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 },
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
