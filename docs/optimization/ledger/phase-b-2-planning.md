# Phase B-2: Document IR Caching - Planning

**Status**: 📋 **PLANNING**
**Date Started**: 2025-11-13
**Expected Duration**: 5-7 days (1 week)
**Prerequisites**: Phase B-1 complete ✅
**Priority**: **HIGH** ⭐⭐

## Executive Summary

**Problem**: LSP operations repeatedly parse the same unchanged files, wasting CPU time and increasing latency.

**Solution**: Cache parsed IR + symbol tables with hash-based invalidation and LRU eviction.

**Predicted Speedup**: **8-10x** for repeated operations on the same file
- Current: Re-parse every operation (~5ms)
- With caching: Hash lookup (~100µs) + IR clone (~500µs) = ~600µs
- **Speedup**: 5ms → 600µs ≈ **8.3x faster**

**User Impact**: **MEDIUM-HIGH**
- Benefits frequent operations on same files (hover, diagnostics, goto-definition)
- Complements Phase B-1 incremental indexing
- Most noticeable when repeatedly querying the same file

---

## Problem Analysis

### Current Behavior (Wasteful Re-parsing)

**Scenario**: User working on `contract.rho` (1KB file, 50 LOC)

```
1. User hovers over symbol "myContract"
   → LSP parses contract.rho (3ms)
   → Builds IR + symbol tables (2ms)
   → Returns hover info
   → **Total: 5ms**

2. User triggers goto-definition on same file (2 seconds later)
   → LSP parses contract.rho AGAIN (3ms)
   → Builds IR + symbol tables AGAIN (2ms)
   → Returns definition location
   → **Total: 5ms**

3. User requests diagnostics (5 seconds later)
   → LSP parses contract.rho AGAIN (3ms)
   → Builds IR + symbol tables AGAIN (2ms)
   → Returns diagnostics
   → **Total: 5ms**

WASTED TIME: 10ms (2 redundant parses)
```

**Why This Happens**:
- Current architecture: Stateless operation model
- Each LSP request treated independently
- No memory of previous parses
- File content unchanged → redundant work

**Frequency**:
- Typical user session: 10-50 operations per file
- Active file: 5-10 operations per minute
- **Total waste**: 50-250ms per file per minute

---

## Proposed Solution

### Architecture Overview

```text
┌────────────────────────────────────────────────────────────┐
│                    LSP Request Handler                      │
└────────────────────────────────────────────────────────────┘
                            ↓
                    Check IR Cache
                            ↓
              ┌─────────────────────────┐
              │   Cache Hit?            │
              └─────────────────────────┘
                   ↙               ↘
              YES                    NO
               ↓                     ↓
    ┌──────────────────┐   ┌──────────────────┐
    │ Return Cached IR │   │  Parse File      │
    │ (~600µs)         │   │  (~5ms)          │
    └──────────────────┘   └──────────────────┘
                                    ↓
                          ┌──────────────────┐
                          │ Store in Cache   │
                          └──────────────────┘
                                    ↓
                          ┌──────────────────┐
                          │ Return Fresh IR  │
                          └──────────────────┘
```

### Data Structure

**Cache Entry**:
```rust
pub struct CachedDocument {
    /// File URI
    uri: Url,

    /// File content hash (blake3)
    content_hash: [u8; 32],

    /// Last modified timestamp
    modified_at: std::time::SystemTime,

    /// Parsed IR tree
    ir: Arc<DocumentIR>,

    /// Symbol table
    symbol_table: Arc<SymbolTable>,

    /// Parsed Tree-Sitter tree
    tree: Arc<Tree>,

    /// Cache insertion time (for LRU)
    cached_at: std::time::Instant,
}
```

**Cache Structure**:
```rust
pub struct DocumentCache {
    /// LRU cache: URI → CachedDocument
    entries: lru::LruCache<Url, CachedDocument>,

    /// Maximum cache size
    max_entries: usize,

    /// Cache statistics
    stats: CacheStats,
}

pub struct CacheStats {
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
    invalidations: AtomicU64,
}
```

### Cache Operations

