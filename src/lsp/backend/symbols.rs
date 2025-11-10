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
        // Phase 6: Use rholang_symbols instead of global_symbols
        let doc_count = self.workspace.documents.len();
        let symbol_count = self.workspace.rholang_symbols.len();

        // If we have documents but no global symbols, we definitely need linking
        // This handles the race condition where documents are indexed but
        // the debounced symbol linker hasn't run yet
        let needs_linking = doc_count > 0 && symbol_count == 0;
        debug!("needs_symbol_linking: doc_count={}, symbol_count={}, needs={}",
               doc_count, symbol_count, needs_linking);
        needs_linking
    }

    /// Links symbols across all workspace files (Phase 4 refactored).
    ///
    /// Phase 4 Refactored: Simplified symbol linking using rholang_symbols
    ///
    /// Priority 2b: Simplified symbol linking - resolves forward references and broadcasts event.
    ///
    /// All symbols (both global and local) are now in rholang_symbols.
    /// No need to build global_inverted_index since all consumers now use rholang_symbols directly.
    ///
    /// This function resolves forward references by:
    /// 1. Collecting all contract declarations from rholang_symbols
    /// 2. Scanning all documents for references to those contracts
    /// 3. Adding any missing references (e.g., references that appeared before declaration)
    ///
    /// Removed (Priority 2b):
    /// - workspace.global_inverted_index (replaced by rholang_symbols)
    /// - per-document inverted_index (now in rholang_symbols with local keys)
    pub(crate) async fn link_symbols(&self) {
        debug!("link_symbols: Resolving forward references and broadcasting event");

        // Get all contract names from rholang_symbols
        let contract_names = self.workspace.rholang_symbols.contract_names();
        debug!("link_symbols: Found {} contracts to link", contract_names.len());

        // Iterate through all workspace documents to find unlinked references
        let document_uris: Vec<Url> = self.workspace.documents.iter()
            .map(|entry| entry.key().clone())
            .collect();

        use crate::lsp::rholang_contracts::SymbolLocation;
        let mut references_added = 0;

        for uri in &document_uris {
            // Get the document's IR and positions
            let doc_opt = self.workspace.documents.get(uri).map(|entry| entry.value().clone());
            if doc_opt.is_none() {
                continue;
            }
            let doc = doc_opt.unwrap();

            // Walk the IR tree to find all contract call references
            use crate::ir::rholang_node::RholangNode;
            fn collect_contract_references(
                node: &RholangNode,
                contract_names: &[String],
                uri: &Url,
                positions: &HashMap<usize, (IrPosition, IrPosition)>,
            ) -> Vec<(String, SymbolLocation)> {
                let mut refs = Vec::new();

                match node {
                    // Handle Send/SendSync - these are contract calls
                    RholangNode::Send { channel, inputs, .. } | RholangNode::SendSync { channel, inputs, .. } => {
                        // Check if channel is a Var that references a contract
                        if let RholangNode::Var { name, .. } = channel.as_ref() {
                            if contract_names.contains(name) {
                                // Get position of the Send node itself (the call site)
                                let node_key = node as *const RholangNode as usize;
                                if let Some((start, _)) = positions.get(&node_key) {
                                    refs.push((
                                        name.clone(),
                                        SymbolLocation::new(uri.clone(), *start)
                                    ));
                                }
                            }
                        }
                        // Also process arguments recursively
                        for arg in inputs.iter() {
                            refs.extend(collect_contract_references(arg, contract_names, uri, positions));
                        }
                    }
                    // Recursively process children in other node types
                    RholangNode::Par { processes, .. } => {
                        if let Some(procs) = processes {
                            for proc in procs.iter() {
                                refs.extend(collect_contract_references(proc, contract_names, uri, positions));
                            }
                        }
                    }
                    RholangNode::New { proc, .. } => {
                        refs.extend(collect_contract_references(proc, contract_names, uri, positions));
                    }
                    RholangNode::Contract { proc, .. } => {
                        refs.extend(collect_contract_references(proc, contract_names, uri, positions));
                    }
                    RholangNode::Block { proc, .. } => {
                        refs.extend(collect_contract_references(proc, contract_names, uri, positions));
                    }
                    RholangNode::Input { proc, .. } => {
                        refs.extend(collect_contract_references(proc, contract_names, uri, positions));
                    }
                    RholangNode::Match { cases, .. } => {
                        for (pattern, body) in cases.iter() {
                            refs.extend(collect_contract_references(pattern, contract_names, uri, positions));
                            refs.extend(collect_contract_references(body, contract_names, uri, positions));
                        }
                    }
                    _ => {}
                }

                refs
            }

            let contract_refs = collect_contract_references(&doc.ir, &contract_names, uri, &*doc.positions);

            // Add these references to rholang_symbols
            for (contract_name, ref_location) in contract_refs {
                // Try to add - it's OK if it already exists (add_reference deduplicates)
                if self.workspace.rholang_symbols.add_reference(&contract_name, ref_location).is_ok() {
                    references_added += 1;
                }
            }
        }

        debug!("link_symbols: Added {} forward references", references_added);

        let file_count = self.workspace.documents.len();
        let symbol_count = self.workspace.rholang_symbols.len();

        // Broadcast workspace change event
        let _ = self.workspace_changes.send(WorkspaceChangeEvent {
            file_count,
            symbol_count,
            change_type: WorkspaceChangeType::SymbolsLinked,
        });

        info!("link_symbols: Completed for {} files, {} symbols, {} forward references resolved",
              file_count, symbol_count, references_added);
    }

    /// Populates the workspace completion index with all symbols from the workspace.
    ///
    /// This method is called during workspace initialization (after indexing completes)
    /// to eagerly populate the completion index, eliminating the lazy initialization penalty.
    ///
    /// Populates from:
    /// - Keywords (always available)
    /// - Global symbol table
    /// - All document symbol tables
    pub(crate) async fn populate_completion_index(&self) {
        let start_time = std::time::Instant::now();
        debug!("populate_completion_index: Starting eager population");

        // Add keywords (always available)
        crate::lsp::features::completion::add_keywords(&self.workspace.completion_index);

        // Add symbols from global table
        let global_table = self.workspace.global_table.read().await;
        crate::lsp::features::completion::populate_from_symbol_table(
            &self.workspace.completion_index,
            &*global_table,
        );
        drop(global_table);

        // Add symbols from all document scopes with document tracking
        for entry in self.workspace.documents.iter() {
            let uri = entry.key();
            let doc = entry.value();
            crate::lsp::features::completion::populate_from_symbol_table_with_tracking(
                &self.workspace.completion_index,
                &doc.symbol_table,
                uri,
            );
        }

        let symbol_count = self.workspace.completion_index.len();
        let elapsed = start_time.elapsed();

        info!("populate_completion_index: Completed with {} symbols in {}ms",
              symbol_count, elapsed.as_millis());
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

                // Get the innermost symbol table by traversing the path from cursor to root
                // This ensures we respect lexical scoping - inner scopes shadow outer scopes
                let symbol_table = path_result
                    .as_ref()
                    .and_then(|(node, path)| {
                        // First check the node itself
                        if let Some(table) = node.metadata()
                            .and_then(|m| m.get("symbol_table"))
                            .and_then(|t| t.downcast_ref::<Arc<SymbolTable>>())
                            .cloned()
                        {
                            return Some(table);
                        }

                        // Then traverse the path backwards (from innermost to outermost)
                        // to find the nearest scope with a symbol table
                        for ancestor in path.iter().rev() {
                            if let Some(table) = ancestor.metadata()
                                .and_then(|m| m.get("symbol_table"))
                                .and_then(|t| t.downcast_ref::<Arc<SymbolTable>>())
                                .cloned()
                            {
                                debug!("Found symbol table in ancestor node");
                                return Some(table);
                            }
                        }

                        None
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
            RholangNode::Par { left, right, processes, .. } => {
                // Par node contains parallel processes. Since find_node_at_position didn't drill down past this Par,
                // we need to manually search through child processes to find which one contains the cursor.
                debug!("Par node at position (byte {}), checking child processes (n-ary={}, binary={})",
                       byte_offset, processes.is_some(), left.is_some() && right.is_some());

                // Get document to access position information
                let doc = self.workspace.documents.get(uri)?;
                let doc = doc.value().clone();

                // Handle n-ary Par (processes vector)
                if let Some(procs) = processes {
                    for (i, proc_node) in procs.iter().enumerate() {
                        // Check if this process node's position range contains the cursor
                        let proc_key = &**proc_node as *const RholangNode as usize;
                        if let Some((proc_start, proc_end)) = doc.positions.get(&proc_key) {
                            debug!("Par process[n-ary {}]: range byte {}-{}, cursor={}",
                                   i, proc_start.byte, proc_end.byte, byte_offset);

                            if proc_start.byte <= byte_offset && byte_offset <= proc_end.byte {
                                debug!("Par process[n-ary {}]: CONTAINS cursor, checking node type", i);

                                // This process contains the cursor, handle it
                                match &**proc_node {
                                    RholangNode::Send { channel, .. } | RholangNode::SendSync { channel, .. } => {
                                        // Check if the channel is a Var and extract its name
                                        if let RholangNode::Var { name, .. } = &**channel {
                                            debug!("Par process[n-ary {}]: Send with Var channel '{}', calling handle_var_symbol", i, name);
                                            return self.handle_var_symbol(uri, position, name, &path, &symbol_table).await;
                                        }
                                    }
                                    RholangNode::Var { name, .. } => {
                                        debug!("Par process[n-ary {}]: Var '{}', calling handle_var_symbol", i, name);
                                        return self.handle_var_symbol(uri, position, name, &path, &symbol_table).await;
                                    }
                                    _ => {
                                        debug!("Par process[n-ary {}]: node type not handled", i);
                                    }
                                }
                            }
                        } else {
                            debug!("Par process[n-ary {}]: no position information available", i);
                        }
                    }

                    // If cursor is before all children (in whitespace/indentation), check the first child
                    if let Some(first_proc) = procs.first() {
                        let first_key = &**first_proc as *const RholangNode as usize;
                        if let Some((first_start, _)) = doc.positions.get(&first_key) {
                            if byte_offset < first_start.byte {
                                debug!("Par: cursor at byte {} is before first child at byte {}, checking first child", byte_offset, first_start.byte);
                                match &**first_proc {
                                    RholangNode::Send { channel, .. } | RholangNode::SendSync { channel, .. } => {
                                        if let RholangNode::Var { name, .. } = &**channel {
                                            debug!("Par: first child is Send with Var channel '{}', calling handle_var_symbol", name);
                                            return self.handle_var_symbol(uri, position, name, &path, &symbol_table).await;
                                        }
                                    }
                                    RholangNode::Var { name, .. } => {
                                        debug!("Par: first child is Var '{}', calling handle_var_symbol", name);
                                        return self.handle_var_symbol(uri, position, name, &path, &symbol_table).await;
                                    }
                                    _ => {
                                        debug!("Par: first child node type not handled for whitespace cursor");
                                    }
                                }
                            }
                        }
                    }
                }
                // Handle binary Par (left/right)
                else if let (Some(left_node), Some(right_node)) = (left, right) {
                    // Check left node
                    let left_key = &**left_node as *const RholangNode as usize;
                    if let Some((left_start, left_end)) = doc.positions.get(&left_key) {
                        debug!("Par binary left: range byte {}-{}, cursor={}", left_start.byte, left_end.byte, byte_offset);

                        if left_start.byte <= byte_offset && byte_offset <= left_end.byte {
                            debug!("Par binary left: CONTAINS cursor, checking node type");
                            // Handle the left node based on its type
                            match &**left_node {
                                RholangNode::Send { channel, .. } | RholangNode::SendSync { channel, .. } => {
                                    // Check if the channel is a Var and extract its name
                                    if let RholangNode::Var { name, .. } = &**channel {
                                        debug!("Par binary left: Send with Var channel '{}', calling handle_var_symbol", name);
                                        return self.handle_var_symbol(uri, position, name, &path, &symbol_table).await;
                                    }
                                }
                                RholangNode::Var { name, .. } => {
                                    debug!("Par binary left: Var '{}', calling handle_var_symbol", name);
                                    return self.handle_var_symbol(uri, position, name, &path, &symbol_table).await;
                                }
                                _ => {
                                    debug!("Par binary left: node type not directly handled, falling through");
                                }
                            }
                        }
                    }

                    // Check right node
                    let right_key = &**right_node as *const RholangNode as usize;
                    if let Some((right_start, right_end)) = doc.positions.get(&right_key) {
                        debug!("Par binary right: range byte {}-{}, cursor={}", right_start.byte, right_end.byte, byte_offset);

                        if right_start.byte <= byte_offset && byte_offset <= right_end.byte {
                            debug!("Par binary right: CONTAINS cursor, checking node type");
                            // Handle the right node based on its type
                            match &**right_node {
                                RholangNode::Send { channel, .. } | RholangNode::SendSync { channel, .. } => {
                                    // Check if the channel is a Var and extract its name
                                    if let RholangNode::Var { name, .. } = &**channel {
                                        debug!("Par binary right: Send with Var channel '{}', calling handle_var_symbol", name);
                                        return self.handle_var_symbol(uri, position, name, &path, &symbol_table).await;
                                    }
                                }
                                RholangNode::Var { name, .. } => {
                                    debug!("Par binary right: Var '{}', calling handle_var_symbol", name);
                                    return self.handle_var_symbol(uri, position, name, &path, &symbol_table).await;
                                }
                                _ => {
                                    debug!("Par binary right: node type not directly handled, falling through");
                                }
                            }
                        }
                    }
                }

                debug!("No matching process found in Par node");
                None
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
                        // Check if the channel is a Var (local variable) or something else (global contract)
                        if let RholangNode::Var { name, .. } = &**channel {
                            debug!("Block: Send with Var channel '{}', calling handle_var_symbol", name);
                            self.handle_var_symbol(uri, position, name, &path, &symbol_table).await
                        } else {
                            self.handle_send_symbol(uri, position, channel, byte_offset).await
                        }
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
                                        // Check if the channel is a Var (local variable) or something else
                                        if let RholangNode::Var { name, .. } = &**channel {
                                            debug!("Par-in-Block: Send with Var channel '{}', calling handle_var_symbol", name);
                                            self.handle_var_symbol(uri, position, name, &path, &symbol_table).await
                                        } else {
                                            self.handle_send_symbol(uri, position, channel, byte_offset).await
                                        }
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
                        // Phase 5: Use rholang_symbols directly instead of global_symbols
                        if let Some(symbol_decl) = self.workspace.rholang_symbols.lookup(name) {
                            debug!(
                                "Found global contract symbol '{}' at {}:{} in {}",
                                name, position.line, position.character, uri
                            );
                            return Some(Arc::new(Symbol {
                                name: name.to_string(),
                                symbol_type: symbol_decl.symbol_type,
                                declaration_uri: symbol_decl.declaration.uri.clone(),
                                declaration_location: symbol_decl.declaration.position,
                                definition_location: symbol_decl.definition.as_ref().map(|d| d.position),
                                contract_pattern: None,
                                contract_identifier_node: None,
                                documentation: None,
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

        // Phase 5: Search rholang_symbols for unbound references (lock-free)
        if let Some(symbol_decl) = self.workspace.rholang_symbols.lookup(name) {
            debug!(
                "Found global symbol '{}' for unbound reference at {}:{} in {}",
                name, position.line, position.character, uri
            );
            return Some(Arc::new(Symbol {
                name: name.to_string(),
                symbol_type: symbol_decl.symbol_type,
                declaration_uri: symbol_decl.declaration.uri.clone(),
                declaration_location: symbol_decl.declaration.position,
                definition_location: symbol_decl.definition.as_ref().map(|d| d.position),
                contract_pattern: None,
                contract_identifier_node: None,
                documentation: None,
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

        // Phase 5: Use rholang_symbols directly instead of global_symbols
        if let Some(symbol_decl) = self.workspace.rholang_symbols.lookup(&contract_name) {
            debug!(
                "Found contract symbol '{}' at {}:{} in {}",
                contract_name, position.line, position.character, uri
            );
            return Some(Arc::new(Symbol {
                name: contract_name.to_string(),
                symbol_type: symbol_decl.symbol_type,
                declaration_uri: symbol_decl.declaration.uri.clone(),
                declaration_location: symbol_decl.declaration.position,
                definition_location: symbol_decl.definition.as_ref().map(|d| d.position),
                contract_pattern: None,
                contract_identifier_node: None,
                documentation: None,
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

        // Check if cursor is within the channel, OR if it's just before it (whitespace tolerance)
        let is_within_channel = ch_start.byte <= byte && byte <= ch_end.byte;
        let is_before_channel = byte < ch_start.byte && (ch_start.byte - byte) <= 10; // Allow up to 10 bytes of whitespace

        if is_within_channel || is_before_channel {
            if is_before_channel {
                debug!(
                    "Send: cursor at byte {} is before channel at byte {} (within whitespace tolerance), handling anyway",
                    byte, ch_start.byte
                );
            }

            // Position is within or just before the channel, extract the name
            if let RholangNode::Var { name: channel_name, .. } = &**channel {
                debug!("Send channel is Var '{}'", channel_name);
                // Phase 5: Use rholang_symbols directly instead of global_symbols
                if let Some(symbol_decl) = self.workspace.rholang_symbols.lookup(channel_name) {
                    debug!(
                        "Found global contract symbol '{}' for Send at {}:{} in {}",
                        channel_name, position.line, position.character, uri
                    );
                    return Some(Arc::new(Symbol {
                        name: channel_name.to_string(),
                        symbol_type: symbol_decl.symbol_type,
                        declaration_uri: symbol_decl.declaration.uri.clone(),
                        declaration_location: symbol_decl.declaration.position,
                        definition_location: symbol_decl.definition.as_ref().map(|d| d.position),
                        contract_pattern: None,
                        contract_identifier_node: None,
                        documentation: None,
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
                // Phase 5: Use rholang_symbols directly instead of global_symbols
                if let Some(symbol_decl) = self.workspace.rholang_symbols.lookup(quoted_name) {
                    debug!(
                        "Found global contract symbol '{}' for Quote at {}:{} in {}",
                        quoted_name, position.line, position.character, uri
                    );
                    return Some(Arc::new(Symbol {
                        name: quoted_name.to_string(),
                        symbol_type: symbol_decl.symbol_type,
                        declaration_uri: symbol_decl.declaration.uri.clone(),
                        declaration_location: symbol_decl.declaration.position,
                        definition_location: symbol_decl.definition.as_ref().map(|d| d.position),
                        contract_pattern: None,
                        contract_identifier_node: None,
                        documentation: None,
                    }));
                }
            }
        }

        None
    }
}
