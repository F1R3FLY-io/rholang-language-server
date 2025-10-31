//! Generic hover implementation
//!
//! Provides language-agnostic hover tooltips using the HoverProvider trait.
//!
//! # Architecture
//!
//! ```text
//! User hovers over symbol
//!       ↓
//! GenericHover::hover()
//!       ├─→ Find node at position
//!       ├─→ Determine semantic category
//!       ├─→ Use LanguageAdapter.hover to get content
//!       └─→ Return hover response
//! ```

use tower_lsp::lsp_types::{Hover, HoverContents, Position as LspPosition, Range, Url};
use tracing::debug;

use crate::ir::semantic_node::{Position, SemanticCategory, SemanticNode};
use crate::lsp::features::node_finder::{find_node_at_position, ir_to_lsp_position};
use crate::lsp::features::traits::{HoverContext, LanguageAdapter};

/// Generic hover feature
///
/// Provides hover tooltips using language-specific HoverProvider implementations.
pub struct GenericHover;

impl GenericHover {
    /// Provide hover information at a given position
    ///
    /// # Arguments
    /// * `root` - Root node of the semantic tree
    /// * `position` - Position where hover was requested (IR coordinates)
    /// * `lsp_position` - Position in LSP coordinates (for context)
    /// * `uri` - URI of the document
    /// * `adapter` - Language adapter for this document's language
    /// * `parent_uri` - Optional parent URI for virtual documents
    ///
    /// # Returns
    /// `Some(Hover)` with hover information, or `None` if no hover available
    pub async fn hover(
        &self,
        root: &dyn SemanticNode,
        position: &Position,
        lsp_position: LspPosition,
        uri: &Url,
        adapter: &LanguageAdapter,
        parent_uri: Option<Url>,
    ) -> Option<Hover> {
        self.hover_with_node(None, root, position, lsp_position, uri, adapter, parent_uri).await
    }

    /// Provide hover information with an optional pre-found node
    ///
    /// # Arguments
    /// * `pre_found_node` - Optional node that was already found (for multi-root scenarios)
    /// * `root` - Root node of the semantic tree
    /// * `position` - Position where hover was requested (IR coordinates)
    /// * `lsp_position` - Position in LSP coordinates (for context)
    /// * `uri` - URI of the document
    /// * `adapter` - Language adapter for this document's language
    /// * `parent_uri` - Optional parent URI for virtual documents
    ///
    /// # Returns
    /// `Some(Hover)` with hover information, or `None` if no hover available
    pub async fn hover_with_node(
        &self,
        pre_found_node: Option<&dyn SemanticNode>,
        root: &dyn SemanticNode,
        position: &Position,
        lsp_position: LspPosition,
        uri: &Url,
        adapter: &LanguageAdapter,
        parent_uri: Option<Url>,
    ) -> Option<Hover> {
        debug!(
            "GenericHover::hover at {:?} in {} (language: {})",
            position,
            uri,
            adapter.language_name()
        );

        // Use pre-found node if provided, otherwise find it
        let node = match pre_found_node {
            Some(n) => {
                debug!("Using pre-found node: type={}", n.type_name());
                n
            }
            None => {
                match find_node_at_position(root, position) {
                    Some(n) => n,
                    None => {
                        debug!("find_node_at_position returned None for position {:?}", position);
                        return None;
                    }
                }
            }
        };
        let category = node.semantic_category();

        debug!(
            "Found node at position: type={}, category={:?}",
            node.type_name(),
            category
        );

        // Create hover context
        let context = HoverContext {
            uri: uri.clone(),
            lsp_position,
            ir_position: *position,
            category,
            language: adapter.language_name().to_string(),
            parent_uri,
        };

        // Get hover contents based on semantic category
        let contents = match category {
            SemanticCategory::Variable | SemanticCategory::Binding => {
                // Try to get symbol name from metadata
                if let Some(symbol_name) = self.extract_symbol_name(node) {
                    adapter.hover.hover_for_symbol(symbol_name, node, &context)?
                } else {
                    debug!("No symbol name found in node metadata");
                    return None;
                }
            }
            SemanticCategory::Invocation => {
                // For invocations, try to get the function name
                if let Some(symbol_name) = self.extract_symbol_name(node) {
                    adapter.hover.hover_for_symbol(symbol_name, node, &context)?
                } else {
                    return None;
                }
            }
            SemanticCategory::Literal => {
                adapter.hover.hover_for_literal(node, &context)?
            }
            SemanticCategory::LanguageSpecific => {
                adapter.hover.hover_for_language_specific(node, &context)?
            }
            _ => {
                debug!("No hover support for category {:?}", category);
                return None;
            }
        };

        // Compute hover range (the node's span)
        let start_pos = ir_to_lsp_position(position);
        let end = node.absolute_end(*position);
        let end_pos = ir_to_lsp_position(&end);

        let range = Range {
            start: start_pos,
            end: end_pos,
        };

        debug!("Returning hover for {} at {:?}", node.type_name(), range);

        Some(Hover {
            contents,
            range: Some(range),
        })
    }