**1. Cache Lookup (Fast Path)**:
```rust
pub fn get(&self, uri: &Url, content_hash: &[u8; 32]) -> Option<Arc<CachedDocument>> {
    self.entries.get(uri).and_then(|cached| {
        if cached.content_hash == *content_hash {
            self.stats.hits.fetch_add(1, Ordering::Relaxed);
            Some(cached.clone())
        } else {
            // Hash mismatch → file changed
            self.stats.invalidations.fetch_add(1, Ordering::Relaxed);
            None
        }
    }).or_else(|| {
        self.stats.misses.fetch_add(1, Ordering::Relaxed);
        None
    })
}
```

**2. Cache Insert**:
```rust
pub fn insert(&mut self, uri: Url, cached_doc: CachedDocument) {
    if let Some(evicted) = self.entries.push(uri, cached_doc) {
        self.stats.evictions.fetch_add(1, Ordering::Relaxed);
    }
}
```

**3. Cache Invalidation** (on file change):
```rust
pub fn invalidate(&mut self, uri: &Url) {
    if self.entries.pop(uri).is_some() {
        self.stats.invalidations.fetch_add(1, Ordering::Relaxed);
    }
}
```

### Invalidation Strategy

**Triggers for invalidation**:
1. **File modification detected** (via `did_change` LSP event)
2. **File saved** (via `did_save` LSP event)
3. **File closed then reopened** (via `did_close` + `did_open`)
4. **Manual invalidation** (via LSP command or debug tool)

**Hash-based validation**:
- Compute `blake3` hash of file content (< 100µs for typical files)
- Compare against cached hash
- Mismatch → Cache miss → Re-parse

**Why blake3**:
- Extremely fast: ~100µs for 1KB file
- Cryptographically secure (no collision risk)
- Better than timestamp-only (catches external edits)

---

## Implementation Plan

### Component B-2.1: Cache Data Structure (1 day)

**Location**: `src/lsp/backend/document_cache.rs`

**Tasks**:
1. Define `CachedDocument` struct
2. Implement `DocumentCache` with `lru::LruCache`
3. Add cache statistics tracking
4. Thread-safe wrapper with `RwLock` or `DashMap`

**API**:
```rust
impl DocumentCache {
    pub fn new(max_entries: usize) -> Self;
    pub fn get(&self, uri: &Url, content_hash: &[u8; 32]) -> Option<Arc<CachedDocument>>;
    pub fn insert(&mut self, uri: Url, cached_doc: CachedDocument);
    pub fn invalidate(&mut self, uri: &Url);
    pub fn clear(&mut self);
    pub fn stats(&self) -> &CacheStats;
}
```

**Tests**:
- Basic insert/get operations
- LRU eviction behavior
- Hash-based invalidation
- Thread-safe concurrent access

---

### Component B-2.2: Hash Computation (1 day)

**Location**: `src/lsp/backend/content_hash.rs`

**Tasks**:
1. Integrate `blake3` crate (add to `Cargo.toml`)
2. Implement fast content hashing
3. Benchmark hashing performance
4. Handle edge cases (empty files, large files)

**API**:
```rust
pub fn compute_content_hash(content: &str) -> [u8; 32] {
    blake3::hash(content.as_bytes()).into()
}

pub fn hash_file(path: &Path) -> io::Result<[u8; 32]> {
    let content = std::fs::read_to_string(path)?;
    Ok(compute_content_hash(&content))
}
```

**Performance Target**: < 200µs for 1KB file

---

### Component B-2.3: Cache Integration with LSP Handlers (1-2 days)

**Location**: `src/lsp/backend/state.rs`, `src/lsp/backend/handlers.rs`

**Tasks**:
1. Add `DocumentCache` to `RholangBackend`
2. Update LSP handlers to check cache first
3. Integrate with `did_change` / `did_save` for invalidation
4. Handle cache misses gracefully (fallback to parse)

**Modified Handlers**:
- `goto_definition`
- `hover`
- `document_symbols`
- `references`
- `rename`

**Example Integration** (hover handler):
```rust
pub async fn hover(&self, params: HoverParams) -> LspResult<Option<Hover>> {
    let uri = params.text_document_position_params.text_document.uri;
    let position = params.text_document_position_params.position;

    // Step 1: Get file content
    let content = self.read_file(&uri).await?;
    let content_hash = compute_content_hash(&content);

    // Step 2: Check cache
    if let Some(cached_doc) = self.document_cache.get(&uri, &content_hash) {
        // Cache hit → Fast path
        return self.hover_from_cached(&cached_doc, position).await;
    }

    // Step 3: Cache miss → Parse + cache
    let doc = self.parse_and_cache(uri, content, content_hash).await?;
    self.hover_from_cached(&doc, position).await
}
```

