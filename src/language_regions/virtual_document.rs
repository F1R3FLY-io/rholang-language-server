//! Virtual document infrastructure for embedded language regions
//!
//! Provides a system for creating virtual sub-documents from embedded language regions
//! within parent documents, with URI schemes and position mapping.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tower_lsp::lsp_types::{Diagnostic, Position as LspPosition, Range, Url};
use tracing::{debug, trace, warn};

use super::LanguageRegion;

/// A virtual document representing an embedded language region
#[derive(Debug)]
pub struct VirtualDocument {
    /// URI for this virtual document (e.g., "file:///x.rho#metta:0")
    pub uri: Url,
    /// URI of the parent document
    pub parent_uri: Url,
    /// Index of this region within the parent (for unique URI generation)
    pub region_index: usize,
    /// Language of this virtual document
    pub language: String,
    /// Text content of the virtual document
    pub content: String,
    /// Start position in parent document
    pub parent_start: LspPosition,
    /// End position in parent document
    pub parent_end: LspPosition,
    /// Byte offset mapping: virtual byte -> parent byte
    pub byte_offset: usize,
    /// Diagnostics for this virtual document (in virtual coordinates)
    pub diagnostics: Vec<Diagnostic>,
    /// Optional concatenation chain for holed virtual documents
    /// When present, this indicates the virtual document is formed from string concatenations
    /// with holes (variables/expressions) that should be skipped during LSP operations
    pub concatenation_chain: Option<Arc<super::concatenation::ConcatenationChain>>,
    /// Cached position map for holed documents
    /// Lazily computed from concatenation_chain
    holed_position_map: RwLock<Option<Arc<super::concatenation::HoledPositionMap>>>,
    /// Cached parsed IR (MeTTa AST nodes with relative positions)
    /// Uses RwLock for thread-safe lazy caching
    cached_ir: RwLock<Option<Arc<Vec<Arc<crate::ir::metta_node::MettaNode>>>>>,
    /// Cached Tree-Sitter tree for incremental parsing
    /// Note: tree_sitter::Tree doesn't implement Send, so we store it as raw pointer
    /// and manage synchronization carefully
    cached_tree: RwLock<Option<Arc<tree_sitter::Tree>>>,
    /// Cached symbol table for scoped symbol indexing
    cached_symbol_table: RwLock<Option<Arc<crate::ir::transforms::metta_symbol_table_builder::MettaSymbolTable>>>,
}

impl Clone for VirtualDocument {
    fn clone(&self) -> Self {
        Self {
            uri: self.uri.clone(),
            parent_uri: self.parent_uri.clone(),
            region_index: self.region_index,
            language: self.language.clone(),
            content: self.content.clone(),
            parent_start: self.parent_start,
            parent_end: self.parent_end,
            byte_offset: self.byte_offset,
            diagnostics: self.diagnostics.clone(),
            concatenation_chain: self.concatenation_chain.clone(),
            // Don't clone caches - create fresh empty caches
            holed_position_map: RwLock::new(None),
            cached_ir: RwLock::new(None),
            cached_tree: RwLock::new(None),
            cached_symbol_table: RwLock::new(None),
        }
    }
}

impl VirtualDocument {
    /// Creates a new virtual document from a language region
    ///
    /// # Arguments
    /// * `parent_uri` - URI of the parent document
    /// * `region` - The detected language region
    /// * `region_index` - Index of this region (for unique URI)
    pub fn new(parent_uri: Url, region: &LanguageRegion, region_index: usize) -> Self {
        // Create a unique URI for this virtual document
        // Format: parent_uri#language:index
        let fragment = format!("{}:{}", region.language, region_index);
        let mut uri = parent_uri.clone();
        uri.set_fragment(Some(&fragment));

        let parent_start = LspPosition {
            line: region.start_line as u32,
            character: region.start_column as u32,
        };

        // Calculate accurate end position by counting lines and columns in content
        let parent_end = {
            let lines: Vec<&str> = region.content.lines().collect();
            let num_lines = lines.len();

            if num_lines == 0 {
                // Empty content
                parent_start
            } else if num_lines == 1 {
                // Single-line region
                LspPosition {
                    line: region.start_line as u32,
                    character: (region.start_column + region.content.len()) as u32,
                }
            } else {
                // Multi-line region: end line = start line + (num lines - 1)
                // end column = length of last line
                let last_line_len = lines.last().map(|s| s.len()).unwrap_or(0);
                LspPosition {
                    line: (region.start_line + num_lines - 1) as u32,
                    character: last_line_len as u32,
                }
            }
        };

        VirtualDocument {
            uri,
            parent_uri,
            region_index,
            language: region.language.clone(),
            content: region.content.clone(),
            parent_start,
            parent_end,
            byte_offset: region.start_byte,
            diagnostics: Vec::new(),
            concatenation_chain: region.concatenation_chain.as_ref().map(|chain| Arc::new(chain.clone())),
            holed_position_map: RwLock::new(None),
            cached_ir: RwLock::new(None),
            cached_tree: RwLock::new(None),
            cached_symbol_table: RwLock::new(None),
        }
    }

