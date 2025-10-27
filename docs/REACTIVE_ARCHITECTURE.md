# ReactiveX Architecture in Rholang Language Server

## Overview

The Rholang Language Server implements a ReactiveX-inspired event processing architecture for handling LSP events, file system changes, and background tasks. This document describes the design, implementation, and usage of the reactive system.

## Architecture Principles

### Core Concepts

1. **Declarative Event Streams**: Events are modeled as asynchronous streams that can be composed using operators
2. **Automatic Resource Management**: Streams automatically clean up resources using `take_until` and drop semantics
3. **Composable Operators**: Stream transformations are built from small, reusable operators
4. **Hot Observables**: Shared state changes are broadcast to multiple subscribers via `tokio::sync::watch`
5. **Timeout Protection**: All long-running operations are wrapped with timeout operators to prevent hangs

## Module Structure

```
src/lsp/backend/
├── state.rs          - Backend state types and event structures
├── streams.rs        - Stream operators and utilities
├── reactive.rs       - Reactive event handler implementations
└── symbols.rs        - Symbol-related operations (extracted)
```

## Stream Operators

### Available Operators (src/lsp/backend/streams.rs)

#### `switch_map<F, Fut, T>(self, f: F)`
Switches to a new inner stream each time the outer stream emits, automatically canceling the previous inner stream.

**Use Case**: Document validation where new edits should cancel previous validation.

```rust
doc_stream
    .switch_map(|doc| {
        // New validation cancels previous
        backend.validate(doc)
    })
```

#### `timeout(self, duration: Duration)`
Wraps each stream item in a timeout, emitting an error if the operation takes too long.

**Use Case**: Protecting against stuck file operations or hung RNode connections.

```rust
file_stream
    .timeout(Duration::from_secs(5))
    .filter_map(|result| async move {
        match result {
            Ok(value) => Some(value),
            Err(_timeout) => {
                error!("Operation timed out");
                None
            }
        }
    })
```

#### `retry<F, Fut, T, E>(self, max_attempts, should_retry)`
Retries failed operations up to a maximum number of attempts.

**Use Case**: Transient file system errors or network failures.

```rust
network_stream
    .retry(3, |err| {
        // Retry on transient errors
        matches!(err, Error::Timeout | Error::ConnectionReset)
    })
```

#### `take_until<Fut>(self, trigger: Fut)`
Takes items from the stream until a trigger future completes, then stops.

**Use Case**: Graceful shutdown - stop processing when shutdown signal received.

```rust
event_stream
    .take_until(async move {
        shutdown_rx.recv().await;
    })
```

#### `chunk_timeout(self, max_size, timeout)`
Batches stream items into chunks, emitting when either max size or timeout is reached.

**Use Case**: File system event batching to reduce redundant processing.

```rust
file_events
    .chunk_timeout(10, Duration::from_millis(100))
    // Emit batch of up to 10 events, or after 100ms
```

## Reactive Event Handlers

### 1. File Watcher (spawn_reactive_file_watcher)

**Purpose**: Watch file system for `.rho` file changes and trigger re-indexing.

**Pipeline**:
```
File System Events
    ↓ file_system_stream_from_arc()
    ↓ chunk_timeout(10, 100ms)           // Batch events
    ↓ map(flatten)                       // Flatten batches
    ↓ filter(!empty)                     // Skip empty batches
    ↓ take_until(shutdown)               // Stop on shutdown
    ↓ Process concurrently with 5s timeout
```

**Key Features**:
- Batches file events to reduce redundant processing
- 5-second timeout per file to prevent stuck operations
- Concurrent processing of file batches
- Graceful shutdown via `take_until`

**Code Location**: `src/lsp/backend/reactive.rs:26-89`

### 2. Document Debouncer (spawn_reactive_document_debouncer)

**Purpose**: Debounce document change events and trigger validation after user stops typing.

**Pipeline**:
```
Document Change Events
    ↓ ReceiverStream
    ↓ take_until(shutdown)               // Stop on shutdown
    ↓ Manual per-URI debouncing (100ms)  // Wait for typing to stop
    ↓ Validation with 10s timeout        // Validate with timeout
    ↓ Automatic cancellation             // Cancel previous validation
```

**Key Features**:
- Per-URI independent debouncing (100ms)
- Automatic cancellation of previous validations
- 10-second timeout for validation operations
- Manual implementation (awaiting `group_by` operator)

**Debounce Strategy**:
1. Events stored in HashMap by URI with timestamp
2. 50ms polling interval checks which URIs are ready
3. URIs idle for 100ms+ trigger validation
4. Previous validation cancelled via oneshot channel

**Code Location**: `src/lsp/backend/reactive.rs:92-193`

### 3. Progressive Indexer (spawn_reactive_progressive_indexer)

**Purpose**: Index workspace files progressively with priority-based scheduling.

**Pipeline**:
```
Indexing Tasks
    ↓ ReceiverStream
    ↓ chunk_timeout(10, 200ms)           // Batch tasks
    ↓ take_until(shutdown)               // Stop on shutdown
    ↓ Priority sorting                   // High-priority first
    ↓ Process sequentially               // Index files
    ↓ Link symbols after batch           // Update cross-file refs
```

**Key Features**:
- Priority-based task scheduling (0 = high, 1 = normal)
- Batched processing for efficiency
- Symbol linking after each batch
- BinaryHeap for priority queue

**Code Location**: `src/lsp/backend/reactive.rs:195-263`