    /// Extract symbol name from node metadata
    ///
    /// Same logic as GenericGotoDefinition - could be refactored into shared utility
    fn extract_symbol_name<'a>(&self, node: &'a dyn SemanticNode) -> Option<&'a str> {
        debug!("extract_symbol_name: node type={}", node.type_name());

        if let Some(metadata) = node.metadata() {
            debug!("extract_symbol_name: node has metadata with {} keys", metadata.len());
            // Check for "symbol_name" key
            if let Some(name_any) = metadata.get("symbol_name") {
                if let Some(name_ref) = name_any.downcast_ref::<String>() {
                    return Some(name_ref.as_str());
                }
                if let Some(name_ref) = name_any.downcast_ref::<std::sync::Arc<String>>() {
                    return Some(name_ref.as_str());
                }
            }

            // Check for "name" key
            if let Some(name_any) = metadata.get("name") {
                if let Some(name_ref) = name_any.downcast_ref::<String>() {
                    return Some(name_ref.as_str());
                }
                if let Some(name_ref) = name_any.downcast_ref::<std::sync::Arc<String>>() {
                    return Some(name_ref.as_str());
                }
            }
        }

        // Try to extract directly from node structure based on type
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
                _ => {}
            }
        }

        // Try to downcast to MettaNode
        if let Some(metta_node) = node.as_any().downcast_ref::<crate::ir::metta_node::MettaNode>() {
            debug!("extract_symbol_name: Successfully downcast to MettaNode");
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
                _ => {
                    debug!("extract_symbol_name: MettaNode variant not Atom or Variable");
                }
            }
        } else {
            debug!("extract_symbol_name: Failed to downcast to MettaNode");
        }

        debug!("extract_symbol_name: Returning None");
        None
    }

    /// Helper: Try hover one character to the left
    ///
    /// Handles cursor at right edge of symbol (IDE convention)
    pub async fn hover_with_fallback(
        &self,
        root: &dyn SemanticNode,
        position: &Position,
        lsp_position: LspPosition,
        uri: &Url,
        adapter: &LanguageAdapter,
        parent_uri: Option<Url>,
    ) -> Option<Hover> {
        // Try at the requested position
        if let Some(hover) = self
            .hover(root, position, lsp_position, uri, adapter, parent_uri.clone())
            .await
        {
            return Some(hover);
        }

        // Try one column to the left
        if position.column > 0 {
            debug!("No hover found, trying one column left for right word boundary");
            let left_pos = Position {
                row: position.row,
                column: position.column - 1,
                byte: position.byte.saturating_sub(1),
            };
            let left_lsp = LspPosition {
                line: lsp_position.line,
                character: lsp_position.character.saturating_sub(1),
            };
            self.hover(root, &left_pos, left_lsp, uri, adapter, parent_uri)
                .await
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
    use std::sync::Arc;
    use tower_lsp::lsp_types::{MarkupContent, MarkupKind};
    use crate::ir::semantic_node::{NodeBase, RelativePosition, Metadata};
    use crate::lsp::features::traits::{HoverProvider, CompletionProvider, DocumentationProvider};
    use crate::ir::symbol_resolution::SymbolResolver;

    // Mock node with symbol name
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
            metadata.insert(
                "symbol_name".to_string(),
                Arc::new(name) as Arc<dyn Any + Send + Sync>,
            );

            Self {
                base: NodeBase::new_simple(
                    RelativePosition {
                        delta_lines: 0,
                        delta_columns: 0,
                        delta_bytes: 0,
                    },
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

    // Mock hover provider
    struct MockHoverProvider;

    impl HoverProvider for MockHoverProvider {
        fn hover_for_symbol(
            &self,
            symbol_name: &str,
            _node: &dyn SemanticNode,
            _context: &HoverContext,
        ) -> Option<HoverContents> {
            Some(HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: format!("**{}** - Test hover", symbol_name),
            }))
        }
    }

    // Mock providers (minimal)
    struct MockCompletion;
    impl CompletionProvider for MockCompletion {
        fn complete_at(
            &self,
            _: &dyn SemanticNode,
            _: &crate::lsp::features::traits::CompletionContext,
        ) -> Vec<tower_lsp::lsp_types::CompletionItem> {
            vec![]
        }
        fn keywords(&self) -> &[&str] {
            &[]
        }
    }

    struct MockDoc;
    impl DocumentationProvider for MockDoc {
        fn documentation_for(
            &self,
            _: &str,
            _: &crate::lsp::features::traits::DocumentationContext,
        ) -> Option<tower_lsp::lsp_types::Documentation> {
            None
        }
    }

    struct MockResolver;
    impl SymbolResolver for MockResolver {
        fn resolve_symbol(
            &self,
            _: &str,
            _: &Position,
            _: &crate::ir::symbol_resolution::ResolutionContext,
        ) -> Vec<crate::ir::symbol_resolution::SymbolLocation> {
            vec![]
        }
        fn supports_language(&self, _: &str) -> bool {
            true
        }
        fn name(&self) -> &'static str {
            "MockResolver"
        }
    }

    #[tokio::test]
    async fn test_hover_variable() {
        let adapter = crate::lsp::features::traits::LanguageAdapter::new(
            "test",
            Arc::new(MockResolver),
            Arc::new(MockHoverProvider),
            Arc::new(MockCompletion),
            Arc::new(MockDoc),
        );

        let hover_feature = GenericHover;
        let node = MockSymbolNode::new_with_name("test_var".to_string(), SemanticCategory::Variable);
        let position = Position {
            row: 0,
            column: 5,
            byte: 5,
        };
        let lsp_pos = LspPosition {
            line: 0,
            character: 5,
        };
        let uri = Url::parse("file:///test.rho").unwrap();

        let result = hover_feature
            .hover(&node, &position, lsp_pos, &uri, &adapter, None)
            .await;

        assert!(result.is_some());
        let hover = result.unwrap();
        match hover.contents {
            HoverContents::Markup(content) => {
                assert!(content.value.contains("test_var"));
                assert!(content.value.contains("Test hover"));
            }
            _ => panic!("Expected markup content"),
        }
        assert!(hover.range.is_some());
    }

    #[tokio::test]
    async fn test_hover_no_symbol() {
        // Node without symbol name in metadata
        #[derive(Debug)]
        struct EmptyNode {
            base: NodeBase,
        }

        impl SemanticNode for EmptyNode {
            fn base(&self) -> &NodeBase {
                &self.base
            }
            fn metadata(&self) -> Option<&Metadata> {
                None
            }
            fn metadata_mut(&mut self) -> Option<&mut Metadata> {
                None
            }
            fn semantic_category(&self) -> SemanticCategory {
                SemanticCategory::Variable
            }
            fn type_name(&self) -> &'static str {
                "EmptyNode"
            }
            fn as_any(&self) -> &dyn Any {
                self
            }
        }

        let adapter = crate::lsp::features::traits::LanguageAdapter::new(
            "test",
            Arc::new(MockResolver),
            Arc::new(MockHoverProvider),
            Arc::new(MockCompletion),
            Arc::new(MockDoc),
        );

        let hover_feature = GenericHover;
        let node = EmptyNode {
            base: NodeBase::new_simple(
                RelativePosition {
                    delta_lines: 0,
                    delta_columns: 0,
                    delta_bytes: 0,
                },
                10,
                0,
                10,
            ),
        };
        let position = Position {
            row: 0,
            column: 5,
            byte: 5,
        };
        let lsp_pos = LspPosition {
            line: 0,
            character: 5,
        };
        let uri = Url::parse("file:///test.rho").unwrap();

        let result = hover_feature
            .hover(&node, &position, lsp_pos, &uri, &adapter, None)
            .await;

        assert!(result.is_none());
    }
}