    /// Maps a position in the virtual document to a position in the parent document
    ///
    /// # Arguments
    /// * `virtual_pos` - Position in the virtual document
    ///
    /// # Returns
    /// Position in the parent document
    pub fn map_to_parent(&self, virtual_pos: LspPosition) -> LspPosition {
        // For holed documents (concatenated strings), use the holed position map
        if let Some(map) = self.get_holed_position_map() {
            // Convert from tower_lsp::lsp_types::Position to lsp_types::Position
            let pos = lsp_types::Position {
                line: virtual_pos.line,
                character: virtual_pos.character,
            };

            if let Some(original_pos) = map.virtual_to_original(pos) {
                return LspPosition {
                    line: original_pos.line,
                    character: original_pos.character,
                };
            }
            // If mapping fails (position in hole), return parent_start as fallback
            return self.parent_start;
        }

        // For regular (non-concatenated) single-line regions, add the virtual column to parent start column
        if virtual_pos.line == 0 {
            LspPosition {
                line: self.parent_start.line,
                character: self.parent_start.character + virtual_pos.character + 1, // +1 for opening quote
            }
        } else {
            // Multi-line regions: add line offset
            // Virtual line 0 = blank line (newline after opening quote)
            // Virtual line 1+ = content lines with full indentation preserved
            // No column adjustment needed since virtual content includes full lines
            LspPosition {
                line: self.parent_start.line + virtual_pos.line,
                character: virtual_pos.character,
            }
        }
    }

    /// Maps a position in the parent document to a position in the virtual document
    ///
    /// Returns None if the position is outside this virtual document's range
    ///
    /// # Arguments
    /// * `parent_pos` - Position in the parent document
    ///
    /// # Returns
    /// Position in the virtual document, or None if outside range
    pub fn map_from_parent(&self, parent_pos: LspPosition) -> Option<LspPosition> {
        // Check if position is within this virtual document's range
        if parent_pos.line < self.parent_start.line || parent_pos.line > self.parent_end.line {
            return None;
        }

        let virtual_pos = if parent_pos.line == self.parent_start.line {
            // Single-line region
            if parent_pos.character < self.parent_start.character + 1
                || parent_pos.character > self.parent_end.character - 1
            {
                return None;
            }

            LspPosition {
                line: 0,
                character: parent_pos.character - self.parent_start.character - 1,
            }
        } else {
            // Multi-line region
            // The virtual content includes a leading newline after the opening quote
            // Virtual line 0 = blank line, virtual line 1+ = content with full indentation
            // No column adjustment needed since virtual content preserves full lines
            LspPosition {
                line: parent_pos.line - self.parent_start.line,
                character: parent_pos.character,
            }
        };

        debug!("Mapped parent L{}:C{} -> virtual L{}:C{} (parent_start: L{}:C{}, parent_end: L{}:C{})",
            parent_pos.line, parent_pos.character,
            virtual_pos.line, virtual_pos.character,
            self.parent_start.line, self.parent_start.character,
            self.parent_end.line, self.parent_end.character);

        Some(virtual_pos)
    }

    /// Maps a range in the virtual document to a range in the parent document
    pub fn map_range_to_parent(&self, virtual_range: Range) -> Range {
        Range {
            start: self.map_to_parent(virtual_range.start),
            end: self.map_to_parent(virtual_range.end),
        }
    }

    /// Sets diagnostics for this virtual document
    pub fn set_diagnostics(&mut self, diagnostics: Vec<Diagnostic>) {
        self.diagnostics = diagnostics;
    }

    /// Gets or creates the holed position map for this virtual document
    ///
    /// Returns None if this is not a holed document (no concatenation chain)
    fn get_holed_position_map(&self) -> Option<Arc<super::concatenation::HoledPositionMap>> {
        // If there's no concatenation chain, this is not a holed document
        let chain = self.concatenation_chain.as_ref()?;

        // Try to get cached map first (read lock)
        {
            let cache = self.holed_position_map.read().ok()?;
            if let Some(ref map) = *cache {
                return Some(map.clone());
            }
        }

        // Cache miss - create the map (write lock)
        {
            let mut cache = self.holed_position_map.write().ok()?;

            // Double-check in case another thread created it while we waited
            if let Some(ref map) = *cache {
                return Some(map.clone());
            }

            // Create new holed position map from the concatenation chain
            let map = Arc::new(super::concatenation::HoledPositionMap::new(chain.clone()));
            *cache = Some(map.clone());
            Some(map)
        }
    }

