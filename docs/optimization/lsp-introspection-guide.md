# LSP Server Introspection Guide

**Phase**: B-2.5 (Future Enhancement)
**Date**: 2025-11-13
**Status**: **Planned** - Not yet implemented
**Purpose**: Enable runtime monitoring of LSP server performance metrics

## Overview

LSP introspection provides visibility into the language server's internal state and performance characteristics through custom LSP methods. This enables:

- Real-time cache hit rate monitoring
- Performance debugging
- Capacity planning
- Anomaly detection

## Motivation

**Problem**: Operators and developers have no visibility into cache performance

Currently, the only way to monitor cache behavior is through debug logging:
```bash
RUST_LOG=rholang_language_server::lsp::backend::indexing=debug \
    rholang-language-server 2>&1 | grep "Cache"
```

**Limitations**:
- Requires server restart with logging enabled
- Performance overhead from debug logging
- No programmatic access to statistics
- Can't monitor in production environments

**Solution**: LSP custom methods for real-time statistics

## Proposed Custom Methods

### 1. `rholang/cacheStats` - Document Cache Statistics

**Purpose**: Monitor document IR cache performance

**Request**: (no parameters)

```typescript
interface CacheStatsRequest {}
```

**Response**:

```typescript
interface CacheStats {
  // Query statistics
  totalQueries: number;      // Total cache lookups since server start
  hits: number;              // Cache hits
  misses: number;            // Cache misses
  hitRate: number;           // hits / totalQueries (0.0-1.0)

  // Capacity statistics
  currentSize: number;       // Current number of cached documents
  maxCapacity: number;       // Maximum cache capacity
  utilizationRate: number;   // currentSize / maxCapacity (0.0-1.0)

  // Eviction statistics
  evictions: number;         // Total LRU evictions
  evictionRate: number;      // evictions / totalQueries (0.0-1.0)

  // Memory estimation (approximate)
  estimatedMemoryMB: number; // currentSize × avgDocSizeMB
}
```

**Example Request** (LSP JSON-RPC):

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "rholang/cacheStats",
  "params": {}
}
```

**Example Response**:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "totalQueries": 1000,
    "hits": 850,
    "misses": 150,
    "hitRate": 0.85,
    "currentSize": 42,
    "maxCapacity": 50,
    "utilizationRate": 0.84,
    "evictions": 5,
    "evictionRate": 0.005,
    "estimatedMemoryMB": 63
  }
}
```

### 2. `rholang/performanceMetrics` - Overall Performance Metrics

**Purpose**: Monitor LSP operation latencies and throughput

**Request**:

```typescript
interface PerformanceMetricsRequest {
  reset?: boolean;  // Reset counters after reading
}
```

**Response**:

```typescript
interface PerformanceMetrics {
  // Indexing metrics
  indexing: {
    totalFiles: number;           // Total files indexed
    avgParseTimeMs: number;       // Average parse time per file
    avgSymbolBuildTimeMs: number; // Average symbol table build time
    cacheHitRate: number;         // Document cache hit rate
  };

  // LSP operation metrics
  operations: {
    gotoDefinition: OperationStats;
    completion: OperationStats;
    hover: OperationStats;
    references: OperationStats;
    rename: OperationStats;
  };

  // System metrics
  system: {
    uptimeSeconds: number;        // Server uptime
    workspaceFileCount: number;   // Total .rho files in workspace
    openDocuments: number;        // Currently open documents
  };
}

interface OperationStats {
  count: number;       // Total operations
  avgLatencyMs: number;    // Average latency
  p50LatencyMs: number;    // 50th percentile
  p95LatencyMs: number;    // 95th percentile
  p99LatencyMs: number;    // 99th percentile
  errors: number;      // Failed operations
}
```

### 3. `rholang/resetCache` - Clear Document Cache

**Purpose**: Manually clear cache for debugging or memory management

**Request**:

```typescript
interface ResetCacheRequest {}
```

**Response**:

```typescript
interface ResetCacheResponse {
  previousSize: number;  // Number of documents cleared
  success: boolean;
}
```

**Example Usage**:

```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "rholang/resetCache",
  "params": {}
}
```

### 4. `rholang/cacheContents` - Inspect Cache Contents

**Purpose**: Debug cache behavior by inspecting cached documents

**Request**:

```typescript
interface CacheContentsRequest {
  limit?: number;  // Max entries to return (default: 10)
}
```

**Response**:

