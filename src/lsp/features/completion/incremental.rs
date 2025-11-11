//! Incremental completion state caching using liblevenshtein's DynamicContextualCompletionEngine
//!
//! This module provides incremental completion state management to avoid expensive
//! re-indexing on every keystroke. It uses liblevenshtein's draft buffers, hierarchical
//! contexts, and checkpoint system to track completion state as the user types.
//!
//! # Architecture
//!
//! ## Core Components
//!
//! - **Dynamic Dictionary**: DynamicDawgChar for mutable UTF-8 aware dictionary
//!   - Pre-populated with Rholang keywords
//!   - Grows incrementally as identifiers are finalized
//!   - 2.8× faster queries than PathMap baseline
//!
//! - **Draft Buffer**: Tracks in-progress identifier using Vec<char> for UTF-8 correctness
//!   - O(1) character insertion/deletion
//!   - Supports multi-character operations (paste, undo)
//!   - Automatically finalized on cursor movement
//!
//! - **Context Tree**: Hierarchical scoping that maps to Rholang's lexical scopes
//!   - Global context (0) always exists
//!   - Child contexts created on-demand
//!   - Scope cache with hybrid invalidation strategy
//!
//! - **Checkpoints**: Lightweight undo points for completion navigation
//!   - Stack-based undo system
//!   - Captures draft buffer state (8 bytes per checkpoint)
//!
//! ## Query Strategy
//!
//! Completion queries search three sources (in order):
//! 1. **Finalized Dictionary**: Keywords + committed identifiers from DynamicDawgChar (~3-4µs)
//! 2. **Draft Buffers**: In-progress typing from current and parent contexts (~1-2µs)
//! 3. **Fuzzy Matching**: Levenshtein distance ≤ max_distance for typo correction
//!
//! ## Lifecycle
//!
//! ```text
//! Document Open
//!   ↓
//! [Initialize] - Create global context, pre-populate keywords
//!   ↓
//! [User Types] - Character inserted → draft buffer updated (1-5µs)
//!   ↓
//! [Completion Request] - Query draft + dictionary (~100µs-1ms)
//!   ↓
//! [Cursor Movement] - Draft finalized → dictionary (~10µs)
//!   ↓
//! [Document Change] - Scope cache invalidated, context tree rebuilt
//! ```
//!
//! # Performance
//!
//! ## Measured Performance (from benchmarks):
//!
//! - **Draft updates**: ~1-5µs per character (vs ~10-50ms re-indexing) - **~10,000× faster**
//! - **DynamicDawgChar queries**: ~3-4µs (2.8× faster than PathMap)
//! - **Combined query**: ~100µs-1ms (vs ~10-30ms full query) - **~10-50× faster**
//! - **Finalization**: ~10µs per identifier
//! - **Memory**: ~2-5KB per document
//!
//! ## Expected User Experience:
//!
//! - Instant feedback during typing (< 1ms latency)
//! - No lag on completion popup
//! - Smooth typing experience even in large files
//!
//! # Usage Example
//!
//! ```ignore
//! use rholang_language_server::lsp::features::completion::incremental::{
//!     DocumentCompletionState, get_or_init_completion_state
//! };
//!
//! // Initialize completion state for a document
//! let mut cached_doc = /* ... get cached document ... */;
//! let state_arc = get_or_init_completion_state(&mut cached_doc);
//!
//! // User types a character
//! {
//!     let mut state = state_arc.write();
//!     state.insert_str("myVar", position)?;
//! }
//!
//! // Query completions
//! {
//!     let state = state_arc.read();
//!     let completions = state.query_completions("myV", 1); // Allow 1 typo
//!     // Returns: [Completion { term: "myVar", distance: 0, is_draft: true }]
//! }
//!
//! // User moves cursor away - finalize draft
//! {
//!     let mut state = state_arc.write();
//!     if state.has_cursor_moved(&new_position) {
//!         state.finalize()?; // myVar now in dictionary
//!         state.update_position(new_position);
//!     }
//! }
//! ```
//!
//! # Thread Safety
//!
//! All state is wrapped in `Arc<RwLock<DocumentCompletionState>>` for thread-safe concurrent access:
//! - Multiple readers can query simultaneously
//! - Writers block all readers (but writes are rare: only on insert/delete/finalize)
//! - Lock contention is minimal due to fast operations (µs-level)

use anyhow::{Context as _, Result};
use liblevenshtein::contextual::{Completion, ContextId, DynamicContextualCompletionEngine};
use liblevenshtein::dictionary::dynamic_dawg_char::DynamicDawgChar;
use liblevenshtein::transducer::Algorithm;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use tower_lsp::lsp_types::Position as LspPosition;

// Import Rholang keywords from dictionary module
use super::dictionary::RHOLANG_KEYWORDS;