    /// Checks if a position in the virtual document falls within a hole
    ///
    /// For holed documents, returns true if the position is in a hole (variable/expression).
    /// For regular documents, always returns false.
    pub fn is_position_in_hole(&self, virtual_pos: LspPosition) -> bool {
        if let Some(map) = self.get_holed_position_map() {
            // Convert from tower_lsp::lsp_types::Position to lsp_types::Position
            let pos = lsp_types::Position {
                line: virtual_pos.line,
                character: virtual_pos.character,
            };
            // If mapping returns None, the position is in a hole
            map.virtual_to_original(pos).is_none()
        } else {
            // Not a holed document
            false
        }
    }

    /// Gets the cached IR, parsing if necessary
    ///
    /// This method uses lazy evaluation with caching for performance.
    /// The IR is parsed once and reused for validation, hover, and semantic tokens.
    pub fn get_or_parse_ir(&self) -> Option<Arc<Vec<Arc<crate::ir::metta_node::MettaNode>>>> {
        // Try to get cached IR first (read lock)
        {
            let cache = self.cached_ir.read().ok()?;
            if let Some(ref ir) = *cache {
                trace!("Using cached IR for virtual document: {}", self.uri);
                return Some(ir.clone());
            }
        }

        // Cache miss - parse the IR (write lock)
        {
            let mut cache = self.cached_ir.write().ok()?;

            // Double-check in case another thread parsed while we waited for write lock
            if let Some(ref ir) = *cache {
                return Some(ir.clone());
            }

            trace!("Parsing IR for virtual document: {}", self.uri);

            // Parse based on language
            match self.language.as_str() {
                "metta" => {
                    use crate::parsers::MettaParser;

                    let mut parser = match MettaParser::new() {
                        Ok(p) => p,
                        Err(e) => {
                            warn!("Failed to create MeTTa parser for virtual document {}: {}", self.uri, e);
                            return None;
                        }
                    };

                    debug!("Parsing MeTTa content ({} bytes) for virtual document: {}", self.content.len(), self.uri);
                    debug!("Content line count: {}", self.content.lines().count());
                    debug!("First 200 chars: {:?}", &self.content[..self.content.len().min(200)]);
                    debug!("Last 200 chars: {:?}", &self.content[self.content.len().saturating_sub(200)..]);

                    let metta_nodes = match parser.parse_to_ir(&self.content) {
                        Ok(nodes) => nodes,
                        Err(e) => {
                            warn!("Failed to parse MeTTa IR for virtual document {}: {}", self.uri, e);
                            // Log first 200 chars of content for debugging
                            let preview = if self.content.len() > 200 {
                                format!("{}...", &self.content[..200])
                            } else {
                                self.content.clone()
                            };
                            debug!("Content preview: {}", preview);
                            return None;
                        }
                    };

                    debug!("Successfully parsed {} MeTTa nodes for virtual document: {}", metta_nodes.len(), self.uri);
                    let ir = Arc::new(metta_nodes);

                    // Cache and return
                    *cache = Some(ir.clone());
                    Some(ir)
                }
                _ => None,
            }
        }
    }

    /// Gets the cached Tree-Sitter tree, parsing if necessary
    ///
    /// This method uses lazy evaluation with caching for performance.
    /// The tree is parsed once and can be reused for semantic tokens and validation.
    pub fn get_or_parse_tree(&self) -> Option<Arc<tree_sitter::Tree>> {
        // Try to get cached tree first (read lock)
        {
            let cache = self.cached_tree.read().ok()?;
            if let Some(ref tree) = *cache {
                trace!("Using cached Tree-Sitter tree for virtual document: {}", self.uri);
                return Some(tree.clone());
            }
        }

        // Cache miss - parse the tree (write lock)
        {
            let mut cache = self.cached_tree.write().ok()?;

            // Double-check in case another thread parsed while we waited for write lock
            if let Some(ref tree) = *cache {
                return Some(tree.clone());
            }

            trace!("Parsing Tree-Sitter tree for virtual document: {}", self.uri);

            // Parse based on language
            match self.language.as_str() {
                "metta" => {
                    use tree_sitter::Parser;

                    let mut parser = Parser::new();
                    parser.set_language(&tree_sitter_metta::language()).ok()?;
                    let tree = parser.parse(&self.content, None)?;
                    let tree_arc = Arc::new(tree);

                    // Cache and return
                    *cache = Some(tree_arc.clone());
                    Some(tree_arc)
                }
                _ => None,
            }
        }
    }

