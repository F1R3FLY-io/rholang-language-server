//! Generic rename implementation
//!
//! Provides language-agnostic symbol renaming using the LanguageAdapter pattern.
//! Works with any language that implements SemanticNode and provides a SymbolResolver.

use std::collections::HashMap;
use std::sync::Arc;
use tower_lsp::lsp_types::{TextEdit, Url, WorkspaceEdit, Position as LspPosition, Range};
use tracing::{debug, trace};

use super::traits::LanguageAdapter;
use super::references::GenericReferences;
use super::node_finder::find_node_at_position;
use crate::ir::semantic_node::{Position, SemanticNode};
use crate::ir::symbol_resolution::{ResolutionContext};
use crate::lsp::rholang_contracts::RholangContracts;

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
    /// * `symbol_table` - Per-document symbol table for local variable resolution
    /// * `inverted_index` - Per-document inverted index for local references
    /// * `rholang_symbols` - Global contract storage for cross-document resolution
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
        symbol_table: &Arc<crate::ir::symbol_table::SymbolTable>,
        inverted_index: &crate::ir::transforms::symbol_table_builder::InvertedIndex,
        rholang_symbols: &Arc<RholangContracts>,
    ) -> Option<WorkspaceEdit> {
        debug!(
            "Renaming symbol at {}:{} in {} to '{}'",
            position.row, position.column, uri, new_name
        );

        // Use GenericReferences to find all occurrences
        let references_finder = GenericReferences;
        let locations = references_finder
            .find_references(root, position, uri, adapter, true, symbol_table, inverted_index, rholang_symbols) // include_declaration = true
            .await?;

        if locations.is_empty() {
            debug!("No references found for rename");
            return None;
        }

        trace!("Found {} locations to rename", locations.len());

        // Group edits by document URI with deduplication
        let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();

        for location in locations {
            let edit = TextEdit {
                range: location.range,
                new_text: new_name.to_string(),
            };

            // Get or create the vector of edits for this URI
            let edits = changes
                .entry(location.uri)
                .or_insert_with(Vec::new);

            // Only add if not already present (deduplicate by range)
            if !edits.iter().any(|e| e.range == edit.range) {
                edits.push(edit);
            }
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
    /// * `symbol_table` - Per-document symbol table for local variable resolution
    /// * `inverted_index` - Per-document inverted index for local references
    /// * `rholang_symbols` - Global contract storage for cross-document resolution
    ///
    /// # Returns
    /// Option containing the range and placeholder text for the rename
    pub async fn prepare_rename(
        &self,
        root: &dyn SemanticNode,
        position: &Position,
        uri: &Url,
        adapter: &LanguageAdapter,
        symbol_table: &Arc<crate::ir::symbol_table::SymbolTable>,
        inverted_index: &crate::ir::transforms::symbol_table_builder::InvertedIndex,
        rholang_symbols: &Arc<RholangContracts>,
    ) -> Option<(Range, String)> {
        debug!(
            "Preparing rename at {}:{} in {}",
            position.row, position.column, uri
        );

        // Find the symbol at this position
        let references_finder = GenericReferences;
        let locations = references_finder
            .find_references(root, position, uri, adapter, false, symbol_table, inverted_index, rholang_symbols) // include_declaration = false
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
    use crate::ir::semantic_node::{NodeBase, Position, SemanticCategory, Metadata};
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

        // Create symbol_table and inverted_index for local variables
        use crate::ir::symbol_table::{SymbolType, Symbol, SymbolTable};
        let symbol_table = Arc::new(SymbolTable::new(None));
        let mut inverted_index = std::collections::HashMap::new();

        // Insert a test symbol
        symbol_table.insert(Arc::new(Symbol::new(
            "test_var".to_string(),
            SymbolType::Variable,
            uri.clone(),
            position.clone(),
        )));

        // Add reference in inverted_index
        inverted_index.insert(position.clone(), vec![Position { row: 5, column: 10, byte: 50 }]);

        // Empty rholang_symbols (no contracts)
        let rholang_symbols = Arc::new(RholangContracts::new());

        let result = rename.rename(&node, &position, &uri, &adapter, "new_name", &symbol_table, &inverted_index, &rholang_symbols).await;

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

        // Create empty symbol_table, inverted_index, and rholang_symbols
        use crate::ir::symbol_table::SymbolTable;
        let symbol_table = Arc::new(SymbolTable::new(None));
        let inverted_index = std::collections::HashMap::new();
        let rholang_symbols = Arc::new(RholangContracts::new());

        let result = rename.rename(&node, &position, &uri, &adapter, "new_name", &symbol_table, &inverted_index, &rholang_symbols).await;

        assert!(result.is_none());
    }
}
