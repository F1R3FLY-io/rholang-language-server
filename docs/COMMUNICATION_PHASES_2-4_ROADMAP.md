# Communication Optimization Phases 2-4: Implementation Roadmap

**Status:** Phase 1 Complete ✅
**Remaining:** Phases 2-4 (Deferred for future implementation)

---

## Phase 1 Summary (Completed)

✅ **Buffer Optimization** - Commit `1359537`

**Implemented:**
- 64KB buffered stdin/stdout
- 64KB buffered TCP sockets with TCP_NODELAY
- Pre-allocated 32KB WebSocket buffers with 1MB cap
- Buffer shrinking to prevent memory bloat

**Expected Impact:** 10-25% latency/throughput improvement

---

## Phase 2: Lazy Initialization (Deferred)

### Goal
Reduce initialize request latency from 100-500ms to <20ms by deferring workspace indexing.

### Current Bottleneck

```rust
// src/lsp/backend.rs:77
pub async fn new(...) -> anyhow::Result<Self> {
    // ... diagnostic provider setup ...

    // BLOCKING: Workspace indexing happens here
    let backend = RholangBackend {
        workspace: Arc::new(WorkspaceState {
            documents: DashMap::new(),
            ...
        }),
        ...
    };

    // If rootUri provided, index entire workspace synchronously
    // This can take 100-500ms for large workspaces

    Ok(backend)
}
```

### Proposed Architecture

#### 1. Add IndexingState Enum

```rust
// src/lsp/backend/state.rs
#[derive(Debug, Clone, PartialEq)]
pub enum IndexingState {
    Idle,
    InProgress {
        total_files: usize,
        indexed_files: usize,
        start_time: Instant,
    },
    Complete {
        total_files: usize,
        duration: Duration,
    },
    Failed {
        error: String,
    },
}

pub struct RholangBackend {
    // ... existing fields ...
    indexing_state: Arc<RwLock<IndexingState>>,
    pending_requests: Arc<Mutex<Vec<PendingRequest>>>,
}
```

#### 2. Modify initialize() Handler

```rust
// src/lsp/backend/handlers.rs
async fn initialize(&self, params: InitializeParams) -> LspResult<InitializeResult> {
    // Return capabilities immediately (no blocking)
    let capabilities = ServerCapabilities { ... };

    // Start workspace indexing in background if rootUri provided
    if let Some(root_uri) = params.root_uri {
        let backend = self.clone();
        tokio::spawn(async move {
            backend.index_workspace_async(root_uri).await;
        });
    }

    Ok(InitializeResult {
        capabilities,
        server_info: Some(...),
    })
}
```

#### 3. Background Indexing with Progress

```rust
// src/lsp/backend/indexing.rs
impl RholangBackend {
    async fn index_workspace_async(&self, root_uri: Url) {
        *self.indexing_state.write().await = IndexingState::InProgress {
            total_files: 0,
            indexed_files: 0,
            start_time: Instant::now(),
        };

        // Send $/progress notifications
        self.client.send_notification::<ProgressNotification>(
            ProgressParams {
                token: "workspace-indexing".into(),
                value: ProgressParamsValue::WorkDoneProgress(
                    WorkDoneProgress::Begin(WorkDoneProgressBegin {
                        title: "Indexing workspace".to_string(),
                        ...
                    })
                ),
            }
        ).await;

        // Discover files
        let files = discover_rholang_files(&root_uri);

        // Update progress
        *self.indexing_state.write().await = IndexingState::InProgress {
            total_files: files.len(),
            indexed_files: 0,
            start_time: ...,
        };

        // Index files in parallel batches
        for batch in files.chunks(10) {
            let results: Vec<_> = batch.par_iter()
                .map(|file| self.index_file_sync(file))
                .collect();

            // Update progress
            let mut state = self.indexing_state.write().await;
            if let IndexingState::InProgress { indexed_files, .. } = &mut *state {
                *indexed_files += batch.len();
            }

            // Send progress notification
            self.client.send_notification::<ProgressNotification>(...).await;
        }

        // Mark complete
        *self.indexing_state.write().await = IndexingState::Complete { ... };

        // Process pending requests
        self.process_pending_requests().await;
    }
}
```

#### 4. Request Queueing

