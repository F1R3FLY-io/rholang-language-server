//! Symbol operations for the LSP backend
//!
//! This module contains all symbol-related functionality including:
//! - Symbol table management and cross-workspace linking
//! - Symbol lookup at cursor positions
//! - Symbol references and usage tracking
//! - LSP symbol-related handlers (goto-definition, references, rename, etc.)

use std::collections::HashMap;
use std::sync::Arc;

use tower_lsp::lsp_types::{
    Position as LspPosition, Url,
};
use tracing::{debug, info, trace};

use crate::ir::rholang_node::{RholangNode, Position as IrPosition, find_node_at_position_with_path};
use crate::ir::symbol_table::{Symbol, SymbolTable, SymbolType};

use super::state::{RholangBackend, WorkspaceChangeEvent, WorkspaceChangeType};

impl RholangBackend {
    /// Checks if symbol linking might be needed (stale global symbols).
    ///
    /// Returns true if:
    /// - There are documents in workspace but no global symbols
    ///
    /// This is used to eagerly trigger symbol linking for critical operations
    /// like rename, goto-definition, and references to avoid race conditions
    /// with the debounced symbol linker.
    pub(crate) async fn needs_symbol_linking(&self) -> bool {
        let workspace = self.workspace.read().await;
        let doc_count = workspace.documents.len();
        let symbol_count = workspace.global_symbols.len();

        // If we have documents but no global symbols, we definitely need linking
        // This handles the race condition where documents are indexed but
        // the debounced symbol linker hasn't run yet
        let needs_linking = doc_count > 0 && symbol_count == 0;
        debug!("needs_symbol_linking: doc_count={}, symbol_count={}, needs={}",
               doc_count, symbol_count, needs_linking);
        needs_linking
    }

    /// Links symbols across all workspace files.
    ///
    /// This function:
    /// 1. Collects all contract symbols from workspace documents
    /// 2. Builds a global symbol table mapping contract names to their declarations
    /// 3. Resolves potential global references to their definitions
    /// 4. Updates the global inverted index for cross-file navigation
    /// 5. Broadcasts workspace change event via hot observable
    pub(crate) async fn link_symbols(&self) {
        // Clone both documents and global_table in a single read lock acquisition
        let (documents, global_table) = {
            let workspace = self.workspace.read().await;
            (workspace.documents.clone(), workspace.global_table.clone())
        };

        // Collect all global contract symbols from workspace.global_table
        // The global_table was populated during indexing by SymbolTableBuilder,
        // which only inserts top-level contracts (is_top_level check at line 370 of symbol_table_builder.rs)
        let mut global_symbols = HashMap::new();

        // Get all contract symbols from the global table
        for (name, symbol) in global_table.symbols.read().unwrap().iter() {
            if matches!(symbol.symbol_type, crate::ir::symbol_table::SymbolType::Contract) {
                debug!("link_symbols: Adding global contract '{}' from {} at {:?}",
                       name, symbol.declaration_uri, symbol.declaration_location);
                global_symbols.insert(
                    name.clone(),
                    (symbol.declaration_uri.clone(), symbol.declaration_location),
                );
            }
        }
        debug!("link_symbols: Collected {} global contract symbols from global_table", global_symbols.len());

        debug!("link_symbols: Processing {} documents", documents.len());
        info!("Linked symbols across {} files", documents.len());

        // Resolve potential global references (without holding any lock)
        let mut resolutions = Vec::new();
        for (doc_uri, doc) in &documents {
            for (name, use_pos) in &doc.potential_global_refs {
                if let Some((def_uri, def_pos)) = global_symbols.get(name).cloned() {
                    // Skip self-references
                    if (doc_uri.clone(), *use_pos) != (def_uri.clone(), def_pos) {
                        resolutions.push(((def_uri, def_pos), (doc_uri.clone(), *use_pos)));
                        trace!(
                            "Resolved potential global usage of '{}' at {:?} to def at {:?}",
                            name, use_pos, def_pos
                        );
                    } else {
                        trace!("Skipping self-reference potential for '{}' at {:?}", name, use_pos);
                    }
                }
            }
        }

        // Build global inverted index (without holding any lock)
        let mut global_inverted_index = HashMap::new();
        for ((def_uri, def_pos), (use_uri, use_pos)) in resolutions {
            global_inverted_index
                .entry((def_uri, def_pos))
                .or_insert_with(Vec::new)
                .push((use_uri, use_pos));
        }

        // Now acquire write lock only to update workspace (minimal lock duration)
        let (file_count, symbol_count) = {
            let mut workspace = self.workspace.write().await;
            workspace.global_symbols = global_symbols;
            workspace.global_inverted_index = global_inverted_index;
            (workspace.documents.len(), workspace.global_symbols.len())
        };

        // Broadcast workspace change event (ReactiveX Phase 2)
        let _ = self.workspace_changes.send(WorkspaceChangeEvent {
            file_count,
            symbol_count,
            change_type: WorkspaceChangeType::SymbolsLinked,
        });
    }

