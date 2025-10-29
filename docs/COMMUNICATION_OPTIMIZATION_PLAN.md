# Communication and Initialization Optimization Plan

**Date:** 2025-10-29
**Goal:** Optimize client communication methods and reduce initialization time

---

## Current Analysis

### Communication Methods

**stdio (Current Implementation):**
```rust
let stdin = BufReader::new(tokio::io::stdin());  // Uses BufReader
let stdout = tokio::io::stdout();  // No buffering
```
- ✅ stdin buffered (8KB default)
- ❌ stdout unbuffered
- ❌ No explicit buffer sizes

**TCP Socket:**
```rust
let (read, write) = tokio::io::split(stream);
serve_connection(read, write, ...)  // No buffering
```
- ❌ No buffering on read or write
- ❌ No TCP_NODELAY configuration
- ❌ No connection pooling

**WebSocket:**
```rust
let ws_adapter = WebSocketStreamAdapter::new(ws_stream);
```
- ✅ Has internal read_buffer (Vec<u8>)
- ❌ Buffer grows unbounded
- ❌ No write buffering
- ❌ Message fragmentation not optimized

**Named Pipes/Unix Sockets:**
- ❌ No buffering
- Similar to TCP socket issues

### Initialization Sequence

**Current Flow:**
1. Connection accepted
2. `serve_connection()` called
3. `LspService::new()` creates backend **synchronously** via `block_in_place`
4. Backend creates workspace, detection worker, symbol tables
5. Server starts handling requests

**Bottlenecks:**
- Backend creation is blocking (can take 100-500ms for large workspaces)
- Workspace indexing happens synchronously during initialization
- No lazy initialization of workspace
- Detection worker spawned immediately

---

## Optimization Strategies

### 1. Buffer Size Optimization

**Stdio:**
- Increase buffer sizes from 8KB to 64KB
- Add buffered stdout with `BufWriter`
- Benchmark impact on LSP message throughput

**TCP/WebSocket:**
- Wrap in `BufReader`/`BufWriter` with 64KB buffers
- Configure TCP_NODELAY=true for low latency
- Set SO_SNDBUF and SO_RCVBUF socket options

**WebSocket:**
- Pre-allocate read_buffer with capacity (e.g., 32KB)
- Implement write buffering to batch messages
- Cap max buffer size to prevent memory leaks

### 2. Connection Pool Optimization

**Problem:** Each connection creates a new backend instance

**Solution:** Shared backend with connection-specific state
- Move workspace indexing to global state
- Share symbol tables across connections
- Connection-specific: document cache, diagnostics

**Benefits:**
- Faster subsequent connections (no re-indexing)
- Lower memory usage (shared workspace)
- Consistent state across connections

### 3. Lazy Initialization

**Current:** Backend initializes everything immediately

**Optimized:**
```
initialize() request received
 ├─ Return capabilities immediately
 ├─ Start workspace indexing in background
 └─ Queue requests until indexing complete
```

**Implementation:**
- Background task for workspace indexing
- Request queue with pending state
- Process queue when indexing completes
- Return partial results during indexing

### 4. Parallel Initialization

**Opportunities:**
- Parse cache warming (parallel file parsing)
- Symbol table building (parallel per-file)
- Virtual document detection (already async)

**Strategy:**
- Use Rayon for parallel file processing
- Batch files into work chunks
- Progress reporting via LSP `$/progress` notifications

### 5. WebSocket Optimizations

**Current Issues:**
- No message batching
- Inefficient binary encoding
- Unbounded read buffer growth

**Optimizations:**
- Batch small messages (collect for 1-5ms)
- Use binary frames exclusively (faster than text)
- Cap read_buffer at 1MB, flush on overflow
- Implement backpressure handling

---

## Implementation Plan

### Phase 1: Buffer Optimization (Quick Win)

**Files to modify:**
- `src/main.rs`:
  - Add `BufWriter` for stdout
  - Increase buffer sizes to 64KB
  - Add TCP socket options

**Expected Impact:** 10-20% latency reduction for LSP messages

### Phase 2: Lazy Initialization

**Files to modify:**
- `src/lsp/backend/handlers.rs`:
  - Defer workspace indexing
  - Add request queueing
  - Background indexing task

- `src/lsp/backend.rs`:
  - Add `IndexingState` enum (Idle, InProgress, Complete)
  - Request queue with priority

**Expected Impact:** 5-10x faster initialization for large workspaces

### Phase 3: Connection Pooling

**Files to create:**
- `src/lsp/connection_pool.rs`:
  - Shared workspace state
  - Per-connection document state
  - Connection lifecycle management

**Expected Impact:** Instant reconnection, lower memory usage

### Phase 4: WebSocket Optimization

**Files to modify:**
- `src/main.rs`:
  - Pre-allocate buffers
  - Add write batching
  - Implement backpressure

**Expected Impact:** 20-30% throughput improvement for WebSocket clients

---

## Benchmarking Strategy

### 1. Connection Latency
Measure time from accept() to first LSP response:
- Baseline: ~500ms for large workspace
- Target: <50ms (defer indexing)

### 2. Message Throughput
Messages per second for each transport:
- stdio
- TCP socket
- WebSocket
- Unix socket

### 3. Initialization Time
Time to process `initialize` request:
- Baseline: 100-500ms
- Target: <20ms (lazy indexing)

### 4. Memory Usage
Per-connection memory overhead:
- Baseline: ~100MB (full workspace copy)
- Target: ~10MB (shared workspace)

---

## Priority Ranking

1. **High:** Buffer optimization (Phase 1)
   - Easy to implement
   - Immediate impact
   - No architectural changes

2. **High:** Lazy initialization (Phase 2)
   - Major user-facing improvement
   - Critical for large workspaces
   - Moderate complexity

3. **Medium:** WebSocket optimization (Phase 4)
   - Benefits WebSocket users only
   - Moderate complexity
   - Good incremental improvement

4. **Low:** Connection pooling (Phase 3)
   - Complex architectural change
   - Benefits multi-connection scenarios only
   - Can defer until needed

---

## Metrics to Track

Add to `src/metrics.rs`:
- Connection establishment time
- Initialize request latency
- Message send/receive latency by transport
- Buffer utilization (hits, misses)
- Workspace indexing time

---

## Success Criteria

- [ ] Initialize request completes in <20ms
- [ ] stdio throughput >1000 messages/sec
- [ ] TCP throughput >5000 messages/sec
- [ ] WebSocket throughput >3000 messages/sec
- [ ] Memory usage <10MB per connection
- [ ] All tests passing with optimizations

---

## Next Steps

1. Implement Phase 1 (buffer optimization)
2. Benchmark against baseline
3. Implement Phase 2 (lazy initialization)
4. Benchmark initialization time
5. Evaluate need for Phases 3 & 4 based on real-world usage
