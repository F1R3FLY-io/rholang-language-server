//! Symbol operations for the LSP backend
//!
//! This module contains all symbol-related functionality including:
//! - Symbol table management and cross-workspace linking
//! - Symbol lookup at cursor positions
//! - Symbol references and usage tracking
//! - LSP symbol-related handlers (goto-definition, references, rename, etc.)

use std::collections::HashMap;
use std::sync::Arc;

use dashmap::DashMap;
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
        // Lock-free access via DashMap
        let doc_count = self.workspace.documents.len();
        let symbol_count = self.workspace.global_symbols.len();

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
        use std::time::Instant;
        let start_time = Instant::now();
        debug!("link_symbols: Starting symbol linking");

        // Lock-free document collection via DashMap
        // Clone global_table separately (still needs lock for consistency)
        let phase1_start = Instant::now();
        let global_table = self.workspace.global_table.read().await;

        // Collect all global contract symbols from workspace.global_table
        // The global_table was populated during indexing by SymbolTableBuilder,
        // which only inserts top-level contracts (is_top_level check at line 370 of symbol_table_builder.rs)
        let mut global_symbols_map = HashMap::new();

        // Get all contract symbols from the global table (lock-free iteration)
        for entry in global_table.symbols.iter() {
            let (name, symbol) = entry.pair();
            if matches!(symbol.symbol_type, crate::ir::symbol_table::SymbolType::Contract) {
                debug!("link_symbols: Adding global contract '{}' from {} at {:?}",
                       name, symbol.declaration_uri, symbol.declaration_location);
                global_symbols_map.insert(
                    name.clone(),
                    (symbol.declaration_uri.clone(), symbol.declaration_location),
                );
            }
        }
        drop(global_table); // Release lock early
        debug!("link_symbols: Phase 1 (collect global symbols) took {:?}", phase1_start.elapsed());

        debug!("link_symbols: Collected {} global contract symbols from global_table", global_symbols_map.len());
        debug!("link_symbols: Processing {} documents", self.workspace.documents.len());

        // Phase 2: Resolve potential global references (lock-free iteration)
        let phase2_start = Instant::now();
        let mut resolutions = Vec::new();
        for entry in self.workspace.documents.iter() {
            let doc_uri = entry.key();
            let doc = entry.value();

            for (name, use_pos) in &doc.potential_global_refs {
                if let Some((def_uri, def_pos)) = global_symbols_map.get(name).cloned() {
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
        debug!("link_symbols: Phase 2 (resolve cross-file refs) took {:?}, found {} resolutions",
               phase2_start.elapsed(), resolutions.len());

        // Phase 3: Build global inverted index from both cross-file refs and local refs
        let phase3_start = Instant::now();
        let mut global_inverted_index_map = HashMap::new();

        // First, add all cross-file global contract references
        for ((def_uri, def_pos), (use_uri, use_pos)) in resolutions {
            global_inverted_index_map
                .entry((def_uri, def_pos))
                .or_insert_with(Vec::new)
                .push((use_uri, use_pos));
        }

        // Second, merge in local inverted indexes from each document
        // These contain within-file symbol usages (new bindings, let bindings, etc.)
        for entry in self.workspace.documents.iter() {
            let doc_uri = entry.key();
            let doc = entry.value();

            debug!("Merging inverted index from {}: {} definitions", doc_uri, doc.inverted_index.len());

            // Each document's inverted_index maps local Position -> Vec<Position>
            // We need to convert to (Url, Position) -> Vec<(Url, Position)> format
            // IMPORTANT: Normalize byte offset to 0 for lookup compatibility
            for (def_pos, use_positions) in &doc.inverted_index {
                debug!("  Definition at {:?} has {} usages", def_pos, use_positions.len());
                let normalized_def_pos = IrPosition {
                    row: def_pos.row,
                    column: def_pos.column,
                    byte: 0, // Normalize to 0 for consistent lookups
                };
                let key = (doc_uri.clone(), normalized_def_pos);
                let uses: Vec<(Url, IrPosition)> = use_positions.iter()
                    .map(|use_pos| (doc_uri.clone(), *use_pos))
                    .collect();

                global_inverted_index_map
                    .entry(key)
                    .or_insert_with(Vec::new)
                    .extend(uses);
            }
        }
        debug!("link_symbols: Phase 3 (build inverted index) took {:?}", phase3_start.elapsed());

        debug!("Built global inverted index with {} definition entries", global_inverted_index_map.len());

        // Phase 4: Update global symbols (lock-free batch insert)
        let phase4_start = Instant::now();
        self.workspace.global_symbols.clear();
        for (name, location) in global_symbols_map {
            self.workspace.global_symbols.insert(name, location);
        }
        debug!("link_symbols: Phase 4 (update global symbols) took {:?}", phase4_start.elapsed());

        // Phase 5: Update global inverted index (lock-free with DashMap)
        let phase5_start = Instant::now();
        self.workspace.global_inverted_index.clear();
        for (key, value) in global_inverted_index_map {
            self.workspace.global_inverted_index.insert(key, value);
        }
        debug!("link_symbols: Phase 5 (populate inverted index) took {:?}", phase5_start.elapsed());

        let file_count = self.workspace.documents.len();
        let symbol_count = self.workspace.global_symbols.len();

        // Phase 6: Broadcast workspace change event (ReactiveX Phase 2)
        let phase6_start = Instant::now();
        let _ = self.workspace_changes.send(WorkspaceChangeEvent {
            file_count,
            symbol_count,
            change_type: WorkspaceChangeType::SymbolsLinked,
        });
        debug!("link_symbols: Phase 6 (broadcast event) took {:?}", phase6_start.elapsed());

        info!("link_symbols: Total time: {:?} for {} files, {} symbols",
              start_time.elapsed(), file_count, symbol_count);
    }

    /// Links symbols across all virtual documents in the workspace.
    ///
    /// This function:
    /// 1. Iterates through all documents in workspace to find their virtual documents
    /// 2. For each virtual document, builds/gets its symbol table
    /// 3. Collects all definition symbols organized by language
    /// 4. Updates the global_virtual_symbols table for cross-document navigation
    ///
    /// This enables goto-definition across all MeTTa (and other embedded language) virtual documents.
    pub(crate) async fn link_virtual_symbols(&self) {
        use tower_lsp::lsp_types::Range;

        // Get workspace document URIs (lock-free)
        let document_uris: Vec<_> = self.workspace.documents.iter()
            .map(|entry| entry.key().clone())
            .collect();

        // Collect symbols from all virtual documents, organized by language
        let mut global_symbols: HashMap<String, HashMap<String, Vec<(Url, Range)>>> = HashMap::new();
        let mut total_virtual_docs = 0;

        for parent_uri in &document_uris {
            // Get all virtual documents for this parent (each call acquires a read lock)
            let virtual_docs_for_parent = {
                let vdocs = self.virtual_docs.read().await;
                vdocs.get_by_parent(parent_uri)
            };

            if virtual_docs_for_parent.is_empty() {
                continue;
            }

            debug!("Linking symbols from {} virtual documents in {}",
                   virtual_docs_for_parent.len(), parent_uri);

            for virtual_doc in virtual_docs_for_parent {
                total_virtual_docs += 1;
                let language = virtual_doc.language.clone();

                // Get or build symbol table for this virtual document
                let symbol_table = match virtual_doc.get_or_build_symbol_table() {
                    Some(table) => table,
                    None => {
                        debug!("No symbol table available for virtual document: {}", virtual_doc.uri);
                        continue;
                    }
                };

                // Extract all definition symbols
                let definitions: Vec<_> = symbol_table.all_occurrences.iter()
                    .filter(|occ| occ.is_definition)
                    .collect();

                trace!("Found {} definitions in virtual document {}", definitions.len(), virtual_doc.uri);

                // Add definitions to global_symbols by language
                let lang_symbols = global_symbols.entry(language.clone()).or_insert_with(HashMap::new);

                for def in definitions {
                    lang_symbols
                        .entry(def.name.clone())
                        .or_insert_with(Vec::new)
                        .push((virtual_doc.uri.clone(), def.range));
                }
            }
        }

        // Update workspace with the collected symbols (lock-free)
        // Clear existing and insert new nested DashMaps
        self.workspace.global_virtual_symbols.clear();
        for (language, symbols_map) in global_symbols.iter() {
            let inner_map = Arc::new(DashMap::new());
            for (symbol_name, locations) in symbols_map {
                inner_map.insert(symbol_name.clone(), locations.clone());
            }
            self.workspace.global_virtual_symbols.insert(language.clone(), inner_map);
        }

        let total_symbols: usize = global_symbols.values()
            .map(|lang_map| lang_map.len())
            .sum();
        let lang_count = global_symbols.len();

        info!("Linked {} symbols across {} virtual documents in {} languages",
              total_symbols, total_virtual_docs, lang_count);

        // Log symbol counts per language
        for (lang, symbols) in &global_symbols {
            debug!("  {}: {} unique symbols", lang, symbols.len());
        }
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
        // Get document from workspace (lock-free)
        debug!("Lock-free document lookup for symbol at {}:{:?}", uri, position);
        let doc = self.workspace.documents.get(uri)?.value().clone();

        debug!("Document found for {}:{:?}", uri, position);
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
            // Lock-free document lookup
            let opt_doc = self.workspace.documents.get(uri).map(|entry| entry.value().clone());

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

        // Debug: log if we found a Var node
        if let RholangNode::Var { name, .. } = &*node {
            debug!("Var node '{}' found at requested byte offset: {}", name, byte_offset);
        }

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

                // Log the inner node type for debugging
                let inner_type = match &**proc {
                    RholangNode::Par { .. } => "Par",
                    RholangNode::Var { .. } => "Var",
                    RholangNode::Contract { .. } => "Contract",
                    RholangNode::Send { .. } => "Send",
                    RholangNode::SendSync { .. } => "SendSync",
                    RholangNode::Quote { .. } => "Quote",
                    other => {
                        debug!("Inner node type discriminant: {:?}", std::mem::discriminant(other));
                        "Other"
                    }
                };
                debug!("Inner node type: {}", inner_type);

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
                    RholangNode::Par { processes, .. } => {
                        // Par node contains parallel processes, need to find which one contains our position
                        // The problem is we don't have the positions map here, so we can't check
                        // Instead, let's try all Send nodes and let handle_send_symbol determine if it's the right one
                        debug!("Par node inside Block, searching through {} processes",
                               processes.as_ref().map(|p| p.len()).unwrap_or(0));

                        if let Some(procs) = processes {
                            for proc_node in procs.iter() {
                                let result = match &**proc_node {
                                    RholangNode::Send { channel, .. } | RholangNode::SendSync { channel, .. } => {
                                        self.handle_send_symbol(uri, position, channel, byte_offset).await
                                    }
                                    RholangNode::Var { name, .. } => {
                                        self.handle_var_symbol(uri, position, name, &path, &symbol_table).await
                                    }
                                    _ => None,
                                };

                                if result.is_some() {
                                    return result;
                                }
                            }
                        }

                        debug!("No matching process found in Par node");
                        None
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
                        // Lock-free global symbol lookup
                        if let Some(entry) = self.workspace.global_symbols.get(name) {
                            let (def_uri, def_pos) = entry.value().clone();
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

        // Search global symbols for unbound references (lock-free)
        if let Some(entry) = self.workspace.global_symbols.get(name) {
            let (def_uri, def_pos) = entry.value().clone();
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

        // Lock-free global symbol lookup
        if let Some(entry) = self.workspace.global_symbols.get(&contract_name) {
            let (def_uri, def_pos) = entry.value().clone();
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
        // Lock-free document lookup
        let doc = self.workspace.documents.get(uri)?.value().clone();

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
                // Lock-free global symbol lookup
                if let Some(entry) = self.workspace.global_symbols.get(channel_name) {
                    let (def_uri, def_pos) = entry.value().clone();
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
            // Lock-free document lookup
            let doc = self.workspace.documents.get(uri)?.value().clone();

            // Check if cursor is within the quoted variable
            let quotable_key = &**quotable as *const RholangNode as usize;
            let (q_start, q_end) = doc.positions.get(&quotable_key)?;

            debug!(
                "Quote content position: start={:?}, end={:?}, cursor={}",
                q_start, q_end, byte
            );

            if q_start.byte <= byte && byte <= q_end.byte {
                // Lock-free global symbol lookup
                if let Some(entry) = self.workspace.global_symbols.get(quoted_name) {
                    let (def_uri, def_pos) = entry.value().clone();
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