---

### Component B-2.4: LRU Eviction Policy (1 day)

**Location**: `src/lsp/backend/document_cache.rs`

**Tasks**:
1. Configure LRU cache size (default: 50 files)
2. Implement eviction callback (optional cleanup)
3. Monitor memory usage
4. Tune cache size based on workspace characteristics

**Cache Size Heuristics**:
- Small workspace (<50 files): Cache all files
- Medium workspace (50-200 files): Cache 50 most recent
- Large workspace (>200 files): Cache 100 most recent

**Configuration**:
```rust
impl DocumentCache {
    pub fn with_capacity(max_entries: usize) -> Self {
        Self {
            entries: lru::LruCache::new(max_entries),
            max_entries,
            stats: CacheStats::default(),
        }
    }
}
```

---

### Component B-2.5: Testing and Validation (1-2 days)

**Test Suite** (`tests/test_document_cache.rs`):

1. **Basic Operations**:
   - Insert → Get (cache hit)
   - Insert → Modify → Get (cache miss)
   - Insert → Invalidate → Get (cache miss)

2. **LRU Behavior**:
   - Insert 60 files into 50-capacity cache
   - Verify oldest 10 evicted
   - Verify LRU ordering maintained

3. **Hash-based Invalidation**:
   - Cache file with hash H1
   - Modify file (hash becomes H2)
   - Verify cache miss on lookup with H2

4. **Integration Tests**:
   - LSP hover on same file (3 times) → Verify 1 parse, 2 cache hits
   - LSP goto-definition after hover → Verify cache hit
   - File modification between requests → Verify cache invalidation

5. **Performance Tests**:
   - Measure cache hit latency (<1ms)
   - Measure cache miss latency (~5ms, same as no cache)
   - Verify 8-10x speedup for repeated operations

**Benchmark Scenarios**:
```rust
#[bench]
fn bench_cache_hit(b: &mut Bencher) {
    // Measure time to retrieve cached document
    // Target: < 600µs (hash lookup + Arc clone)
}

#[bench]
fn bench_cache_miss(b: &mut Bencher) {
    // Measure time for parse + cache insert
    // Should be ~ same as no cache (~5ms)
}
```

---

### Component B-2.6: Documentation (1 day)

**Documents to Create**:
1. `docs/optimization/ledger/phase-b-2-architecture.md` - Architecture overview
2. `docs/optimization/ledger/phase-b-2-progress-summary.md` - Progress tracking
3. Update `README.md` - Document caching feature
4. Code comments - Explain cache invalidation strategy

---

## Performance Analysis

### Baseline (No Cache)

**Scenario**: User hovers over symbol in `contract.rho` (1KB file)

| Operation | Time | Description |
|-----------|------|-------------|
| Read file | 200µs | I/O from disk (cached by OS) |
| Compute hash | - | N/A (not computed) |
| Tree-Sitter parse | 2ms | Parse to CST |
| Convert to IR | 1.5ms | CST → DocumentIR |
| Build symbol table | 1.5ms | Traverse IR |
| **Total** | **5.2ms** | Per operation |

**For 3 operations**: 5.2ms × 3 = **15.6ms total**

### With Cache (Phase B-2)

**First Operation (Cache Miss)**:
| Operation | Time | Description |
|-----------|------|-------------|
| Read file | 200µs | I/O from disk |
| Compute hash | 100µs | Blake3 hash |
| Cache lookup | 50µs | HashMap get (miss) |
| Tree-Sitter parse | 2ms | Parse to CST |
| Convert to IR | 1.5ms | CST → DocumentIR |
| Build symbol table | 1.5ms | Traverse IR |
| Cache insert | 100µs | LRU cache push |
| **Total** | **5.45ms** | Slightly slower (hash overhead) |