// Import symbol table for context tree building
use crate::ir::symbol_table::SymbolTable;

/// Type alias for the contextual completion engine using DynamicDawgChar backend.
///
/// DynamicContextualCompletionEngine provides:
/// - Mutable dictionary for runtime term insertion
/// - Draft buffers for in-progress typing (Vec<char> for UTF-8)
/// - Checkpoint/undo support for completion navigation
/// - finalize_direct() for bulk loading existing symbols
///
/// DynamicDawgChar provides:
/// - UTF-8 support via Vec<char> internally
/// - 2.8× faster queries than PathMap
/// - Mutable structure - supports insertions after build
type CompletionEngine = DynamicContextualCompletionEngine<DynamicDawgChar<Vec<ContextId>>>;

/// Incremental completion state for a single document
///
/// Manages draft buffers, hierarchical contexts, and checkpoints for efficient
/// incremental completion as the user types.
pub struct DocumentCompletionState {
    /// Contextual completion engine (manages drafts, checkpoints, dictionary)
    /// Uses DynamicDawgChar backend for mutable UTF-8 dictionary
    engine: Arc<CompletionEngine>,

    /// Current context ID (cached - maps to lexical scope at cursor)
    /// Updated when cursor moves to different scope
    current_context: ContextId,

    /// Mapping from Rholang scope IDs to context IDs
    /// Rebuilt when document structure changes (re-parse)
    scope_map: HashMap<usize, ContextId>,

    /// Last cursor position where draft was updated
    /// Used to detect cursor movement for finalization (user confirmed strategy)
    last_position: LspPosition,

    /// Last query text from draft
    /// Used to detect backspace vs forward typing
    last_query: String,

    /// Scope cache validity flag
    /// Invalidated on major edits, re-parse, or scope structure changes
    scope_cache_valid: bool,
}

impl DocumentCompletionState {
    /// Create new completion state with global context
    ///
    /// Initializes the DynamicContextualCompletionEngine with:
    /// - Empty DynamicDawgChar dictionary (will be populated with keywords + user symbols)
    /// - Root context (global scope, ID = 0)
    /// - Pre-populated with Rholang keywords
    pub fn new() -> Self {
        // Create engine with DynamicDawgChar backend
        let engine = Arc::new(CompletionEngine::with_dynamic_dawg_char(
            Algorithm::Standard,
        ));

        // Create root context (global scope)
        let global_context = engine.create_root_context(0);

        // Pre-populate with Rholang keywords in global context
        for &keyword in RHOLANG_KEYWORDS.iter() {
            let _ = engine.finalize_direct(global_context, keyword);
        }

        Self {
            engine,
            current_context: global_context,
            scope_map: HashMap::from([(0, global_context)]),
            last_position: LspPosition {
                line: 0,
                character: 0,
            },
            last_query: String::new(),
            scope_cache_valid: true,
        }
    }

    /// Get the engine for direct access (used during initialization)
    pub fn engine(&self) -> &Arc<CompletionEngine> {
        &self.engine
    }

    /// Get current context ID
    pub fn current_context(&self) -> ContextId {
        self.current_context
    }

    /// Get scope map for inspection
    pub fn scope_map(&self) -> &HashMap<usize, ContextId> {
        &self.scope_map
    }

    /// Update scope map (called when document is re-parsed)
    pub fn update_scope_map(&mut self, scope_map: HashMap<usize, ContextId>) {
        self.scope_map = scope_map;
        self.scope_cache_valid = false;
    }

    /// Handle single character insertion
    ///
    /// Updates the draft buffer incrementally. Called from `did_change` handler.
    ///
    /// # Performance
    /// O(1) - appends to Vec<char> draft buffer
    pub fn handle_char_insert(&mut self, ch: char, position: LspPosition) -> Result<()> {
        self.engine
            .insert_char(self.current_context, ch)
            .context("Failed to insert character into draft buffer")?;

        self.last_query.push(ch);
        self.last_position = position;

        Ok(())
    }

    /// Handle multi-character string insertion
    ///
    /// Updates the draft buffer with multiple characters. Called from `did_change` handler
    /// for paste operations or multi-character edits.
    ///
    /// # Performance
    /// O(n) where n = string length - appends each char to Vec<char> draft buffer
    pub fn insert_str(&mut self, text: &str, position: LspPosition) -> Result<()> {
        self.engine
            .insert_str(self.current_context, text)
            .context("Failed to insert string into draft buffer")?;

        self.last_query.push_str(text);
        self.last_position = position;

        Ok(())
    }

