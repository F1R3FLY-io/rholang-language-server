# Phase 11: Incremental Indexing

**Status**: Design Phase
**Date**: 2025-01-11
**Author**: Phase 11 Design Team
**Depends On**: Phase 10 (Symbol Deletion Support)

## Executive Summary

Phase 11 implements **incremental workspace indexing** to eliminate the performance bottleneck of full workspace re-indexing on file changes. Current implementation re-indexes the entire workspace (O(n) files) on every `didChange` event, causing 100ms-10s lag in large projects. The solution tracks dirty files and re-indexes only changed files (O(1) per change), achieving **10-100x faster** workspace updates.

## Problem Statement

### Current Behavior

When a file changes in the workspace (via `didChange`, `didSave`, or file watcher), the system performs these operations:

**Current Flow** (`src/lsp/backend/indexing.rs:323-412`):
```rust
pub(super) async fn index_file(&self, uri: &Url, text: &str, ...) {
    // 1. Parse file (10-50ms for large files)
    let tree = parse_code(text);
    let document_ir = parse_to_document_ir(&tree, &rope);

    // 2. Process document (10-100ms)
    let cached = self.process_document(document_ir, uri, &rope, content_hash).await?;

    // 3. Update workspace (fast - lock-free DashMap)
    self.update_workspace_document(&uri, Arc::new(cached)).await;

    // 4. Link symbols ACROSS ALL FILES (100ms-10s for large workspaces!)
    self.link_symbols().await;  // ← BOTTLENECK

    // 5. Update completion index (10-50ms)
    populate_from_symbol_table(&self.workspace.completion_index, &global_table);
}
```

**Bottleneck Analysis** (`src/lsp/backend/symbols.rs:link_symbols()`):
- **Operation**: Rebuilds global symbol cross-references for **ALL files** in workspace
- **Time Complexity**: O(n × m) where n = files, m = symbols per file
- **Measured Performance**:
  - 10 files: ~10ms
  - 100 files: ~100ms
  - 1000 files: ~1-5s
  - 10000 files: ~10-50s (unusable!)

**Impact on User Experience**:
- **Typing Lag**: Every keystroke triggers `didChange` → full re-index → UI freeze
- **Save Lag**: `didSave` blocks for seconds in large workspaces
- **File Watch Lag**: External file changes (git checkout, build tools) cause multi-second freezes
- **Threshold**: Becomes unusable around 500-1000 files

### Root Causes

1. **Global Symbol Linking** (`link_symbols()`):
   - Iterates ALL documents every time
   - Rebuilds cross-file references from scratch
   - No tracking of which files actually changed

2. **Global Completion Index** (`populate_from_symbol_table()`):
   - Rebuilds entire index on every change
   - Phase 10 added deletion support but didn't implement incremental updates

3. **No Dirty Tracking**:
   - System doesn't know which files changed
   - Forces conservative "recompute everything" approach

## Solution Design

### Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                    Incremental Indexing System                  │
└─────────────────────────────────────────────────────────────────┘
         │
         ├── DirtyFileTracker (NEW)
         │   ├─ Track changed files per indexing cycle
         │   ├─ Batch changes within 100ms window (debouncing)
         │   └─ Priority queue (open files > workspace files)
         │
         ├── Incremental Symbol Linker (ENHANCED)
         │   ├─ OLD: link_symbols() rebuilds ALL files
         │   └─ NEW: link_symbols_incremental(dirty_files) updates ONLY dirty files
         │
         ├── Incremental Completion Index (ENHANCED)
         │   ├─ Leverages Phase 10's remove_term() for deletions
         │   ├─ Adds insert_symbols_from_file(uri, symbols)
         │   └─ Tracks symbol → file ownership for removal
         │
         └── File Change Pipeline (MODIFIED)
             ├─ didChange → mark dirty → debounce → incremental update
             ├─ didSave → mark dirty → incremental update
             └─ file watcher → mark dirty → batch update

