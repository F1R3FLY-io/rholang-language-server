# Phase 1 Optimization: Implementation Status

## Overview

Implementing lock-free concurrent access to WorkspaceState using DashMap instead of a monolithic RwLock.

**Expected Impact**: 2-5x throughput improvement for LSP operations

## Completed Work

### 1. ✅ Core Data Structure Updates

**File**: `src/lsp/models.rs`

- Added `DashMap` import
- Updated `WorkspaceState` structure:
  - `documents`: `HashMap` → `Arc<DashMap<Url, Arc<CachedDocument>>>`
  - `global_symbols`: `HashMap` → `Arc<DashMap<String, (Url, IrPosition)>>`
  - `global_contracts`: `Vec` → `Arc<DashMap<Url, Vec<Arc<RholangNode>>>>`
  - `global_calls`: `Vec` → `Arc<DashMap<Url, Vec<Arc<RholangNode>>>>`
  - `global_virtual_symbols`: Nested HashMap → `Arc<DashMap<String, Arc<DashMap<...>>>>`
  - Kept separate `RwLock` for infrequent bulk operations:
    - `global_table`: `Arc<tokio::sync::RwLock<SymbolTable>>`
    - `global_inverted_index`: `Arc<tokio::sync::RwLock<HashMap<...>>>`
    - `global_index`: `Arc<tokio::sync::RwLock<GlobalSymbolIndex>>`
- Added `WorkspaceState::new()` constructor
- Added `Default` implementation

### 2. ✅ Backend State Updates

**File**: `src/lsp/backend/state.rs`

- Removed outer `RwLock` wrapper from `workspace` field
- Changed from `Arc<RwLock<WorkspaceState>>` to `Arc<WorkspaceState>`
- Added documentation explaining the optimization

### 3. ✅ Symbol Resolution Updates

**File**: `src/ir/symbol_resolution/global.rs`

- Updated `GlobalVirtualSymbolResolver` to use lock-free access
  - Changed constructor from `Arc<RwLock<WorkspaceState>>` → `Arc<WorkspaceState>`
  - Removed `.read().await` calls
  - Updated to use `DashMap` API (`.get()`, `.value()`)

- Updated `AsyncGlobalVirtualSymbolResolver` similarly
  - Lock-free concurrent access
  - No blocking on reads

### 4. ✅ Error Handling

**File**: `src/main.rs`

- Added comprehensive Rayon panic handler (lines 900-966)
- Matches tokio panic handler for consistency
- Logs to panic.log file
- Provides helpful context and stack overflow detection

## Remaining Work

### Priority 1: Fix Compilation Errors

The following files need updates to remove `.read().await` / `.write().await` calls:

#### `src/lsp/backend/symbols.rs`

**Errors**:
- Lines 32, 56, 139, 230: Remove `.read().await`
- Lines 111, 195: Remove `.write().await`

**Pattern to fix**:
```rust
// Before (with RwLock):
let workspace = self.workspace.read().await;
let doc = workspace.documents.get(&uri);

// After (with DashMap):
let doc = self.workspace.documents.get(&uri);
```

For nested access:
```rust
// Before:
let workspace = self.workspace.read().await;
workspace.global_virtual_symbols.get(lang).and_then(|m| m.get(sym))

// After:
self.workspace.global_virtual_symbols
    .get(lang)
    .and_then(|entry| entry.value().get(sym).map(|v| v.value().clone()))
```

#### Files with similar patterns to fix:

1. **`src/lsp/backend/indexing.rs`**
   - Document insertion: Direct `workspace.documents.insert()`
   - Symbol updates: Direct `workspace.global_symbols.insert()`
   - Contract/call tracking: Use `workspace.global_contracts.entry().or_insert(vec![])`

2. **`src/lsp/backend/handlers.rs`**
   - Document lookups: Direct `workspace.documents.get()`
   - Symbol resolution: Direct `workspace.global_symbols.get()`

### Priority 2: Update Access Patterns

#### Document Access

**Before (monolithic lock)**:
```rust
let workspace = self.workspace.read().await;
if let Some(cached) = workspace.documents.get(&uri) {
    // use cached
}
```

**After (lock-free)**:
```rust
// No lock acquisition - instant access
if let Some(entry) = self.workspace.documents.get(&uri) {
    let cached = entry.value();
    // use cached
}
```

#### Symbol Insertion (Indexing)

**Before**:
```rust
let mut workspace = self.workspace.write().await; // BLOCKS ALL READERS!
workspace.global_symbols.insert(name, (uri, pos));
```

**After**:
```rust
// Non-blocking - only locks this specific key
self.workspace.global_symbols.insert(name, (uri, pos));
```

#### Bulk Operations (Still Need Locks)