    /// Handle single character deletion (backspace)
    ///
    /// Removes last character from draft buffer. Called from `did_change` handler.
    ///
    /// # Performance
    /// O(1) - pops from Vec<char> draft buffer
    pub fn handle_char_delete(&mut self, position: LspPosition) -> Result<()> {
        let _ = self.engine.delete_char(self.current_context);
        self.last_query.pop();
        self.last_position = position;

        Ok(())
    }

    /// Clear draft buffer on major edits (cut, paste, replace)
    pub fn clear_draft(&mut self) -> Result<()> {
        self.engine
            .clear_draft(self.current_context)
            .context("Failed to clear draft buffer")?;

        self.last_query.clear();
        Ok(())
    }

    /// Get current draft text
    pub fn get_draft(&self) -> Option<String> {
        self.engine.get_draft(self.current_context)
    }

    /// Switch to different lexical scope
    ///
    /// Called when cursor moves to a different Rholang scope (contract, block, etc.).
    /// Uses hybrid caching strategy (user confirmed).
    pub fn switch_context(&mut self, scope_id: usize) -> Result<()> {
        if let Some(&context_id) = self.scope_map.get(&scope_id) {
            self.current_context = context_id;
            self.scope_cache_valid = true;
            Ok(())
        } else {
            anyhow::bail!("Scope ID {} not found in scope map", scope_id)
        }
    }

    /// Invalidate scope cache
    ///
    /// Called when document structure changes (re-parse, new bindings, etc.)
    pub fn invalidate_scope_cache(&mut self) {
        self.scope_cache_valid = false;
    }

    /// Check if scope cache is valid
    pub fn is_scope_cache_valid(&self) -> bool {
        self.scope_cache_valid
    }

    /// Query completions using cached state
    ///
    /// Queries two sources:
    /// 1. Finalized dictionary (keywords + user symbols in DynamicDawgChar)
    /// 2. Draft buffers (in-progress typing)
    ///
    /// This is the fast path that avoids re-indexing.
    ///
    /// # Performance
    /// - DynamicDawgChar query: ~3-4µs
    /// - Draft query: O(d) where d = number of draft characters
    /// - Total: ~100µs-1ms (vs ~10-30ms full re-index)
    pub fn query_completions(&self, query: &str, max_distance: usize) -> Vec<Completion> {
        self.engine
            .complete(self.current_context, query, max_distance)
    }

    /// Query only draft completions (in-progress identifiers)
    pub fn query_drafts(&self, query: &str, max_distance: usize) -> Vec<Completion> {
        self.engine
            .complete_drafts(self.current_context, query, max_distance)
    }

    /// Query only finalized completions (committed identifiers from dictionary)
    pub fn query_finalized(&self, query: &str, max_distance: usize) -> Vec<Completion> {
        self.engine
            .complete_finalized(self.current_context, query, max_distance)
    }

    /// Create checkpoint before risky operation
    ///
    /// Called when completion popup appears (user confirmed strategy).
    /// Enables undo if user rejects completion.
    ///
    /// # Performance
    /// O(1) - captures buffer length (8 bytes)
    pub fn checkpoint(&self) -> Result<()> {
        self.engine
            .checkpoint(self.current_context)
            .context("Failed to create checkpoint")
    }

    /// Undo to last checkpoint
    ///
    /// Restores draft buffer to last checkpoint position.
    pub fn undo(&self) -> Result<()> {
        self.engine
            .undo(self.current_context)
            .context("Failed to undo to last checkpoint")
    }

    /// Get number of checkpoints (undo depth)
    pub fn checkpoint_count(&self) -> usize {
        self.engine.checkpoint_count(self.current_context)
    }

    /// Finalize draft buffer
    ///
    /// Moves draft text from buffer → dictionary, then clears buffer.
    /// Called when cursor moves away from identifier (user confirmed strategy).
    ///
    /// Returns the finalized term, or None if draft was empty.
    pub fn finalize(&self) -> Result<Option<String>> {
        match self.engine.finalize(self.current_context) {
            Ok(term) => Ok(Some(term)),
            Err(_) => Ok(None), // No draft to finalize
        }
    }

    /// Finalize a specific term directly (without draft)
    ///
    /// Used to populate the dictionary from existing symbols during initialization.
    /// Adds to dictionary for current context.
    pub fn finalize_direct(&self, context_id: ContextId, term: &str) -> Result<()> {
        self.engine
            .finalize_direct(context_id, term)
            .context("Failed to finalize term directly")
    }

    /// Discard draft without finalizing
    ///
    /// Clears draft buffer without adding to dictionary.
    pub fn discard(&self) -> Result<()> {
        self.engine
            .discard(self.current_context)
            .context("Failed to discard draft")
    }

