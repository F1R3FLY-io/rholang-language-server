# Performance Optimization Implementation Plan

This document outlines the specific optimizations to implement based on profiling analysis and architectural review.

## Current Architecture Analysis

### Lock Contention Points (CRITICAL)

**Location**: `src/lsp/models.rs:116-130` - `WorkspaceState` structure

```rust
pub struct WorkspaceState {
    pub documents: HashMap<Url, Arc<CachedDocument>>,
    pub global_symbols: HashMap<String, (Url, IrPosition)>,
    pub global_table: Arc<SymbolTable>,
    pub global_inverted_index: HashMap<(Url, IrPosition), Vec<(Url, IrPosition)>>,
    pub global_contracts: Vec<(Url, Arc<RholangNode>)>,
    pub global_calls: Vec<(Url, Arc<RholangNode>)>,
    pub global_index: Arc<RwLock<GlobalSymbolIndex>>,
    pub global_virtual_symbols: HashMap<String, HashMap<String, Vec<(Url, Range)>>>,
}
```

**Problem**: Single RwLock protecting entire `WorkspaceState` causes:
- LSP operations block each other
- High CPU with low throughput
- Write operations (indexing) block all reads (goto_definition, references, etc.)

**Impact**: 2-5x throughput improvement expected

### Optimized Architecture

#### Phase 1: Granular Locking (IMMEDIATE - High Impact)

Replace monolithic `HashMap` structures with lock-free `DashMap`:

```rust
pub struct WorkspaceState {
    // Lock-free concurrent access (most frequently accessed)
    pub documents: Arc<DashMap<Url, Arc<CachedDocument>>>,
    pub global_symbols: Arc<DashMap<String, (Url, IrPosition)>>,

    // Separate locks for infrequent updates
    pub global_table: Arc<RwLock<SymbolTable>>,
    pub global_inverted_index: Arc<RwLock<HashMap<(Url, IrPosition), Vec<(Url, IrPosition)>>>>,

    // Lock-free concurrent collections
    pub global_contracts: Arc<DashMap<Url, Vec<Arc<RholangNode>>>>,
    pub global_calls: Arc<DashMap<Url, Vec<Arc<RholangNode>>>>,

    // Already has separate lock - keep as is
    pub global_index: Arc<RwLock<GlobalSymbolIndex>>,

    // Nested lock-free structure
    pub global_virtual_symbols: Arc<DashMap<String, DashMap<String, Vec<(Url, Range)>>>>,
}
```

**Benefits**:
- Reads never block each other
- Writes only block on specific key
- No contention for hot paths (document lookup, symbol resolution)
- Scales linearly with CPU cores

#### Phase 2: Symbol Resolution Caching (HIGH Impact)

Add LRU cache for resolved symbols:

```rust
use lru::LruCache;
use std::sync::Mutex;

pub struct CachedSymbolResolver {
    base: Box<dyn SymbolResolver>,
    cache: Arc<Mutex<LruCache<(String, Position, Url), Vec<SymbolLocation>>>>,
    hit_count: Arc<AtomicU64>,
    miss_count: Arc<AtomicU64>,
}
```

**Expected Improvement**: 5-10x faster for repeated lookups

**Cache Strategy**:
- Cache size: 10,000 entries (configurable)
- Eviction: LRU (least recently used)
- Invalidation: On document change (by URI)
- Key: `(symbol_name, position, document_uri)`

#### Phase 3: Parallel Workspace Indexing (Already Implemented)

The hybrid tokio/rayon strategy is already in place via `async_detection.rs`. Validate performance:

```rust
// Already implemented in src/language_regions/async_detection.rs
pub async fn detect_virtual_documents(
    documents: Vec<(Url, String)>,
) -> Vec<VirtualDocument> {
    // Uses tokio::spawn_blocking + rayon internally
}
```

**Validation**: Run benchmarks to confirm 2-4x speedup on multi-core

## Implementation Order

### Priority 1: Lock Contention (IMMEDIATE)

**Files to Modify**:
1. `src/lsp/models.rs` - Update `WorkspaceState` structure
2. `src/lsp/backend/workspace.rs` - Update all workspace access patterns
3. `src/lsp/backend/indexing.rs` - Update indexing logic
4. `src/lsp/backend/symbols.rs` - Update symbol linking logic

