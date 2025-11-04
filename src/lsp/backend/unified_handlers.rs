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
        all_roots: Vec<Arc<dyn SemanticNode>>, // All top-level nodes (typically single Par node)
        symbol_table: Arc<crate::ir::symbol_table::SymbolTable>,
    },
    /// MeTTa embedded in Rholang (virtual document)
    MettaVirtual {
        virtual_uri: Url,
        parent_uri: Url,
        root: Arc<dyn SemanticNode>,
        all_roots: Vec<Arc<dyn SemanticNode>>, // All top-level MeTTa nodes
        symbol_table: Arc<crate::ir::transforms::metta_symbol_table_builder::MettaSymbolTable>,
        virtual_doc: Arc<crate::language_regions::VirtualDocument>,
    },
    /// Other embedded language (future)
    #[allow(dead_code)]
    Other {
        language: String,
        uri: Url,
        root: Arc<dyn SemanticNode>,
        all_roots: Vec<Arc<dyn SemanticNode>>, // All top-level nodes
    },
}

impl LanguageContext {
    /// Get a short description of the language context for logging
    /// (avoids Debug formatting large structures which can hang)
    fn describe(&self) -> String {
        match self {
            LanguageContext::Rholang { all_roots, .. } => {
                format!("Rholang with {} root(s)", all_roots.len())
            }
            LanguageContext::MettaVirtual { all_roots, .. } => {
                format!("MettaVirtual with {} root(s)", all_roots.len())
            }
            LanguageContext::Other { language, all_roots, .. } => {
                format!("Other({}) with {} root(s)", language, all_roots.len())
            }
        }
    }
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

                    // Store all top-level nodes + use first node as representative root
                    // (root is used for backward compatibility, all_roots for correct node finding)
                    let root: Arc<dyn SemanticNode> = ir_vec[0].clone();

                    // Compute positions for all nodes together, tracking prev_end across all roots
                    use crate::ir::metta_node::compute_positions_with_prev_end;
                    use crate::ir::semantic_node::Position as IrPosition;
                    let mut combined_positions = std::collections::HashMap::new();
                    let mut prev_end = IrPosition { row: 0, column: 0, byte: 0 };

                    for metta_node in ir_vec.iter() {
                        let (positions, new_prev_end) = compute_positions_with_prev_end(metta_node, prev_end);
                        combined_positions.extend(positions);
                        prev_end = new_prev_end;
                    }

                    let all_roots: Vec<Arc<dyn SemanticNode>> = ir_vec.iter()
                        .map(|node| node.clone() as Arc<dyn SemanticNode>)
                        .collect();

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
                        all_roots,
                        symbol_table: symbol_table.unwrap(),
                        virtual_doc: virtual_doc.clone(),
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

