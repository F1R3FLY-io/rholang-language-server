# Document IR Cache Capacity Tuning Guide

**Phase**: B-2
**Date**: 2025-11-13
**Purpose**: Guide for tuning cache capacity based on workspace size and available memory

## Overview

The document IR cache uses an LRU (Least Recently Used) eviction policy to automatically manage memory usage. The cache capacity determines how many parsed documents can be held in memory before older entries are evicted.

## Default Configuration

- **Default Capacity**: 50 documents
- **Estimated Memory per Document**: ~1-2 MB
- **Total Memory (default)**: ~50-100 MB

## Capacity Tuning Formula

### Basic Formula

```
Optimal Capacity = min(
    Workspace File Count × Hit Rate Factor,
    Available Memory ÷ Avg Document Size
)
```

Where:
- **Workspace File Count**: Total number of `.rho` files in the project
- **Hit Rate Factor**: 0.8-1.0 (percentage of files accessed repeatedly)
- **Available Memory**: Memory budget for LSP server (typically 200-500 MB)
- **Avg Document Size**: ~1-2 MB per cached document

### Example Calculations

#### Small Project (<50 files)
```
Workspace Files: 30
Hit Rate Factor: 0.9 (90% of files accessed repeatedly)
Available Memory: 200 MB
Avg Document Size: 1.5 MB

Optimal Capacity = min(30 × 0.9, 200 ÷ 1.5)
                 = min(27, 133)
                 = 27
```

**Recommendation**: 30-50 capacity (round up for headroom)

#### Medium Project (50-200 files)
```
Workspace Files: 150
Hit Rate Factor: 0.8 (80% hit rate)
Available Memory: 300 MB
Avg Document Size: 1.5 MB

Optimal Capacity = min(150 × 0.8, 300 ÷ 1.5)
                 = min(120, 200)
                 = 120
```

**Recommendation**: 100-150 capacity

#### Large Project (>200 files)
```
Workspace Files: 500
Hit Rate Factor: 0.7 (70% hit rate, more diverse access pattern)
Available Memory: 500 MB
Avg Document Size: 1.5 MB

Optimal Capacity = min(500 × 0.7, 500 ÷ 1.5)
                 = min(350, 333)
                 = 333
```

**Recommendation**: 200-300 capacity (constrained by memory)

## Recommended Capacities by Workspace Size

| Workspace Size | File Count | Recommended Capacity | Memory Usage | Expected Hit Rate |
|----------------|------------|---------------------|--------------|-------------------|
| **Tiny** | 1-20 | 20 | ~20-40 MB | >95% |
| **Small** | 20-50 | 50 (default) | ~50-100 MB | >90% |
| **Medium** | 50-150 | 100 | ~100-200 MB | >85% |
| **Large** | 150-300 | 200 | ~200-400 MB | >80% |
| **Very Large** | 300-500 | 300 | ~300-600 MB | >75% |
| **Enterprise** | 500+ | 500 | ~500-1000 MB | >70% |

## Configuration Methods

### Option 1: Code Configuration (Custom LSP Server)

If you're building a custom LSP server binary:

```rust
use rholang_language_server::lsp::models::WorkspaceState;

// Create workspace with custom cache capacity
let mut workspace = WorkspaceState::new().await?;

// Replace default cache with custom capacity
workspace.document_cache = Arc::new(
    DocumentCache::with_capacity(200)  // 200 documents
);
```

### Option 2: Environment Variable (Future Enhancement)

**Status**: Not yet implemented (planned for Phase B-3)

```bash
# Proposed environment variable
export RHOLANG_LSP_CACHE_CAPACITY=200
rholang-language-server
```

### Option 3: LSP Server Configuration (Future Enhancement)

**Status**: Not yet implemented (planned for Phase B-3)

```json
{
  "rholang": {
    "cache": {
      "capacity": 200,
      "maxMemoryMB": 400
    }
  }
}
```

## Memory Estimation

### Per-Document Memory Breakdown

| Component | Typical Size | Notes |
|-----------|-------------|-------|
| **Parsed IR (AST)** | 500 KB - 1 MB | Proportional to file size + complexity |
| **Symbol Table** | 50-200 KB | Proportional to number of contracts/variables |
| **Position Index** | 20-50 KB | BTreeMap of positions → nodes |
| **Tree-Sitter Tree** | 100-300 KB | CST representation |
| **Completion State** | 50-100 KB | Optional, lazy-initialized |
| **Metadata** | 10-20 KB | Cache entry overhead |
| **Total** | **~1-2 MB** | Varies by file complexity |

### Workspace Memory Estimation

```
Total Cache Memory = Capacity × Avg Document Size

Examples:
- 50 capacity  × 1.5 MB = 75 MB
- 100 capacity × 1.5 MB = 150 MB
- 200 capacity × 1.5 MB = 300 MB
- 500 capacity × 1.5 MB = 750 MB
```

## Performance vs Memory Trade-offs

### Undercapacity (Capacity < Optimal)

**Symptoms**:
- Lower cache hit rate (<70%)
- Frequent evictions
- Increased CPU usage (re-parsing)
- Slower LSP responses