**Steps**:
1. Add `dashmap` dependency (already present in `Cargo.toml`)
2. Update `WorkspaceState` structure
3. Update all read/write access patterns
4. Run tests to ensure correctness
5. Benchmark before/after

**Estimated Impact**: 2-5x throughput improvement

### Priority 2: Symbol Resolution Caching

**Files to Create/Modify**:
1. `src/ir/symbol_resolution/cached.rs` - New caching layer
2. `src/lsp/backend/metta.rs` - Use cached resolver
3. `src/lsp/backend/handlers.rs` - Use cached resolver for Rholang

**Steps**:
1. Add `lru` crate to `Cargo.toml`
2. Implement `CachedSymbolResolver`
3. Wire into LSP handlers
4. Add cache invalidation on document change
5. Add metrics (hit/miss rates)

**Estimated Impact**: 5-10x for repeated lookups

### Priority 3: Performance Validation

**Generate Flame Graphs**:
```bash
# Before optimizations
cargo flamegraph --output baseline.svg --bench lsp_operations_benchmark

# After Phase 1
cargo flamegraph --output phase1.svg --bench lsp_operations_benchmark

# After Phase 2
cargo flamegraph --output phase2.svg --bench lsp_operations_benchmark

# Compare
firefox baseline.svg phase1.svg phase2.svg
```

**Run Benchmarks**:
```bash
# Save baseline
cargo bench -- --save-baseline before-opt

# After each phase
cargo bench -- --baseline before-opt
```

## Detailed Implementation: Phase 1

### Step 1: Update WorkspaceState

```rust
// src/lsp/models.rs
use dashmap::DashMap;

pub struct WorkspaceState {
    // Lock-free concurrent document cache
    pub documents: Arc<DashMap<Url, Arc<CachedDocument>>>,

    // Lock-free global symbols
    pub global_symbols: Arc<DashMap<String, (Url, IrPosition)>>,

    // Separate lock for symbol table (infrequent updates)
    pub global_table: Arc<tokio::sync::RwLock<SymbolTable>>,

    // Separate lock for inverted index
    pub global_inverted_index: Arc<tokio::sync::RwLock<
        HashMap<(Url, IrPosition), Vec<(Url, IrPosition)>>
    >>,

    // Lock-free contract/call tracking
    pub global_contracts: Arc<DashMap<Url, Vec<Arc<RholangNode>>>>,
    pub global_calls: Arc<DashMap<Url, Vec<Arc<RholangNode>>>>,

    // Keep existing (already has separate lock)
    pub global_index: Arc<tokio::sync::RwLock<GlobalSymbolIndex>>,

    // Nested DashMap for virtual symbols (language -> symbol -> locations)
    pub global_virtual_symbols: Arc<DashMap<String, Arc<DashMap<String, Vec<(Url, Range)>>>>>,
}

impl WorkspaceState {
    pub fn new() -> Self {
        Self {
            documents: Arc::new(DashMap::new()),
            global_symbols: Arc::new(DashMap::new()),
            global_table: Arc::new(tokio::sync::RwLock::new(SymbolTable::new())),
            global_inverted_index: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            global_contracts: Arc::new(DashMap::new()),
            global_calls: Arc::new(DashMap::new()),
            global_index: Arc::new(tokio::sync::RwLock::new(GlobalSymbolIndex::new())),
            global_virtual_symbols: Arc::new(DashMap::new()),
        }
    }
}
```

### Step 2: Update Access Patterns

**Before (monolithic RwLock)**:
```rust
// src/lsp/backend/handlers.rs
let workspace = self.workspace.read().await;
let doc = workspace.documents.get(&uri);
```

**After (lock-free DashMap)**:
```rust
// src/lsp/backend/handlers.rs
let workspace = &self.workspace; // No lock!
let doc = workspace.documents.get(&uri);
```

**Symbol Lookup (Before)**:
```rust
let workspace = self.workspace.read().await;
if let Some(loc) = workspace.global_symbols.get(symbol_name) {
    // ...
}
```

**Symbol Lookup (After)**:
```rust
// No lock needed!
if let Some(entry) = self.workspace.global_symbols.get(symbol_name) {
    let (uri, pos) = entry.value();
    // ...
}
```

### Step 3: Update Indexing Logic

