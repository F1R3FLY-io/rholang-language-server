//! Unified LSP handlers using language adapters
//!
//! This module provides a unified dispatch system for LSP requests that works
//! across all supported languages (Rholang, MeTTa, etc.) using the LanguageAdapter
//! architecture.
//!
//! # Architecture
//!
//! Instead of having separate handler methods for each language, we use a single
//! unified handler that:
//! 1. Determines the language at the given position
//! 2. Retrieves the appropriate LanguageAdapter
//! 3. Calls the generic feature implementation (GenericGotoDefinition, etc.)
//! 4. Returns the result in LSP format
//!
//! # Benefits
//!
//! - **DRY**: Write handler logic once, works for all languages
//! - **Consistency**: All languages get the same behavior
//! - **Extensibility**: Adding a new language only requires creating an adapter
//! - **Testability**: Each layer can be tested independently
//!
//! # Current Status
//!
//! Phase 4b is complete! All unified handlers are fully implemented with virtual
//! document support. The handlers work seamlessly across Rholang and embedded
//! MeTTa code.
//!
//! # Migration Path
//!
//! 1. ✅ Phase 1-3: Traits, Generic Features, Language Adapters
//! 2. ✅ Phase 4a: Create unified_handlers.rs skeleton
//! 3. ✅ Phase 4b: Implement full unified handlers with virtual document support
//! 4. ⏳ Phase 4c: Wire up unified handlers in backend.rs (current)
//! 5. Phase 4d: Remove old language-specific handlers
//!
//! # Example Flow
//!
//! ```text
//! LSP Client
//!     ↓
//! textDocument/hover request
//!     ↓
//! unified_hover() [this module]
//!     ↓
//! Detect language at position (Rholang/MeTTa/etc.)
//!     ↓
//! Get language adapter (RholangAdapter/MettaAdapter)
//!     ↓
//! GenericHover::hover() [generic feature]
//!     ↓
//! HoverProvider::hover_for_symbol() [language-specific]
//!     ↓
//! Return LSP Hover response
//! ```

use std::sync::Arc;
use tower_lsp::lsp_types::{
    GotoDefinitionResponse, Hover, Location, Position as LspPosition,
    Range, ReferenceParams, RenameParams, Url, WorkspaceEdit,
};
use tracing::{debug, trace, warn};

use crate::ir::semantic_node::{Position, SemanticNode};
use crate::lsp::features::{
    goto_definition::GenericGotoDefinition,
    hover::GenericHover,
    node_finder::lsp_to_ir_position,
    references::GenericReferences,
    rename::GenericRename,
    LanguageAdapter,
};

use super::RholangBackend;

/// Language detection result
#[derive(Debug, Clone)]
pub enum LanguageContext {
    /// Pure Rholang document
    Rholang {
        uri: Url,
        root: Arc<dyn SemanticNode>,
        symbol_table: Arc<crate::ir::symbol_table::SymbolTable>,
    },
    /// MeTTa embedded in Rholang (virtual document)
    MettaVirtual {
        virtual_uri: Url,
        parent_uri: Url,
        root: Arc<dyn SemanticNode>,
        symbol_table: Arc<crate::ir::transforms::metta_symbol_table_builder::MettaSymbolTable>,
    },
    /// Other embedded language (future)
    #[allow(dead_code)]
    Other {
        language: String,
        uri: Url,
        root: Arc<dyn SemanticNode>,
    },
}