Performance Characteristics:
┌────────────────────────┬─────────────────┬──────────────────┬────────────┐
│ Operation              │ Current (Full)  │ Phase 11 (Incr.) │ Improvement│
├────────────────────────┼─────────────────┼──────────────────┼────────────┤
│ Single file change     │ O(n) = 100ms-5s │ O(1) = 5-50ms    │ 10-100x    │
│ Batch changes (10)     │ O(n) × 10       │ O(10) = 50-500ms │ 10-100x    │
│ Workspace init         │ O(n) = 1-60s    │ O(n) = 1-60s     │ No change  │
│ Memory overhead        │ Baseline        │ +2-5MB           │ Negligible │
└────────────────────────┴─────────────────┴──────────────────┴────────────┘
```

### Component 1: DirtyFileTracker

**Location**: `src/lsp/backend/dirty_tracker.rs` (NEW)

**Purpose**: Track which files have changed since last indexing cycle

**Data Structure**:
```rust
/// Tracks dirty files and batches them for incremental indexing
pub struct DirtyFileTracker {
    /// Files marked dirty since last indexing cycle
    /// Uses DashMap for lock-free concurrent access
    dirty_files: Arc<DashMap<Url, DirtyFileMetadata>>,

    /// Debouncing: batch changes within this window
    debounce_window: std::time::Duration,  // Default: 100ms

    /// Last indexing cycle completion time
    last_cycle: Arc<RwLock<std::time::Instant>>,
}

#[derive(Debug, Clone)]
pub struct DirtyFileMetadata {
    /// Priority: 0 = high (open file), 1 = normal (workspace file)
    pub priority: u8,

    /// When this file was marked dirty
    pub marked_at: std::time::Instant,

    /// Reason for being dirty (for debugging)
    pub reason: DirtyReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirtyReason {
    /// User edited in editor
    DidChange,

    /// User saved file
    DidSave,

    /// External file system change
    FileWatcher,

    /// File opened for first time
    DidOpen,
}

impl DirtyFileTracker {
    pub fn new() -> Self { ... }

    /// Mark a file as dirty
    pub fn mark_dirty(&self, uri: Url, priority: u8, reason: DirtyReason) {
        self.dirty_files.insert(uri, DirtyFileMetadata {
            priority,
            marked_at: std::time::Instant::now(),
            reason,
        });
    }

    /// Get all dirty files and clear tracker
    /// Returns files sorted by priority (high-priority first)
    pub fn drain_dirty(&self) -> Vec<(Url, DirtyFileMetadata)> {
        let mut files: Vec<_> = self.dirty_files
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect();

        // Clear dirty set
        self.dirty_files.clear();

        // Sort by priority (0 = high comes first)
        files.sort_by_key(|(_, meta)| meta.priority);

        files
    }