```typescript
interface CacheContentsResponse {
  entries: CacheEntryInfo[];
  totalEntries: number;
}

interface CacheEntryInfo {
  uri: string;           // Document URI
  sizeKB: number;        // Approximate size in KB
  lastAccessedMs: number;    // Milliseconds since last access
  cachedAtMs: number;    // Milliseconds since cached
  version: number;       // Document version
}
```

## Implementation Guide

### Phase 1: Add Custom Method Handlers

**Location**: `src/lsp/backend/handlers.rs`

```rust
impl LanguageServer for RholangBackend {
    // ... existing methods ...

    async fn cache_stats(&self) -> LspResult<CacheStats> {
        let stats = self.workspace.document_cache.stats();

        Ok(CacheStats {
            total_queries: stats.total_queries,
            hits: stats.hits,
            misses: stats.misses,
            hit_rate: stats.hit_rate(),
            current_size: stats.current_size,
            max_capacity: stats.max_capacity,
            utilization_rate: stats.current_size as f64 / stats.max_capacity as f64,
            evictions: stats.evictions,
            eviction_rate: stats.evictions as f64 / stats.total_queries.max(1) as f64,
            estimated_memory_mb: stats.current_size as f64 * 1.5, // Estimate: 1.5 MB per doc
        })
    }
}
```

### Phase 2: Register Custom Methods

**Location**: `src/lsp/backend.rs`

```rust
use tower_lsp::jsonrpc::Result as LspResult;
use tower_lsp::lsp_types::request::{Request};

// Define custom request types
#[derive(Debug)]
struct CacheStatsRequest;

impl Request for CacheStatsRequest {
    type Params = ();
    type Result = CacheStats;
    const METHOD: &'static str = "rholang/cacheStats";
}

// Implement handler
async fn handle_cache_stats(&self, _params: ()) -> LspResult<CacheStats> {
    self.cache_stats().await
}
```

### Phase 3: VSCode Extension Integration

**Location**: VSCode extension (`rholang-vscode-client`)

```typescript
// src/cacheMonitor.ts
import * as vscode from 'vscode';
import { LanguageClient } from 'vscode-languageclient/node';

export class CacheMonitor {
    private statusBarItem: vscode.StatusBarItem;
    private client: LanguageClient;

    constructor(client: LanguageClient) {
        this.client = client;
        this.statusBarItem = vscode.window.createStatusBarItem(
            vscode.StatusBarAlignment.Right,
            100
        );
        this.statusBarItem.command = 'rholang.showCacheStats';
    }

    async updateCacheStats() {
        const stats = await this.client.sendRequest('rholang/cacheStats', {});

        // Update status bar
        const hitRate = (stats.hitRate * 100).toFixed(1);
        this.statusBarItem.text = `$(database) Cache: ${hitRate}%`;
        this.statusBarItem.tooltip = `Document Cache
        Hit Rate: ${hitRate}%
        Size: ${stats.currentSize}/${stats.maxCapacity}
        Evictions: ${stats.evictions}`;
        this.statusBarItem.show();
    }

    showDetailedStats() {
        const stats = await this.client.sendRequest('rholang/cacheStats', {});

        const panel = vscode.window.createWebviewPanel(
            'rholangCacheStats',
            'Rholang LSP Cache Statistics',
            vscode.ViewColumn.One,
            {}
        );

        panel.webview.html = this.generateStatsHTML(stats);
    }

    private generateStatsHTML(stats: any): string {
        return `<!DOCTYPE html>
        <html>
        <head>
            <style>
                body { font-family: var(--vscode-font-family); }
                .stat { margin: 10px 0; }
                .value { font-weight: bold; }
            </style>
        </head>
        <body>
            <h2>Document Cache Statistics</h2>
            <div class="stat">
                Hit Rate: <span class="value">${(stats.hitRate * 100).toFixed(1)}%</span>
            </div>
            <div class="stat">
                Cache Size: <span class="value">${stats.currentSize} / ${stats.maxCapacity}</span>
            </div>
            <div class="stat">
                Total Queries: <span class="value">${stats.totalQueries}</span>
            </div>
            <div class="stat">
                Hits: <span class="value">${stats.hits}</span>
            </div>
            <div class="stat">
                Misses: <span class="value">${stats.misses}</span>
            </div>
            <div class="stat">
                Evictions: <span class="value">${stats.evictions}</span>
            </div>
            <div class="stat">
                Estimated Memory: <span class="value">${stats.estimatedMemoryMB.toFixed(1)} MB</span>
            </div>
        </body>
        </html>`;
    }
}

// Register commands
export function activate(context: vscode.ExtensionContext, client: LanguageClient) {
    const monitor = new CacheMonitor(client);

    // Update every 5 seconds
    setInterval(() => monitor.updateCacheStats(), 5000);

    // Register show stats command
    context.subscriptions.push(
        vscode.commands.registerCommand('rholang.showCacheStats', () => {
            monitor.showDetailedStats();
        })
    );
}
```

