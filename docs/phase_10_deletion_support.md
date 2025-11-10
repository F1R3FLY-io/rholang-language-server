# Phase 10: Symbol Deletion Support for Incremental Completion

**Status**: Planning Complete, Implementation Pending
**Approved**: User approved shared dictionary architecture
**Dependencies**: liblevenshtein DI support (in progress by user)

---

## Table of Contents

1. [Overview](#overview)
2. [Problem Statement](#problem-statement)
3. [Architecture](#architecture)
4. [Implementation Plan](#implementation-plan)
5. [API Design](#api-design)
6. [Performance Characteristics](#performance-characteristics)
7. [Testing Strategy](#testing-strategy)
8. [Migration Path](#migration-path)
9. [References](#references)

---

## Overview

Phase 10 adds **symbol deletion support** to the incremental completion system (Phase 9) to handle variable renames and deletions. This ensures the completion dictionary stays synchronized with the source code, eliminating stale symbol suggestions.

### Key Objectives

1. **Deletion**: Remove symbols from dictionary when variables are deleted or renamed
2. **Workspace Coordination**: Single shared dictionary across all documents
3. **Performance**: <10µs per deletion, deferred compaction on idle
4. **Atomicity**: Cross-document updates immediately visible

### Architecture Decision

**Shared Dictionary with Dependency Injection** (approved):
- ONE `DynamicDawgChar` for entire workspace
- Injected into each `DocumentCompletionState` via DI
- Shallow Arc cloning enables concurrent access
- Direct dictionary access via `engine.transducer().read().dictionary()`

---

## Problem Statement

### Current Limitation (Phase 9)

The incremental completion system (Phase 9) only **adds** symbols to the dictionary. It never **removes** them:

```rust
// Phase 9: Addition only
state.finalize_direct(ctx, "myVariable")?;  // ✅ Added

// User renames: myVariable → myNewVariable
state.finalize_direct(ctx, "myNewVariable")?;  // ✅ Added
// ❌ Problem: "myVariable" still in dictionary (stale entry)
```

**Impact**:
- Completion suggests deleted/renamed variables
- Dictionary grows unbounded
- Confusing UX (suggests non-existent symbols)

### Example Scenario

```rholang
// File: contract.rho

// Initial code:
contract process(@oldName, ret) = { ... }

// User refactors to:
contract process(@newName, ret) = { ... }

// Problem: Completion still suggests "oldName" ❌
```

### Solution Requirements

1. **Detect** symbol deletions/renames via symbol table diffing
2. **Remove** old names from dictionary
3. **Compact** dictionary periodically to maintain performance
4. **Coordinate** updates across all workspace documents

---

## Architecture

### Component Overview

```
┌─────────────────────────────────────────────────────────────┐
│                      WorkspaceState                          │
│                                                              │
│  ┌────────────────────────────────────────────────────┐    │
│  │  shared_completion_dict: Arc<DynamicDawgChar>      │    │
│  │  (ONE dictionary for ALL documents)                 │    │
│  └──────────────────┬─────────────────────────────────┘    │
│                     │ (Arc clone - shallow)                 │
│         ┌───────────┼───────────┬────────────┐             │
│         ↓           ↓           ↓            ↓             │
│    ┌────────┐  ┌────────┐  ┌────────┐  ┌────────┐        │
│    │  Doc1  │  │  Doc2  │  │  Doc3  │  │  DocN  │        │
│    │ State  │  │ State  │  │ State  │  │ State  │        │
│    └────────┘  └────────┘  └────────┘  └────────┘        │
│         │           │           │            │             │
│         └───────────┴───────────┴────────────┘             │
│                     │                                       │
│              All reference same                             │
│           shared_completion_dict                            │
│          (via engine.transducer())                          │
└─────────────────────────────────────────────────────────────┘
```

### Data Flow

```
Document Edit
    ↓
Re-parse & Build New Symbol Table
    ↓
Diff Old vs New Symbol Table
    ↓
Detect: [deleted_symbols, renamed_symbols]
    ↓
┌─────────────────────────────────────┐
│  For each deleted symbol:           │
│    dict.remove(term)                │
│  For each renamed (old, new):       │
│    dict.remove(old)                 │
│    dict.insert(new)                 │
└─────────────────────────────────────┘
    ↓
Update immediately visible in ALL documents
    ↓
Idle Timer (500ms) checks needs_compaction()
    ↓
If needed: dict.compact() restores minimality
```

### Shared Dictionary Access Pattern

Based on liblevenshtein documentation:

```rust
// Access dictionary from engine (existing accessor)
let transducer = engine.transducer().read().unwrap();
let dict = transducer.dictionary();

// Perform operations
dict.remove(term);
dict.insert_with_value(term, contexts);

// Lock released when transducer guard drops
```

**Thread Safety**:
- `Arc<RwLock<Transducer<D>>>` enables concurrent access
- Multiple readers (queries) can hold read lock simultaneously
- Writers (deletions) acquire write lock (blocks readers briefly)
- Shallow Arc clone: All `DocumentCompletionState` instances share same dictionary

---

## Implementation Plan

### Phase 10.0: Optimized Workspace Initialization (NEW - Based on liblevenshtein Example)

**Discovery**: The liblevenshtein `parallel_workspace_indexing.rs` example reveals a superior architecture for initial workspace loading that differs from incremental updates.

**Problem with Naive Approach**:
```rust
// ❌ Naive: Build DynamicDawg per document, merge DawGs
for doc in workspace {
    let dawg = DynamicDawg::new();
    for term in doc.symbols {
        dawg.insert_with_value(term, vec![doc_id]);
    }
    dawgs.push(dawg);
}
let merged = merge_dawgs(dawgs);  // Slow! DAWG union is expensive
```

**Optimized Approach** (from example):
```rust
// ✅ Optimized: Build HashMap per document, merge HashMaps, build single DAWG
// Phase 1: Parallel HashMap construction (no locks!)
let hashmaps: Vec<HashMap<String, Vec<ContextId>>> = documents
    .par_iter()
    .map(|doc| {
        let mut dict = HashMap::new();
        for symbol in doc.extract_symbols() {
            dict.insert(symbol.name, vec![doc.id]);
        }
        dict
    })
    .collect();

// Phase 2: Binary tree reduction (parallel HashMap merge)
let merged_hashmap = merge_binary_tree(hashmaps);

// Phase 3: Build final DynamicDawg from complete merged HashMap
let shared_dict: Arc<DynamicDawgChar<Vec<ContextId>>> = Arc::new(DynamicDawgChar::new());
for (term, contexts) in merged_hashmap {
    shared_dict.insert_with_value(&term, contexts);
}
```

**File**: `src/lsp/backend/indexing.rs`

**New Functions**:

```rust
/// Build per-document symbol HashMap (Phase 10.0)
fn build_document_symbol_map(
    uri: &Url,
    doc: &CachedDocument,
    context_id: ContextId,
) -> HashMap<String, Vec<ContextId>> {
    let mut map = HashMap::new();

    // Extract symbols from document's symbol table
    for (name, _symbol) in doc.symbol_table.iter_symbols() {
        map.insert(name.clone(), vec![context_id]);
    }

    // Add keywords
    for keyword in RHOLANG_KEYWORDS {
        map.entry(keyword.to_string())
            .or_insert_with(Vec::new)
            .push(context_id);
    }

    map
}

/// Merge two symbol HashMaps (Phase 10.0)
fn merge_symbol_maps(
    map1: HashMap<String, Vec<ContextId>>,
    map2: &HashMap<String, Vec<ContextId>>,
) -> HashMap<String, Vec<ContextId>> {
    let mut merged = map1;

    for (term, contexts2) in map2 {
        merged
            .entry(term.clone())
            .and_modify(|contexts1| {
                *contexts1 = merge_context_ids(contexts1, contexts2);
            })
            .or_insert_with(|| contexts2.clone());
    }

    merged
}

/// Adaptive context ID deduplication (from liblevenshtein example)
fn merge_context_ids(left: &[ContextId], right: &[ContextId]) -> Vec<ContextId> {
    let total_len = left.len() + right.len();

    if total_len > 50 {
        // Large: Use FxHashSet (O(n) dedup, faster for large lists)
        let mut set: rustc_hash::FxHashSet<_> = left.iter().copied().collect();
        set.extend(right.iter().copied());
        let mut merged: Vec<_> = set.into_iter().collect();
        merged.sort_unstable();
        merged
    } else {
        // Small: Use Vec (O(n log n), lower constant overhead)
        let mut merged = left.to_vec();
        merged.extend_from_slice(right);
        merged.sort_unstable();
        merged.dedup();
        merged
    }
}

/// Binary tree reduction for parallel HashMap merge (Phase 10.0)
fn merge_symbol_maps_binary_tree(
    mut maps: Vec<HashMap<String, Vec<ContextId>>>,
) -> HashMap<String, Vec<ContextId>> {
    use rayon::prelude::*;

    if maps.is_empty() {
        return HashMap::new();
    }

    let mut round = 1;
    while maps.len() > 1 {
        tracing::debug!("Phase 10.0: Merge round {}, {} maps", round, maps.len());

        maps = maps
            .par_chunks(2)
            .map(|chunk| {
                if chunk.len() == 2 {
                    merge_symbol_maps(chunk[0].clone(), &chunk[1])
                } else {
                    chunk[0].clone()
                }
            })
            .collect();

        round += 1;
    }

    maps.into_iter().next().unwrap()
}

/// Initialize workspace completion dictionary (Phase 10.0)
///
/// Uses optimized HashMap→merge→DAWG pattern from liblevenshtein example
/// for 100-167× speedup over naive sequential approach.
pub(super) async fn initialize_workspace_completion(
    &self,
) -> Result<Arc<DynamicDawgChar<Vec<ContextId>>>, String> {
    use rayon::prelude::*;

    tracing::info!("Phase 10.0: Initializing workspace completion dictionary");
    let start = std::time::Instant::now();

    // Phase 1: Build HashMap per document (parallel)
    let documents: Vec<_> = self.workspace.documents.iter()
        .map(|entry| (entry.key().clone(), entry.value().clone()))
        .collect();

    let symbol_maps: Vec<_> = documents
        .par_iter()
        .enumerate()
        .map(|(idx, (uri, doc))| {
            let context_id = idx as ContextId;
            build_document_symbol_map(uri, doc, context_id)
        })
        .collect();

    tracing::debug!("Phase 10.0: Built {} symbol maps in {:?}",
        symbol_maps.len(), start.elapsed());

    // Phase 2: Binary tree merge (parallel)
    let merge_start = std::time::Instant::now();
    let merged_map = merge_symbol_maps_binary_tree(symbol_maps);
    tracing::debug!("Phase 10.0: Merged maps in {:?}", merge_start.elapsed());

    // Phase 3: Build final DynamicDawgChar
    let dawg_start = std::time::Instant::now();
    let shared_dict = Arc::new(DynamicDawgChar::new());

    for (term, contexts) in merged_map {
        shared_dict.insert_with_value(&term, contexts);
    }

    tracing::info!(
        "Phase 10.0: Workspace completion initialized: {} terms, {} docs, took {:?}",
        shared_dict.term_count(),
        documents.len(),
        start.elapsed()
    );

    Ok(shared_dict)
}
```

**Update WorkspaceState Initialization**:

```rust
impl WorkspaceState {
    pub async fn new_with_initialization(backend: &RholangBackend) -> Result<Self, String> {
        let mut state = Self::new();

        // Use optimized parallel initialization (Phase 10.0)
        state.shared_completion_dict = backend.initialize_workspace_completion().await?;

        Ok(state)
    }
}
```

**Estimated Effort**: 4 hours
**Performance Gain**: 100-167× faster initial load (from liblevenshtein benchmarks)
**Priority**: P1 (enables efficient workspace init)

---

### Phase 10.1: Undo Incorrect Changes ✅

**Problem**: Added incorrect implementations that don't leverage shared dictionary

**Files**:
- `/home/dylon/Workspace/f1r3fly.io/liblevenshtein-rust/src/contextual/engine.rs`
- `/home/dylon/Workspace/f1r3fly.io/rholang-language-server/src/lsp/features/completion/incremental.rs`

**Actions**:
1. Remove duplicate `remove_term_from_context()`, `needs_compaction()`, `compact()` from generic impl block (engine.rs lines ~1059-1173)
2. Keep DynamicDawgChar-specific impl only (lines ~188-252)
3. Remove wrapper methods from `DocumentCompletionState` - will access dict directly

**Rationale**: Methods should call dictionary directly via `transducer()` accessor, not add abstraction layers.

---

### Phase 10.2: Add Shared Dictionary to WorkspaceState

**File**: `src/lsp/models.rs`

**Change**:
```rust
pub struct WorkspaceState {
    // ... existing fields ...

    /// Phase 10: Workspace-wide shared completion dictionary
    ///
    /// All documents inject this into their DocumentCompletionState.
    /// Benefits:
    /// - Cross-document symbol visibility (immediate updates)
    /// - Single compaction point (not per-document)
    /// - Reduced memory (1 dict vs N dicts)
    ///
    /// Type: DynamicDawgChar for UTF-8 correctness (emoji, CJK support)
    pub shared_completion_dict: Arc<DynamicDawgChar<Vec<ContextId>>>,
}

impl WorkspaceState {
    pub fn new() -> Self {
        Self {
            // ... existing ...
            shared_completion_dict: Arc::new(DynamicDawgChar::new()),
        }
    }
}
```

**Estimated Effort**: 15 minutes
**Testing**: Verify `WorkspaceState::new()` initializes shared dict

---

### Phase 10.3: Update DocumentCompletionState Constructor

**File**: `src/lsp/features/completion/incremental.rs`

**Current**:
```rust
pub fn new(symbol_table: &Arc<SymbolTable>) -> Result<Self> {
    let engine = Arc::new(DynamicContextualCompletionEngine::with_dynamic_dawg_char(
        Algorithm::Standard
    ));
    // ... creates per-document dictionary
}
```

**New**:
```rust
pub fn new(
    symbol_table: &Arc<SymbolTable>,
    shared_dict: Arc<DynamicDawgChar<Vec<ContextId>>>,  // NEW: injected
) -> Result<Self> {
    // Use injected shared dictionary (NOT per-document)
    let engine = Arc::new(DynamicContextualCompletionEngine::with_dictionary(
        shared_dict,
        Algorithm::Standard
    ));

    // ... rest unchanged
}
```

**Update Call Sites**:
- `get_or_init_completion_state()` in `src/lsp/backend/indexing.rs`
- Pass `workspace.shared_completion_dict.clone()` (shallow Arc clone)

**Estimated Effort**: 30 minutes
**Testing**: Verify multiple docs share same dictionary

---

### Phase 10.4: Add Deletion Methods to DocumentCompletionState

**File**: `src/lsp/features/completion/incremental.rs`

**Add methods** (replace incorrect lines 402-470):

```rust
/// Remove a term from the shared dictionary (Phase 10.4)
///
/// Removes the term's association with the given context. If this was the last
/// context using the term, the term is completely removed from the dictionary.
///
/// # Performance
/// - Deletion: ~5-10µs
/// - Immediately visible across all documents
///
/// # Example
/// ```ignore
/// // Variable renamed from "oldName" to "newName"
/// state.remove_term(context_id, "oldName")?;
/// state.finalize_direct(context_id, "newName")?;
/// ```
pub fn remove_term(&self, context_id: ContextId, term: &str) -> Result<bool> {
    // Access shared dictionary via transducer (existing accessor)
    let transducer = self.engine.transducer().read()
        .map_err(|e| anyhow::anyhow!("Failed to acquire read lock: {}", e))?;
    let dict = transducer.dictionary();

    // Get current context list for this term
    let contexts = dict.get_value(term).unwrap_or_default();

    // Remove this context from the list
    let new_contexts: Vec<ContextId> = contexts
        .into_iter()
        .filter(|&ctx| ctx != context_id)
        .collect();

    // If list is now empty, remove term entirely; otherwise update
    let removed = new_contexts.len() < contexts.len();
    if removed {
        if new_contexts.is_empty() {
            dict.remove(term);  // Last context - remove term
        } else {
            dict.insert_with_value(term, new_contexts);  // Update context list
        }
    }

    Ok(removed)
}

/// Check if shared dictionary needs compaction (Phase 10.4)
///
/// Returns true if deletions have occurred and compaction would restore
/// the dictionary to minimal form, improving query performance by 10-20%.
///
/// The dictionary remains fully functional (returns correct results) even
/// if non-minimal, so compaction can be deferred to idle periods.
///
/// # Strategy
/// Check this periodically (e.g., on idle after 500ms) and call compact_dictionary()
/// if true. Avoids disrupting user with compaction during active typing.
pub fn needs_compaction(&self) -> bool {
    if let Ok(transducer) = self.engine.transducer().read() {
        transducer.dictionary().needs_compaction()
    } else {
        false  // Lock poisoned - assume no compaction needed
    }
}

/// Compact shared dictionary to restore minimality (Phase 10.4)
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
///     tracing::debug!("Compacted {} terms (Phase 10.4)", count);
/// }
/// ```
pub fn compact_dictionary(&self) -> Result<usize> {
    let transducer = self.engine.transducer().read()
        .map_err(|e| anyhow::anyhow!("Failed to acquire read lock: {}", e))?;
    Ok(transducer.dictionary().compact())
}
```

**Estimated Effort**: 1 hour
**Testing**: Unit tests for each method

---

### Phase 10.5: Implement Symbol Table Diffing

**File**: `src/lsp/backend/indexing.rs`

**Add helper function**:

```rust
/// Compare old vs new symbol tables to detect deletions/renames (Phase 10.5)
///
/// # Algorithm
/// 1. Iterate through old symbol table
/// 2. Check if each symbol exists in new table
/// 3. If missing:
///    - Check if renamed (same position, different name)
///    - Otherwise: deleted
///
/// # Returns
/// - `deleted`: Vector of symbol names that were deleted
/// - `renamed`: Vector of (old_name, new_name) tuples
///
/// # Performance
/// O(n + m) where n = old symbols, m = new symbols
/// - Symbol lookup: O(1) via HashMap
/// - Position lookup: O(1) via position index
fn diff_symbol_tables(
    old_table: &SymbolTable,
    new_table: &SymbolTable,
) -> (Vec<String>, Vec<(String, String)>) {
    let mut deleted = Vec::new();
    let mut renamed = Vec::new();

    // Walk old table, check if symbols still exist
    for (name, old_symbol) in old_table.iter_symbols() {
        if !new_table.contains_symbol(name) {
            // Symbol no longer exists by this name

            // Check if it was renamed (same position, different name)
            if let Some(new_symbol) = new_table.symbol_at_position(&old_symbol.range.start) {
                if new_symbol.name != name {
                    renamed.push((name.clone(), new_symbol.name.clone()));
                    continue;
                }
            }

            // Not renamed - must be deleted
            deleted.push(name.clone());
        }
    }

    (deleted, renamed)
}
```

**Estimated Effort**: 2 hours
**Testing**:
- Test deletion detection
- Test rename detection
- Test no changes case
- Test multiple simultaneous changes

---

### Phase 10.6: Integrate Deletion into did_change Handler

**File**: `src/lsp/backend/indexing.rs`

**Update `update_completion_state_incremental()`**:

```rust
pub(super) async fn update_completion_state_incremental(
    &self,
    uri: &Url,
    changes: &[TextDocumentContentChangeEvent],
    mut cached_doc_arc: Arc<CachedDocument>,
) {
    // ... existing draft buffer update logic ...

    // Phase 10.6: Detect symbol deletions/renames on structural changes
    if scope_cache_invalidated {
        // Get old symbol table (before re-parse)
        let old_table = &cached_doc_arc.symbol_table;

        // Trigger re-parse to get new symbol table
        // (This should already happen when scope_cache_invalidated is true)
        let new_doc = match self.reparse_document(uri).await {
            Ok(doc) => doc,
            Err(e) => {
                tracing::warn!("Failed to re-parse document for deletion detection: {}", e);
                return;
            }
        };

        let new_table = &new_doc.symbol_table;

        // Diff old vs new to find deletions/renames
        let (deleted, renamed) = diff_symbol_tables(old_table, new_table);

        if !deleted.is_empty() || !renamed.is_empty() {
            tracing::debug!(
                "Phase 10.6: Detected {} deletions, {} renames in {}",
                deleted.len(),
                renamed.len(),
                uri
            );

            // Apply deletions/renames to shared dictionary
            if let Some(state_arc) = &cached_doc_arc.completion_state {
                let state = state_arc.read();
                let context_id = state.current_context;

                // Remove deleted symbols
                for term in deleted {
                    if let Err(e) = state.remove_term(context_id, &term) {
                        tracing::warn!("Failed to remove deleted term '{}': {}", term, e);
                    }
                }

                // Handle renames (remove old, add new)
                for (old_name, new_name) in renamed {
                    if let Err(e) = state.remove_term(context_id, &old_name) {
                        tracing::warn!("Failed to remove renamed term '{}': {}", old_name, e);
                    }
                    if let Err(e) = state.finalize_direct(context_id, &new_name) {
                        tracing::warn!("Failed to add renamed term '{}': {}", new_name, e);
                    }
                }
            }
        }

        // Update cached document reference
        cached_doc_arc = new_doc;
    }
}
```

**Estimated Effort**: 3 hours
**Testing**: Integration tests for deletion scenarios

---

### Phase 10.7: Add Workspace-Level Idle Compaction Timer

**File**: `src/lsp/backend.rs` (or `src/lsp/backend/handlers.rs`)

**Add to backend initialization** (`RholangBackend::new()` or similar):

```rust
/// Phase 10.7: Spawn idle compaction timer
///
/// Checks every 500ms if the shared completion dictionary needs compaction.
/// Compaction restores dictionary minimality after deletions, improving
/// query performance by 10-20%.
///
/// This is a fire-and-forget background task that runs for the lifetime
/// of the language server.
fn spawn_compaction_timer(workspace: Arc<WorkspaceState>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(500));

        loop {
            interval.tick().await;

            // Fast check: does dictionary need compaction?
            if workspace.shared_completion_dict.needs_compaction() {
                let start = std::time::Instant::now();

                match workspace.shared_completion_dict.compact() {
                    Ok(count) => {
                        let elapsed = start.elapsed();
                        tracing::debug!(
                            "Phase 10.7: Compacted workspace dictionary: {} terms, took {:?}",
                            count,
                            elapsed
                        );

                        // Warn if compaction took too long (may block queries)
                        if elapsed > Duration::from_millis(50) {
                            tracing::warn!(
                                "Compaction took {:?} (>50ms), may cause query latency",
                                elapsed
                            );
                        }
                    }
                    Err(e) => {
                        tracing::error!("Phase 10.7: Compaction failed: {}", e);
                    }
                }
            }
        }
    });
}

// Call during backend initialization
impl RholangBackend {
    pub fn new(/* ... */) -> Self {
        // ... existing initialization ...

        // Start compaction timer
        spawn_compaction_timer(Arc::clone(&workspace));

        // ... rest ...
    }
}
```

**Estimated Effort**: 1 hour
**Testing**:
- Verify timer runs every 500ms
- Verify compaction is triggered after deletions
- Measure compaction latency

---

### Phase 10.8: Add Comprehensive Tests

**File**: `tests/test_completion_deletion.rs` (NEW)

**Test Suite**:

```rust
#[cfg(test)]
mod completion_deletion_tests {
    use super::*;

    /// Test 1: Symbol deletion removes from shared dictionary
    #[tokio::test]
    async fn test_symbol_deletion_removes_from_shared_dict() {
        // Setup: Document with variable "oldVar"
        // Action: Delete "oldVar" from source
        // Verify: "oldVar" not in completion results
    }

    /// Test 2: Symbol rename updates shared dictionary
    #[tokio::test]
    async fn test_symbol_rename_updates_shared_dict() {
        // Setup: Document with "oldName"
        // Action: Rename to "newName"
        // Verify:
        //   - "oldName" not in completions
        //   - "newName" in completions
    }

    /// Test 3: Cross-document deletion visibility
    #[tokio::test]
    async fn test_cross_document_deletion() {
        // Setup: 2 documents, both reference "sharedVar"
        // Action: Delete "sharedVar" from doc1
        // Verify:
        //   - doc1 completions don't include "sharedVar"
        //   - doc2 still sees "sharedVar" (different context)
    }

    /// Test 4: Compaction reduces dictionary size
    #[tokio::test]
    async fn test_compaction_after_many_deletions() {
        // Setup: Insert 1000 symbols
        // Action: Delete 900 symbols, call compact()
        // Verify:
        //   - Dictionary size reduced (internal node count)
        //   - Query performance improved
    }

    /// Test 5: Deletion preserves other contexts
    #[tokio::test]
    async fn test_deletion_preserves_other_contexts() {
        // Setup: Term "shared" in context 1 and 2
        // Action: Remove from context 1
        // Verify:
        //   - Context 1: "shared" not in completions
        //   - Context 2: "shared" still in completions
    }

    /// Test 6: Idle compaction timer triggers automatically
    #[tokio::test]
    async fn test_idle_compaction_timer() {
        // Setup: Backend with compaction timer
        // Action: Perform deletions, wait >500ms
        // Verify: Compaction was triggered (via logs or metrics)
    }

    /// Test 7: Concurrent deletion safety
    #[tokio::test]
    async fn test_concurrent_deletion_from_multiple_documents() {
        // Setup: 10 documents, all modify shared dictionary
        // Action: Concurrent deletions from all documents
        // Verify: No data races, all deletions applied
    }

    /// Test 8: Symbol table diffing accuracy
    #[test]
    fn test_diff_symbol_tables() {
        // Test diff_symbol_tables() helper function
        // Verify correct detection of deletions and renames
    }
}
```

**Estimated Effort**: 4-6 hours
**Coverage Target**: >90% of new code

---

### Phase 10.9: Add Telemetry and Logging

**Files**:
- `src/lsp/backend/indexing.rs`
- `src/lsp/backend.rs`

**Metrics to Add**:

```rust
// In update_completion_state_incremental()
tracing::debug!(
    "Phase 10: Detected {} deletions, {} renames in {}",
    deleted.len(),
    renamed.len(),
    uri
);

// In compaction timer
tracing::debug!(
    "Phase 10: Compacted {} terms, took {:?}",
    count,
    elapsed
);

// Performance warnings
if elapsed > Duration::from_millis(50) {
    tracing::warn!(
        "Compaction took {:?} (>50ms), may affect responsiveness",
        elapsed
    );
}
```

**Estimated Effort**: 30 minutes

---

### Phase 10.10: Update Documentation

**File**: `docs/code_completion_implementation.md`

**Add section**:

```markdown
## Phase 10: Symbol Deletion Support (Completed YYYY-MM-DD)

### Problem Solved

Phase 9 (incremental completion) only added symbols to the dictionary, never removed them.
This caused stale symbols to persist after variable renames or deletions, confusing users
with suggestions for non-existent identifiers.

### Solution Architecture

**Shared Dictionary with Dependency Injection**:
- Single `DynamicDawgChar<Vec<ContextId>>` shared across all workspace documents
- Injected into each `DocumentCompletionState` via constructor
- Shallow Arc cloning enables concurrent access with zero-copy semantics
- Direct dictionary access via `engine.transducer().read().dictionary()`

### Implementation Details

1. **Symbol Table Diffing**: Compare old vs new symbol tables on document changes
2. **Deletion**: Remove symbols via `dict.remove(term)` on shared dictionary
3. **Compaction**: Idle timer (500ms) calls `dict.compact()` to restore minimality
4. **Cross-Document Updates**: Changes immediately visible in all documents

### Performance Characteristics

| Operation | Latency | Notes |
|-----------|---------|-------|
| Deletion | ~5-10µs | Per symbol, immediate |
| Compaction | ~5-20ms | 1K-5K symbols, deferred to idle |
| Cross-document visibility | Atomic | No synchronization lag |

### API Usage

```rust
// Deletion
state.remove_term(context_id, "oldSymbol")?;

// Rename (delete + add)
state.remove_term(context_id, "oldName")?;
state.finalize_direct(context_id, "newName")?;

// Manual compaction check
if state.needs_compaction() {
    let count = state.compact_dictionary()?;
}
```

### Testing

17 unit tests + 8 integration tests covering:
- Symbol deletion
- Symbol rename
- Cross-document coordination
- Compaction behavior
- Concurrent access safety

### References

- [liblevenshtein parallel indexing pattern](../../liblevenshtein-rust/docs/algorithms/07-contextual-completion/patterns/parallel-workspace-indexing.md)
- [liblevenshtein completion engine accessor pattern](../../liblevenshtein-rust/docs/algorithms/07-contextual-completion/implementation/completion-engine.md#accessor-methods)
```

**Estimated Effort**: 1 hour

---

## API Design

### Public Methods

```rust
impl DocumentCompletionState {
    /// Create new state with injected shared dictionary (Phase 10.3)
    pub fn new(
        symbol_table: &Arc<SymbolTable>,
        shared_dict: Arc<DynamicDawgChar<Vec<ContextId>>>,
    ) -> Result<Self>;

    /// Remove term from shared dictionary (Phase 10.4)
    pub fn remove_term(&self, context_id: ContextId, term: &str) -> Result<bool>;

    /// Check if compaction needed (Phase 10.4)
    pub fn needs_compaction(&self) -> bool;

    /// Compact shared dictionary (Phase 10.4)
    pub fn compact_dictionary(&self) -> Result<usize>;
}
```

### Internal Helpers

```rust
// In src/lsp/backend/indexing.rs (Phase 10.5)
fn diff_symbol_tables(
    old_table: &SymbolTable,
    new_table: &SymbolTable,
) -> (Vec<String>, Vec<(String, String)>);

// In src/lsp/backend.rs (Phase 10.7)
fn spawn_compaction_timer(workspace: Arc<WorkspaceState>);
```

---

## Performance Characteristics

### Latency Targets

| Operation | Target | Measured | Notes |
|-----------|--------|----------|-------|
| Single deletion | <10µs | TBD | Direct dictionary access |
| Batch deletion (10 symbols) | <100µs | TBD | Sequential deletions |
| Compaction (1K symbols) | 5-10ms | TBD | Full rebuild |
| Compaction (5K symbols) | 10-20ms | TBD | Linear scaling |
| Compaction (10K symbols) | 20-40ms | TBD | May warn user |

### Memory Characteristics

**Before Phase 10 (per-document dictionaries)**:
- 100 documents × 1K symbols = 100 dictionaries
- Memory: ~100 × 30KB = ~3MB
- Redundancy: High (many duplicate symbols)

**After Phase 10 (shared dictionary)**:
- 1 dictionary for entire workspace
- Memory: ~30KB (with deduplication)
- Reduction: ~99% (3MB → 30KB)

### Compaction Trigger Strategy

**Idle Timer Approach** (chosen):
- Check every 500ms
- Trigger compaction if `needs_compaction()` returns true
- Deferred to avoid blocking user input

**Alternative Considered** (rejected):
- **Threshold-based**: Compact after N deletions
  - Problem: May compact too frequently (wasteful) or too rarely (stale)
- **Manual**: User-triggered compaction
  - Problem: Poor UX, requires user awareness

---

## Testing Strategy

### Unit Tests (17 tests)

**File**: `src/lsp/features/completion/incremental.rs`

1. `test_remove_term_single_context()` - Remove from one context
2. `test_remove_term_multiple_contexts()` - Remove from one, keep in others
3. `test_remove_term_nonexistent()` - Remove term that doesn't exist
4. `test_needs_compaction_after_deletions()` - Flag set after removals
5. `test_compact_dictionary()` - Compaction reduces size

**File**: `src/lsp/backend/indexing.rs`

6. `test_diff_symbol_tables_no_changes()` - Empty diff
7. `test_diff_symbol_tables_deletion()` - Detect deleted symbol
8. `test_diff_symbol_tables_rename()` - Detect renamed symbol
9. `test_diff_symbol_tables_multiple()` - Multiple changes
10. `test_diff_symbol_tables_position_mismatch()` - Handle position changes

### Integration Tests (8 tests)

**File**: `tests/test_completion_deletion.rs`

11. `test_symbol_deletion_removes_from_shared_dict()` - End-to-end deletion
12. `test_symbol_rename_updates_shared_dict()` - End-to-end rename
13. `test_cross_document_deletion()` - Multi-document coordination
14. `test_compaction_after_many_deletions()` - Performance verification
15. `test_deletion_preserves_other_contexts()` - Context isolation
16. `test_idle_compaction_timer()` - Timer behavior
17. `test_concurrent_deletion_from_multiple_documents()` - Thread safety

### Performance Benchmarks

**File**: `benches/completion_deletion_bench.rs` (NEW)

```rust
// Benchmark deletion latency
fn bench_single_deletion(c: &mut Criterion) {
    // Measure: dict.remove(term)
}

// Benchmark batch deletion
fn bench_batch_deletion(c: &mut Criterion) {
    // Measure: 10, 100, 1000 deletions
}

// Benchmark compaction
fn bench_compaction(c: &mut Criterion) {
    // Measure: compact() with 1K, 5K, 10K symbols
}
```

**Target**: Run benchmarks before/after to verify <10µs deletion, <20ms compaction

---

## Migration Path

### Phase 1: Backward Compatibility (Optional)

If needed, support both architectures temporarily:

```rust
pub enum CompletionDictionary {
    PerDocument(Arc<DynamicDawgChar<Vec<ContextId>>>),  // Old
    Shared(Arc<DynamicDawgChar<Vec<ContextId>>>),       // New
}
```

**Decision**: Skip - no migration needed (Phase 10 is new feature)

### Phase 2: Feature Flag (Recommended)

Add feature flag to enable/disable deletion support:

```rust
// In Cargo.toml
[features]
default = ["completion-deletion"]
completion-deletion = []

// In code
#[cfg(feature = "completion-deletion")]
fn apply_deletions(...) { ... }
```

**Benefit**: Easy rollback if issues arise

### Phase 3: Gradual Rollout

1. **Week 1**: Enable for internal testing
2. **Week 2**: Enable for beta users
3. **Week 3**: Enable for all users
4. **Week 4**: Remove feature flag

---

## References

### liblevenshtein Documentation

1. **Parallel Workspace Indexing Pattern**:
   - File: `/home/dylon/Workspace/f1r3fly.io/liblevenshtein-rust/docs/algorithms/07-contextual-completion/patterns/parallel-workspace-indexing.md`
   - Key: Shallow Arc cloning (line 244), `union_with()` merge pattern

2. **Completion Engine Accessor Pattern**:
   - File: `/home/dylon/Workspace/f1r3fly.io/liblevenshtein-rust/docs/algorithms/07-contextual-completion/implementation/completion-engine.md`
   - Key: `transducer()` accessor (lines 835-997), lock management

### Internal Documentation

3. **Phase 9 Implementation**:
   - File: `docs/code_completion_implementation.md`
   - Context: Incremental completion baseline

4. **Symbol Table Design**:
   - File: `src/ir/symbol_table.rs`
   - Key: Scope hierarchy, symbol lookup

### External Resources

5. **DynamicDawg Deletion**:
   - File: `liblevenshtein-rust/src/dictionary/dynamic_dawg_char.rs`
   - Methods: `remove()`, `compact()`, `needs_compaction()`

---

## Timeline Estimate

| Phase | Task | Effort | Priority |
|-------|------|--------|----------|
| 10.0 | Optimized workspace initialization (HashMap→DAWG) | 4 hours | P1 (100-167× speedup) |
| 10.1 | Undo incorrect changes | 30 min | P0 (blocker) |
| 10.2 | Add shared dict to WorkspaceState | 15 min | P0 |
| 10.3 | Update DocumentCompletionState constructor | 30 min | P0 |
| 10.4 | Add deletion methods | 1 hour | P0 |
| 10.5 | Implement symbol table diffing | 2 hours | P0 |
| 10.6 | Integrate into did_change | 3 hours | P0 |
| 10.7 | Add compaction timer | 1 hour | P1 |
| 10.8 | Add tests (30 tests total) | 5-7 hours | P1 |
| 10.9 | Add telemetry | 30 min | P2 |
| 10.10 | Update docs | 1 hour | P2 |

**Total Estimated Effort**: 18-21 hours (was 13-15 before Phase 10.0)
**Critical Path**: 10.1 → 10.2 → 10.3 → 10.4 → 10.5 → 10.6
**Optional Optimization**: 10.0 (can be done in parallel or deferred)
**Target Completion**: TBD (pending liblevenshtein DI support)

---

## Open Questions

1. **liblevenshtein DI Timeline**: When will DI support be merged?
2. **Feature Flag**: Should we add `completion-deletion` feature flag?
3. **Compaction Threshold**: Is 500ms idle timer optimal, or should it be configurable?
4. **Error Handling**: How to handle deletion failures (e.g., lock poisoning)?
5. **Metrics**: Should we add Prometheus metrics for deletion/compaction latency?

---

**Document Version**: 1.0
**Last Updated**: 2025-01-05
**Status**: Planning Complete, Awaiting liblevenshtein DI Support
