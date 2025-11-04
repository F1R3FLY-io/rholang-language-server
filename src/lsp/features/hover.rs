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
use crate::lsp::features::node_finder::{find_node_at_position, find_node_with_path, ir_to_lsp_position};
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

        // Use pre-found node if provided, otherwise find it with parent context
        let (node, parent) = match pre_found_node {
            Some(n) => {
                debug!("Using pre-found node: type={}", n.type_name());
                (n, None) // No parent context for pre-found nodes
            }
            None => {
                match find_node_with_path(root, position) {
                    Some((n, path)) => {
                        // Extract parent (second-to-last in path, last is the node itself)
                        let parent = if path.len() >= 2 {
                            Some(path[path.len() - 2])
                        } else {
                            None
                        };
                        (n, parent)
                    }
                    None => {
                        debug!("find_node_with_path returned None for position {:?}", position);
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

        // Phase 7: Extract documentation from node or parent (now returns owned String with markdown)
        let documentation = self.extract_documentation(node, parent);
        if let Some(ref doc) = documentation {
            debug!("Found documentation for hover: {} chars", doc.len());
        }

        // Clone documentation for use in fallback case
        let doc_for_fallback = documentation.clone();

        // Create hover context
        let context = HoverContext {
            uri: uri.clone(),
            lsp_position,
            ir_position: *position,
            category,
            language: adapter.language_name().to_string(),
            parent_uri,
            documentation,
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
                // For other categories, check if we have documentation from parent context
                if let Some(ref doc) = doc_for_fallback {
                    debug!("Using documentation from parent for {} node", node.type_name());
                    use tower_lsp::lsp_types::{MarkupContent, MarkupKind};

                    // Try to extract symbol name from parent node
                    let formatted_content = if let Some(parent_node) = parent {
                        if let Some(symbol_name) = self.extract_symbol_name(parent_node) {
                            // Format with symbol name like RholangHoverProvider does
                            format!("**{}**\n\n{}\n\n---\n\n*Rholang symbol*", symbol_name, doc)
                        } else {
                            doc.clone()
                        }
                    } else {
                        doc.clone()
                    };

                    HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: formatted_content,
                    })
                } else {
                    debug!("No hover support for category {:?}", category);
                    return None;
                }
            }
        };

        // Compute hover range (the node's span)
        let start_pos = ir_to_lsp_position(position);
        let end = node.base().end();
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

    /// Extract documentation from node or parent metadata (Phase 7: with structured docs)
    ///
    /// Checks the node first, then falls back to parent if documentation not found.
    /// This handles cases where documentation is attached to parent declarations
    /// (e.g., Contract) but user hovers over child nodes (e.g., contract name Var).
    ///
    /// # Phase 7 Enhancement
    ///
    /// - Tries `StructuredDocumentation` first, returns markdown with rich formatting
    /// - Falls back to plain `String` for backwards compatibility
    /// - Returns owned String since markdown needs to be generated
    fn extract_documentation(
        &self,
        node: &dyn SemanticNode,
        parent: Option<&dyn SemanticNode>,
    ) -> Option<String> {
        use crate::ir::transforms::documentation_attacher::DOC_METADATA_KEY;
        use crate::ir::StructuredDocumentation;

        // Helper to extract from metadata
        let extract_from_metadata = |metadata: &std::collections::HashMap<String, std::sync::Arc<dyn std::any::Any + Send + Sync>>| -> Option<String> {
            if let Some(doc_any) = metadata.get(DOC_METADATA_KEY) {
                // Phase 7: Try StructuredDocumentation first (new format with rich display)
                if let Some(structured_doc) = doc_any.downcast_ref::<StructuredDocumentation>() {
                    debug!("Found StructuredDocumentation with {} params, {} examples",
                        structured_doc.params.len(), structured_doc.examples.len());
                    return Some(structured_doc.to_markdown());
                }
                // Backwards compatibility: Fall back to plain String
                else if let Some(doc_ref) = doc_any.downcast_ref::<String>() {
                    debug!("Found plain String documentation");
                    return Some(doc_ref.clone());
                }
            }
            None
        };

        // Try node first
        if let Some(metadata) = node.metadata() {
            if let Some(doc) = extract_from_metadata(metadata) {
                debug!("Found documentation in node: {}", node.type_name());
                return Some(doc);
            }
        }

        // Fall back to parent
        if let Some(parent_node) = parent {
            if let Some(metadata) = parent_node.metadata() {
                if let Some(doc) = extract_from_metadata(metadata) {
                    debug!(
                        "Found documentation in parent: {} (child was: {})",
                        parent_node.type_name(),
                        node.type_name()
                    );
                    return Some(doc);
                }
            }
        }

        None
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
                RholangNode::Contract { name: contract_name, .. } => {
                    // For Contract nodes, extract name from the name field
                    if let RholangNode::Var { name, .. } = &**contract_name {
                        debug!("Extracted symbol name from RholangNode::Contract: {}", name);
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
    use crate::ir::semantic_node::{NodeBase, Position, Metadata};
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
                    Position {
                        row: 0,
                        column: 0,
                        byte: 0,
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
                Position {
                    row: 0,
                    column: 0,
                    byte: 0,
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