For operations that need consistency across multiple fields:

```rust
// Update global index (infrequent)
let mut global_index = self.workspace.global_index.write().await;
global_index.insert_batch(symbols);
```

### Priority 3: Testing

#### Correctness Tests

```rust
#[tokio::test]
async fn test_concurrent_document_access() {
    let backend = create_test_backend().await;

    // Spawn 100 concurrent readers
    let handles: Vec<_> = (0..100).map(|i| {
        let backend = backend.clone();
        tokio::spawn(async move {
            backend.workspace.documents.get(&test_uri(i));
        })
    }).collect();

    for handle in handles {
        handle.await.unwrap();
    }
}

#[tokio::test]
async fn test_concurrent_symbol_insert() {
    let backend = create_test_backend().await;

    // Concurrent inserts should not conflict
    let handles: Vec<_> = (0..100).map(|i| {
        let backend = backend.clone();
        tokio::spawn(async move {
            backend.workspace.global_symbols.insert(
                format!("symbol_{}", i),
                (test_uri(), test_pos())
            );
        })
    }).collect();

    for handle in handles {
        handle.await.unwrap();
    }

    // All symbols should be present
    assert_eq!(backend.workspace.global_symbols.len(), 100);
}
```

#### Performance Benchmarks

```bash
# Save baseline before optimization
git checkout main
cargo bench -- --save-baseline before-phase1

# After implementing Phase 1
git checkout dylon/metta-integration
cargo bench -- --baseline before-phase1

# Expected improvements:
# - goto_definition: 50-200ms → 10-50ms (75-80% reduction)
# - concurrent operations: 2-5x throughput
```

## Implementation Checklist

- [x] Update WorkspaceState structure with DashMap
- [x] Remove outer RwLock wrapper
- [x] Update GlobalVirtualSymbolResolver
- [x] Update AsyncGlobalVirtualSymbolResolver
- [ ] Fix symbols.rs access patterns (6 locations)
- [ ] Fix indexing.rs access patterns
- [ ] Fix handlers.rs access patterns
- [ ] Run full test suite
- [ ] Run benchmarks
- [ ] Generate flame graphs
- [ ] Verify no deadlocks/race conditions

## Migration Guide

### For New Code

Always access workspace fields directly (no `.read()` or `.write()`):

```rust
// Document lookup
if let Some(doc) = self.workspace.documents.get(&uri) {
    let cached = doc.value();
    // use cached...
}

// Symbol lookup
if let Some(entry) = self.workspace.global_symbols.get(symbol_name) {
    let (uri, pos) = entry.value();
    // use uri, pos...
}

// Insert/update (non-blocking for other keys)
self.workspace.documents.insert(uri, Arc::new(cached_doc));
self.workspace.global_symbols.insert(name.clone(), (uri.clone(), pos));
```

### For Bulk Operations

Only use locks for fields that need atomic multi-field updates:

```rust
// Bulk index update (infrequent)
let mut global_index = self.workspace.global_index.write().await;
global_index.rebuild_from_documents(&documents);
drop(global_index); // Release lock ASAP

// Bulk inverted index update
let mut inv_index = self.workspace.global_inverted_index.write().await;
inv_index.insert_batch(entries);
drop(inv_index);
```

## Performance Expectations

### Before Optimization

- **goto_definition**: 50-200ms (high variance due to lock contention)
- **Concurrent requests**: Serialize behind RwLock
- **Workspace indexing**: Blocks all LSP operations

### After Phase 1

- **goto_definition**: 10-50ms (80% reduction, consistent)
- **Concurrent requests**: True parallelism, 2-5x throughput
- **Workspace indexing**: Only blocks bulk operations, not document/symbol lookups

### Metrics to Track

```rust
// Add to logging
let start = Instant::now();
let result = self.workspace.documents.get(&uri);
let elapsed = start.elapsed();

if elapsed.as_micros() > 100 {
    warn!("Slow workspace access: {}μs", elapsed.as_micros());
}
```

Expected: <10μs for DashMap access (vs 100-1000μs with contended RwLock)

## Next Steps

1. **Complete Phase 1** (this document)
   - Fix remaining compilation errors
   - Update all access patterns
   - Test thoroughly

2. **Proceed to Phase 2** (symbol resolution caching)
   - Implement LRU cache layer
   - Expected 5-10x improvement for repeated lookups

3. **Validation**
   - Generate flame graphs showing reduced lock contention
   - Benchmark before/after comparison
   - Monitor production metrics

## References

- Optimization Plan: `docs/OPTIMIZATION_PLAN.md`
- DashMap Documentation: https://docs.rs/dashmap/
- Performance Profiling Guide: `docs/PERFORMANCE_PROFILING_GUIDE.md`