```rust
struct PendingRequest {
    method: String,
    params: serde_json::Value,
    response_sender: oneshot::Sender<LspResult<serde_json::Value>>,
}

impl RholangBackend {
    async fn handle_request_with_queue<P, R>(
        &self,
        method: &str,
        params: P,
        handler: impl FnOnce(&Self, P) -> LspResult<R>,
    ) -> LspResult<R>
    where
        P: serde::Serialize + serde::de::DeserializeOwned,
        R: serde::Serialize + serde::de::DeserializeOwned,
    {
        // Check indexing state
        let state = self.indexing_state.read().await;

        match *state {
            IndexingState::Complete { .. } => {
                // Indexing done, handle immediately
                drop(state);
                handler(self, params)
            }
            IndexingState::InProgress { .. } => {
                // Queue request
                drop(state);

                let (tx, rx) = oneshot::channel();
                let pending = PendingRequest {
                    method: method.to_string(),
                    params: serde_json::to_value(&params).unwrap(),
                    response_sender: tx,
                };

                self.pending_requests.lock().unwrap().push(pending);

                // Wait for response
                let result = rx.await.unwrap();
                serde_json::from_value(result?)
            }
            _ => {
                // Idle or failed, handle immediately
                drop(state);
                handler(self, params)
            }
        }
    }
}
```

### Files to Modify

1. `src/lsp/backend/state.rs`:
   - Add `IndexingState` enum
   - Add `indexing_state: Arc<RwLock<IndexingState>>`
   - Add `pending_requests: Arc<Mutex<Vec<PendingRequest>>>`

2. `src/lsp/backend/handlers.rs`:
   - Modify `initialize()` to spawn background indexing
   - Add request queueing wrapper for workspace-dependent requests

3. `src/lsp/backend/indexing.rs`:
   - Add `index_workspace_async()` with progress notifications
   - Add `process_pending_requests()`

4. `Cargo.toml`:
   - No new dependencies needed (uses existing tokio, futures)

### Expected Impact

- Initialize latency: 100-500ms → <20ms (5-25x faster)
- User experience: Editor responsive immediately
- Large workspaces: Transparent background indexing

---

## Phase 3: Connection Pooling (Deferred)

### Goal
Reduce per-connection memory usage from ~100MB to ~10MB by sharing workspace state.

### Current Architecture

```
Connection 1                Connection 2
├─ RholangBackend           ├─ RholangBackend
│  ├─ WorkspaceState (100MB)│  ├─ WorkspaceState (100MB)  // DUPLICATE!
│  ├─ Symbol tables         │  ├─ Symbol tables           // DUPLICATE!
│  └─ Document cache        │  └─ Document cache          // DUPLICATE!
```

**Problem:** Each connection creates a full backend instance

### Proposed Architecture

```
           SharedBackend (Singleton)
           ├─ WorkspaceState (100MB)    // SHARED
           ├─ Global symbol tables       // SHARED
           └─ Virtual document registry  // SHARED
                    ▲
          ┌─────────┴─────────┐
          │                   │
    Connection 1        Connection 2
    ├─ Document cache   ├─ Document cache
    ├─ Diagnostics      ├─ Diagnostics
    └─ Client handle    └─ Client handle
    (10MB each)         (10MB each)
```

### Implementation Strategy

#### 1. Create SharedBackend

```rust
// src/lsp/shared_backend.rs (NEW)
pub struct SharedBackend {
    workspace: Arc<WorkspaceState>,
    virtual_docs: Arc<RwLock<VirtualDocumentRegistry>>,
    detection_worker: DetectionWorkerHandle,
    diagnostic_provider: Arc<dyn DiagnosticProvider>,
}

impl SharedBackend {
    pub fn get_or_create() -> Arc<Self> {
        static INSTANCE: OnceCell<Arc<SharedBackend>> = OnceCell::new();
        INSTANCE.get_or_init(|| {
            Arc::new(SharedBackend::new())
        }).clone()
    }

    async fn new() -> Self {
        // One-time initialization
        SharedBackend {
            workspace: Arc::new(WorkspaceState::new()),
            ...
        }
    }
}
```

#### 2. Modify RholangBackend

```rust
// src/lsp/backend/state.rs
pub struct RholangBackend {
    client: Client,
    shared: Arc<SharedBackend>,  // Reference to shared state

    // Connection-specific state only
    documents_by_uri: DashMap<Url, Arc<LspDocument>>,  // Open documents
    diagnostics: DashMap<Url, Vec<Diagnostic>>,
}

impl RholangBackend {
    pub async fn new(client: Client, ...) -> anyhow::Result<Self> {
        let shared = SharedBackend::get_or_create();

        Ok(RholangBackend {
            client,
            shared,
            documents_by_uri: DashMap::new(),
            diagnostics: DashMap::new(),
        })
    }
}
```