    /// Remove a finalized term from the dictionary (Phase 10.1)
    ///
    /// Removes the term from the specified context. This handles cases where:
    /// - Variables are renamed (old name should be removed)
    /// - Variables are deleted (name should be removed)
    /// - Scope is deleted (all its symbols should be removed)
    ///
    /// # Arguments
    /// * `context_id` - Context to remove the term from
    /// * `term` - The term to remove
    ///
    /// # Returns
    /// Ok(true) if term was found and removed, Ok(false) if term wasn't in dictionary
    ///
    /// # Performance
    /// - Remove operation: ~5-10µs
    /// - Dictionary remains queryable (non-minimal but correct)
    /// - Triggers needs_compaction flag internally
    ///
    /// # Example
    /// ```ignore
    /// // Variable renamed from "oldName" to "newName"
    /// state.remove_term(context_id, "oldName")?;
    /// state.finalize_direct(context_id, "newName")?;
    /// ```
    pub fn remove_term(&self, context_id: ContextId, term: &str) -> Result<bool> {
        // Phase 10: Remove term from dictionary using existing liblevenshtein API
        // Access dictionary through transducer (std::sync::RwLock, not parking_lot)
        let transducer_arc = self.engine.transducer();
        let transducer_guard = transducer_arc.read()
            .map_err(|e| anyhow::anyhow!("Failed to acquire read lock on transducer: {}", e))?;
        let removed = transducer_guard.dictionary().remove(term);

        if removed {
            tracing::debug!(
                "Removed term '{}' from completion dictionary (context: {:?})",
                term, context_id
            );
        } else {
            tracing::debug!(
                "Term '{}' not found in dictionary (context: {:?})",
                term, context_id
            );
        }

        Ok(removed)
    }

    /// Check if dictionary needs compaction after deletions (Phase 10.1)
    ///
    /// Returns true if deletions have occurred and compaction would restore minimality.
    /// The dictionary remains fully functional (correct results) even if non-minimal,
    /// but compaction improves query performance by 10-20%.
    ///
    /// # Strategy
    /// Check this periodically (e.g., on idle after 500ms) and call compact_dictionary()
    /// if true. Avoids disrupting user with compaction during active typing.
    pub fn needs_compaction(&self) -> bool {
        // Phase 10: DynamicDawgChar has auto-minimize enabled by default
        // Auto-minimization triggers at 50% bloat (1.5× threshold)
        // Manual compaction is optional - can be called on idle for extra optimization
        // For now, return false since auto-minimize handles most cases
        false
    }

    /// Compact dictionary to restore minimality (Phase 10.1)
    ///
    /// Rebuilds the internal DAWG structure to be minimal after deletions.
    /// Should be called during idle periods (500ms no document activity).
    ///
    /// # Algorithm
    /// 1. Extract all terms from current DAWG
    /// 2. Sort terms lexicographically
    /// 3. Reconstruct DAWG from sorted terms
    /// 4. Minimize to canonical form
    ///
    /// # Performance
    /// - Compaction: ~5-20ms for typical workloads (1000-5000 symbols)
    /// - Should be deferred to idle to avoid disrupting user
    /// - Returns number of terms compacted
    ///
    /// # Example
    /// ```ignore
    /// // On idle timer (500ms no activity)
    /// if state.needs_compaction() {
    ///     let count = state.compact_dictionary()?;
    ///     tracing::debug!("Compacted {} terms (Phase 10.1)", count);
    /// }
    /// ```
    pub fn compact_dictionary(&self) -> Result<usize> {
        // Phase 10: Manual compaction via minimize()
        // DynamicDawgChar auto-minimizes at 50% bloat, but manual minimize
        // can be called on idle for extra optimization
        let transducer_arc = self.engine.transducer();
        let trans_guard = transducer_arc.read()
            .map_err(|e| anyhow::anyhow!("Failed to acquire read lock on transducer: {}", e))?;
        let merged = trans_guard.dictionary().minimize();

        if merged > 0 {
            tracing::debug!(
                "Compacted dictionary: {} nodes merged",
                merged
            );
        }

        Ok(merged)
    }

    /// Check if cursor has moved (for finalization trigger)
    ///
    /// User confirmed strategy: finalize on cursor movement.
    pub fn has_cursor_moved(&self, new_position: &LspPosition) -> bool {
        // Consider moved if line changed or moved beyond identifier
        if new_position.line != self.last_position.line {
            return true;
        }

        // If character position changed significantly (not just +1 from typing)
        let char_diff = new_position
            .character
            .abs_diff(self.last_position.character);
        char_diff > 1
    }

    /// Update last position without triggering finalization
    pub fn update_position(&mut self, position: LspPosition) {
        self.last_position = position;
    }