impl RholangBackend {
    /// Detect the language at a given position in a document
    ///
    /// # Arguments
    /// * `uri` - Document URI
    /// * `position` - LSP position
    ///
    /// # Returns
    /// Language context with IR root and metadata
    ///
    /// # Implementation
    /// 1. Check for virtual document URIs (fragments like #metta:0)
    /// 2. If virtual URI, load virtual document from registry
    /// 3. Otherwise check if position is within any virtual document range in parent
    /// 4. Fall back to parent document language (Rholang)
    pub(super) async fn detect_language(
        &self,
        uri: &Url,
        position: &LspPosition,
    ) -> Option<LanguageContext> {
        debug!("detect_language: uri={}, position={:?}", uri, position);

        // Check if this is a virtual document URI (has fragment like #metta:0)
        if let Some(fragment) = uri.fragment() {
            debug!("Detected virtual document URI with fragment: {}", fragment);

            // Look up virtual document in registry
            let virtual_docs = self.virtual_docs.read().await;
            if let Some(virtual_doc) = virtual_docs.get(uri) {
                debug!(
                    "Found virtual document: language={}, parent={}",
                    virtual_doc.language, virtual_doc.parent_uri
                );

                // Get IR from virtual document (MeTTa nodes)
                let ir = virtual_doc.get_or_parse_ir();
                if let Some(ir_vec) = ir {
                    if ir_vec.is_empty() {
                        warn!("Virtual document has empty IR");
                        return None;
                    }

                    // Convert Vec<MettaNode> to single root node (use first node as representative)
                    let root: Arc<dyn SemanticNode> = ir_vec[0].clone();

                    // Get symbol table from virtual document
                    let symbol_table = virtual_doc.get_or_build_symbol_table();
                    if symbol_table.is_none() {
                        warn!("Failed to build symbol table for virtual document");
                        return None;
                    }

                    return Some(LanguageContext::MettaVirtual {
                        virtual_uri: uri.clone(),
                        parent_uri: virtual_doc.parent_uri.clone(),
                        root,
                        symbol_table: symbol_table.unwrap(),
                    });
                } else {
                    warn!("Failed to parse IR for virtual document");
                    return None;
                }
            } else {
                warn!("Virtual document URI not found in registry: {}", uri);
                return None;
            }
        }

        // Not a virtual URI - check if position is within a virtual document region
        if uri.path().ends_with(".rho") {
            // First check if position falls within any virtual document
            let virtual_docs = self.virtual_docs.read().await;
            let virtual_docs_for_parent = virtual_docs.get_by_parent(uri);

            for virtual_doc in virtual_docs_for_parent {
                // Check if position is within this virtual document's range
                let in_range = if virtual_doc.parent_start.line == virtual_doc.parent_end.line {
                    // Single-line region
                    position.line == virtual_doc.parent_start.line
                        && position.character >= virtual_doc.parent_start.character
                        && position.character <= virtual_doc.parent_end.character
                } else {
                    // Multi-line region
                    (position.line > virtual_doc.parent_start.line && position.line < virtual_doc.parent_end.line)
                        || (position.line == virtual_doc.parent_start.line && position.character >= virtual_doc.parent_start.character)
                        || (position.line == virtual_doc.parent_end.line && position.character <= virtual_doc.parent_end.character)
                };

                if in_range {
                    debug!(
                        "Position {:?} is within virtual document {} ({})",
                        position, virtual_doc.uri, virtual_doc.language
                    );

                    let ir = virtual_doc.get_or_parse_ir();
                    if let Some(ir_vec) = ir {
                        if ir_vec.is_empty() {
                            warn!("Virtual document has empty IR");
                            continue;
                        }

                        let root: Arc<dyn SemanticNode> = ir_vec[0].clone();

                        // Get symbol table from virtual document
                        let symbol_table = virtual_doc.get_or_build_symbol_table();
                        if symbol_table.is_none() {
                            warn!("Failed to build symbol table for virtual document");
                            continue;
                        }

                        return Some(LanguageContext::MettaVirtual {
                            virtual_uri: virtual_doc.uri.clone(),
                            parent_uri: virtual_doc.parent_uri.clone(),
                            root,
                            symbol_table: symbol_table.unwrap(),
                        });
                    } else {
                        warn!("Failed to parse IR for virtual document");
                        continue;
                    }
                }
            }
            drop(virtual_docs); // Release read lock

            // Not in a virtual document - return Rholang context
            if let Some(doc) = self.workspace.documents.get(uri) {
                debug!("Detected Rholang document: {}", uri);
                return Some(LanguageContext::Rholang {
                    uri: uri.clone(),
                    root: Arc::new((*doc.ir).clone()) as Arc<dyn SemanticNode>,
                    symbol_table: doc.symbol_table.clone(),
                });
            }
        }

        warn!("detect_language: No language context found for {:?}", uri);
        None
    }