    /// Gets the symbol at the given position in a document.
    ///
    /// This function handles various node types:
    /// - Variable references (Var nodes)
    /// - Contract declarations and names
    /// - Contract calls (Send/SendSync nodes)
    /// - Quoted contract references
    ///
    /// Returns the symbol with its declaration location and type.
    pub(crate) async fn get_symbol_at_position(
        &self,
        uri: &Url,
        position: LspPosition,
    ) -> Option<Arc<Symbol>> {
        // Get document from workspace
        let opt_doc = {
            debug!("Acquiring workspace read lock for symbol at {}:{:?}", uri, position);
            let workspace = self.workspace.read().await;
            debug!("Workspace read lock acquired for {}:{:?}", uri, position);
            workspace.documents.get(uri).cloned()
        };

        let doc = opt_doc?;
        let text = &doc.text;

        // Convert LSP position to byte offset
        let byte_offset = Self::byte_offset_from_position(
            text,
            position.line as usize,
            position.character as usize,
        )?;

        let pos = IrPosition {
            row: position.line as usize,
            column: position.character as usize,
            byte: byte_offset,
        };

        // Get node with path for parent checking
        let (node_path_opt, symbol_table_opt) = {
            let opt_doc = {
                let workspace = self.workspace.read().await;
                workspace.documents.get(uri).cloned()
            };

            if let Some(doc) = opt_doc {
                let path_result = find_node_at_position_with_path(&doc.ir, &*doc.positions, pos);
                let symbol_table = path_result
                    .as_ref()
                    .and_then(|(node, _)| {
                        node.metadata()
                            .and_then(|m| m.get("symbol_table"))
                            .and_then(|t| t.downcast_ref::<Arc<SymbolTable>>())
                            .cloned()
                    })
                    .unwrap_or_else(|| doc.symbol_table.clone());
                (path_result, Some(symbol_table))
            } else {
                (None, None)
            }
        };

        let (node, path) = node_path_opt?;
        let symbol_table = symbol_table_opt?;

        // Log node type for debugging
        let node_type_name = match &*node {
            RholangNode::Var { .. } => "Var",
            RholangNode::Contract { .. } => "Contract",
            RholangNode::Send { .. } => "Send",
            RholangNode::SendSync { .. } => "SendSync",
            RholangNode::Quote { .. } => "Quote",
            other => {
                debug!("Unknown node type discriminant: {:?}", std::mem::discriminant(other));
                "Other"
            }
        };
        debug!("RholangNode at position: {}", node_type_name);

        // Handle different node types
        match &*node {
            RholangNode::Var { name, .. } => {
                self.handle_var_symbol(uri, position, name, &path, &symbol_table)
                    .await
            }
            RholangNode::Contract { name, .. } => {
                self.handle_contract_symbol(uri, position, name).await
            }
            RholangNode::Send { channel, .. } | RholangNode::SendSync { channel, .. } => {
                self.handle_send_symbol(uri, position, channel, byte_offset)
                    .await
            }
            RholangNode::Quote { quotable, .. } => {
                self.handle_quote_symbol(uri, position, quotable, byte_offset)
                    .await
            }
            RholangNode::Block { proc, .. } | RholangNode::Parenthesized { expr: proc, .. } => {
                // Block and Parenthesized are just wrappers, handle the inner expression
                debug!("Block/Parenthesized node encountered, checking inner expression");
                match &**proc {
                    RholangNode::Var { name, .. } => {
                        self.handle_var_symbol(uri, position, name, &path, &symbol_table)
                            .await
                    }
                    RholangNode::Contract { name, .. } => {
                        self.handle_contract_symbol(uri, position, name).await
                    }
                    RholangNode::Send { channel, .. } | RholangNode::SendSync { channel, .. } => {
                        self.handle_send_symbol(uri, position, channel, byte_offset)
                            .await
                    }
                    RholangNode::Quote { quotable, .. } => {
                        self.handle_quote_symbol(uri, position, quotable, byte_offset)
                            .await
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Handles symbol lookup for Var nodes
    async fn handle_var_symbol(
        &self,
        uri: &Url,
        position: LspPosition,
        name: &str,
        path: &[Arc<RholangNode>],
        symbol_table: &Arc<SymbolTable>,
    ) -> Option<Arc<Symbol>> {
        // Check if this Var is the name of a Contract
        if path.len() >= 2 {
            if let RholangNode::Contract { name: contract_name, .. } = &*path[path.len() - 2] {
                if let RholangNode::Var { name: var_name, .. } = &**contract_name {
                    if var_name == name {
                        // This Var is a contract name - handle as global symbol
                        debug!("Var '{}' is a contract name", name);
                        let workspace = self.workspace.read().await;
                        if let Some((def_uri, def_pos)) = workspace.global_symbols.get(name).cloned() {
                            debug!(
                                "Found global contract symbol '{}' at {}:{} in {}",
                                name, position.line, position.character, uri
                            );
                            return Some(Arc::new(Symbol {
                                name: name.to_string(),
                                symbol_type: SymbolType::Contract,
                                declaration_uri: def_uri.clone(),
                                declaration_location: def_pos,
                                definition_location: Some(def_pos),
                                contract_pattern: None,
                            }));
                        }
                    }
                }
            }
        }

        // Handle regular variables
        if let Some(symbol) = symbol_table.lookup(name) {
            debug!(
                "Found symbol '{}' at {}:{} in {}",
                name, position.line, position.character, uri
            );
            return Some(symbol);
        }

        // Search global symbols for unbound references
        let workspace = self.workspace.read().await;
        if let Some((def_uri, def_pos)) = workspace.global_symbols.get(name).cloned() {
            debug!(
                "Found global symbol '{}' for unbound reference at {}:{} in {}",
                name, position.line, position.character, uri
            );
            return Some(Arc::new(Symbol {
                name: name.to_string(),
                symbol_type: SymbolType::Contract,
                declaration_uri: def_uri.clone(),
                declaration_location: def_pos,
                definition_location: Some(def_pos),
                contract_pattern: None,
            }));
        }

        debug!(
            "Symbol '{}' at {}:{} in {} not found in symbol table or global",
            name, position.line, position.character, uri
        );
        None
    }

    /// Handles symbol lookup for Contract nodes
    async fn handle_contract_symbol(
        &self,
        uri: &Url,
        position: LspPosition,
        name: &Arc<RholangNode>,
    ) -> Option<Arc<Symbol>> {
        // Extract contract name (can be Var or StringLiteral)
        let contract_name = match &**name {
            RholangNode::Var { name, .. } => Some(name.clone()),
            RholangNode::StringLiteral { value, .. } => Some(value.clone()),
            _ => None,
        }?;

        let workspace = self.workspace.read().await;
        if let Some((def_uri, def_pos)) = workspace.global_symbols.get(&contract_name).cloned() {
            debug!(
                "Found contract symbol '{}' at {}:{} in {}",
                contract_name, position.line, position.character, uri
            );
            return Some(Arc::new(Symbol {
                name: contract_name.to_string(),
                symbol_type: SymbolType::Contract,
                declaration_uri: def_uri.clone(),
                declaration_location: def_pos,
                definition_location: Some(def_pos),
                contract_pattern: None,
            }));
        }

        None
    }

    /// Handles symbol lookup for Send/SendSync nodes (contract calls)
    async fn handle_send_symbol(
        &self,
        uri: &Url,
        position: LspPosition,
        channel: &Arc<RholangNode>,
        byte: usize,
    ) -> Option<Arc<Symbol>> {
        let workspace = self.workspace.read().await;
        let doc = workspace.documents.get(uri)?;

        // Check if position is within the channel node
        let channel_key = &**channel as *const RholangNode as usize;
        let (ch_start, ch_end) = doc.positions.get(&channel_key)?;

        debug!(
            "Send channel position: start={:?}, end={:?}, cursor={}",
            ch_start, ch_end, byte
        );

        if ch_start.byte <= byte && byte <= ch_end.byte {
            // Position is within the channel, extract the name
            if let RholangNode::Var { name: channel_name, .. } = &**channel {
                debug!("Send channel is Var '{}'", channel_name);
                if let Some((def_uri, def_pos)) = workspace.global_symbols.get(channel_name).cloned() {
                    debug!(
                        "Found global contract symbol '{}' for Send at {}:{} in {}",
                        channel_name, position.line, position.character, uri
                    );
                    return Some(Arc::new(Symbol {
                        name: channel_name.to_string(),
                        symbol_type: SymbolType::Contract,
                        declaration_uri: def_uri.clone(),
                        declaration_location: def_pos,
                        definition_location: Some(def_pos),
                        contract_pattern: None,
                    }));
                }
            }
        }

        None
    }

    /// Handles symbol lookup for Quote nodes (quoted contract references)
    async fn handle_quote_symbol(
        &self,
        uri: &Url,
        position: LspPosition,
        quotable: &Arc<RholangNode>,
        byte: usize,
    ) -> Option<Arc<Symbol>> {
        if let RholangNode::Var { name: quoted_name, .. } = &**quotable {
            let workspace = self.workspace.read().await;
            let doc = workspace.documents.get(uri)?;

            // Check if cursor is within the quoted variable
            let quotable_key = &**quotable as *const RholangNode as usize;
            let (q_start, q_end) = doc.positions.get(&quotable_key)?;

            debug!(
                "Quote content position: start={:?}, end={:?}, cursor={}",
                q_start, q_end, byte
            );

            if q_start.byte <= byte && byte <= q_end.byte {
                if let Some((def_uri, def_pos)) = workspace.global_symbols.get(quoted_name).cloned() {
                    debug!(
                        "Found global contract symbol '{}' for Quote at {}:{} in {}",
                        quoted_name, position.line, position.character, uri
                    );
                    return Some(Arc::new(Symbol {
                        name: quoted_name.to_string(),
                        symbol_type: SymbolType::Contract,
                        declaration_uri: def_uri.clone(),
                        declaration_location: def_pos,
                        definition_location: Some(def_pos),
                        contract_pattern: None,
                    }));
                }
            }
        }

        None
    }
}