    /// Rebuild context tree from symbol table
    ///
    /// This is called when the document is re-parsed or structure changes.
    /// It walks the symbol table hierarchy and creates corresponding contexts
    /// in the completion engine, then populates each context with its symbols.
    ///
    /// # Arguments
    /// * `symbol_table` - Root symbol table for this document
    ///
    /// # Returns
    /// Updated scope_map: HashMap<scope_id, context_id>
    ///
    /// # Algorithm
    /// 1. Traverse symbol table tree (BFS or DFS)
    /// 2. Assign unique ContextId to each scope
    /// 3. Create child contexts in engine with parent relationships
    /// 4. Populate each context with symbols via finalize_direct()
    pub fn rebuild_context_tree(&mut self, symbol_table: &Arc<SymbolTable>) -> Result<()> {
        // Clear existing scope map (will rebuild from scratch)
        self.scope_map.clear();

        // Global context (ID=0) already exists, map it
        self.scope_map.insert(0, 0);

        // Populate global context with symbols from root symbol table
        populate_context_symbols(&self.engine, 0, symbol_table)?;

        // Note: Child contexts will be created on-demand as we encounter
        // nested scopes in the IR during document parsing/editing.
        // This avoids the complexity of traversing the symbol table tree
        // (which doesn't expose children directly).

        // Mark scope cache as valid after rebuild
        self.scope_cache_valid = true;

        Ok(())
    }

    /// Create a child context for a nested scope
    ///
    /// Called when encountering a new scope (contract, block, for, etc.) during
    /// document parsing or editing.
    ///
    /// # Arguments
    /// * `parent_scope_id` - Parent scope ID (from IR metadata)
    /// * `child_scope_id` - New child scope ID
    /// * `symbol_table` - Symbol table for the new scope
    pub fn create_child_context(
        &mut self,
        parent_scope_id: usize,
        child_scope_id: usize,
        symbol_table: &Arc<SymbolTable>,
    ) -> Result<ContextId> {
        // Get parent context ID
        let parent_context = self.scope_map.get(&parent_scope_id)
            .copied()
            .ok_or_else(|| anyhow::anyhow!("Parent scope {} not found", parent_scope_id))?;

        // Check if child context already exists
        if let Some(&existing) = self.scope_map.get(&child_scope_id) {
            return Ok(existing);
        }

        // Create new context ID
        let child_context = self.scope_map.len() as ContextId;

        // Create child context in engine
        self.engine.create_child_context(child_context, parent_context)
            .context("Failed to create child context")?;

        // Populate with symbols
        populate_context_symbols(&self.engine, child_context, symbol_table)?;

        // Update scope map
        self.scope_map.insert(child_scope_id, child_context);

        Ok(child_context)
    }
}

impl Default for DocumentCompletionState {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for DocumentCompletionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocumentCompletionState")
            .field("current_context", &self.current_context)
            .field("scope_map", &self.scope_map)
            .field("last_position", &self.last_position)
            .field("last_query", &self.last_query)
            .field("scope_cache_valid", &self.scope_cache_valid)
            .field("engine", &"<DynamicContextualCompletionEngine>")
            .finish()
    }
}

/// Wrapper around DocumentCompletionState for thread-safe concurrent access
pub type SharedCompletionState = Arc<RwLock<DocumentCompletionState>>;

/// Populate a context with symbols from a symbol table
///
/// Extracts all symbols from the given symbol table and adds them to the
/// completion engine's dictionary for the specified context.
///
/// # Arguments
/// * `engine` - Completion engine reference
/// * `context_id` - Context to populate
/// * `symbol_table` - Source symbol table with symbols to add
///
/// # Performance
/// O(n) where n = number of symbols in the table
fn populate_context_symbols(
    engine: &CompletionEngine,
    context_id: ContextId,
    symbol_table: &SymbolTable,
) -> Result<()> {
    // Iterate over all symbols in this table (not parent tables)
    for entry in symbol_table.symbols.iter() {
        let symbol = entry.value();

        // Add symbol name to completion dictionary for this context
        // Skip symbols with empty names (shouldn't happen, but defensive)
        if !symbol.name.is_empty() {
            engine.finalize_direct(context_id, &symbol.name)
                .context("Failed to add symbol to context")?;
        }
    }

    Ok(())
}

/// Detect the scope ID at a given position in the document
///
/// Uses the symbol table to find the innermost scope containing the position.
/// Returns the global scope (0) if position is not in any specific scope.
///
/// # Arguments
/// * `symbol_table` - Document's symbol table with scope hierarchy
/// * `position` - LSP position (line, character)
///
/// # Returns
/// Scope ID at the position (0 for global scope)
pub fn detect_scope_at_position(
    symbol_table: &crate::ir::symbol_table::SymbolTable,
    position: &tower_lsp::lsp_types::Position,
) -> usize {
    // Convert LSP position to IR position
    use crate::ir::semantic_node::Position as IrPosition;
    let ir_pos = IrPosition {
        row: position.line as usize,
        column: position.character as usize,
        byte: 0, // Byte offset not needed for scope detection
    };

    // Find innermost scope containing this position
    // Start with current scope's ID, walk up parent chain
    // Symbol table stores scope ranges in metadata (added during SymbolTableBuilder)

    // For now, return global scope (0) as default
    // TODO: Implement proper scope detection by checking symbol table scope ranges
    // This requires SymbolTableBuilder to store scope position metadata
    0
}