    /// Get the appropriate language adapter for a language context
    ///
    /// # Arguments
    /// * `context` - Language context from detect_language()
    ///
    /// # Returns
    /// Language adapter for the detected language with real symbol resolvers
    ///
    /// # Resolution Strategies
    ///
    /// This function routes to three different symbol resolution strategies:
    ///
    /// 1. **Rholang**: Uses `RholangSymbolResolver` with hierarchical symbol table
    ///    - Lexical scoping with parent chain traversal
    ///    - Local → document → global scope hierarchy
    ///
    /// 2. **MeTTa**: Uses `ComposableSymbolResolver` with pattern matching
    ///    - Lexical scope + arity-based pattern filter + global fallback
    ///    - Supports MeTTa's pattern matching semantics
    ///
    /// 3. **Generic (Other)**: Uses `GenericSymbolResolver` with global scope
    ///    - Single flat namespace (no lexical hierarchy)
    ///    - Multiple declarations/definitions per symbol
    ///    - Cross-document linking via global_virtual_symbols
    ///    - Default for future embedded languages
    fn get_adapter(&self, context: &LanguageContext) -> Option<LanguageAdapter> {
        match context {
            LanguageContext::Rholang { symbol_table, .. } => {
                Some(crate::lsp::features::adapters::create_rholang_adapter(symbol_table.clone()))
            }
            LanguageContext::MettaVirtual { symbol_table, parent_uri, .. } => {
                Some(crate::lsp::features::adapters::create_metta_adapter(
                    symbol_table.clone(),
                    self.workspace.clone(),
                    parent_uri.clone(),
                ))
            }
            LanguageContext::Other { language, .. } => {
                // Use generic global scope resolver for unknown languages
                Some(crate::lsp::features::adapters::create_generic_adapter(
                    self.workspace.clone(),
                    language.clone(),
                ))
            }
        }
    }

    /// Unified goto-definition handler
    ///
    /// Works for all languages by dispatching to the appropriate adapter.
    ///
    /// # Arguments
    /// * `uri` - Document URI (may be virtual with fragment)
    /// * `position` - LSP position where goto-definition was requested
    ///
    /// # Returns
    /// Definition location(s), or None if not found
    ///
    /// # Implementation Flow
    /// 1. Detect language at position (Rholang or MeTTa virtual doc)
    /// 2. Get appropriate language adapter
    /// 3. Convert LSP position to IR position
    /// 4. Call GenericGotoDefinition with adapter
    /// 5. Return definition locations
    pub(super) async fn unified_goto_definition(
        &self,
        uri: &Url,
        position: LspPosition,
    ) -> Option<GotoDefinitionResponse> {
        use crate::lsp::features::goto_definition::GenericGotoDefinition;

        debug!("unified_goto_definition: uri={}, position={:?}", uri, position);

        // Detect language at position
        let context = self.detect_language(uri, &position).await?;
        debug!("Detected language context for goto_definition: {:?}", context);

        // Get adapter for this language
        let adapter = self.get_adapter(&context)?;

        // Extract root and URI from context
        let (root, doc_uri) = match context {
            LanguageContext::Rholang { uri, root, .. } => (root, uri),
            LanguageContext::MettaVirtual {
                virtual_uri, root, ..
            } => (root, virtual_uri),
            LanguageContext::Other { uri, root, .. } => (root, uri),
        };

        // Convert LSP position to IR position
        let ir_position = lsp_to_ir_position(position);

        // Call generic goto-definition feature (uses tree-based position finding)
        let goto_def_feature = GenericGotoDefinition;
        goto_def_feature
            .goto_definition(root.as_ref(), &ir_position, &doc_uri, &adapter)
            .await
    }

    /// Unified hover handler
    ///
    /// Works for all languages by dispatching to the appropriate adapter.
    ///
    /// # Arguments
    /// * `uri` - Document URI (may be virtual with fragment)
    /// * `position` - LSP position where hover was requested
    ///
    /// # Returns
    /// Hover information, or None if not available
    ///
    /// # Implementation Flow
    /// 1. Detect language at position (Rholang or MeTTa virtual doc)
    /// 2. Get appropriate language adapter
    /// 3. Convert LSP position to IR position
    /// 4. Call GenericHover with adapter
    /// 5. Return hover result
    pub(super) async fn unified_hover(
        &self,
        uri: &Url,
        position: LspPosition,
    ) -> Option<Hover> {
        debug!("unified_hover: uri={}, position={:?}", uri, position);

        // Detect language at position
        let context = self.detect_language(uri, &position).await?;
        debug!("Detected language context: {:?}", context);

        // Get adapter for this language
        let adapter = self.get_adapter(&context)?;

        // Extract root, URI, and parent_uri from context
        let (root, doc_uri, parent_uri) = match context {
            LanguageContext::Rholang { uri, root, .. } => {
                (root, uri, None)
            }
            LanguageContext::MettaVirtual {
                virtual_uri,
                parent_uri,
                root,
                ..
            } => {
                (root, virtual_uri, Some(parent_uri))
            }
            LanguageContext::Other { uri, root, .. } => {
                (root, uri, None)
            }
        };

        // Convert LSP position to IR position
        let ir_position = lsp_to_ir_position(position);

        // Call generic hover feature
        let hover_feature = GenericHover;
        hover_feature
            .hover(
                root.as_ref(),
                &ir_position,
                position,
                &doc_uri,
                &adapter,
                parent_uri,
            )
            .await
    }