### 4. Unified Event Pipeline (spawn_unified_event_pipeline)

**Purpose**: Merge all event streams into single coordinated pipeline (not yet activated).

**Design**:
```
File Events ─┐
             ├→ select_all() → Unified Pipeline → Event Router
Document ────┤
Events      │
             │
Indexing ────┘
Tasks
```

**Status**: Implemented but marked `#[allow(dead_code)]`. Available for future architectural consolidation.

**Code Location**: `src/lsp/backend/reactive.rs:265-339`

## Hot Observables

### Workspace Change Events

**Purpose**: Broadcast workspace state changes to multiple subscribers.

**Implementation**: `tokio::sync::watch` channel
```rust
pub struct WorkspaceChangeEvent {
    file_count: usize,
    symbol_count: usize,
    change_type: WorkspaceChangeType,
}

// Broadcast to all subscribers
self.workspace_changes.send(WorkspaceChangeEvent { ... });

// Subscribe to changes
let mut rx = backend.workspace_changes.subscribe();
while let Ok(event) = rx.recv().await {
    // Handle workspace change
}
```

**Subscribers**: Can be used for:
- Status bar updates
- Progress indicators
- Telemetry collection
- Reactive UI updates

**Code Location**: `src/lsp/backend/state.rs:39-61`

## Error Handling

### Timeout Strategy

**File Operations**: 5 seconds
- File I/O is fast; timeout indicates system issue
- Prevents blocking on network-mounted filesystems
- Logs error and continues with other files

**Validation**: 10 seconds
- May involve RNode gRPC communication
- Accounts for network latency and RNode processing
- Longer timeout for potentially expensive operations

**Implementation**:
```rust
match tokio::time::timeout(duration, operation).await {
    Ok(Ok(result)) => // Operation succeeded
    Ok(Err(e)) => // Operation failed
    Err(_) => // Timeout
}
```

### Cancellation Strategy

**Document Validation**:
- Each URI has a cancellation token (oneshot channel)
- New validation for same URI cancels previous
- Prevents redundant validation of stale content

**Implementation**:
```rust
// Store cancellation token
let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
backend.validation_cancel.insert(uri, cancel_tx);

// Spawn validation with cancellation
tokio::select! {
    result = validate() => { /* handle */ }
    _ = cancel_rx => { /* cancelled */ }
}
```

## Performance Characteristics

### Batching

**File Events**: Max 10 events per 100ms
- Reduces redundant processing of rapid file changes
- Example: Git operations touching many files processed as single batch

**Indexing Tasks**: Max 10 tasks per 200ms
- Balances responsiveness with batch efficiency
- Symbol linking after batch amortizes overhead

### Debouncing

**Document Changes**: 100ms per URI
- Reduced from 300ms for better responsiveness
- Per-URI ensures independent editing doesn't interfere
- Strike balance between responsiveness and CPU usage

### Concurrency

**File Processing**: Fully concurrent within batch
- Each file in batch processed in separate task
- All tasks awaited before processing next batch

**Validation**: One per URI, concurrent across URIs
- Prevents redundant validation of same file
- Different files validated concurrently

## Testing

### Test Coverage

**Reactive Optimization Tests** (`tests/reactive_optimizations.rs`):
- `test_document_isolation`: Validates per-URI independence
- `test_deeply_nested_scopes`: Complex AST handling
- `test_complex_program_validation`: Real-world code patterns
- `test_multiple_documents_validated`: Concurrent validation
- `test_rapid_document_operations`: Debouncing under load
- `test_system_responsiveness_under_load`: Stress testing

**Performance Tests** (`tests/performance_tests.rs`):
- Timeout thresholds for operations
- Complexity checks (O(n) vs O(n²))
- Large file handling

## Design Patterns

### Stream Composition

```rust
// Compose operators declaratively
let stream = source_stream
    .chunk_timeout(max_size, timeout)
    .map(transform)
    .filter(predicate)
    .timeout(duration)
    .take_until(shutdown_signal);
```

### Resource Cleanup

```rust
// Automatic cleanup via take_until
reactive_stream
    .take_until(async move {
        shutdown_rx.recv().await;
        info!("Shutting down gracefully");
    })
```

### Error Recovery

```rust
// Continue on errors, log and skip
while let Some(item) = stream.next().await {
    match process(item).await {
        Ok(_) => trace!("Success"),
        Err(e) => error!("Failed: {}, continuing", e),
    }
}
```

## Future Enhancements

### Potential Improvements

1. **Group-By Operator**: Enable cleaner per-URI debouncing
2. **Retry with Backoff**: Exponential backoff for transient failures
3. **Throttle Operator**: Rate limiting for expensive operations
4. **Activate Unified Pipeline**: Consolidate event streams
5. **Metrics Collection**: Track stream throughput and latency

### Extension Points

- Custom operators via `StreamExt` trait
- Pluggable timeout strategies
- Configurable batch sizes and timeouts
- Observable metrics via hot observables

## References

- ReactiveX: http://reactivex.io/
- tokio-stream: https://docs.rs/tokio-stream/
- Futures Combinators: https://docs.rs/futures/

## Commit History

```
5707453 feat: Add timeout protection to reactive file and validation processing
5504b80 refactor: Extract symbol operations to dedicated module
b479ad4 fix: Fix file events receiver Arc ownership and performance thresholds
ec3c954 feat: Add retry and timeout stream operators
4d0f0d9 feat: Add unified event stream pipeline
26f01de feat: Add hot observables for workspace state changes
```