**Second Operation (Cache Hit)**:
| Operation | Time | Description |
|-----------|------|-------------|
| Read file | 200µs | I/O from disk (for hash) |
| Compute hash | 100µs | Blake3 hash |
| Cache lookup | 50µs | HashMap get (hit) |
| Arc clone | 250µs | Clone Arc pointers |
| **Total** | **600µs** | **8.7x faster** |

**Third Operation (Cache Hit)**:
| Operation | Time | Description |
|-----------|------|-------------|
| (Same as second) | 600µs | Cache hit |

**For 3 operations**: 5.45ms + 600µs + 600µs = **6.65ms total**

**Speedup**: 15.6ms → 6.65ms = **2.35x faster overall**
**Cache hit speedup**: 5.2ms → 600µs = **8.7x faster per hit**

### Memory Overhead

**Per Cached Document**:
- `DocumentIR`: ~10-50KB (depends on file size)
- `SymbolTable`: ~5-20KB
- `Tree`: ~20-100KB (Tree-Sitter tree)
- `Metadata`: ~1KB
- **Total per file**: ~36-171KB

**For 50-file cache**: ~1.8-8.5MB
**For 100-file cache**: ~3.6-17MB

**Acceptable**: <20MB overhead for significant speedup

---

## Success Metrics

### Quantitative Metrics

1. **Cache Hit Rate**: >80% for typical workflow
2. **Cache Hit Latency**: <1ms (target: 600µs)
3. **Speedup for Repeated Operations**: >8x
4. **Memory Overhead**: <20MB for 100-file cache
5. **No Regressions**: All existing tests pass

### Qualitative Metrics

1. **User Perceived Responsiveness**: Faster hover/goto-definition
2. **Stability**: No increase in bug reports
3. **Maintainability**: Code remains understandable

---

## Risks and Mitigation

### Risk 1: Cache Invalidation Bugs

**Symptom**: Stale data shown to user after file modification

**Mitigation**:
- Hash-based validation (catches ALL content changes)
- Comprehensive integration tests
- Feature flag for disabling cache (fallback)
- Cache statistics for monitoring

### Risk 2: Memory Pressure

**Symptom**: Excessive memory usage for large workspaces

**Mitigation**:
- Configurable cache size
- LRU eviction policy
- Monitor cache size in telemetry
- Document memory overhead in user guide

### Risk 3: Hash Computation Overhead

**Symptom**: Hashing adds latency to operations

**Mitigation**:
- Use extremely fast `blake3` (~100µs for 1KB)
- Benchmark hash performance
- Consider timestamp-only validation for MVP

---

## Timeline

### Conservative Estimate (7 days)

| Day | Component | Tasks |
|-----|-----------|-------|
| 1 | B-2.1 | Cache data structure + basic tests |
| 2 | B-2.2 | Hash computation + benchmarks |
| 3 | B-2.3 | LSP handler integration (part 1) |
| 4 | B-2.3 | LSP handler integration (part 2) |
| 5 | B-2.4 | LRU eviction + configuration |
| 6 | B-2.5 | Testing + validation |
| 7 | B-2.6 | Documentation |

### Optimistic Estimate (5 days)

| Day | Component | Tasks |
|-----|-----------|-------|
| 1 | B-2.1 + B-2.2 | Cache structure + hashing |
| 2 | B-2.3 | LSP handler integration |
| 3 | B-2.4 | LRU eviction |
| 4 | B-2.5 | Testing |
| 5 | B-2.6 | Documentation |

---

## Dependencies

### Crates to Add

```toml
[dependencies]
blake3 = "1.5"  # Fast content hashing
lru = "0.12"    # LRU cache implementation
```

### Prerequisites

- Phase B-1 complete ✅
- Existing IR structures support `Arc` cloning ✅
- LSP handlers support async operations ✅

---

## Next Actions

1. **Today**: Create cache data structure (B-2.1)
2. **Tomorrow**: Implement hash computation (B-2.2)
3. **Day 3**: Integrate with LSP handlers (B-2.3)
4. **Day 4-5**: Testing and validation (B-2.5)
5. **Day 6**: Documentation (B-2.6)

---

**Phase B-2 Status**: 📋 **PLANNING COMPLETE**
**Next Step**: Implement cache data structure (B-2.1)
**Expected Completion**: 2025-11-20 (1 week from start)
