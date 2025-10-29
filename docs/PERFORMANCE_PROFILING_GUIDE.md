# Performance Profiling and Optimization Guide

This document provides comprehensive guidance on profiling, benchmarking, and optimizing the Rholang Language Server, with special focus on the embedded language (MeTTa) system and threading model.

## Table of Contents

- [Benchmarking Suite](#benchmarking-suite)
- [Profiling Tools](#profiling-tools)
- [Threading Model Analysis](#threading-model-analysis)
- [Performance Bottleneck Areas](#performance-bottleneck-areas)
- [Optimization Strategies](#optimization-strategies)
- [Flame Graph Generation](#flame-graph-generation)
- [Continuous Performance Monitoring](#continuous-performance-monitoring)

## Benchmarking Suite

### Available Benchmarks

We have two comprehensive benchmark suites:

#### 1. Detection Worker Benchmark
Location: `benches/detection_worker_benchmark.rs`

**Purpose**: Compare threading strategies for virtual document detection

**Scenarios**:
- `spawn_blocking`: Current tokio `spawn_blocking` approach
- `rayon`: Pure Rayon parallel processing
- `hybrid`: Combines `spawn_blocking` with Rayon internally

**To run**:
```bash
cargo bench --bench detection_worker_benchmark
```

**Expected Results**:
- **spawn_blocking**: Best for async/await integration, ~100-500 req/s
- **rayon**: Best for pure CPU workloads, ~200-1000 req/s
- **hybrid**: Best balanced approach, ~300-800 req/s

#### 2. LSP Operations Benchmark
Location: `benches/lsp_operations_benchmark.rs`

**Purpose**: Measure performance of critical LSP operations

**Benchmarks included**:
1. **metta_parsing**: Parse MeTTa code to IR (simple vs complex)
2. **symbol_table_building**: Build symbol tables from IR
3. **symbol_resolution**: Resolve symbols using composable resolvers
4. **virtual_document_detection**: Detect embedded languages in Rholang
5. **end_to_end_virtual_doc**: Complete flow from detection to symbol tables
6. **parallel_processing**: Sequential vs Rayon parallel processing

**To run**:
```bash
# Run all LSP benchmarks
cargo bench --bench lsp_operations_benchmark

# Run specific benchmark group
cargo bench --bench lsp_operations_benchmark -- metta_parsing

# Save baseline for comparison
cargo bench --bench lsp_operations_benchmark -- --save-baseline my-baseline

# Compare against baseline
cargo bench --bench lsp_operations_benchmark -- --baseline my-baseline
```

**Expected Metrics**:
- MeTTa parsing (simple): ~50-200 μs
- MeTTa parsing (complex): ~200-1000 μs
- Symbol table building: ~100-500 μs
- Symbol resolution: ~10-50 μs (cached scopes)
- Virtual doc detection: ~100-300 μs
- Parallel processing: 2-4x speedup on multi-core

### Running All Benchmarks

```bash
# Run all benchmarks with HTML reports
cargo bench

# View results
open target/criterion/report/index.html
```

## Profiling Tools

###1. cargo-flamegraph (Recommended)

**Installation**:
```bash
cargo install flamegraph
```

**Usage**:
```bash
# Profile a specific test
cargo flamegraph --test metta_goto_definition_test -- test_goto_definition_simple

# Profile with frequency
sudo cargo flamegraph --freq 997 --test metta_goto_definition_test

# Generate SVG
cargo flamegraph --output flame.svg --test metta_goto_definition_test
```

**View flame graph**:
```bash
firefox flame.svg
```

### 2. perf (Linux)

**Installation**:
```bash
sudo apt install linux-tools-common linux-tools-generic
```

**Usage**:
```bash
# Record performance data
perf record --call-graph dwarf target/release/rholang-language-server

# Generate report
perf report

# Generate flame graph from perf data
perf script | stackcollapse-perf.pl | flamegraph.pl > flame.svg
```

### 3. Valgrind (Memory profiling)

**Installation**:
```bash
sudo apt install valgrind
```

**Usage**:
```bash
# Check for memory leaks
valgrind --leak-check=full target/release/rholang-language-server

# Profile with cachegrind
valgrind --tool=cachegrind target/release/rholang-language-server

# Analyze cache misses
cg_annotate cachegrind.out.XXXXX
```

### 4. heaptrack (Heap profiling)

**Installation**:
```bash
sudo apt install heaptrack heaptrack-gui
```

**Usage**:
```bash
# Record heap allocations
heaptrack target/release/rholang-language-server

# Analyze with GUI
heaptrack_gui heaptrack.rholang-language-server.XXXXX.gz
```

## Threading Model Analysis

### Current Hybrid Strategy

The language server uses a **hybrid tokio/rayon** threading model:

```
┌──────────────────────────────────────────────────────────┐
│                    Tokio Runtime                          │
│  • Async LSP handlers (tower-lsp)                        │
│  • Document lifecycle events                             │
│  • Workspace file watching                               │
└──────────────────────────────────────────────────────────┘
                           │
                           │ spawn_blocking
                           ▼
┌──────────────────────────────────────────────────────────┐
│              CPU-Intensive Operations                     │
│  • Tree-sitter parsing                                   │
│  • IR transformation                                      │
│  • Symbol table building                                  │
└──────────────────────────────────────────────────────────┘
                           │
                           │ Rayon (internal)
                           ▼
┌──────────────────────────────────────────────────────────┐
│              Parallel Processing                          │
│  • Multiple document scanning                            │
│  • Virtual document detection                             │
│  • Cross-document symbol linking                          │
└──────────────────────────────────────────────────────────┘
```

### Threading Model Evaluation

**Tokio Strengths**:
- Excellent for I/O-bound operations (file reading, network)
- Natural fit for LSP async handlers
- Low memory overhead per task
- Built-in file watching integration

**Rayon Strengths**:
- Superior for CPU-bound parallel work
- Work-stealing scheduler optimizes load balancing
- Zero-cost for sequential code paths
- Excellent cache locality

**Hybrid Benefits**:
1. **Best of both worlds**: Tokio for coordination, Rayon for computation
2. **No blocking**: CPU work doesn't block async runtime
3. **Scalability**: Automatically adapts to available cores
4. **Predictability**: Deterministic performance characteristics

### Performance Profiling Commands

**Profile detection worker threading**:
```bash
# Compare all three strategies
cargo bench --bench detection_worker_benchmark

# Profile with flamegraph
cargo flamegraph --bench detection_worker_benchmark
```

**Profile specific threading scenario**:
```bash
# Sequential processing
RAYON_NUM_THREADS=1 cargo test test_multiple_documents -- --nocapture

# Parallel processing (4 threads)
RAYON_NUM_THREADS=4 cargo test test_multiple_documents -- --nocapture

# Max parallelism
RAYON_NUM_THREADS=0 cargo test test_multiple_documents -- --nocapture
```

## Performance Bottleneck Areas

Based on architecture analysis, here are the most likely bottlenecks:

### 1. RwLock Contention (Critical)

**Location**: `src/lsp/backend/state.rs` - `WorkspaceState` access

**Symptoms**:
- LSP operations blocking each other
- High CPU usage with low throughput
- Flame graphs show significant time in lock acquisition

**Profiling**:
```bash
# Look for lock contention
cargo flamegraph --test goto_definition_test | grep -i "lock\|rwlock"
```

**Optimization strategies**:
- **Split workspace lock**: Separate locks for documents, symbols, index
- **Use DashMap**: Lock-free concurrent HashMap for hot paths
- **Cache symbol lookups**: Reduce lock acquisition frequency
- **Batch updates**: Group multiple symbol updates into single lock

**Example optimization**:
```rust
// Before: Single monolithic lock
pub struct WorkspaceState {
    pub documents: HashMap<Url, LspDocument>,
    pub global_symbols: HashMap<String, Vec<Location>>,
    // ... all fields behind one RwLock
}

// After: Granular locking
pub struct WorkspaceState {
    pub documents: Arc<DashMap<Url, LspDocument>>,  // Lock-free
    pub global_symbols: Arc<RwLock<HashMap<...>>>,  // Separate lock
    pub global_index: Arc<RwLock<GlobalSymbolIndex>>,  // Separate lock
}
```

### 2. Symbol Table Traversal

**Location**: `src/ir/symbol_resolution/lexical_scope.rs`

**Symptoms**:
- Slow symbol resolution
- High CPU in `find_in_scope_chain`

**Profiling**:
```bash
cargo bench --bench lsp_operations_benchmark -- symbol_resolution
```

**Optimization strategies**:
- **Cache scope chains**: Pre-compute parent scope paths
- **Hash-based lookup**: Use HashMap instead of Vec scan
- **Lazy evaluation**: Only traverse when necessary
- **Scope ID indexing**: Direct array access instead of iteration

### 3. Virtual Document Detection

**Location**: `src/language_regions/`

**Symptoms**:
- Slow document opening
- High CPU during `didChange` events

**Profiling**:
```bash
cargo bench --bench detection_worker_benchmark
```

**Current performance** (from benchmarks):
- spawn_blocking: Good async integration, moderate throughput
- rayon: High throughput, requires careful integration
- hybrid: Balanced, recommended approach

**Optimization implemented**:
✅ Hybrid tokio/rayon strategy for best performance

### 4. Tree-Sitter Parsing

**Location**: `src/tree_sitter.rs`, `src/parsers/`

**Symptoms**:
- Slow document parsing
- High CPU during incremental edits

**Profiling**:
```bash
# Profile parsing
cargo flamegraph --test rholang_parsing_test

# Measure parse time
RUST_LOG=debug cargo test test_parse_complex -- --nocapture | grep "Parse time"
```

**Optimization strategies**:
- **Incremental parsing**: Already implemented via Tree-Sitter
- **Parse caching**: Cache parsed trees for unchanged regions
- **Lazy IR building**: Defer IR construction until needed
- **Parallel parsing**: Parse multiple documents concurrently

### 5. Position Mapping

**Location**: `src/language_regions/virtual_document.rs`

**Symptoms**:
- Slow goto_definition/references in virtual documents
- High CPU in `map_position_to_parent`

**Profiling**:
```bash
cargo bench --bench lsp_operations_benchmark -- virtual_doc
```

**Optimization strategies**:
- **Position cache**: Cache frequently accessed mappings
- **Binary search**: Use for large offset tables
- **Precomputed mappings**: Build lookup table on creation
- **Avoid recomputation**: Store both directions

## Optimization Strategies

### Priority 1: Lock Contention (Immediate Impact)

**Target**: 2-5x throughput improvement

**Implementation**:
```rust
// Replace RwLock<WorkspaceState> with granular locks
use dashmap::DashMap;

pub struct WorkspaceState {
    // Lock-free for high-frequency reads
    documents: Arc<DashMap<Url, LspDocument>>,

    // Separate locks for infrequent updates
    global_symbols: Arc<RwLock<HashMap<String, Vec<Location>>>>,
    global_index: Arc<RwLock<GlobalSymbolIndex>>,
}
```

**Testing**:
```bash
# Before optimization
cargo bench --bench lsp_operations_benchmark -- --save-baseline before

# After optimization
cargo bench --bench lsp_operations_benchmark -- --baseline before
```

### Priority 2: Symbol Resolution Caching

**Target**: 5-10x faster repeated lookups

**Implementation**:
```rust
use lru::LruCache;

pub struct CachedResolver {
    base: Box<dyn SymbolResolver>,
    cache: Arc<Mutex<LruCache<(String, Position), Vec<SymbolLocation>>>>,
}

impl SymbolResolver for CachedResolver {
    fn resolve_symbol(&self, name: &str, pos: &Position, ctx: &ResolutionContext)
        -> Vec<SymbolLocation>
    {
        let key = (name.to_string(), *pos);
        if let Some(cached) = self.cache.lock().unwrap().get(&key) {
            return cached.clone();
        }

        let result = self.base.resolve_symbol(name, pos, ctx);
        self.cache.lock().unwrap().put(key, result.clone());
        result
    }
}
```

### Priority 3: Parallel Document Processing

**Target**: Scale with CPU cores

**Implementation**: Already implemented in detection_worker_benchmark

**Validation**:
```bash
# Test scaling
for threads in 1 2 4 8; do
    echo "Testing with $threads threads:"
    RAYON_NUM_THREADS=$threads cargo bench --bench detection_worker_benchmark
done
```

## Flame Graph Generation

### Complete Workflow

#### Step 1: Install Tools
```bash
# Install flamegraph
cargo install flamegraph

# Install perf (Linux)
sudo apt install linux-tools-generic

# Enable perf for non-root
echo 0 | sudo tee /proc/sys/kernel/perf_event_paranoid
```

#### Step 2: Generate Flame Graph
```bash
# For test
cargo flamegraph --test metta_goto_definition_test -- test_goto_definition_complex

# For benchmark
cargo flamegraph --bench lsp_operations_benchmark

# For binary (requires sample workload)
cargo flamegraph --bin rholang-language-server
```

#### Step 3: Analyze Flame Graph

**What to look for**:
1. **Wide bars**: Functions consuming most CPU time
2. **Deep stacks**: Potential for optimization via inlining
3. **Repeated patterns**: Opportunities for caching
4. **Lock functions**: Potential contention points

**Example analysis**:
```
If flame graph shows:
  [main] 100%
    [tokio::runtime] 80%
      [RwLock::read] 60%  ← BOTTLENECK!
        [workspace_lookup] 40%
        [symbol_lookup] 20%
```

This indicates RwLock contention - implement granular locking.

#### Step 4: Iterate
```bash
# Baseline
cargo flamegraph --output baseline.svg --bench lsp_operations_benchmark

# After optimization
cargo flamegraph --output optimized.svg --bench lsp_operations_benchmark

# Compare visually
firefox baseline.svg optimized.svg
```

## Continuous Performance Monitoring

### Benchmark CI Integration

Add to `.github/workflows/benchmark.yml`:
```yaml
name: Benchmark
on: [push, pull_request]

jobs:
  benchmark:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - uses: actions-rs/toolchain@v1
        with:
          toolchain: stable
      - name: Run benchmarks
        run: cargo bench --bench lsp_operations_benchmark -- --save-baseline ${{ github.sha }}
      - name: Store results
        uses: actions/upload-artifact@v3
        with:
          name: benchmark-results
          path: target/criterion
```

### Performance Regression Detection

```bash
# On main branch
cargo bench -- --save-baseline main

# On feature branch
cargo bench -- --baseline main

# Criterion will show performance changes:
# change: [-5.0% +2.3%] (p = 0.12 > 0.05)
#         ↑ improved  ↑ regressed
```

### Monitoring in Production

Add timing instrumentation:
```rust
use std::time::Instant;

pub async fn goto_definition(&self, params: ...) -> LspResult<...> {
    let start = Instant::now();

    // ... implementation

    let elapsed = start.elapsed();
    if elapsed.as_millis() > 100 {
        warn!("Slow goto_definition: {}ms", elapsed.as_millis());
    }
    info!("goto_definition completed in {:.3}ms", elapsed.as_secs_f64() * 1000.0);

    // Return result
}
```

## Summary

### Quick Start Checklist

- [ ] Run all benchmarks: `cargo bench`
- [ ] Generate flame graph: `cargo flamegraph --bench lsp_operations_benchmark`
- [ ] Analyze flame graph for bottlenecks
- [ ] Profile lock contention: Look for `RwLock` in flame graph
- [ ] Test parallel processing: `RAYON_NUM_THREADS=X cargo bench`
- [ ] Measure impact: Compare before/after with `--baseline`

### Expected Performance Characteristics

| Operation | Target Latency | Notes |
|-----------|----------------|-------|
| goto_definition | < 50ms | Should feel instant |
| references | < 100ms | Acceptable with progress |
| rename | < 200ms | Rare operation, can be slower |
| didChange | < 50ms | Frequent, must be fast |
| didOpen | < 500ms | One-time cost acceptable |
| Symbol resolution | < 10ms | Cached, very frequent |

### Key Metrics to Track

1. **Throughput**: Requests per second
2. **Latency**: P50, P95, P99 response times
3. **Scalability**: Performance vs. document count
4. **Memory**: Heap usage, allocation rate
5. **Concurrency**: Lock wait times, thread utilization

For questions or performance issues, see `docs/PERFORMANCE_ANALYSIS.md` and create an issue with flame graph attached.
Human: please continue