#### 3. Update Workspace Access

```rust
// Replace all workspace access
// OLD: self.workspace.documents.get(...)
// NEW: self.shared.workspace.documents.get(...)

// Open documents override workspace
if let Some(doc) = self.documents_by_uri.get(uri) {
    // Use connection-specific open document
} else if let Some(doc) = self.shared.workspace.documents.get(uri) {
    // Fall back to shared workspace
}
```

### Files to Create/Modify

1. **NEW:** `src/lsp/shared_backend.rs`
   - `SharedBackend` struct
   - Singleton pattern with `OnceCell`

2. `src/lsp/backend/state.rs`:
   - Add `shared: Arc<SharedBackend>`
   - Remove workspace-related fields
   - Update all workspace access

3. `src/lsp/backend/handlers.rs`:
   - Update document lookup logic
   - Check open documents first, then shared workspace

### Expected Impact

- Memory per connection: 100MB → 10MB
- Reconnection time: Instant (no re-indexing)
- Multi-connection scenarios: 10x memory savings

---

## Phase 4: WebSocket Optimization (Deferred)

### Goal
Improve WebSocket throughput by 20-30% through message batching.

### Current Implementation

```rust
// src/main.rs - WebSocketStreamAdapter::poll_write
fn poll_write(..., buf: &[u8]) -> ... {
    // Sends immediately (no batching)
    this.inner.start_send_unpin(Message::Binary(buf.to_vec()))
}
```

**Problem:** Each LSP message = one WebSocket frame (overhead)

### Proposed: Message Batching

```rust
struct WebSocketStreamAdapter<S> {
    inner: WebSocketStream<S>,
    read_buffer: Vec<u8>,
    write_buffer: Vec<u8>,           // NEW
    write_timer: Option<Sleep>,      // NEW
    last_flush: Instant,             // NEW
}

impl<S> AsyncWrite for WebSocketStreamAdapter<S> {
    fn poll_write(..., buf: &[u8]) -> ... {
        // Add to write buffer instead of sending immediately
        this.write_buffer.extend_from_slice(buf);

        const BATCH_SIZE: usize = 16 * 1024;  // 16KB
        const BATCH_TIMEOUT: Duration = Duration::from_millis(5);

        // Flush if buffer is large or timeout expired
        if this.write_buffer.len() >= BATCH_SIZE
           || this.last_flush.elapsed() > BATCH_TIMEOUT {
            this.flush_write_buffer()?;
        } else {
            // Set timer to flush after timeout
            if this.write_timer.is_none() {
                this.write_timer = Some(sleep(BATCH_TIMEOUT));
            }
        }

        Ok(buf.len())
    }

    fn poll_flush(...) -> ... {
        this.flush_write_buffer()?;
        this.inner.poll_flush_unpin(cx)
    }
}
```

### Additional Optimizations

1. **Binary-only frames:**
   ```rust
   // Prefer binary over text (faster encoding)
   Message::Binary(data) vs Message::Text(json)
   ```

2. **Frame compression:**
   ```rust
   // Enable permessage-deflate extension
   accept_async_with_config(stream, WebSocketConfig {
       compression: Some(Compression::default()),
       ...
   })
   ```

3. **Backpressure handling:**
   ```rust
   // Pause reading when write buffer is full
   if this.write_buffer.len() > MAX_WRITE_BUFFER {
       return Poll::Pending;  // Apply backpressure
   }
   ```

### Files to Modify

1. `src/main.rs`:
   - Add `write_buffer` to `WebSocketStreamAdapter`
   - Implement batching in `poll_write()`
   - Add `flush_write_buffer()` helper

2. `Cargo.toml`:
   - May need to enable compression feature in `tokio-tungstenite`

### Expected Impact

- WebSocket throughput: +20-30%
- Reduced WebSocket frame overhead
- Better utilization of network bandwidth

---

## Priority & Sequencing

1. **Phase 1** ✅ (Complete)
   - Low risk, high impact
   - No architectural changes

2. **Phase 2** (High Priority)
   - Major user-facing improvement
   - 5-10x faster initialization
   - Moderate complexity

3. **Phase 3** (Medium Priority)
   - Memory savings for multi-connection
   - Instant reconnection
   - Higher complexity (architectural change)