**Concurrent Document Insertion**:
```rust
// src/lsp/backend/indexing.rs
pub async fn index_document(&self, uri: Url, text: String) {
    // Parse and analyze
    let cached_doc = self.parse_and_analyze(&uri, &text).await;

    // Insert without blocking other operations
    self.workspace.documents.insert(uri.clone(), Arc::new(cached_doc));

    // Extract and index symbols (also lock-free)
    for (symbol, pos) in &cached_doc.symbol_table.symbols {
        self.workspace.global_symbols.insert(symbol.clone(), (uri.clone(), pos.clone()));
    }
}
```

**Parallel Indexing with Rayon**:
```rust
use rayon::prelude::*;

pub async fn index_workspace(&self, root_path: &Path) {
    let files: Vec<PathBuf> = collect_rho_files(root_path);

    // Process in parallel using Rayon
    files.par_iter().for_each(|file_path| {
        if let Ok(text) = std::fs::read_to_string(file_path) {
            let uri = Url::from_file_path(file_path).unwrap();

            // Each worker can insert concurrently without blocking
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    self.index_document(uri, text).await;
                })
            });
        }
    });
}
```

## Detailed Implementation: Phase 2

### Cached Symbol Resolver

```rust
// src/ir/symbol_resolution/cached.rs
use lru::LruCache;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};

pub struct CachedSymbolResolver {
    base: Box<dyn SymbolResolver>,
    cache: Arc<Mutex<LruCache<CacheKey, Vec<SymbolLocation>>>>,
    hit_count: Arc<AtomicU64>,
    miss_count: Arc<AtomicU64>,
}

#[derive(Hash, Eq, PartialEq, Clone)]
struct CacheKey {
    symbol_name: String,
    position: Position,
    uri: Url,
}

impl CachedSymbolResolver {
    pub fn new(base: Box<dyn SymbolResolver>, cache_size: usize) -> Self {
        Self {
            base,
            cache: Arc::new(Mutex::new(LruCache::new(cache_size))),
            hit_count: Arc::new(AtomicU64::new(0)),
            miss_count: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn invalidate_document(&self, uri: &Url) {
        let mut cache = self.cache.lock().unwrap();
        cache.retain(|key, _| &key.uri != uri);
    }

    pub fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hit_count.load(Ordering::Relaxed),
            misses: self.miss_count.load(Ordering::Relaxed),
            size: self.cache.lock().unwrap().len(),
        }
    }
}

impl SymbolResolver for CachedSymbolResolver {
    fn resolve_symbol(
        &self,
        symbol_name: &str,
        position: &Position,
        context: &ResolutionContext,
    ) -> Vec<SymbolLocation> {
        let key = CacheKey {
            symbol_name: symbol_name.to_string(),
            position: *position,
            uri: context.uri.clone(),
        };

        // Try cache first
        {
            let mut cache = self.cache.lock().unwrap();
            if let Some(cached) = cache.get(&key) {
                self.hit_count.fetch_add(1, Ordering::Relaxed);
                return cached.clone();
            }
        }

        // Cache miss - resolve and cache
        self.miss_count.fetch_add(1, Ordering::Relaxed);
        let result = self.base.resolve_symbol(symbol_name, position, context);

        let mut cache = self.cache.lock().unwrap();
        cache.put(key, result.clone());

        result
    }

    fn supports_language(&self, language: &str) -> bool {
        self.base.supports_language(language)
    }
}

#[derive(Debug)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub size: usize,
}
```

### Integration

```rust
// src/lsp/backend/metta.rs
use crate::ir::symbol_resolution::cached::CachedSymbolResolver;

impl RholangBackend {
    pub(super) async fn goto_definition_metta(
        &self,
        virtual_doc: &Arc<VirtualDocument>,
        position: LspPosition,
    ) -> LspResult<Option<GotoDefinitionResponse>> {
        // Create cached resolver
        let base = Box::new(ComposableSymbolResolver::new(
            Box::new(LexicalScopeResolver::new(/* ... */)),
            vec![Box::new(MettaPatternFilter::new(/* ... */))],
            Some(Box::new(AsyncGlobalVirtualSymbolResolver::new(/* ... */))),
        ));

        let cached_resolver = CachedSymbolResolver::new(base, 10_000);

        // Use cached resolver
        let locations = cached_resolver.resolve_symbol(/* ... */);

        // ... rest of implementation
    }
}
```

## Performance Metrics

### Before Optimization (Baseline)

**Expected metrics**:
- goto_definition: 50-200ms (high variance due to lock contention)
- references: 100-500ms
- rename: 200-1000ms
- Workspace indexing (100 files): 5-15 seconds (sequential bottleneck)