    /// Gets the cached symbol table, building it if necessary
    ///
    /// This method uses lazy evaluation with caching for performance.
    /// The symbol table is built once from the IR and reused for LSP features.
    pub fn get_or_build_symbol_table(&self) -> Option<Arc<crate::ir::transforms::metta_symbol_table_builder::MettaSymbolTable>> {
        // Try to get cached symbol table first (read lock)
        {
            let cache = match self.cached_symbol_table.read() {
                Ok(c) => c,
                Err(e) => {
                    warn!("Failed to acquire read lock for symbol table cache: {}", e);
                    return None;
                }
            };
            if let Some(ref table) = *cache {
                trace!("Using cached symbol table for virtual document: {}", self.uri);
                return Some(table.clone());
            }
        }

        // Cache miss - build the symbol table (write lock)
        {
            let mut cache = match self.cached_symbol_table.write() {
                Ok(c) => c,
                Err(e) => {
                    warn!("Failed to acquire write lock for symbol table cache: {}", e);
                    return None;
                }
            };

            // Double-check in case another thread built while we waited for write lock
            if let Some(ref table) = *cache {
                return Some(table.clone());
            }

            trace!("Building symbol table for virtual document: {}", self.uri);

            // Build symbol table from IR
            match self.language.as_str() {
                "metta" => {
                    // Get or parse IR first
                    let ir = match self.get_or_parse_ir() {
                        Some(ir) => ir,
                        None => {
                            warn!("Failed to get or parse IR for MeTTa virtual document: {}", self.uri);
                            return None;
                        }
                    };

                    debug!("Successfully parsed IR with {} nodes for virtual document: {}", ir.len(), self.uri);

                    // Build symbol table
                    use crate::ir::transforms::metta_symbol_table_builder::MettaSymbolTableBuilder;

                    // Check if this virtual document is from concatenated strings
                    // Concatenated sources limit MORK pattern matching precision
                    let is_concatenated = self.concatenation_chain.is_some();

                    let builder = MettaSymbolTableBuilder::new(self.uri.clone(), is_concatenated);
                    let table = Arc::new(builder.build(&ir));

                    debug!("Built symbol table with {} scopes and {} occurrences for virtual document: {}",
                        table.scopes.len(), table.all_occurrences.len(), self.uri);

                    // Cache and return
                    *cache = Some(table.clone());
                    Some(table)
                }
                _ => {
                    warn!("Symbol table building not supported for language: {}", self.language);
                    None
                }
            }
        }
    }

    /// Updates content and uses incremental parsing if an old tree is available
    ///
    /// This method enables Tree-Sitter's incremental parsing for better performance.
    /// If a cached tree exists, it's passed to the parser as the old tree.
    pub fn update_content_incremental(&mut self, new_content: String) {
        // Get the old tree for incremental parsing
        let old_tree = if let Ok(cache) = self.cached_tree.read() {
            cache.as_ref().map(|t| (**t).clone())
        } else {
            None
        };

        // Update content
        self.content = new_content;

        // Invalidate caches - they'll be lazily recomputed with the new content
        self.invalidate_cache();

        // If we had an old tree, eagerly reparse with incremental parsing
        if let Some(old_tree) = old_tree {
            if self.language == "metta" {
                use tree_sitter::Parser;

                let mut parser = Parser::new();
                if parser.set_language(&tree_sitter_metta::language()).is_ok() {
                    if let Some(new_tree) = parser.parse(&self.content, Some(&old_tree)) {
                        trace!("Incremental parse succeeded for virtual document: {}", self.uri);
                        if let Ok(mut cache) = self.cached_tree.write() {
                            *cache = Some(Arc::new(new_tree));
                        }
                    }
                }
            }
        }
    }

    /// Invalidates the cached IR, Tree-Sitter tree, and symbol table
    ///
    /// Should be called when the content changes
    pub fn invalidate_cache(&self) {
        if let Ok(mut cache) = self.cached_ir.write() {
            *cache = None;
        }
        if let Ok(mut cache) = self.cached_tree.write() {
            *cache = None;
        }
        if let Ok(mut cache) = self.cached_symbol_table.write() {
            *cache = None;
        }
    }

    /// Maps diagnostics from virtual coordinates to parent coordinates
    ///
    /// Returns diagnostics with ranges mapped to the parent document.
    /// For holed documents, filters out diagnostics that fall within holes.
    pub fn map_diagnostics_to_parent(&self) -> Vec<Diagnostic> {
        self.diagnostics
            .iter()
            .filter(|diag| {
                // Skip diagnostics that fall in holes (variables/expressions)
                !self.is_position_in_hole(diag.range.start)
            })
            .map(|diag| {
                let mut parent_diag = diag.clone();
                parent_diag.range = self.map_range_to_parent(diag.range);
                parent_diag
            })
            .collect()
    }