## Use Cases

### 1. Capacity Planning

**Goal**: Determine optimal cache capacity for a workspace

**Process**:
1. Start with default capacity (50)
2. Monitor hit rate using `rholang/cacheStats`
3. If hit rate < 80%, increase capacity
4. If eviction rate > 5%, increase capacity
5. Repeat until hit rate stabilizes at 80-90%

**VSCode Extension**:
```typescript
async function autoTuneCapacity(client: LanguageClient) {
    const stats = await client.sendRequest('rholang/cacheStats', {});

    if (stats.hitRate < 0.8) {
        const newCapacity = Math.ceil(stats.maxCapacity * 1.5);
        vscode.window.showInformationMessage(
            `Cache hit rate is ${(stats.hitRate * 100).toFixed(1)}%. Consider increasing capacity to ${newCapacity}.`
        );
    }
}
```

### 2. Performance Debugging

**Goal**: Identify performance bottlenecks in LSP operations

**Process**:
1. Enable introspection
2. Perform operations (goto-definition, completion, etc.)
3. Query `rholang/performanceMetrics`
4. Identify slow operations (p99 > 200ms)
5. Investigate root cause

**Example**:
```bash
# Check if goto-definition is slow
curl -X POST http://localhost:9000 \
    -H "Content-Type: application/json" \
    -d '{
        "jsonrpc": "2.0",
        "id": 1,
        "method": "rholang/performanceMetrics"
    }'

# Result shows p99 = 500ms for goto-definition
# → Investigate pattern matching performance
```

### 3. Production Monitoring

**Goal**: Monitor LSP health in production deployments

**Process**:
1. Collect metrics periodically (every 60s)
2. Send to monitoring system (Prometheus, Datadog, etc.)
3. Alert on anomalies (hit rate drop, high latency)

**Example (Prometheus exporter)**:
```rust
// Expose metrics via HTTP endpoint
async fn metrics_handler() -> String {
    let stats = cache.stats();

    format!(
        r#"# HELP rholang_cache_hit_rate Document cache hit rate
# TYPE rholang_cache_hit_rate gauge
rholang_cache_hit_rate {{}}
# HELP rholang_cache_size Current cache size
# TYPE rholang_cache_size gauge
rholang_cache_size {{}}
"#,
        stats.hit_rate(),
        stats.current_size
    )
}
```

## Security Considerations

### 1. Authorization

**Risk**: Unauthorized access to LSP internals

**Mitigation**:
- Introspection methods are LSP custom methods (no external HTTP)
- Only accessible to LSP client (VSCode extension)
- No sensitive data exposed (only statistics)

### 2. Resource Exhaustion

**Risk**: Frequent introspection calls could degrade performance

**Mitigation**:
- Cache statistics are cheap to compute (O(1))
- Rate limit introspection calls in VSCode extension
- Provide sampling-based metrics (not every operation)

### 3. Information Disclosure

**Risk**: Cache contents reveal file paths and structure

**Mitigation**:
- `rholang/cacheContents` only returns URIs (no file content)
- Limit number of entries returned (default: 10)
- Only enabled in development builds (feature flag)

## Implementation Timeline

### Phase B-2.5: Basic Introspection (Next)
- [ ] Implement `rholang/cacheStats`
- [ ] VSCode status bar integration
- [ ] Basic monitoring dashboard

### Phase C: Advanced Metrics (Future)
- [ ] Implement `rholang/performanceMetrics`
- [ ] Histogram-based latency tracking
- [ ] Prometheus exporter

### Phase D: Production Monitoring (Future)
- [ ] Distributed tracing integration (OpenTelemetry)
- [ ] Automated anomaly detection
- [ ] Alert rules for common issues

## References

- [LSP Specification](https://microsoft.github.io/language-server-protocol/)
- [LSP Custom Methods](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#customMethods)
- [Cache Capacity Tuning Guide](./cache-capacity-tuning-guide.md)
- [Phase B-2 Implementation](./ledger/phase-b-2-implementation-complete.md)

---

**Last Updated**: 2025-11-13
**Maintainer**: F1R3FLY.io
**Status**: Planned (Phase B-2.5)
**Priority**: Medium (nice-to-have for monitoring)
