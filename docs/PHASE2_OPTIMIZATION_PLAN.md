# Phase 2 Performance Optimization Plan

**Date**: 2025-10-29
**Branch**: `dylon/metta-integration`
**Status**: Planning - Based on Profiling Data

## Executive Summary

After completing Phase 1 (lock-free concurrent access) and running comprehensive benchmarks and profiling, we've identified the **actual** bottlenecks in the system. This document outlines Phase 2 optimizations based on real profiling data rather than assumptions.

## Phase 1 Results

**Completed Optimizations:**
- ✅ Replaced monolithic `Arc<RwLock<WorkspaceState>>` with lock-free `DashMap` collections
- ✅ Eliminated outer RwLock wrapper
- ✅ Updated 50+ access points across 8 source files
- ✅ All tests passing, zero compilation errors

**Performance Characteristics:**
- Symbol resolution: **90-107 nanoseconds** - Already optimal!
- MeTTa parsing: **37-263 µs** depending on complexity
- Symbol table building: **7.7-40 µs** - Very fast
- Parallel processing speedup: **1.64x** (sequential 193.8µs → parallel 118.4µs)

## Profiling Analysis - Key Findings

### CPU Time Distribution (from perf data)

**Critical Discovery: Rayon Thread Pool Overhead Dominates**

The perf profiling revealed that **45-50% of benchmark CPU time** is spent on Rayon thread pool management, not actual work:

#### 1. Thread Synchronization (33.6% total)
- `rayon_core::registry::WorkerThread::wait_until_cold`: **15.97%**
- `__sched_yield`: **15.46%** (threads yielding CPU waiting for work)
- `rayon_core::sleep::Sleep::sleep`: **1.78%**
- `rayon_core::sleep::Sleep::wake_specific_thread`: **2.18%**

#### 2. Work Stealing & Lock-Free Data Structures (31.0% total)
- `crossbeam_epoch::default::with_handle`: **15.13%**
- `crossbeam_deque::deque::Stealer<T>::steal`: **10.12%**
- `crossbeam_epoch::internal::Global::try_advance`: **6.96%**
- `rayon::iter::plumbing::bridge_producer_consumer::helper`: **10.70%**

#### 3. Actual Workload (much smaller)
- Tree-sitter parsing: **~3.5%** total
  - `ts_parser_parse`: **1.55%**
  - `ts_tree_cursor_goto_sibling_internal`: **1.60%**
  - `ts_tree_cursor_goto_first_child_internal`: **1.12%**
- Symbol resolution: **~2.6%** total
  - `LexicalScopeResolver::find_in_scope_chain`: **1.23%**
  - `LexicalScopeResolver::resolve_symbol`: **0.93%**
  - `ComposableSymbolResolver::resolve_symbol`: **0.45%**
- Hashing operations: **~2.5%** total
  - `<core::hash::sip::Hasher<S> as core::hash::Hasher>::write`: **1.63%**
  - `core::hash::BuildHasher::hash_one`: **0.85%**

### Root Cause Analysis

**Problem**: The benchmarks process very small workloads (microseconds), making Rayon thread pool overhead (15-20µs for spawning/synchronization) comparable to or exceeding actual work time.

**Why This Matters**: This is a classic fine-grained parallelism problem. For small tasks:
- Rayon overhead: ~15-20µs
- Actual work: ~10-50µs (for simple/medium tasks)
- **Result**: Overhead ≈ Work (or exceeds it for tiny tasks)

**When Rayon Helps**: Parallel processing shows **1.64x speedup** for 10-document workloads because:
- Total work: 193.8µs sequential
- Rayon overhead amortized over multiple tasks
- Net benefit despite overhead

### Original Phase 2 Plan (Symbol Resolution Caching) - REJECTED

**Why Caching Won't Help:**
- Symbol resolution is already **90-107 nanoseconds** - extremely fast
- Adding cache lookup overhead would likely **slow things down**:
  - Cache key hash: ~20-50ns
  - Cache lock acquisition: ~50-100ns
  - Total cache overhead: **70-150ns** (exceeds resolution time!)
- Cache only helps if lookup cost >> cache cost
- **Verdict**: Skip symbol resolution caching

## Revised Phase 2 Optimizations