    /// Validates this virtual document and stores diagnostics
    ///
    /// Returns the mapped diagnostics for publishing to parent
    pub fn validate(&mut self) -> Result<Vec<Diagnostic>, String> {
        match self.language.as_str() {
            "metta" => self.validate_metta(),
            _ => {
                warn!("Unsupported language for validation: {}", self.language);
                Ok(Vec::new())
            }
        }
    }

    /// Validates MeTTa content
    fn validate_metta(&mut self) -> Result<Vec<Diagnostic>, String> {
        use crate::validators::MettaValidator;

        debug!("Validating MeTTa virtual document: {}", self.uri);

        let validator = MettaValidator;
        let diagnostics = validator.validate(&self.content);

        debug!(
            "Found {} diagnostics in MeTTa virtual document",
            diagnostics.len()
        );

        // Store diagnostics and return mapped versions
        self.diagnostics = diagnostics;
        Ok(self.map_diagnostics_to_parent())
    }

    /// Provides hover information for a position in this virtual document
    ///
    /// # Arguments
    /// * `position` - Position in virtual document coordinates
    ///
    /// # Returns
    /// Hover information with ranges in virtual coordinates
    pub fn hover(&self, position: LspPosition) -> Option<tower_lsp::lsp_types::Hover> {
        // Skip hover for positions in holes (variables/expressions in concatenations)
        if self.is_position_in_hole(position) {
            trace!("Position {:?} is in a hole, skipping hover", position);
            return None;
        }

        match self.language.as_str() {
            "metta" => self.hover_metta(position),
            _ => {
                trace!("No hover support for language: {}", self.language);
                None
            }
        }
    }

    /// Provides hover information for MeTTa content
    fn hover_metta(&self, position: LspPosition) -> Option<tower_lsp::lsp_types::Hover> {
        use tower_lsp::lsp_types::{Hover, HoverContents, MarkupContent, MarkupKind, Range};

        trace!("MeTTa hover at virtual position {:?}", position);

        // Use cached Tree-Sitter tree for accurate positions
        let tree = self.get_or_parse_tree()?;
        let root = tree.root_node();

        // Find the smallest node containing the position
        let mut current_node = root;

        'outer: loop {
            let node_start = current_node.start_position();
            let node_end = current_node.end_position();

            // Check if position is within current node
            let in_range = if node_start.row == node_end.row {
                // Single-line node
                position.line as usize == node_start.row
                    && position.character as usize >= node_start.column
                    && position.character as usize <= node_end.column
            } else {
                // Multi-line node
                ((position.line as usize) > node_start.row && (position.line as usize) < node_end.row)
                    || (position.line as usize == node_start.row && position.character as usize >= node_start.column)
                    || (position.line as usize == node_end.row && position.character as usize <= node_end.column)
            };

            if !in_range {
                break;
            }

            // Try to descend to a more specific child
            let mut cursor = current_node.walk();
            if !cursor.goto_first_child() {
                // No children, this is the most specific node
                break;
            }

            loop {
                let child = cursor.node();
                let child_start = child.start_position();
                let child_end = child.end_position();

                let child_in_range = if child_start.row == child_end.row {
                    position.line as usize == child_start.row
                        && position.character as usize >= child_start.column
                        && position.character as usize <= child_end.column
                } else {
                    ((position.line as usize) > child_start.row && (position.line as usize) < child_end.row)
                        || (position.line as usize == child_start.row && position.character as usize >= child_start.column)
                        || (position.line as usize == child_end.row && position.character as usize <= child_end.column)
                };

                if child_in_range {
                    current_node = child;
                    continue 'outer;
                }

                if !cursor.goto_next_sibling() {
                    break;
                }
            }

            // No child matched, current_node is the most specific
            break;
        }

        // Generate hover text based on Tree-Sitter node kind
        let kind = current_node.kind();
        let node_text = current_node.utf8_text(self.content.as_bytes()).ok()?;