### After Phase 1 (Target)

- goto_definition: 10-50ms (80% reduction)
- references: 20-100ms (80% reduction)
- rename: 50-200ms (75% reduction)
- Workspace indexing (100 files): 2-5 seconds (3-5x improvement)

### After Phase 2 (Target)

- goto_definition: 1-10ms (90-95% reduction for cached)
- references: 5-20ms (90-95% reduction for cached)
- rename: 20-50ms (90% reduction for cached)
- Cache hit rate: >80% for typical usage

## Testing Strategy

### Correctness Tests

```rust
#[tokio::test]
async fn test_concurrent_document_access() {
    let workspace = WorkspaceState::new();

    // Spawn 100 concurrent readers
    let handles: Vec<_> = (0..100).map(|i| {
        let ws = workspace.clone();
        tokio::spawn(async move {
            ws.documents.get(&test_uri(i));
        })
    }).collect();

    // All should complete without deadlock
    for handle in handles {
        handle.await.unwrap();
    }
}

#[tokio::test]
async fn test_concurrent_symbol_lookup() {
    let workspace = WorkspaceState::new();

    // Concurrent readers + writer
    let read_handles: Vec<_> = (0..50).map(|_| {
        let ws = workspace.clone();
        tokio::spawn(async move {
            ws.global_symbols.get("test_symbol");
        })
    }).collect();

    let write_handle = {
        let ws = workspace.clone();
        tokio::spawn(async move {
            ws.global_symbols.insert("new_symbol".to_string(), (test_uri(), test_pos()));
        })
    };

    // All should complete
    for handle in read_handles {
        handle.await.unwrap();
    }
    write_handle.await.unwrap();
}
```

### Performance Benchmarks

```rust
// benches/workspace_operations.rs
fn bench_concurrent_document_access(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let workspace = rt.block_on(async {
        let ws = WorkspaceState::new();
        // Populate with test data
        for i in 0..1000 {
            ws.documents.insert(test_uri(i), test_doc());
        }
        ws
    });

    c.bench_function("concurrent_document_access", |b| {
        b.iter(|| {
            rt.block_on(async {
                let handles: Vec<_> = (0..100).map(|i| {
                    let ws = workspace.clone();
                    tokio::spawn(async move {
                        black_box(ws.documents.get(&test_uri(i % 1000)));
                    })
                }).collect();

                for handle in handles {
                    handle.await.unwrap();
                }
            });
        })
    });
}
```

## Rollout Plan

1. **Day 1**: Implement Phase 1 (granular locking)
   - Update `WorkspaceState` structure
   - Update access patterns in handlers
   - Run correctness tests

2. **Day 2**: Validate Phase 1
   - Run benchmarks
   - Generate flame graphs
   - Fix any issues

3. **Day 3**: Implement Phase 2 (caching)
   - Implement `CachedSymbolResolver`
   - Wire into LSP handlers
   - Add cache invalidation

4. **Day 4**: Validate Phase 2
   - Run benchmarks
   - Monitor cache hit rates
   - Tune cache size

5. **Day 5**: Final validation
   - Full benchmark suite
   - Flame graph comparison
   - Documentation update

## Success Criteria

- [ ] Benchmark shows 2-5x improvement in throughput
- [ ] Flame graphs show reduced time in lock acquisition
- [ ] All tests pass
- [ ] No deadlocks or race conditions
- [ ] Cache hit rate >80% for typical usage
- [ ] LSP operations feel instant (<50ms P95)

## Monitoring

Add metrics to track performance in production:

```rust
pub struct PerformanceMetrics {
    pub goto_definition_latency: Histogram,
    pub references_latency: Histogram,
    pub rename_latency: Histogram,
    pub cache_hit_rate: Gauge,
    pub workspace_size: Gauge,
}
```

Log slow operations:
```rust
let start = Instant::now();
let result = goto_definition(params).await;
let elapsed = start.elapsed();

if elapsed.as_millis() > 100 {
    warn!("Slow goto_definition: {}ms for {}", elapsed.as_millis(), uri);
}
```

## References

- Performance Profiling Guide: `docs/PERFORMANCE_PROFILING_GUIDE.md`
- Virtual Language Architecture: `docs/VIRTUAL_LANGUAGE_EXTENSION_SYSTEM.md`
- DashMap Documentation: https://docs.rs/dashmap/
- LRU Cache: https://docs.rs/lru/