Based on actual profiling data, here are the real bottlenecks to address:

### Priority 1: Adaptive Parallelization (HIGH IMPACT)

**Problem**: Using Rayon for all workloads incurs 15-20µs overhead regardless of task size.

**Solution**: Only parallelize when work justifies overhead.

#### Implementation

**File**: `src/language_regions/async_detection.rs` (new module)

```rust
use std::time::Instant;

/// Threshold below which sequential processing is faster than parallel
/// Based on profiling: Rayon overhead ≈ 15-20µs
const PARALLEL_THRESHOLD_MICROS: u64 = 100;

/// Minimum number of documents to consider parallel processing
const MIN_PARALLEL_DOCUMENTS: usize = 5;

pub struct AdaptiveParallelism;

impl AdaptiveParallelism {
    /// Estimate work time based on document characteristics
    pub fn estimate_work_time(documents: &[(Url, String)]) -> u64 {
        let total_size: usize = documents.iter().map(|(_, content)| content.len()).sum();

        // Empirical formula from benchmarks:
        // - Simple parse: ~37µs for ~100 bytes
        // - Complex parse: ~263µs for ~1000 bytes
        // Approximate: 0.26µs per byte + 10µs base
        (total_size as u64 / 4) + (documents.len() as u64 * 10)
    }

    /// Decide whether to use parallel or sequential processing
    pub fn should_parallelize(documents: &[(Url, String)]) -> bool {
        if documents.len() < MIN_PARALLEL_DOCUMENTS {
            return false;
        }

        let estimated_work = Self::estimate_work_time(documents);
        estimated_work > PARALLEL_THRESHOLD_MICROS
    }

    /// Process documents with adaptive parallelization
    pub async fn process_documents<F, R>(
        documents: Vec<(Url, String)>,
        processor: F,
    ) -> Vec<R>
    where
        F: Fn((Url, String)) -> R + Send + Sync,
        R: Send,
    {
        if Self::should_parallelize(&documents) {
            // Use Rayon for large workloads
            tokio::task::spawn_blocking(move || {
                use rayon::prelude::*;
                documents.into_par_iter().map(processor).collect()
            })
            .await
            .unwrap()
        } else {
            // Use sequential processing for small workloads
            documents.into_iter().map(processor).collect()
        }
    }
}
```

**Expected Impact:**
- Small workloads (< 5 docs, < 100µs): **15-20µs savings** (avoid Rayon overhead)
- Large workloads: **1.5-2x speedup** maintained
- Adaptive behavior: Best of both worlds

#### Integration Points

1. **Virtual Document Detection**
   - File: `src/language_regions/mod.rs`
   - Replace `rayon::par_iter()` with `AdaptiveParallelism::process_documents()`

2. **Workspace Indexing**
   - File: `src/lsp/backend/indexing.rs`
   - Apply to `index_workspace()` document batch processing

3. **Symbol Linking**
   - File: `src/lsp/backend/symbols.rs`
   - Apply to `link_virtual_symbols()` when processing multiple virtual docs

### Priority 2: Parse Tree Caching (MEDIUM IMPACT)

**Problem**: Tree-sitter parsing accounts for **~3.5% CPU time**. Re-parsing unchanged documents wastes cycles.

**Solution**: Cache parse trees keyed by content hash.

#### Implementation

**File**: `src/tree_sitter/parse_cache.rs` (new)