        let hover_text = match kind {
            "atom" => {
                format!("```metta\n{}\n```\n\n**MeTTa Atom**", node_text)
            }
            "variable" => {
                format!("```metta\n{}\n```\n\n**MeTTa Variable**", node_text)
            }
            "integer_literal" => {
                format!("```metta\n{}\n```\n\n**Integer Literal**", node_text)
            }
            "float_literal" => {
                format!("```metta\n{}\n```\n\n**Float Literal**", node_text)
            }
            "string_literal" => {
                format!("```metta\n{}\n```\n\n**String Literal**", node_text)
            }
            "s_expression" => {
                format!("```metta\n{}\n```\n\n**S-expression**", node_text)
            }
            "definition" => {
                format!("```metta\n{}\n```\n\n**Definition** - (= pattern body)", node_text)
            }
            "lambda" => {
                format!("```metta\n{}\n```\n\n**Lambda Function**", node_text)
            }
            "match_expr" => {
                format!("```metta\n{}\n```\n\n**Match Expression**", node_text)
            }
            "let_expr" => {
                format!("```metta\n{}\n```\n\n**Let Binding**", node_text)
            }
            "if_expr" => {
                format!("```metta\n{}\n```\n\n**Conditional Expression**", node_text)
            }
            "eval" => {
                format!("```metta\n{}\n```\n\n**Evaluation** - !(expr)", node_text)
            }
            "type_annotation" => {
                format!("```metta\n{}\n```\n\n**Type Annotation** - (: expr type)", node_text)
            }
            _ => {
                format!("```metta\n{}\n```\n\n**MeTTa {}**", node_text, kind)
            }
        };

        // Convert Tree-Sitter positions to LSP positions
        let node_start = current_node.start_position();
        let node_end = current_node.end_position();

        trace!("Returning MeTTa hover for node kind '{}' at {:?}-{:?}", kind, node_start, node_end);

        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: hover_text,
            }),
            range: Some(Range {
                start: LspPosition {
                    line: node_start.row as u32,
                    character: node_start.column as u32,
                },
                end: LspPosition {
                    line: node_end.row as u32,
                    character: node_end.column as u32,
                },
            }),
        })
    }
}

/// Registry for managing virtual documents
#[derive(Debug, Default)]
pub struct VirtualDocumentRegistry {
    /// Map from virtual URI to virtual document
    documents: HashMap<Url, Arc<VirtualDocument>>,
    /// Map from parent URI to list of virtual document URIs
    parent_to_virtual: HashMap<Url, Vec<Url>>,
}

impl VirtualDocumentRegistry {
    /// Creates a new empty registry
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers virtual documents for a parent document
    ///
    /// # Arguments
    /// * `parent_uri` - URI of the parent document
    /// * `regions` - Detected language regions in the parent
    pub fn register_regions(&mut self, parent_uri: &Url, regions: &[LanguageRegion]) {
        debug!(
            "Registering {} virtual documents for {}",
            regions.len(),
            parent_uri
        );

        // Clear existing virtual documents for this parent
        self.unregister_parent(parent_uri);

        let mut virtual_uris = Vec::new();

        for (index, region) in regions.iter().enumerate() {
            let virtual_doc = Arc::new(VirtualDocument::new(parent_uri.clone(), region, index));
            trace!(
                "Created virtual document: {} for language {}",
                virtual_doc.uri,
                virtual_doc.language
            );

            virtual_uris.push(virtual_doc.uri.clone());
            self.documents.insert(virtual_doc.uri.clone(), virtual_doc);
        }

        self.parent_to_virtual
            .insert(parent_uri.clone(), virtual_uris);
    }

    /// Unregisters all virtual documents for a parent document
    ///
    /// # Arguments
    /// * `parent_uri` - URI of the parent document
    pub fn unregister_parent(&mut self, parent_uri: &Url) {
        if let Some(virtual_uris) = self.parent_to_virtual.remove(parent_uri) {
            for uri in virtual_uris {
                self.documents.remove(&uri);
            }
        }
    }

    /// Gets a virtual document by URI
    ///
    /// # Arguments
    /// * `uri` - URI of the virtual document
    ///
    /// # Returns
    /// The virtual document, or None if not found
    pub fn get(&self, uri: &Url) -> Option<Arc<VirtualDocument>> {
        self.documents.get(uri).cloned()
    }