    /// Check if we should flush based on debounce window
    pub fn should_flush(&self) -> bool {
        if self.dirty_files.is_empty() {
            return false;
        }

        // Find oldest dirty file
        let oldest = self.dirty_files
            .iter()
            .map(|entry| entry.value().marked_at)
            .min();

        if let Some(oldest_time) = oldest {
            oldest_time.elapsed() >= self.debounce_window
        } else {
            false
        }
    }
}
```

**Integration Points**:
1. `didChange` handler: `tracker.mark_dirty(uri, 0, DirtyReason::DidChange)`
2. `didSave` handler: `tracker.mark_dirty(uri, 0, DirtyReason::DidSave)`
3. File watcher: `tracker.mark_dirty(uri, 1, DirtyReason::FileWatcher)`
4. Background task: Periodically checks `should_flush()` → `drain_dirty()` → incremental index

### Component 2: Incremental Symbol Linker

**Location**: `src/lsp/backend/symbols.rs` (ENHANCED)

**Current Implementation** (lines 1-100):
```rust
pub(super) async fn link_symbols(&self) {
    // Problem: Iterates ALL documents every time!
    for doc_entry in self.workspace.documents.iter() {
        let (uri, doc) = (doc_entry.key(), doc_entry.value());
        // ... traverse IR, build cross-references ...
    }
}
```

**New Implementation**:
```rust
/// Links symbols across all files (full rebuild - used only for workspace init)
pub(super) async fn link_symbols(&self) {
    self.link_symbols_full().await
}

/// FULL symbol linking - rebuilds all cross-references from scratch
/// Only used during workspace initialization
async fn link_symbols_full(&self) {
    // Existing implementation (unchanged)
    for doc_entry in self.workspace.documents.iter() {
        // ... existing code ...
    }
}

/// INCREMENTAL symbol linking - updates only changed files
/// Used for didChange, didSave, file watcher events
pub(super) async fn link_symbols_incremental(&self, dirty_uris: &[Url]) {
    use tracing::debug;

    debug!("Incremental symbol linking for {} files", dirty_uris.len());

    // Phase 1: Remove old symbols from dirty files
    for uri in dirty_uris {
        // Remove from rholang_symbols (lock-free)
        let removed_contracts = self.workspace.rholang_symbols.remove_contracts_from_uri(uri);
        let removed_refs = self.workspace.rholang_symbols.remove_references_from_uri(uri);
        debug!("Removed {} contracts and {} references from {}",
               removed_contracts, removed_refs, uri);

        // Remove from global_table (needs write lock - but only once for batch)
        let mut global_table = self.workspace.global_table.write().await;
        global_table.symbols.retain(|_, s| &s.declaration_uri != uri);
        drop(global_table);
    }

    // Phase 2: Re-index dirty files
    for uri in dirty_uris {
        if let Some(doc) = self.workspace.documents.get(uri) {
            // Symbols already added during process_document_blocking()
            // Just need to re-run SymbolIndexBuilder for global_index

            let mut index_builder = SymbolIndexBuilder::new(
                self.workspace.global_index.clone(),
                uri.clone(),
                doc.positions.clone(),
            );
            index_builder.index_tree(&doc.ir);

            debug!("Re-indexed symbols for {}", uri);
        }
    }

    // Phase 3: Rebuild cross-file references for dirty files ONLY
    let global_table = self.workspace.global_table.read().await;

    for uri in dirty_uris {
        if let Some(doc) = self.workspace.documents.get(uri) {
            // Build inverted index (declaration → references) for THIS file only
            // This is much faster than rebuilding for entire workspace
            for symbol_entry in global_table.symbols.iter() {
                let (_, symbol) = (symbol_entry.key(), symbol_entry.value());

                // Check if symbol is declared or used in this file
                if &symbol.declaration_uri == uri {
                    // Symbol declared in dirty file - rebuild its reference list
                    // ... (existing logic from link_symbols) ...
                }
            }
        }
    }

    drop(global_table);

    debug!("Incremental symbol linking complete for {} files", dirty_uris.len());
}
```

**Performance Analysis**:
- **Old `link_symbols()`**: O(n × m) - iterate all n files, m symbols each
  - 1000 files × 100 symbols = 100,000 iterations
  - Time: ~1-5 seconds

- **New `link_symbols_incremental()`**: O(k × m) - iterate only k dirty files
  - 1 file × 100 symbols = 100 iterations
  - Time: ~5-50ms
  - **Speedup**: 1000x for single-file changes!

### Component 3: Incremental Completion Index

**Location**: `src/lsp/features/completion/indexing.rs` (ENHANCED)

**Current Implementation**:
```rust
/// Phase 4: Populate completion index (10-50ms for 1000 symbols)
pub fn populate_from_symbol_table(
    index: &WorkspaceCompletionIndex,
    symbol_table: &SymbolTable,
) {
    // Problem: Rebuilds ENTIRE index every time!
    for (_, symbol) in symbol_table.symbols.iter() {
        index.insert(symbol.name.clone(), SymbolMetadata { ... });
    }
}
```

**New Implementation**:
```rust
/// Remove all symbols from a specific file (leverages Phase 10 deletion)
pub fn remove_symbols_from_file(
    index: &WorkspaceCompletionIndex,
    uri: &Url,
) {
    // Track which symbols belong to which files (NEW metadata)
    // Uses file-to-symbols mapping in WorkspaceCompletionIndex

    if let Some(symbols) = index.get_symbols_for_file(uri) {
        for symbol_name in symbols {
            index.remove_term(&symbol_name);
        }
    }

    // Clear file tracking
    index.clear_file_symbols(uri);
}

/// Add symbols from a specific file to the index
pub fn insert_symbols_from_file(
    index: &WorkspaceCompletionIndex,
    uri: &Url,
    symbol_table: &SymbolTable,
) {
    for (_, symbol) in symbol_table.symbols.iter() {
        index.insert(symbol.name.clone(), SymbolMetadata { ... });

        // Track that this symbol belongs to this file
        index.add_file_symbol_mapping(uri, &symbol.name);
    }
}

/// Incremental update: remove old symbols, insert new ones
pub fn update_symbols_for_file(
    index: &WorkspaceCompletionIndex,
    uri: &Url,
    symbol_table: &SymbolTable,
) {
    // Phase 1: Remove old symbols from this file
    remove_symbols_from_file(index, uri);

    // Phase 2: Insert new symbols from this file
    insert_symbols_from_file(index, uri, symbol_table);

    // Phase 3: Check if compaction needed (Phase 10)
    if index.needs_compaction() {
        index.compact_dictionary();
    }
}
```

**Required Changes to `WorkspaceCompletionIndex`**:
```rust
// In src/lsp/features/completion/dictionary.rs

pub struct WorkspaceCompletionIndex {
    // Existing fields
    static_dict: DoubleArrayTrie,
    dynamic_dict: Arc<RwLock<DynamicDawg>>,
    metadata_map: Arc<RwLock<HashMap<String, SymbolMetadata>>>,