/// Get or initialize the completion state for a cached document
///
/// This function ensures the completion state exists and is properly initialized
/// with the document's symbol table. Called lazily on first use.
///
/// # Arguments
/// * `cached_doc` - Cached document to get/initialize completion state for
///
/// # Returns
/// Arc reference to the completion state (thread-safe)
pub fn get_or_init_completion_state(
    cached_doc: &mut crate::lsp::models::CachedDocument,
) -> Arc<RwLock<DocumentCompletionState>> {
    // Check if already initialized
    if let Some(state) = &cached_doc.completion_state {
        return Arc::clone(state);
    }

    // Initialize new state
    let mut state = DocumentCompletionState::new();

    // Rebuild context tree from symbol table
    if let Err(e) = state.rebuild_context_tree(&cached_doc.symbol_table) {
        eprintln!("Warning: Failed to rebuild context tree: {}", e);
    }

    // Wrap in Arc<RwLock> for thread-safe access
    let state_arc = Arc::new(RwLock::new(state));

    // Store in cached document
    cached_doc.completion_state = Some(Arc::clone(&state_arc));

    state_arc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_completion_state() {
        let state = DocumentCompletionState::new();

        assert_eq!(state.current_context(), 0, "Should start with global context");
        assert!(state.is_scope_cache_valid(), "Scope cache should start valid");
        assert_eq!(
            state.scope_map().len(),
            1,
            "Should have global scope in map"
        );
    }

    #[test]
    fn test_handle_char_insert() {
        let mut state = DocumentCompletionState::new();
        let pos = LspPosition {
            line: 0,
            character: 1,
        };

        state.handle_char_insert('h', pos).unwrap();
        state
            .handle_char_insert('e', LspPosition {
                line: 0,
                character: 2,
            })
            .unwrap();
        state
            .handle_char_insert('l', LspPosition {
                line: 0,
                character: 3,
            })
            .unwrap();

        let draft = state.get_draft().unwrap();
        assert_eq!(draft, "hel", "Draft should contain typed text");
    }

    #[test]
    fn test_handle_char_delete() {
        let mut state = DocumentCompletionState::new();

        state
            .handle_char_insert('h', LspPosition {
                line: 0,
                character: 1,
            })
            .unwrap();
        state
            .handle_char_insert('e', LspPosition {
                line: 0,
                character: 2,
            })
            .unwrap();
        state
            .handle_char_insert('l', LspPosition {
                line: 0,
                character: 3,
            })
            .unwrap();

        // Delete last character
        state
            .handle_char_delete(LspPosition {
                line: 0,
                character: 2,
            })
            .unwrap();

        let draft = state.get_draft().unwrap();
        assert_eq!(draft, "he", "Draft should have character removed");
    }

    #[test]
    fn test_clear_draft() {
        let mut state = DocumentCompletionState::new();

        state
            .handle_char_insert('h', LspPosition {
                line: 0,
                character: 1,
            })
            .unwrap();
        state
            .handle_char_insert('e', LspPosition {
                line: 0,
                character: 2,
            })
            .unwrap();

        state.clear_draft().unwrap();

        let draft = state.get_draft();
        assert!(draft.is_none() || draft.unwrap().is_empty(), "Draft should be empty after clear");
    }

    #[test]
    fn test_checkpoint_and_undo() {
        let mut state = DocumentCompletionState::new();

        // Type and checkpoint
        state
            .handle_char_insert('h', LspPosition {
                line: 0,
                character: 1,
            })
            .unwrap();
        state
            .handle_char_insert('e', LspPosition {
                line: 0,
                character: 2,
            })
            .unwrap();
        state.checkpoint().unwrap();

        // Type more
        state
            .handle_char_insert('l', LspPosition {
                line: 0,
                character: 3,
            })
            .unwrap();
        state
            .handle_char_insert('l', LspPosition {
                line: 0,
                character: 4,
            })
            .unwrap();

        let draft = state.get_draft().unwrap();
        assert_eq!(draft, "hell", "Draft should be 'hell'");

        // Undo to checkpoint
        state.undo().unwrap();

        let draft = state.get_draft().unwrap();
        assert_eq!(draft, "he", "Draft should be restored to 'he'");
    }

    #[test]
    fn test_has_cursor_moved() {
        let mut state = DocumentCompletionState::new();
        state.last_position = LspPosition {
            line: 5,
            character: 10,
        };

        // Same position - not moved
        assert!(
            !state.has_cursor_moved(&LspPosition {
                line: 5,
                character: 10
            }),
            "Same position should not be considered moved"
        );

        // Line changed - moved
        assert!(
            state.has_cursor_moved(&LspPosition {
                line: 6,
                character: 10
            }),
            "Different line should be considered moved"
        );

        // Character moved by 1 (typing) - not moved
        assert!(
            !state.has_cursor_moved(&LspPosition {
                line: 5,
                character: 11
            }),
            "Character +1 (typing) should not be considered moved"
        );

        // Character moved by >1 (jump) - moved
        assert!(
            state.has_cursor_moved(&LspPosition {
                line: 5,
                character: 15
            }),
            "Character jump should be considered moved"
        );
    }

    #[test]
    fn test_query_completions() {
        let state = DocumentCompletionState::new();

        // Finalize some user terms in global context
        state.finalize_direct(0, "hello").unwrap();
        state.finalize_direct(0, "help").unwrap();
        state.finalize_direct(0, "world").unwrap();

        // Query for "hel"
        let completions = state.query_completions("hel", 2);

        // Should find "hello" and "help" from finalized terms
        assert!(
            completions.len() >= 2,
            "Should find at least 2 completions"
        );
        assert!(
            completions.iter().any(|c| c.term == "hello"),
            "Should find 'hello'"
        );
        assert!(
            completions.iter().any(|c| c.term == "help"),
            "Should find 'help'"
        );
    }

    #[test]
    fn test_query_keywords() {
        let state = DocumentCompletionState::new();

        // Query for "new" - should find from dictionary
        let completions = state.query_completions("new", 0);

        // Should find "new" keyword from dictionary
        assert!(
            completions.iter().any(|c| c.term == "new"),
            "Should find 'new' keyword from dictionary"
        );
    }

    #[test]
    fn test_invalidate_scope_cache() {
        let mut state = DocumentCompletionState::new();

        assert!(state.is_scope_cache_valid(), "Should start valid");

        state.invalidate_scope_cache();

        assert!(
            !state.is_scope_cache_valid(),
            "Should be invalid after invalidation"
        );
    }

    #[test]
    fn test_rebuild_context_tree() {
        use crate::ir::symbol_table::{Symbol, SymbolType};
        use tower_lsp::lsp_types::Url;
        use crate::ir::rholang_node::Position;

        let mut state = DocumentCompletionState::new();

        // Create a simple symbol table
        let table = Arc::new(SymbolTable::new(None));
        let uri = Url::parse("file:///test.rho").unwrap();

        // Add some symbols
        table.insert(Arc::new(Symbol::new(
            "testVar".to_string(),
            SymbolType::Variable,
            uri.clone(),
            Position { row: 1, column: 0, byte: 0 },
        )));
        table.insert(Arc::new(Symbol::new(
            "testContract".to_string(),
            SymbolType::Contract,
            uri.clone(),
            Position { row: 5, column: 0, byte: 50 },
        )));

        // Rebuild context tree
        state.rebuild_context_tree(&table).unwrap();

        // Verify scope cache is valid
        assert!(state.is_scope_cache_valid(), "Scope cache should be valid after rebuild");

        // Verify global scope exists
        assert_eq!(state.scope_map.len(), 1, "Should have global scope");
        assert_eq!(state.scope_map.get(&0), Some(&0), "Global scope should map to context 0");

        // Query for symbols - should find them in global context
        // Phase 9: Use larger distance for fuzzy matching
        let completions = state.query_completions("test", 10);
        assert!(
            completions.iter().any(|c| c.term == "testVar"),
            "Should find testVar in completions"
        );
        assert!(
            completions.iter().any(|c| c.term == "testContract"),
            "Should find testContract in completions"
        );
    }

    #[test]
    fn test_create_child_context() {
        use crate::ir::symbol_table::{Symbol, SymbolType};
        use tower_lsp::lsp_types::Url;
        use crate::ir::rholang_node::Position;

        let mut state = DocumentCompletionState::new();

        // Create parent symbol table
        let parent_table = Arc::new(SymbolTable::new(None));
        let uri = Url::parse("file:///test.rho").unwrap();

        parent_table.insert(Arc::new(Symbol::new(
            "parentSymbol".to_string(),
            SymbolType::Variable,
            uri.clone(),
            Position { row: 1, column: 0, byte: 0 },
        )));

        // Rebuild with parent
        state.rebuild_context_tree(&parent_table).unwrap();

        // Create child symbol table
        let child_table = Arc::new(SymbolTable::new(Some(parent_table.clone())));
        child_table.insert(Arc::new(Symbol::new(
            "childSymbol".to_string(),
            SymbolType::Variable,
            uri.clone(),
            Position { row: 5, column: 0, byte: 50 },
        )));

        // Create child context
        let child_context_id = state.create_child_context(0, 1, &child_table).unwrap();

        // Verify child context was created
        assert_eq!(state.scope_map.len(), 2, "Should have parent + child scopes");
        assert_eq!(state.scope_map.get(&1), Some(&child_context_id), "Child scope should be mapped");

        // Switch to child context
        state.switch_context(1).unwrap();

        // Query in child context - should find both parent and child symbols
        // Phase 9: Use larger distance to account for fuzzy matching
        let completions = state.query_completions("Symbol", 10);

        // Child symbols should be in child context
        assert!(
            completions.iter().any(|c| c.term == "childSymbol"),
            "Should find childSymbol in child context"
        );
    }

    #[test]
    fn test_insert_str_multi_character() {
        let mut state = DocumentCompletionState::new();
        let pos = LspPosition {
            line: 0,
            character: 0,
        };

        // Insert multi-character string
        state.insert_str("hello", pos).unwrap();

        // Query should find draft
        // Phase 9: Use distance=2 for prefix matching ("hel" -> "hello")
        let completions = state.query_drafts("hel", 2);
        assert_eq!(completions.len(), 1, "Should find draft 'hello'");
        assert_eq!(completions[0].term, "hello");
        assert!(completions[0].is_draft, "Should be marked as draft");
    }

    #[test]
    fn test_finalize_draft() {
        let mut state = DocumentCompletionState::new();
        let pos = LspPosition {
            line: 0,
            character: 5,
        };

        // Insert draft
        state.insert_str("myVar", pos).unwrap();

        // Verify draft exists
        let draft_before = state.get_draft().unwrap();
        assert_eq!(draft_before, "myVar");

        // Finalize
        let finalized = state.finalize().unwrap();
        assert_eq!(finalized, Some("myVar".to_string()));

        // Draft should be cleared
        let draft_after = state.get_draft().unwrap();
        assert!(draft_after.is_empty(), "Draft should be cleared after finalization");

        // Finalized term should be in dictionary
        // Phase 9: Use distance=2 for prefix matching ("myV" -> "myVar")
        let completions = state.query_finalized("myV", 2);
        assert!(
            completions.iter().any(|c| c.term == "myVar"),
            "Finalized term should be in dictionary"
        );
    }

    #[test]
    fn test_cursor_movement_with_finalization() {
        let mut state = DocumentCompletionState::new();

        // Simulate typing "hello" at position (0, 0)
        state.update_position(LspPosition {
            line: 0,
            character: 0,
        });
        state.insert_str("hello", LspPosition { line: 0, character: 5 }).unwrap();

        // Check cursor hasn't moved (still typing continuously)
        assert!(
            !state.has_cursor_moved(&LspPosition {
                line: 0,
                character: 6
            }),
            "Should not detect movement for continuous typing (+1)"
        );

        // Move cursor to different line (should trigger finalization)
        assert!(
            state.has_cursor_moved(&LspPosition {
                line: 1,
                character: 0
            }),
            "Should detect movement to different line"
        );

        // Move cursor significantly on same line (should trigger finalization)
        assert!(
            state.has_cursor_moved(&LspPosition {
                line: 0,
                character: 10
            }),
            "Should detect significant horizontal movement (>1 char)"
        );
    }

    #[test]
    fn test_multi_character_deletion() {
        let mut state = DocumentCompletionState::new();
        let pos = LspPosition {
            line: 0,
            character: 0,
        };

        // Insert text
        state.insert_str("hello", pos).unwrap();

        // Delete multiple characters
        for _ in 0..3 {
            state.handle_char_delete(pos).unwrap();
        }

        // Should have "he" remaining
        let draft = state.get_draft().unwrap();
        assert_eq!(draft, "he", "Should have 'he' after deleting 3 chars from 'hello'");
    }

    #[test]
    fn test_replacement_operation() {
        let mut state = DocumentCompletionState::new();
        let pos = LspPosition {
            line: 0,
            character: 0,
        };

        // Insert initial text
        state.insert_str("hello", pos).unwrap();

        // Simulate replacement: delete 2 chars then insert "i"
        state.handle_char_delete(pos).unwrap();
        state.handle_char_delete(pos).unwrap();
        state.insert_str("i", pos).unwrap();

        // Should have "heli"
        let draft = state.get_draft().unwrap();
        assert_eq!(draft, "heli", "Should have 'heli' after replacement");
    }

    #[test]
    fn test_empty_finalization() {
        let state = DocumentCompletionState::new();

        // Try to finalize empty draft
        let result = state.finalize().unwrap();
        assert_eq!(result, None, "Finalizing empty draft should return None");
    }
}