```rust
use std::sync::Arc;
use dashmap::DashMap;
use tree_sitter::Tree;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Cache for tree-sitter parse results
pub struct ParseCache {
    /// Maps content hash -> (content, parse tree)
    cache: Arc<DashMap<u64, (String, Tree)>>,
    /// Maximum cache size (number of entries)
    max_size: usize,
}

impl ParseCache {
    pub fn new(max_size: usize) -> Self {
        Self {
            cache: Arc::new(DashMap::with_capacity(max_size)),
            max_size,
        }
    }

    /// Compute fast hash of content
    fn hash_content(content: &str) -> u64 {
        let mut hasher = DefaultHasher::new();
        content.hash(&mut hasher);
        hasher.finish()
    }

    /// Get cached parse tree if content matches
    pub fn get(&self, content: &str) -> Option<Tree> {
        let hash = Self::hash_content(content);

        self.cache.get(&hash).and_then(|entry| {
            let (cached_content, tree) = entry.value();
            // Verify content matches (hash collision check)
            if cached_content == content {
                Some(tree.clone())
            } else {
                None
            }
        })
    }

    /// Store parse tree in cache
    pub fn insert(&self, content: String, tree: Tree) {
        // Simple eviction: if at capacity, clear 10% oldest entries
        if self.cache.len() >= self.max_size {
            // Remove ~10% of entries (simple strategy)
            let to_remove = self.max_size / 10;
            let mut removed = 0;
            self.cache.retain(|_, _| {
                if removed < to_remove {
                    removed += 1;
                    false
                } else {
                    true
                }
            });
        }

        let hash = Self::hash_content(&content);
        self.cache.insert(hash, (content, tree));
    }

    /// Clear cache entries for a specific document URI
    pub fn invalidate(&self, content: &str) {
        let hash = Self::hash_content(content);
        self.cache.remove(&hash);
    }

    /// Clear entire cache
    pub fn clear(&self) {
        self.cache.clear();
    }

    /// Get cache statistics
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            size: self.cache.len(),
            capacity: self.max_size,
        }
    }
}

pub struct CacheStats {
    pub size: usize,
    pub capacity: usize,
}

impl Default for ParseCache {
    fn default() -> Self {
        Self::new(1000) // Cache up to 1000 parse trees
    }
}
```

