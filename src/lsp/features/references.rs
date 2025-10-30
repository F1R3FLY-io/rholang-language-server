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
use crate::lsp::rholang_global_symbols::RholangGlobalSymbols;

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
    /// * `rholang_symbols` - Global Rholang symbol storage (Priority 2: replaces inverted_index)
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
        rholang_symbols: &Arc<RholangGlobalSymbols>,
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

        // Create resolution context
        let context = ResolutionContext {
            uri: uri.clone(),
            scope_id: None,
            ir_node: None,
            language: adapter.language_name().to_string(),
            parent_uri: None,
        };

        // Use resolver to find the definition
        let definitions = adapter.resolver.resolve_symbol(
            symbol_name,
            position,
            &context,
        );

        if definitions.is_empty() {
            debug!("No definition found for '{}'", symbol_name);
            return None;
        }

        debug!("Found {} definition(s) for '{}'", definitions.len(), symbol_name);

        // Priority 2: Look up symbol in rholang_symbols (replaces inverted_index lookup)
        let symbol_decl = rholang_symbols.lookup(symbol_name)?;

        debug!("Found symbol declaration for '{}' with {} reference(s)",
               symbol_name, symbol_decl.references.len());

        // Collect all reference locations
        let mut locations: Vec<Location> = Vec::new();

        // Include the declaration if requested
        if include_declaration {
            let decl_lsp_pos = ir_to_lsp_position(&symbol_decl.declaration.position);
            locations.push(Location {
                uri: symbol_decl.declaration.uri.clone(),
                range: Range {
                    start: decl_lsp_pos,
                    end: LspPosition {
                        line: decl_lsp_pos.line,
                        character: decl_lsp_pos.character + symbol_name.len() as u32,
                    },
                },
            });
        }

        // Add all references from rholang_symbols (replaces inverted_index lookup)
        for ref_loc in &symbol_decl.references {
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

        debug!("Found {} total reference location(s)", locations.len());

        if locations.is_empty() {
            None
        } else {
            Some(locations)
        }
    }

    /// Extract symbol name from node metadata or structure
    fn extract_symbol_name<'a>(&self, node: &'a dyn SemanticNode) -> Option<&'a str> {
        use crate::ir::rholang_node::RholangNode;

        // Special case: For NameDecl nodes, extract name from the Var child
        if node.type_name() == "Rholang::NameDecl" {
            if let Some(rholang_node) = node.as_any().downcast_ref::<RholangNode>() {
                if let RholangNode::NameDecl { var, .. } = rholang_node {
                    if let RholangNode::Var { name, .. } = var.as_ref() {
                        return Some(name.as_str());
                    }
                }
            }
        }

        // Special case: For Var nodes, extract name directly
        if node.type_name() == "Rholang::Var" {
            if let Some(rholang_node) = node.as_any().downcast_ref::<RholangNode>() {
                if let RholangNode::Var { name, .. } = rholang_node {
                    return Some(name.as_str());
                }
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

        // Create mock rholang_symbols with a test symbol
        let rholang_symbols = Arc::new(RholangGlobalSymbols::new());
        rholang_symbols.add_symbol(
            "test_var".to_string(),
            SymbolType::NewBind,
            uri.clone(),
            pos.clone(),
        );

        let result = refs.find_references(&node, &pos, &uri, &adapter, true, &rholang_symbols).await;

        assert!(result.is_some());
        let locs = result.unwrap();
        assert_eq!(locs.len(), 1); // Just the declaration
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

        // Create empty rholang_symbols
        let rholang_symbols = Arc::new(RholangGlobalSymbols::new());

        let result = refs.find_references(&node, &pos, &uri, &adapter, true, &rholang_symbols).await;

        assert!(result.is_none());
    }
}
