//! Generic rename implementation
//!
//! Provides language-agnostic symbol renaming using the LanguageAdapter pattern.
//! Works with any language that implements SemanticNode and provides a SymbolResolver.

use std::collections::HashMap;
use std::sync::Arc;
use dashmap::DashMap;
use tower_lsp::lsp_types::{TextEdit, Url, WorkspaceEdit, Position as LspPosition, Range};
use tracing::{debug, trace};

use super::traits::LanguageAdapter;
use super::references::GenericReferences;
use super::node_finder::find_node_at_position;
use crate::ir::semantic_node::{Position, SemanticNode};
use crate::ir::symbol_resolution::{ResolutionContext};

/// Generic rename implementation for any language
///
/// Uses GenericReferences to find all occurrences, then creates a WorkspaceEdit
/// to rename them all atomically.
pub struct GenericRename;

impl GenericRename {
    /// Rename a symbol at the given position
    ///
    /// # Arguments
    /// * `root` - Root of the semantic tree
    /// * `position` - Position of the symbol to rename
    /// * `uri` - URI of the document
    /// * `adapter` - Language-specific adapter
    /// * `new_name` - The new name for the symbol
    /// * `inverted_index` - Inverted index mapping definitions to usage sites
    ///
    /// # Returns
    /// WorkspaceEdit containing all the text edits needed to rename the symbol
    pub async fn rename(
        &self,
        root: &dyn SemanticNode,
        position: &Position,
        uri: &Url,
        adapter: &LanguageAdapter,
        new_name: &str,
        inverted_index: &Arc<DashMap<(Url, Position), Vec<(Url, Position)>>>,
    ) -> Option<WorkspaceEdit> {
        debug!(
            "Renaming symbol at {}:{} in {} to '{}'",
            position.row, position.column, uri, new_name
        );

        // Use GenericReferences to find all occurrences
        let references_finder = GenericReferences;
        let locations = references_finder
            .find_references(root, position, uri, adapter, true, inverted_index) // include_declaration = true
            .await?;

        if locations.is_empty() {
            debug!("No references found for rename");
            return None;
        }

        trace!("Found {} locations to rename", locations.len());

        // Group edits by document URI
        let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();

        for location in locations {
            let edit = TextEdit {
                range: location.range,
                new_text: new_name.to_string(),
            };

            changes
                .entry(location.uri)
                .or_insert_with(Vec::new)
                .push(edit);
        }

        debug!(
            "Created rename edits for {} document(s)",
            changes.len()
        );

        Some(WorkspaceEdit {
            changes: Some(changes),
            document_changes: None,
            change_annotations: None,
        })
    }

    /// Prepare rename - check if rename is valid at this position
    ///
    /// This is called before rename() to validate that the position contains
    /// a renameable symbol.
    ///
    /// # Arguments
    /// * `root` - Root of the semantic tree
    /// * `position` - Position to check
    /// * `uri` - URI of the document
    /// * `adapter` - Language-specific adapter
    /// * `inverted_index` - Inverted index mapping definitions to usage sites
    ///
    /// # Returns
    /// Option containing the range and placeholder text for the rename
    pub async fn prepare_rename(
        &self,
        root: &dyn SemanticNode,
        position: &Position,
        uri: &Url,
        adapter: &LanguageAdapter,
        inverted_index: &Arc<DashMap<(Url, Position), Vec<(Url, Position)>>>,
    ) -> Option<(Range, String)> {
        debug!(
            "Preparing rename at {}:{} in {}",
            position.row, position.column, uri
        );

        // Find the symbol at this position
        let references_finder = GenericReferences;
        let locations = references_finder
            .find_references(root, position, uri, adapter, false, inverted_index) // include_declaration = false
            .await?;

        if locations.is_empty() {
            debug!("No symbol found at position for prepare_rename");
            return None;
        }

        // Get the first location to determine the range and current name
        let first_location = &locations[0];

        // Extract current symbol name from the tree at this position
        let node = find_node_at_position(root, position)?;
        let symbol_name = self.extract_symbol_name(node)?;

        trace!(
            "Prepare rename: symbol '{}' at range {:?}",
            symbol_name,
            first_location.range
        );

        Some((first_location.range, symbol_name.to_string()))
    }