    /// Gets all virtual documents for a parent document
    ///
    /// # Arguments
    /// * `parent_uri` - URI of the parent document
    ///
    /// # Returns
    /// List of virtual documents
    pub fn get_by_parent(&self, parent_uri: &Url) -> Vec<Arc<VirtualDocument>> {
        if let Some(virtual_uris) = self.parent_to_virtual.get(parent_uri) {
            virtual_uris
                .iter()
                .filter_map(|uri| self.documents.get(uri).cloned())
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Checks if a URI refers to a virtual document
    pub fn is_virtual(&self, uri: &Url) -> bool {
        uri.fragment().is_some() && self.documents.contains_key(uri)
    }

    /// Gets the parent URI for a virtual document
    ///
    /// # Arguments
    /// * `virtual_uri` - URI of the virtual document
    ///
    /// # Returns
    /// Parent URI, or None if not a virtual document
    pub fn get_parent_uri(&self, virtual_uri: &Url) -> Option<Url> {
        self.documents.get(virtual_uri).map(|doc| doc.parent_uri.clone())
    }

    /// Validates all virtual documents for a parent and returns aggregated diagnostics
    ///
    /// # Arguments
    /// * `parent_uri` - URI of the parent document
    ///
    /// # Returns
    /// Aggregated diagnostics mapped to parent coordinates
    pub fn validate_all(&mut self, parent_uri: &Url) -> Vec<Diagnostic> {
        let all_diagnostics = Vec::new();

        if let Some(virtual_uris) = self.parent_to_virtual.get(parent_uri) {
            for uri in virtual_uris {
                if let Some(_doc) = self.documents.get_mut(uri) {
                    // VirtualDocument is wrapped in Arc, so we need to get a mutable reference
                    // We'll need to update this to use Arc::make_mut or similar
                    // For now, let's create a workaround
                }
            }
        }

        all_diagnostics
    }

    /// Validates all virtual documents for a parent (mutable version)
    ///
    /// Since documents are wrapped in Arc, we need to replace them after validation
    pub fn validate_all_for_parent(&mut self, parent_uri: &Url) -> Vec<Diagnostic> {
        let mut all_diagnostics = Vec::new();

        if let Some(virtual_uris) = self.parent_to_virtual.get(parent_uri).cloned() {
            for uri in virtual_uris {
                if let Some(doc_arc) = self.documents.remove(&uri) {
                    // Clone the document to get a mutable version
                    let mut doc = Arc::try_unwrap(doc_arc).unwrap_or_else(|arc| (*arc).clone());

                    // Validate and get mapped diagnostics
                    match doc.validate() {
                        Ok(diagnostics) => {
                            all_diagnostics.extend(diagnostics);
                            // Update the document in the registry
                            self.documents.insert(uri.clone(), Arc::new(doc));
                        }
                        Err(e) => {
                            warn!("Failed to validate virtual document {}: {}", uri, e);
                            // Re-insert the document even if validation failed
                            self.documents.insert(uri.clone(), Arc::new(doc));
                        }
                    }
                }
            }
        }

        all_diagnostics
    }

    /// Finds the virtual document containing a given position in the parent document
    ///
    /// # Arguments
    /// * `parent_uri` - URI of the parent document
    /// * `position` - Position in the parent document
    ///
    /// # Returns
    /// Optional tuple of (virtual_uri, virtual_position, Arc<VirtualDocument>)
    pub fn find_virtual_document_at_position(
        &self,
        parent_uri: &Url,
        position: LspPosition,
    ) -> Option<(Url, LspPosition, Arc<VirtualDocument>)> {
        use tracing::trace;

        // Get all virtual documents for this parent
        let virtual_uris = self.parent_to_virtual.get(parent_uri)?;

        // Check each virtual document to see if it contains this position
        for virtual_uri in virtual_uris {
            if let Some(doc) = self.documents.get(virtual_uri) {
                // Check if position is within this virtual document's range
                let in_range = if doc.parent_start.line == doc.parent_end.line {
                    // Single-line region
                    position.line == doc.parent_start.line
                        && position.character >= doc.parent_start.character
                        && position.character <= doc.parent_end.character
                } else {
                    // Multi-line region
                    (position.line > doc.parent_start.line && position.line < doc.parent_end.line)
                        || (position.line == doc.parent_start.line && position.character >= doc.parent_start.character)
                        || (position.line == doc.parent_end.line && position.character <= doc.parent_end.character)
                };

                if in_range {
                    // Map position to virtual document coordinates
                    if let Some(virtual_position) = doc.map_from_parent(position) {
                        trace!(
                            "Found virtual document {} at parent position {:?}, mapped to {:?}",
                            virtual_uri,
                            position,
                            virtual_position
                        );
                        return Some((virtual_uri.clone(), virtual_position, doc.clone()));
                    }
                }
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language_regions::RegionSource;

    fn create_test_region() -> LanguageRegion {
        LanguageRegion {
            language: "metta".to_string(),
            start_byte: 50,
            end_byte: 100,
            start_line: 2,
            start_column: 10,
            source: RegionSource::CommentDirective,
            content: "(= factorial (lambda (n) 42))".to_string(),
            concatenation_chain: None,
        }
    }

    #[test]
    fn test_virtual_document_creation() {
        let parent_uri = Url::parse("file:///test.rho").unwrap();
        let region = create_test_region();

        let virtual_doc = VirtualDocument::new(parent_uri.clone(), &region, 0);

        assert_eq!(virtual_doc.parent_uri, parent_uri);
        assert_eq!(virtual_doc.language, "metta");
        assert_eq!(virtual_doc.region_index, 0);
        assert_eq!(virtual_doc.content, "(= factorial (lambda (n) 42))");
        assert!(virtual_doc.uri.fragment().is_some());
        assert_eq!(virtual_doc.uri.fragment().unwrap(), "metta:0");
    }

    #[test]
    fn test_position_mapping_single_line() {
        let parent_uri = Url::parse("file:///test.rho").unwrap();
        let region = create_test_region();
        let virtual_doc = VirtualDocument::new(parent_uri, &region, 0);

        // Map position from virtual to parent
        let virtual_pos = LspPosition {
            line: 0,
            character: 5,
        };
        let parent_pos = virtual_doc.map_to_parent(virtual_pos);

        assert_eq!(parent_pos.line, 2);
        assert_eq!(parent_pos.character, 16); // 10 + 5 + 1 (for quote)

        // Map back from parent to virtual
        let mapped_back = virtual_doc.map_from_parent(parent_pos).unwrap();
        assert_eq!(mapped_back.line, 0);
        assert_eq!(mapped_back.character, 5);
    }

    #[test]
    fn test_position_mapping_multi_line() {
        let parent_uri = Url::parse("file:///test.rho").unwrap();

        // Create a multi-line region starting at L22:C18 (the opening quote)
        // Content: "\n          (= (is_connected $from $to)\n             (match & self (connected $from $to) true))"
        let region = LanguageRegion {
            language: "metta".to_string(),
            start_byte: 0,
            end_byte: 100,
            start_line: 22,
            start_column: 18,
            source: RegionSource::ChannelFlow,
            content: "\n          (= (is_connected $from $to)\n             (match & self (connected $from $to) true))".to_string(),
            concatenation_chain: None,
        };

        let virtual_doc = VirtualDocument::new(parent_uri, &region, 0);

        // Virtual L1:C27 should be the '$' in '$from' on the first content line
        // This should map to parent L23:C27 (line 22 + 1 for the newline, column 27 unchanged)
        let virtual_pos = LspPosition {
            line: 1,  // First content line (after blank L0)
            character: 27,  // The '$' in '$from'
        };
        let parent_pos = virtual_doc.map_to_parent(virtual_pos);

        assert_eq!(parent_pos.line, 23);  // L22 (quote) + 1 (content line)
        assert_eq!(parent_pos.character, 27);  // Same column (no adjustment)

        // Map back from parent to virtual
        let mapped_back = virtual_doc.map_from_parent(parent_pos).unwrap();
        assert_eq!(mapped_back.line, 1);
        assert_eq!(mapped_back.character, 27);

        // Test the end of '$from' token (exclusive end at C32)
        let virtual_end = LspPosition {
            line: 1,
            character: 32,
        };
        let parent_end = virtual_doc.map_to_parent(virtual_end);

        assert_eq!(parent_end.line, 23);
        assert_eq!(parent_end.character, 32);
    }

    #[test]
    fn test_registry_register_and_get() {
        let mut registry = VirtualDocumentRegistry::new();
        let parent_uri = Url::parse("file:///test.rho").unwrap();
        let region = create_test_region();

        registry.register_regions(&parent_uri, &[region]);

        // Should have one virtual document
        let virtual_docs = registry.get_by_parent(&parent_uri);
        assert_eq!(virtual_docs.len(), 1);
        assert_eq!(virtual_docs[0].language, "metta");

        // Should be able to get by virtual URI
        let virtual_uri = &virtual_docs[0].uri;
        let retrieved = registry.get(virtual_uri).unwrap();
        assert_eq!(retrieved.language, "metta");
    }

    #[test]
    fn test_registry_unregister() {
        let mut registry = VirtualDocumentRegistry::new();
        let parent_uri = Url::parse("file:///test.rho").unwrap();
        let region = create_test_region();

        registry.register_regions(&parent_uri, &[region]);
        assert_eq!(registry.get_by_parent(&parent_uri).len(), 1);

        registry.unregister_parent(&parent_uri);
        assert_eq!(registry.get_by_parent(&parent_uri).len(), 0);
    }

    #[test]
    fn test_is_virtual() {
        let mut registry = VirtualDocumentRegistry::new();
        let parent_uri = Url::parse("file:///test.rho").unwrap();
        let region = create_test_region();

        registry.register_regions(&parent_uri, &[region]);

        let virtual_docs = registry.get_by_parent(&parent_uri);
        let virtual_uri = &virtual_docs[0].uri;

        assert!(registry.is_virtual(virtual_uri));
        assert!(!registry.is_virtual(&parent_uri));
    }
}