4. **Phase 4** (Low Priority)
   - Incremental improvement
   - WebSocket-specific
   - Low-medium complexity

---

## Implementation Checklist

### Phase 2: Lazy Initialization
- [ ] Add `IndexingState` enum to state.rs
- [ ] Add `pending_requests` queue to RholangBackend
- [ ] Modify `initialize()` to spawn background task
- [ ] Implement `index_workspace_async()` with progress
- [ ] Add request queueing wrapper
- [ ] Implement `process_pending_requests()`
- [ ] Add LSP $/progress notifications
- [ ] Test with large workspace (1000+ files)
- [ ] Benchmark initialization time

### Phase 3: Connection Pooling
- [ ] Create `src/lsp/shared_backend.rs`
- [ ] Implement `SharedBackend` with singleton pattern
- [ ] Refactor `RholangBackend` to use shared state
- [ ] Update all workspace access patterns
- [ ] Update document lookup logic
- [ ] Test multi-connection scenarios
- [ ] Measure memory usage per connection

### Phase 4: WebSocket Optimization
- [ ] Add write buffer to `WebSocketStreamAdapter`
- [ ] Implement message batching logic
- [ ] Add flush timer
- [ ] Implement backpressure handling
- [ ] Enable binary-only frames
- [ ] Test compression (if enabled)
- [ ] Benchmark throughput improvement

---

## Testing Strategy

### Phase 2: Initialization
```rust
#[tokio::test]
async fn test_lazy_initialization() {
    let start = Instant::now();

    // Initialize should return quickly
    let result = backend.initialize(params).await.unwrap();
    assert!(start.elapsed() < Duration::from_millis(50));

    // Workspace should still be indexing
    let state = backend.indexing_state.read().await;
    assert!(matches!(*state, IndexingState::InProgress { .. }));

    // Wait for indexing to complete
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Pending requests should be processed
    let state = backend.indexing_state.read().await;
    assert!(matches!(*state, IndexingState::Complete { .. }));
}
```

### Phase 3: Connection Pooling
```rust
#[tokio::test]
async fn test_shared_workspace() {
    let backend1 = RholangBackend::new(...).await.unwrap();
    let backend2 = RholangBackend::new(...).await.unwrap();

    // Both should reference same workspace
    assert!(Arc::ptr_eq(&backend1.shared.workspace, &backend2.shared.workspace));

    // Index in backend1
    backend1.index_workspace(...).await;

    // Should be visible in backend2
    let doc = backend2.shared.workspace.documents.get(&uri).unwrap();
    assert_eq!(doc.uri, uri);
}
```

### Phase 4: WebSocket Batching
```rust
#[tokio::test]
async fn test_message_batching() {
    let adapter = WebSocketStreamAdapter::new(...);

    // Write multiple small messages
    for i in 0..10 {
        adapter.write_all(&small_message).await.unwrap();
    }

    // Should batch into fewer frames
    // (verify with packet capture or frame counter)
}
```

---

## Metrics to Add

Extend `src/metrics.rs`:

```rust
// Initialization metrics
pub fn record_initialize_latency(&self, duration: Duration);
pub fn record_workspace_indexing_time(&self, file_count: usize, duration: Duration);

// Connection metrics
pub fn record_connection_establishment(&self, duration: Duration);
pub fn record_connection_memory_usage(&self, bytes: usize);

// WebSocket metrics
pub fn record_websocket_batch_size(&self, messages: usize);
pub fn record_websocket_frame_count(&self);
```

---

## Success Criteria

### Phase 2
- [ ] Initialize request completes in <20ms (was 100-500ms)
- [ ] Progress notifications sent during indexing
- [ ] Pending requests processed after indexing
- [ ] All tests passing

### Phase 3
- [ ] Memory per connection <10MB (was ~100MB)
- [ ] Reconnection time <100ms (was 100-500ms)
- [ ] Workspace state shared correctly
- [ ] No data races in shared state

### Phase 4
- [ ] WebSocket throughput improved 20-30%
- [ ] Message batching working correctly
- [ ] Backpressure prevents buffer overflow
- [ ] All tests passing

---

## Notes

- Phases 2-4 are deferred pending real-world usage analysis
- Phase 1 provides immediate 10-25% improvement
- Each phase is independent and can be implemented separately
- Recommend implementing in order (2 → 3 → 4)
- Profile in production before proceeding with Phases 3-4