    // NEW: Track file → symbols mapping for incremental updates
    file_symbols: Arc<RwLock<HashMap<Url, HashSet<String>>>>,
}

impl WorkspaceCompletionIndex {
    /// Get all symbols belonging to a file
    pub fn get_symbols_for_file(&self, uri: &Url) -> Option<HashSet<String>> {
        let file_symbols = self.file_symbols.read();
        file_symbols.get(uri).cloned()
    }

    /// Track that a symbol belongs to a file
    pub fn add_file_symbol_mapping(&self, uri: &Url, symbol: &str) {
        let mut file_symbols = self.file_symbols.write();
        file_symbols
            .entry(uri.clone())
            .or_insert_with(HashSet::new)
            .insert(symbol.to_string());
    }

    /// Clear all symbol mappings for a file
    pub fn clear_file_symbols(&self, uri: &Url) {
        let mut file_symbols = self.file_symbols.write();
        file_symbols.remove(uri);
    }
}
```

### Component 4: Incremental Update Pipeline

**Location**: `src/lsp/backend/handlers.rs` (MODIFIED)

**Current Flow**:
```rust
pub(super) async fn did_change(&self, params: DidChangeTextDocumentParams) {
    // 1. Update document text
    // 2. Re-parse file
    // 3. Re-index file
    // 4. link_symbols() ← FULL REBUILD!
    // 5. Validate
}
```

**New Flow (Phase 11)**:
```rust
pub(super) async fn did_change(&self, params: DidChangeTextDocumentParams) {
    // 1. Update document text (unchanged)
    // 2. Re-parse file (unchanged)
    // 3. Re-index file (unchanged)

    // 4. Mark dirty (NEW)
    self.dirty_tracker.mark_dirty(
        params.text_document.uri.clone(),
        0,  // High priority (open file)
        DirtyReason::DidChange,
    );

    // 5. Trigger debounced incremental update (NEW)
    //    Background task will batch changes and call link_symbols_incremental()

    // 6. Validate (unchanged)
}
```

**Background Debouncing Task**:
```rust
/// Spawned during server initialization
async fn incremental_indexing_loop(backend: Arc<RholangBackend>) {
    use tokio::time::{sleep, Duration};

    loop {
        sleep(Duration::from_millis(100)).await;

        // Check if we have dirty files to flush
        if backend.dirty_tracker.should_flush() {
            let dirty_files = backend.dirty_tracker.drain_dirty();

            if !dirty_files.is_empty() {
                let uris: Vec<Url> = dirty_files
                    .iter()
                    .map(|(uri, _)| uri.clone())
                    .collect();

                // Incremental symbol linking (10-100x faster!)
                backend.link_symbols_incremental(&uris).await;

                // Incremental completion index update
                for uri in &uris {
                    if let Some(doc) = backend.workspace.documents.get(uri) {
                        update_symbols_for_file(
                            &backend.workspace.completion_index,
                            uri,
                            &doc.symbol_table,
                        );
                    }
                }

                tracing::info!(
                    "Incremental indexing complete for {} files",
                    uris.len()
                );
            }
        }
    }
}
```

## Implementation Plan

### Phase 11.1: Dirty File Tracking (Foundation)

**Files to Create**:
- `src/lsp/backend/dirty_tracker.rs` - DirtyFileTracker implementation

**Files to Modify**:
- `src/lsp/backend/state.rs` - Add `dirty_tracker: Arc<DirtyFileTracker>` field
- `src/lsp/backend/handlers.rs` - Mark files dirty in `didChange`, `didSave`, `didOpen`
- `src/lsp/backend/indexing.rs` - Mark files dirty in file watcher handler

**Tests**:
- `test_dirty_tracker_single_file()` - Mark and drain single file
- `test_dirty_tracker_batch()` - Mark multiple files, drain batch
- `test_dirty_tracker_priority()` - High-priority files come first
- `test_dirty_tracker_debounce()` - Debouncing window works

**Acceptance Criteria**:
- ✅ Dirty files tracked correctly
- ✅ Batching works within 100ms window
- ✅ Priority ordering works (open files before workspace files)

### Phase 11.2: Incremental Symbol Linking

**Files to Modify**:
- `src/lsp/backend/symbols.rs` - Add `link_symbols_incremental()`
- `src/lsp/backend/indexing.rs` - Call incremental linker for dirty files

**Tests**:
- `test_incremental_symbol_linking_single_file()` - Change one file, verify only it is re-linked
- `test_incremental_symbol_linking_cross_refs()` - Verify cross-file references update correctly
- `test_incremental_vs_full_equivalence()` - Incremental produces same result as full rebuild

**Benchmarks**:
```rust
#[bench]
fn bench_full_symbol_linking_1000_files(b: &mut Bencher) {
    // Measure current O(n) approach
}

#[bench]
fn bench_incremental_symbol_linking_1_file(b: &mut Bencher) {
    // Measure new O(1) approach
}
```

**Acceptance Criteria**:
- ✅ Incremental linking produces identical results to full rebuild
- ✅ 10-100x faster for single-file changes
- ✅ Cross-file references remain correct

### Phase 11.3: Incremental Completion Index

**Files to Modify**:
- `src/lsp/features/completion/dictionary.rs` - Add file → symbols tracking
- `src/lsp/features/completion/indexing.rs` - Add incremental update functions

**Tests**:
- `test_incremental_completion_remove()` - Remove symbols from file
- `test_incremental_completion_insert()` - Insert symbols from file
- `test_incremental_completion_update()` - Remove + insert together
- `test_completion_file_tracking()` - File → symbols mapping works

**Benchmarks**:
```rust
#[bench]
fn bench_full_completion_rebuild_1000_symbols(b: &mut Bencher) {
    // Measure current full rebuild
}

#[bench]
fn bench_incremental_completion_update_100_symbols(b: &mut Bencher) {
    // Measure incremental update
}
```

**Acceptance Criteria**:
- ✅ Incremental update produces identical index to full rebuild
- ✅ 5-50x faster for single-file changes
- ✅ Compaction triggers correctly (Phase 10 integration)

### Phase 11.4: Background Debouncing Task

**Files to Modify**:
- `src/lsp/backend/reactive.rs` - Add incremental indexing loop
- `src/lsp/backend.rs` - Spawn background task during initialization

**Tests**:
- `test_debouncing_batches_changes()` - Multiple changes within 100ms batched
- `test_incremental_pipeline_end_to_end()` - Full pipeline works

**Acceptance Criteria**:
- ✅ Changes batched within 100ms window
- ✅ High-priority files processed first
- ✅ Background task doesn't block LSP requests

### Phase 11.5: Performance Validation

**Benchmarks**:
```rust
// Location: benches/incremental_indexing.rs

#[bench]
fn bench_didChange_current_1000_files(b: &mut Bencher) {
    // Baseline: Current implementation with 1000-file workspace
    // Expected: ~1-5 seconds per change
}

#[bench]
fn bench_didChange_phase11_1000_files(b: &mut Bencher) {
    // Phase 11: Incremental implementation with 1000-file workspace
    // Expected: ~5-50ms per change
    // Target: 10-100x faster
}

#[bench]
fn bench_batch_changes_10_files(b: &mut Bencher) {
    // Phase 11: Batch update of 10 files
    // Expected: ~50-500ms total
}
```

**Profiling**:
```bash
# Generate flamegraph for full indexing
CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph \
    --bench incremental_indexing \
    -- --bench bench_didChange_current_1000_files

# Generate flamegraph for incremental indexing
CARGO_PROFILE_RELEASE_DEBUG=true cargo flamegraph \
    --bench incremental_indexing \
    -- --bench bench_didChange_phase11_1000_files
```

**Acceptance Criteria**:
- ✅ Single-file change: 10-100x faster (1-5s → 5-50ms)
- ✅ Batch changes: Linear scaling with dirty count
- ✅ Memory overhead: <5MB additional
- ✅ No regressions in completion or goto-definition accuracy

## Testing Strategy

### Unit Tests

**Dirty Tracker**:
- Mark and drain single file
- Mark multiple files with priorities
- Debouncing window timing
- Clear dirty set after drain

**Symbol Linker**:
- Incremental vs full equivalence
- Cross-file references remain correct
- Remove symbols from dirty files
- Re-add symbols for dirty files

**Completion Index**:
- File → symbols mapping
- Remove symbols from file
- Insert symbols from file
- Compaction integration

### Integration Tests

**End-to-End Pipeline**:
```rust
#[tokio::test]
async fn test_incremental_indexing_pipeline() {
    // 1. Initialize workspace with 100 files
    let backend = create_test_backend().await;
    for i in 0..100 {
        index_test_file(&backend, i).await;
    }

    // 2. Modify one file
    let uri = test_file_uri(50);
    backend.did_change(change_params(&uri)).await;

    // 3. Wait for incremental indexing (debounce + process)
    tokio::time::sleep(Duration::from_millis(200)).await;

    // 4. Verify symbol linking is correct
    let symbols = backend.workspace.rholang_symbols.get_all();
    assert_correct_cross_references(&symbols);

    // 5. Verify completion index is correct
    let completions = backend.workspace.completion_index.query_prefix("test");
    assert_completions_match_full_rebuild(&completions);
}
```

**Performance Tests**:
```rust
#[tokio::test]
async fn test_incremental_faster_than_full() {
    let backend = create_test_backend_with_1000_files().await;

    // Measure full rebuild
    let start = std::time::Instant::now();
    backend.link_symbols().await;
    let full_time = start.elapsed();

    // Measure incremental update
    let start = std::time::Instant::now();
    backend.link_symbols_incremental(&[test_file_uri(0)]).await;
    let incremental_time = start.elapsed();

    // Verify speedup
    let speedup = full_time.as_millis() / incremental_time.as_millis();
    assert!(speedup >= 10, "Expected 10x+ speedup, got {}x", speedup);
}
```

### Regression Tests

**Existing Features Must Work**:
- ✅ Goto-definition across files
- ✅ Find references across files
- ✅ Code completion includes all symbols
- ✅ Workspace symbols search
- ✅ Document symbols
- ✅ Rename across files

## Rollout Plan

### Stage 1: Feature Flag (Week 1)

Add feature flag to enable/disable incremental indexing:

```rust
// In Cargo.toml
[features]
default = []
incremental-indexing = []

// In code
#[cfg(feature = "incremental-indexing")]
async fn index_changed_file(&self, uri: &Url) {
    self.dirty_tracker.mark_dirty(uri.clone(), 0, DirtyReason::DidChange);
    // ... incremental path ...
}

#[cfg(not(feature = "incremental-indexing"))]
async fn index_changed_file(&self, uri: &Url) {
    // ... existing full rebuild path ...
}
```

### Stage 2: Internal Testing (Week 2)

- Enable feature flag for development
- Test with rholang-language-server codebase (50+ files)
- Test with larger Rholang projects (100-1000 files)
- Monitor for correctness issues

### Stage 3: Beta Release (Week 3)

- Enable by default in beta builds
- Collect user feedback
- Monitor error rates via telemetry

### Stage 4: General Availability (Week 4)

- Remove feature flag
- Enable by default for all users
- Document in CHANGELOG.md

## Risks and Mitigation

### Risk 1: Correctness Issues

**Risk**: Incremental updates might miss dependencies, causing stale symbols

**Mitigation**:
- Extensive testing (see Testing Strategy)
- Equivalence tests: `assert_eq!(incremental_result, full_rebuild_result)`
- Fallback mechanism: If incremental update fails, fall back to full rebuild
- Logging: Track all incremental operations for debugging

### Risk 2: Memory Overhead

**Risk**: Tracking file → symbols mappings increases memory usage

**Mitigation**:
- Measure memory usage with `heaptrack` or `valgrind`
- Optimize mapping structures (use `SmallVec`, `HashSet` instead of `Vec`)
- Target: <5MB additional memory for 10,000 files

### Risk 3: Race Conditions

**Risk**: Concurrent file changes might cause dirty tracking to miss updates

**Mitigation**:
- Use lock-free `DashMap` for dirty tracker
- Atomic operations for debounce timing
- Integration tests with concurrent changes

### Risk 4: Performance Regression

**Risk**: Incremental path might be slower for small workspaces

**Mitigation**:
- Benchmark both paths
- Use heuristic: If workspace <100 files, use full rebuild (cheaper overhead)
- Profiling to identify bottlenecks

## Success Metrics

### Performance Targets

| Metric | Baseline (Current) | Target (Phase 11) | Stretch Goal |
|--------|-------------------|-------------------|--------------|
| Single file change (1000 files) | 1-5s | 5-50ms | <10ms |
| Batch 10 files (1000 files) | 10-50s | 50-500ms | <100ms |
| Workspace init (1000 files) | 1-60s | 1-60s | <30s (future) |
| Memory overhead | 0MB | <5MB | <2MB |

### User Experience Metrics

- **Typing Lag**: <50ms from keystroke to UI update (currently 100ms-5s)
- **Save Lag**: <100ms from save to validation (currently 1-10s)
- **File Watch Lag**: <200ms for external changes (currently 1-10s)

### Code Quality Metrics

- **Test Coverage**: >90% for incremental indexing code
- **Benchmark Coverage**: All critical paths benchmarked
- **Documentation**: Complete API docs and architecture guide

## Future Enhancements

### Phase 11+: Dependency Tracking

**Problem**: Some file changes affect other files (e.g., changing a contract signature)

**Solution**: Track file dependencies and mark dependent files dirty

**Expected Improvement**: Even smarter incremental updates

### Phase 12: Parallel Incremental Indexing

**Problem**: Batching 100+ dirty files still takes time

**Solution**: Use Rayon to process dirty files in parallel

**Expected Improvement**: 4-8x faster batch updates

### Phase 13: Persistent Workspace Cache

**Problem**: Workspace initialization still O(n) - slow for large projects

**Solution**: Serialize workspace state to disk, reload on startup

**Expected Improvement**: 10-100x faster startup (1-60s → <5s)

## References

- **Phase 10 Documentation**: `docs/phase_10_deletion_support.md` - Symbol deletion API
- **Phase 9 Documentation**: `docs/phase_9_prefix_zipper_integration.md` - Completion index
- **Current Indexing**: `src/lsp/backend/indexing.rs` - Existing implementation
- **Current Symbol Linking**: `src/lsp/backend/symbols.rs` - Full rebuild logic
- **Completion Index**: `src/lsp/features/completion/dictionary.rs` - Phase 10 deletion support

---

**Status**: Ready for Implementation
**Next Steps**: Begin Phase 11.1 (Dirty File Tracking)