                        // Store all top-level nodes + use first node as representative root
                        let root: Arc<dyn SemanticNode> = ir_vec[0].clone();
                        let all_roots: Vec<Arc<dyn SemanticNode>> = ir_vec.iter()
                            .map(|node| node.clone() as Arc<dyn SemanticNode>)
                            .collect();

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
                            all_roots,
                            symbol_table: symbol_table.unwrap(),
                            virtual_doc: virtual_doc.clone(),
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
                let root: Arc<dyn SemanticNode> = Arc::new((*doc.ir).clone()) as Arc<dyn SemanticNode>;
                let all_roots = vec![root.clone()]; // Rholang has single root (Par node)
                return Some(LanguageContext::Rholang {
                    uri: uri.clone(),
                    root,
                    all_roots,
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
        debug!("Detected language context for goto_definition: {}", context.describe());

        // Get adapter for this language
        let adapter = self.get_adapter(&context)?;

        // Extract root, URI, and position from context
        // For MettaVirtual, we need to map position from parent to virtual coordinates
        match context {
            LanguageContext::Rholang { uri, root, all_roots, symbol_table, .. } => {
                let ir_position = lsp_to_ir_position(position);

                // Check if the adapter has a specialized goto-definition provider
                if let Some(goto_def_provider) = &adapter.goto_definition {
                    debug!("Using specialized goto-definition provider for Rholang");

                    // Build context for specialized provider
                    use crate::lsp::features::traits::GotoDefinitionContext;
                    let context = GotoDefinitionContext {
                        uri: uri.clone(),
                        ir_position,
                        all_roots: all_roots.clone(),
                        symbol_table: symbol_table.clone(),
                        language: "rholang".to_string(),
                        parent_uri: None,
                    };

                    return goto_def_provider.goto_definition(&context).await;
                }

                // Use generic handler (default for Rholang)
                let goto_def_feature = GenericGotoDefinition;
                goto_def_feature
                    .goto_definition(root.as_ref(), &ir_position, &uri, &adapter)
                    .await
            }
            LanguageContext::MettaVirtual {
                virtual_uri,
                all_roots,
                virtual_doc,
                symbol_table,
                ..
            } => {
                // Convert parent position to virtual position
                debug!("MettaVirtual handler: About to call map_from_parent with position {:?}, all_roots.len()={}", position, all_roots.len());
                let virtual_position = match virtual_doc.map_from_parent(position) {
                    Some(pos) => pos,
                    None => {
                        debug!("Position {:?} is outside virtual document range", position);
                        return None;
                    }
                };
                debug!(
                    "Mapped parent position {:?} to virtual position {:?}",
                    position, virtual_position
                );
                let ir_position = lsp_to_ir_position(virtual_position);

                // Check if the adapter has a specialized goto-definition provider
                if let Some(goto_def_provider) = &adapter.goto_definition {
                    debug!("Using specialized goto-definition provider for MeTTa");

                    // Build context for specialized provider
                    use crate::lsp::features::traits::GotoDefinitionContext;
                    let context = GotoDefinitionContext {
                        uri: virtual_uri.clone(),
                        ir_position,
                        all_roots: all_roots.clone(),
                        symbol_table: symbol_table.clone(),
                        language: "metta".to_string(),
                        parent_uri: Some(virtual_doc.parent_uri.clone()),
                    };

                    // Get result from specialized provider (in virtual coordinates)
                    if let Some(result) = goto_def_provider.goto_definition(&context).await {
                        debug!("Specialized provider returned result in virtual coordinates, mapping to parent");

                        // Map from virtual to parent coordinates
                        let mapped_result = match result {
                            GotoDefinitionResponse::Scalar(loc) => {
                                let parent_range = virtual_doc.map_range_to_parent(loc.range);
                                debug!("Mapped virtual range {:?} to parent range {:?}", loc.range, parent_range);
                                GotoDefinitionResponse::Scalar(Location {
                                    uri: virtual_doc.parent_uri.clone(),
                                    range: parent_range,
                                })
                            }
                            GotoDefinitionResponse::Array(locs) => {
                                let parent_locs: Vec<Location> = locs
                                    .into_iter()
                                    .map(|loc| {
                                        let parent_range = virtual_doc.map_range_to_parent(loc.range);
                                        Location {
                                            uri: virtual_doc.parent_uri.clone(),
                                            range: parent_range,
                                        }
                                    })
                                    .collect();
                                GotoDefinitionResponse::Array(parent_locs)
                            }
                            GotoDefinitionResponse::Link(link) => {
                                // Links don't need mapping, just return as-is
                                GotoDefinitionResponse::Link(link)
                            }
                        };

                        return Some(mapped_result);
                    } else {
                        return None;
                    }
                }

                // Fallback to generic handler if no specialized provider
                debug!("No specialized goto-definition provider, using generic handler");

                // Iterate through all top-level nodes to find one that contains this position
                // We need to track prev_end as we go through each root since MeTTa uses relative positions
                use crate::lsp::features::node_finder::find_node_at_position_with_prev_end;
                use crate::ir::semantic_node::Position as IrPosition;

                let mut prev_end = IrPosition { row: 0, column: 0, byte: 0 };
                for (i, root) in all_roots.iter().enumerate() {
                    // Try to find the node in this root with the correct prev_end
                    if let Some(node) = find_node_at_position_with_prev_end(root.as_ref(), &ir_position, &prev_end) {
                        debug!("Found node in root {} at position {:?}", i, ir_position);

                        // Now try goto_definition using this specific root and node
                        let goto_def_feature = GenericGotoDefinition;
                        if let Some(result) = goto_def_feature
                            .goto_definition(root.as_ref(), &ir_position, &virtual_uri, &adapter)
                            .await
                        {
                            debug!("Found definition in root node {} (virtual coordinates)", i);

                            // Map the result from virtual to parent coordinates
                            let mapped_result = match result {
                                GotoDefinitionResponse::Scalar(loc) => {
                                    let parent_range = virtual_doc.map_range_to_parent(loc.range);
                                    debug!("Mapped virtual range {:?} to parent range {:?}", loc.range, parent_range);
                                    GotoDefinitionResponse::Scalar(Location {
                                        uri: virtual_doc.parent_uri.clone(),
                                        range: parent_range,
                                    })
                                }
                                GotoDefinitionResponse::Array(locs) => {
                                    let parent_locs: Vec<Location> = locs
                                        .into_iter()
                                        .map(|loc| {
                                            let parent_range = virtual_doc.map_range_to_parent(loc.range);
                                            Location {
                                                uri: virtual_doc.parent_uri.clone(),
                                                range: parent_range,
                                            }
                                        })
                                        .collect();
                                    GotoDefinitionResponse::Array(parent_locs)
                                }
                                GotoDefinitionResponse::Link(link) => {
                                    // Links don't need mapping, just return as-is
                                    GotoDefinitionResponse::Link(link)
                                }
                            };

                            return Some(mapped_result);
                        }
                    }

                    // Update prev_end for next root
                    prev_end = root.base().end();
                }
                debug!("No definition found in any of the {} root nodes", all_roots.len());
                None
            }
            LanguageContext::Other { language, uri, root, all_roots, .. } => {
                let ir_position = lsp_to_ir_position(position);

                // Check if the adapter has a specialized goto-definition provider
                if let Some(goto_def_provider) = &adapter.goto_definition {
                    debug!("Using specialized goto-definition provider for {}", language);

                    // Build context for specialized provider
                    use crate::lsp::features::traits::GotoDefinitionContext;
                    // Note: We don't have symbol_table for generic "Other" languages yet
                    // but we can still pass an empty Arc for future extensibility
                    let context = GotoDefinitionContext {
                        uri: uri.clone(),
                        ir_position,
                        all_roots: all_roots.clone(),
                        symbol_table: Arc::new(()) as Arc<dyn std::any::Any + Send + Sync>,
                        language: language.clone(),
                        parent_uri: None,
                    };

                    return goto_def_provider.goto_definition(&context).await;
                }

                // Use generic handler (default for other languages)
                let goto_def_feature = GenericGotoDefinition;
                goto_def_feature
                    .goto_definition(root.as_ref(), &ir_position, &uri, &adapter)
                    .await
            }
        }
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
        debug!("Detected language context for hover: {}", context.describe());

        // Get adapter for this language
        let adapter = self.get_adapter(&context)?;

        // Extract root, URI, position, and parent_uri from context
        // For MettaVirtual, we need to map position from parent to virtual coordinates
        match context {
            LanguageContext::Rholang { uri, root, .. } => {
                let ir_position = lsp_to_ir_position(position);
                let hover_feature = GenericHover;
                hover_feature
                    .hover(
                        root.as_ref(),
                        &ir_position,
                        position,
                        &uri,
                        &adapter,
                        None,
                    )
                    .await
            }
            LanguageContext::MettaVirtual {
                virtual_uri,
                parent_uri,
                all_roots,
                virtual_doc,
                ..
            } => {
                // Convert parent position to virtual position
                debug!("MettaVirtual handler: About to call map_from_parent with position {:?}, all_roots.len()={}", position, all_roots.len());
                let virtual_position = match virtual_doc.map_from_parent(position) {
                    Some(pos) => pos,
                    None => {
                        debug!("Position {:?} is outside virtual document range", position);
                        return None;
                    }
                };
                debug!(
                    "Mapped parent position {:?} to virtual position {:?}",
                    position, virtual_position
                );
                let ir_position = lsp_to_ir_position(virtual_position);

                // Iterate through all top-level nodes to find one that contains this position
                // We need to track prev_end as we go through each root since MeTTa uses relative positions
                use crate::lsp::features::node_finder::find_node_at_position_with_prev_end;
                use crate::ir::semantic_node::Position as IrPosition;

                let mut prev_end = IrPosition { row: 0, column: 0, byte: 0 };
                for (i, root) in all_roots.iter().enumerate() {
                    // Try to find the node in this root with the correct prev_end
                    if let Some(node) = find_node_at_position_with_prev_end(root.as_ref(), &ir_position, &prev_end) {
                        debug!("Found node in root {} at position {:?}", i, ir_position);

                        // Now try hover using the pre-found node
                        // We pass the node we already found to avoid re-searching
                        let hover_feature = GenericHover;
                        if let Some(result) = hover_feature
                            .hover_with_node(
                                Some(node),
                                root.as_ref(),
                                &ir_position,
                                virtual_position,
                                &virtual_uri,
                                &adapter,
                                Some(parent_uri.clone()),
                            )
                            .await
                        {
                            debug!("Found hover in root node {}", i);
                            return Some(result);
                        }
                    }

                    // Update prev_end for next root
                    prev_end = root.base().end();
                }
                debug!("No hover found in any of the {} root nodes", all_roots.len());
                None
            }
            LanguageContext::Other { uri, root, .. } => {
                let ir_position = lsp_to_ir_position(position);
                let hover_feature = GenericHover;
                hover_feature
                    .hover(
                        root.as_ref(),
                        &ir_position,
                        position,
                        &uri,
                        &adapter,
                        None,
                    )
                    .await
            }
        }
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
        debug!("Detected language context for references: {}", context.describe());

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

        // Get cached document to access symbol_table and inverted_index
        let doc = self.workspace.documents.get(&doc_uri)?;

        // Call generic find-references feature with two-tier resolution
        let refs_feature = GenericReferences;
        refs_feature
            .find_references(
                root.as_ref(),
                &ir_position,
                &doc_uri,
                &adapter,
                include_declaration,
                &doc.symbol_table,
                &doc.inverted_index,
                &self.workspace.rholang_symbols
            )
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
        debug!("Detected language context for rename: {}", context.describe());

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

        // Get cached document to access symbol_table and inverted_index
        let doc = self.workspace.documents.get(&doc_uri)?;

        // Call generic rename feature with two-tier resolution
        let rename_feature = GenericRename;
        rename_feature
            .rename(
                root.as_ref(),
                &ir_position,
                &doc_uri,
                &adapter,
                new_name,
                &doc.symbol_table,
                &doc.inverted_index,
                &self.workspace.rholang_symbols
            )
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
        let root: Arc<dyn SemanticNode> = Arc::new(crate::ir::rholang_node::RholangNode::Nil {
            base: crate::ir::semantic_node::NodeBase::new_simple(
                crate::ir::semantic_node::Position {
                    row: 0,
                    column: 0,
                    byte: 0
                },
                0,
                0,
                0,
            ),
            metadata: None,
        }) as Arc<dyn SemanticNode>;

        let _context = LanguageContext::Rholang {
            uri: uri.clone(),
            root: root.clone(),
            all_roots: vec![root],
            symbol_table: Arc::new(crate::ir::symbol_table::SymbolTable::new(None)),
        };
    }
}