    /// Unified find-references handler
    ///
    /// Works for all languages by dispatching to the appropriate adapter.
    ///
    /// # Arguments
    /// * `params` - LSP ReferenceParams containing URI, position, and context
    ///
    /// # Returns
    /// List of reference locations, or None if not found
    ///
    /// # Implementation Flow
    /// 1. Extract URI and position from params
    /// 2. Detect language at position
    /// 3. Get appropriate language adapter
    /// 4. Convert LSP position to IR position
    /// 5. Call GenericReferences with adapter
    /// 6. Return reference locations
    pub(super) async fn unified_references(
        &self,
        params: ReferenceParams,
    ) -> Option<Vec<Location>> {
        let uri = &params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let include_declaration = params.context.include_declaration;

        debug!(
            "unified_references: uri={}, position={:?}, include_decl={}",
            uri, position, include_declaration
        );

        // Detect language at position
        let context = self.detect_language(uri, &position).await?;
        debug!("Detected language context for references: {:?}", context);

        // Get adapter for this language
        let adapter = self.get_adapter(&context)?;

        // Extract root and URI from context
        let (root, doc_uri) = match context {
            LanguageContext::Rholang { uri, root, .. } => (root, uri),
            LanguageContext::MettaVirtual {
                virtual_uri, root, ..
            } => (root, virtual_uri),
            LanguageContext::Other { uri, root, .. } => (root, uri),
        };

        // Convert LSP position to IR position
        let ir_position = lsp_to_ir_position(position);

        // Call generic find-references feature (Priority 2: use rholang_symbols instead of global_inverted_index)
        let refs_feature = GenericReferences;
        refs_feature
            .find_references(root.as_ref(), &ir_position, &doc_uri, &adapter, include_declaration, &self.workspace.rholang_symbols)
            .await
    }

    /// Unified rename handler
    ///
    /// Works for all languages by dispatching to the appropriate adapter.
    ///
    /// # Arguments
    /// * `params` - LSP RenameParams containing URI, position, and new name
    ///
    /// # Returns
    /// WorkspaceEdit with all text edits needed for the rename
    ///
    /// # Implementation Flow
    /// 1. Extract URI, position, and new name from params
    /// 2. Detect language at position
    /// 3. Get appropriate language adapter
    /// 4. Convert LSP position to IR position
    /// 5. Call GenericRename with adapter
    /// 6. Return workspace edit
    pub(super) async fn unified_rename(
        &self,
        params: RenameParams,
    ) -> Option<WorkspaceEdit> {
        let uri = &params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let new_name = &params.new_name;

        debug!(
            "unified_rename: uri={}, position={:?}, new_name={}",
            uri, position, new_name
        );

        // Detect language at position
        let context = self.detect_language(uri, &position).await?;
        debug!("Detected language context for rename: {:?}", context);

        // Get adapter for this language
        let adapter = self.get_adapter(&context)?;

        // Extract root and URI from context
        let (root, doc_uri) = match context {
            LanguageContext::Rholang { uri, root, .. } => (root, uri),
            LanguageContext::MettaVirtual {
                virtual_uri, root, ..
            } => (root, virtual_uri),
            LanguageContext::Other { uri, root, .. } => (root, uri),
        };

        // Convert LSP position to IR position
        let ir_position = lsp_to_ir_position(position);

        // Call generic rename feature (Priority 2: use rholang_symbols instead of global_inverted_index)
        let rename_feature = GenericRename;
        rename_feature
            .rename(root.as_ref(), &ir_position, &doc_uri, &adapter, new_name, &self.workspace.rholang_symbols)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_language_context_construction() {
        // Test that LanguageContext variants can be constructed
        let uri = Url::parse("file:///test.rho").unwrap();

        // This is just a type check - actual functionality will be tested via integration tests
        let _context = LanguageContext::Rholang {
            uri: uri.clone(),
            root: Arc::new(crate::ir::rholang_node::RholangNode::Nil {
                base: crate::ir::semantic_node::NodeBase::new_simple(
                    crate::ir::semantic_node::RelativePosition {
                        delta_lines: 0,
                        delta_columns: 0,
                        delta_bytes: 0
                    },
                    0,
                    0,
                    0,
                ),
                metadata: None,
            }) as Arc<dyn SemanticNode>,
            symbol_table: Arc::new(crate::ir::symbol_table::SymbolTable::new(None)),
        };
    }
}