    /// Extract symbol name from a node or its structure
    ///
    /// Tries multiple metadata keys and node structure to find the symbol name.
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
            // Try symbol_name first
            if let Some(name_any) = metadata.get("symbol_name") {
                if let Some(name_ref) = name_any.downcast_ref::<String>() {
                    return Some(name_ref.as_str());
                }
                if let Some(name_ref) = name_any.downcast_ref::<Arc<String>>() {
                    return Some(name_ref.as_str());
                }
            }

            // Try name as fallback
            if let Some(name_any) = metadata.get("name") {
                if let Some(name_ref) = name_any.downcast_ref::<String>() {
                    return Some(name_ref.as_str());
                }
                if let Some(name_ref) = name_any.downcast_ref::<Arc<String>>() {
                    return Some(name_ref.as_str());
                }
            }
        }
        None
    }
}

// Note: find_node_at_position is now imported from node_finder module

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::Any;
    use std::collections::HashMap;
    use crate::ir::semantic_node::{NodeBase, RelativePosition, SemanticCategory, Metadata};
    use crate::ir::symbol_resolution::{SymbolResolver, SymbolLocation, ResolutionConfidence, SymbolKind};
    use crate::lsp::features::traits::{HoverProvider, CompletionProvider, DocumentationProvider};

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

    struct MockResolver { locations: Vec<SymbolLocation> }
    impl SymbolResolver for MockResolver {
        fn resolve_symbol(&self, _: &str, _: &Position, _: &ResolutionContext) -> Vec<SymbolLocation> {
            self.locations.clone()
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
    async fn test_rename_with_multiple_occurrences() {
        let uri = Url::parse("file:///test.rho").unwrap();

        let locations = vec![
            SymbolLocation {
                uri: uri.clone(),
                range: Range {
                    start: LspPosition { line: 0, character: 0 },
                    end: LspPosition { line: 0, character: 8 },
                },
                kind: SymbolKind::Variable,
                confidence: ResolutionConfidence::Exact,
                metadata: None,
            },
            SymbolLocation {
                uri: uri.clone(),
                range: Range {
                    start: LspPosition { line: 5, character: 10 },
                    end: LspPosition { line: 5, character: 18 },
                },
                kind: SymbolKind::Variable,
                confidence: ResolutionConfidence::Exact,
                metadata: None,
            },
        ];

        let adapter = LanguageAdapter::new(
            "test",
            Arc::new(MockResolver { locations }),
            Arc::new(MockHover),
            Arc::new(MockCompletion),
            Arc::new(MockDoc),
        );

        let node = MockNode::new_with_name("test_var".to_string());
        let position = Position { row: 0, column: 5, byte: 5 };
        let rename = GenericRename;

        // Create empty inverted index for test
        let inverted_index = HashMap::new();

        let result = rename.rename(&node, &position, &uri, &adapter, "new_name", &inverted_index).await;

        assert!(result.is_some());
        let workspace_edit = result.unwrap();
        assert!(workspace_edit.changes.is_some());

        let changes = workspace_edit.changes.unwrap();
        assert_eq!(changes.len(), 1);
        assert!(changes.contains_key(&uri));

        let edits = &changes[&uri];
        assert_eq!(edits.len(), 2);
        assert_eq!(edits[0].new_text, "new_name");
        assert_eq!(edits[1].new_text, "new_name");
    }

    #[tokio::test]
    async fn test_rename_no_occurrences() {
        let adapter = LanguageAdapter::new(
            "test",
            Arc::new(MockResolver { locations: vec![] }),
            Arc::new(MockHover),
            Arc::new(MockCompletion),
            Arc::new(MockDoc),
        );

        let node = MockNode::new_with_name("test_var".to_string());
        let position = Position { row: 0, column: 5, byte: 5 };
        let uri = Url::parse("file:///test.rho").unwrap();
        let rename = GenericRename;

        // Create empty inverted index for test
        let inverted_index = HashMap::new();

        let result = rename.rename(&node, &position, &uri, &adapter, "new_name", &inverted_index).await;

        assert!(result.is_none());
    }
}