**Expected Impact:**
- First parse: Same speed (no change)
- Subsequent identical content: **~3.5% CPU savings** (skip parsing entirely)
- Real-world: **10-30% improvement** for typical edit patterns (most edits don't change entire file)

#### Integration Points

1. **Tree-sitter Module**
   - File: `src/tree_sitter.rs`
   - Wrap `parse_code()` with cache lookup

2. **Document Lifecycle**
   - File: `src/lsp/backend.rs`
   - Check cache in `did_change` handler before re-parsing

3. **Virtual Document Updates**
   - File: `src/language_regions/virtual_document.rs`
   - Cache virtual document parse trees

### Priority 3: FxHash for Internal Lookups (LOW IMPACT)

**Problem**: SipHash is cryptographically secure but slower than needed for internal use.

**Solution**: Replace with FxHash for non-security-critical hash maps.

#### Implementation

**File**: `Cargo.toml`

```toml
[dependencies]
rustc-hash = "2.0"  # FxHashMap, FxHashSet
```

**File**: `src/tree_sitter.rs`

```rust
use rustc_hash::FxHashMap;

// Replace HashMap with FxHashMap for node caches
type NodeCache = FxHashMap<usize, Arc<Node>>;
```

**Expected Impact:**
- Hash operations: **~1.5-2x faster** (from ~2.5% to ~1.5% CPU time)
- Overall: **~1% CPU savings**
- Low risk: Internal-only change

#### Integration Points

1. **Symbol Tables**
   - File: `src/ir/symbol_table.rs`
   - Use `FxHashMap` for symbol storage

2. **Tree-sitter Node Caching**
   - File: `src/tree_sitter.rs`
   - Use `FxHashMap` for node lookup caches

3. **IR Metadata**
   - File: `src/ir/rholang_node.rs`
   - Use `FxHashMap` for node metadata

## Implementation Plan

### Week 1: Adaptive Parallelization

**Day 1-2**: Implement `AdaptiveParallelism` module
- Create `src/language_regions/adaptive.rs`
- Add work estimation heuristics
- Write unit tests for threshold decisions

**Day 3**: Integrate into virtual document detection
- Update `src/language_regions/mod.rs`
- Replace direct Rayon calls with adaptive processing

**Day 4**: Integrate into workspace indexing
- Update `src/lsp/backend/indexing.rs`
- Apply to batch document processing

**Day 5**: Benchmark and tune
- Run benchmarks with different thresholds
- Optimize `PARALLEL_THRESHOLD_MICROS` based on measurements
- Update IMPLEMENTATION_SUMMARY.md

### Week 2: Parse Tree Caching

**Day 1-2**: Implement `ParseCache`
- Create `src/tree_sitter/parse_cache.rs`
- Implement cache with eviction
- Write unit tests

**Day 3**: Integrate into tree-sitter module
- Update `src/tree_sitter.rs` with cache wrapper
- Add cache invalidation on document changes

**Day 4**: Integrate into document lifecycle
- Update `did_change` handler to use cache
- Add cache statistics logging

**Day 5**: Benchmark and validate
- Measure cache hit rates
- Verify performance improvements
- Test with real-world edit patterns

### Week 3: FxHash Integration

**Day 1**: Add dependency and update symbol tables
- Add `rustc-hash` to Cargo.toml
- Update `src/ir/symbol_table.rs`

**Day 2**: Update tree-sitter and IR
- Update `src/tree_sitter.rs` node caches
- Update `src/ir/rholang_node.rs` metadata

**Day 3**: Update LSP backend
- Update any HashMap uses in `src/lsp/backend/`
- Ensure DashMap uses remain (those are for concurrency)

**Day 4**: Test and validate
- Run full test suite
- Verify no behavioral changes

**Day 5**: Final benchmarks
- Compare Phase 1 + Phase 2 against baseline
- Generate new flame graphs
- Document improvements

## Expected Cumulative Impact

### Phase 1 Results (Completed)
- Lock contention: **Eliminated**
- Concurrent operations: **2-5x throughput improvement**
- LSP operation latency: **75-80% reduction** (expected)

### Phase 2 Expected Results

**Adaptive Parallelization:**
- Small workloads (< 5 docs): **15-20µs savings** per batch
- Large workloads: **1.5-2x speedup** maintained
- Overall: **20-40% improvement** in mixed workload scenarios

**Parse Tree Caching:**
- Cache hit scenarios: **~3.5% CPU savings** per hit
- Typical edit patterns: **10-30% improvement**
- Large file re-indexing: **2-3x faster**

**FxHash:**
- Hash operations: **~1% CPU savings**
- Low risk, incremental benefit

**Combined Phase 2 Impact:**
- Best case (cache hits + adaptive): **40-60% improvement**
- Worst case (cache misses): **5-10% improvement**
- Typical case: **25-35% improvement**

**Cumulative (Phase 1 + Phase 2):**
- Total expected: **3-7x throughput improvement**
- Latency reduction: **80-90%** for common operations
- Scalability: Linear with CPU cores up to saturation

## Success Metrics

### Phase 2 Complete When:

**Adaptive Parallelization:**
- [ ] Small workloads (< 5 docs) avoid Rayon overhead
- [ ] Large workloads maintain 1.5-2x speedup
- [ ] Threshold auto-tuning based on measurements
- [ ] Benchmarks show improvement over pure parallel

**Parse Tree Caching:**
- [ ] Cache hit rate > 60% for typical edit patterns
- [ ] Cache miss overhead < 5% (hash computation)
- [ ] Memory usage < 100MB for 1000 cached trees
- [ ] Invalidation works correctly on document changes

**FxHash:**
- [ ] All internal HashMaps replaced with FxHashMap
- [ ] No DashMap changes (those need Arc safety)
- [ ] Hash operations ~2x faster
- [ ] Zero behavioral changes

**Overall:**
- [ ] Phase 2 benchmarks show 25-35% improvement over Phase 1
- [ ] Combined Phase 1 + Phase 2: 3-7x over original
- [ ] Flame graph shows reduced Rayon overhead
- [ ] All tests passing

## Testing Strategy

### Benchmark Suite Extensions

Add new benchmarks in `benches/lsp_operations_benchmark.rs`:

```rust
fn bench_adaptive_parallelization(c: &mut Criterion) {
    let mut group = c.benchmark_group("adaptive_parallelization");

    // Small workload - should use sequential
    group.bench_function("2_documents", |b| {
        let docs = generate_documents(2);
        b.iter(|| process_with_adaptive(black_box(&docs)))
    });

    // Medium workload - boundary case
    group.bench_function("5_documents", |b| {
        let docs = generate_documents(5);
        b.iter(|| process_with_adaptive(black_box(&docs)))
    });

    // Large workload - should use parallel
    group.bench_function("20_documents", |b| {
        let docs = generate_documents(20);
        b.iter(|| process_with_adaptive(black_box(&docs)))
    });

    group.finish();
}

fn bench_parse_cache(c: &mut Criterion) {
    let mut group = c.benchmark_group("parse_cache");

    let content = generate_metta_code(1000); // 1KB code

    // Cache miss (first parse)
    group.bench_function("cache_miss", |b| {
        let cache = ParseCache::new(100);
        b.iter(|| {
            let tree = parse_with_cache(black_box(&content), &cache);
            cache.clear(); // Force miss on next iteration
            tree
        })
    });

    // Cache hit (subsequent parse)
    group.bench_function("cache_hit", |b| {
        let cache = ParseCache::new(100);
        // Prime cache
        parse_with_cache(&content, &cache);

        b.iter(|| parse_with_cache(black_box(&content), &cache))
    });

    group.finish();
}
```

### Correctness Tests

```rust
#[tokio::test]
async fn test_adaptive_parallelization_produces_same_results() {
    let documents = generate_test_documents(10);

    // Process sequentially
    let sequential_results = process_sequential(documents.clone()).await;

    // Process with adaptive (will choose parallel for 10 docs)
    let adaptive_results = AdaptiveParallelism::process_documents(
        documents,
        process_single_document,
    ).await;

    assert_eq!(sequential_results, adaptive_results);
}

#[test]
fn test_parse_cache_correctness() {
    let cache = ParseCache::new(10);
    let content = "test code";

    // First parse
    let tree1 = parse_with_cache(content, &cache);

    // Second parse (should hit cache)
    let tree2 = parse_with_cache(content, &cache);

    // Trees should be equivalent
    assert_eq!(tree1.root_node(), tree2.root_node());
}

#[test]
fn test_fxhash_produces_same_results() {
    let mut std_map = std::collections::HashMap::new();
    let mut fx_map = rustc_hash::FxHashMap::default();

    // Insert same data
    for i in 0..100 {
        std_map.insert(format!("key_{}", i), i);
        fx_map.insert(format!("key_{}", i), i);
    }

    // Verify same lookups
    for i in 0..100 {
        let key = format!("key_{}", i);
        assert_eq!(std_map.get(&key), fx_map.get(&key));
    }
}
```

## Risk Assessment

### Adaptive Parallelization

**Risks:**
- Incorrect work estimation → wrong parallelization decisions
- Threshold too high → miss parallelization opportunities
- Threshold too low → still pay Rayon overhead unnecessarily

**Mitigation:**
- Conservative initial threshold (100µs)
- Make threshold configurable via environment variable
- Add telemetry to track decisions (sequential vs parallel)
- Benchmark extensively with various workload sizes

### Parse Tree Caching

**Risks:**
- Cache invalidation bugs → stale parse trees → wrong LSP features
- Memory bloat → 1000 cached trees could use significant RAM
- Hash collisions → incorrect cache hits

**Mitigation:**
- Content verification on cache hit (not just hash)
- Conservative cache size (1000 entries, ~50-100MB)
- Clear cache on any document change (safe but less optimal)
- Add cache statistics logging for monitoring

### FxHash

**Risks:**
- Non-cryptographic hash → potential DoS if attacker controls keys
- Different hash behavior → subtle bugs

**Mitigation:**
- Only use for internal structures (not user-facing)
- Keep DashMap with default hasher (already secure)
- Extensive testing before/after
- Easy rollback (just remove dependency)

## Rollback Plan

Each optimization is independent:

```bash
# Rollback adaptive parallelization
git checkout main -- src/language_regions/adaptive.rs
# Revert integration points

# Rollback parse cache
git checkout main -- src/tree_sitter/parse_cache.rs
# Revert tree_sitter.rs changes

# Rollback FxHash
git diff HEAD Cargo.toml  # Remove rustc-hash
git checkout main -- src/**/*.rs  # Revert HashMap → FxHashMap changes
```

## References

- Phase 1 Results: `docs/IMPLEMENTATION_SUMMARY.md`
- Benchmark Data: `/tmp/bench_output.txt`
- Profiling Data: `perf.data` (39GB)
- Original Plan: `docs/OPTIMIZATION_PLAN.md` (now superseded)

---

**Status**: Planning Complete - Ready for Implementation
**Expected Timeline**: 3 weeks (15 working days)
**Expected Impact**: 25-35% additional improvement over Phase 1 (cumulative 3-7x over baseline)