**Example**: 30 capacity for 100-file project
- Hit rate: ~60% (instead of ~85%)
- Performance: 2-3x slower than optimal

**Fix**: Increase capacity or add more RAM

### Overcapacity (Capacity > Optimal)

**Symptoms**:
- Wasted memory
- Possible memory pressure on system
- No performance benefit (diminishing returns)

**Example**: 500 capacity for 50-file project
- Hit rate: ~95% (only marginally better than 90%)
- Memory: 750 MB (vs 75 MB optimal) = 10x waste

**Fix**: Reduce capacity to save memory

### Optimal Capacity

**Characteristics**:
- Cache hit rate: 80-90%
- Evictions: Rare (<5% of accesses)
- Memory usage: Stable, predictable
- CPU usage: Minimal re-parsing overhead

## Monitoring Cache Performance

### LSP Server Statistics (Phase B-2.5 - Future)

**Proposed LSP Custom Method**: `rholang/cacheStats`

```typescript
interface CacheStats {
  totalQueries: number;      // Total cache lookups
  hits: number;              // Cache hits
  misses: number;            // Cache misses
  evictions: number;         // LRU evictions
  hitRate: number;           // hits / totalQueries (0.0-1.0)
  currentSize: number;       // Current entries in cache
  maxCapacity: number;       // Maximum capacity
}
```

**Usage** (VSCode extension example):
```typescript
const stats = await client.sendRequest('rholang/cacheStats');
console.log(`Cache hit rate: ${(stats.hitRate * 100).toFixed(1)}%`);
```

### Manual Monitoring (Current)

Enable debug logging to monitor cache behavior:

```bash
RUST_LOG=rholang_language_server::lsp::backend::indexing=debug \
    rholang-language-server
```

Look for log messages:
- `Cache HIT for file:///...` - Cache hit
- `Cache MISS for file:///...` - Cache miss (re-parsing)
- Cache statistics in debug output

## Adaptive Capacity (Future Phase C)

**Status**: Planned for Phase C (Dependency-Aware Optimization)

### Proposed: Dynamic Capacity Adjustment

```rust
struct AdaptiveCache {
    min_capacity: usize,
    max_capacity: usize,
    target_hit_rate: f64,

    // Automatically adjust capacity based on hit rate
    fn adjust_capacity(&mut self) {
        if self.hit_rate() < self.target_hit_rate {
            self.increase_capacity();
        } else if self.memory_pressure() {
            self.decrease_capacity();
        }
    }
}
```

### Proposed: Memory-Aware Eviction

```rust
// Evict based on both LRU and memory pressure
struct SmartEviction {
    // Evict larger documents first when memory constrained
    fn evict(&mut self) -> Option<CachedDocument> {
        if self.memory_usage > threshold {
            self.evict_largest_document()
        } else {
            self.evict_lru()
        }
    }
}
```

## Troubleshooting

### Problem: High cache miss rate (< 70%)

**Diagnosis**:
1. Check workspace size: `find . -name "*.rho" | wc -l`
2. Compare to cache capacity
3. Check for rapid file changes (hot reloading)

**Solutions**:
- Increase cache capacity to 80-100% of workspace size
- Add more RAM
- Enable persistent cache (Phase B-3)

### Problem: High memory usage

**Diagnosis**:
1. Check cache capacity vs workspace size
2. Monitor memory per document (varies by file size)
3. Check for memory leaks (use `valgrind` or `heaptrack`)

**Solutions**:
- Reduce cache capacity
- Clear cache periodically: `cache.clear()`
- Enable compression (Phase C)

### Problem: Frequent evictions

**Diagnosis**:
1. Check cache statistics: `evictions / totalQueries` ratio
2. Compare current size to capacity
3. Check access patterns (are files accessed in LRU order?)

**Solutions**:
- Increase capacity (if evictions > 5% of queries)
- Use working-set-based eviction (Phase C)
- Pin frequently accessed files (Phase C)

## Best Practices

1. **Start with defaults**: Use 50 capacity for most projects
2. **Monitor hit rate**: Aim for >80% hit rate
3. **Scale with workspace**: Increase capacity as project grows
4. **Budget memory**: Allocate 100-500 MB for cache
5. **Consider file churn**: Projects with frequent changes need larger cache
6. **Tune iteratively**: Adjust based on actual usage patterns

## Future Enhancements

### Phase B-3: Persistent Cache
- Serialize cache to disk on shutdown
- Load cache on startup (warm start)
- Reduces initial indexing time

### Phase C: Dependency-Aware Cache
- Invalidate dependent files when dependencies change
- Smart eviction based on dependency graph
- Prefetch related files

### Phase D: Compressed Cache
- Compress cached IR using zstd or lz4
- Trade CPU for memory (2-3x size reduction)
- Configurable compression level

## References

- [Phase B-2 Implementation](./ledger/phase-b-2-implementation-complete.md)
- [Phase B-2 Baseline Measurements](./ledger/phase-b-2-baseline-measurements.md)
- [LSP Introspection Guide](./lsp-introspection-guide.md) (to be created)

---

**Last Updated**: 2025-11-13
**Maintainer**: F1R3FLY.io
**Status**: Active (Phase B-2)
