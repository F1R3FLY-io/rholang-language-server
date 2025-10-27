//! Reactive stream-based event handlers for LSP backend
//!
//! This module provides ReactiveX-style implementations of backend event handlers
//! using tokio-stream operators for better composability and less manual state management.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, error, info, trace};

use super::state::{DocumentChangeEvent, IndexingTask, RholangBackend};
use super::streams::{self, BackendEvent, StreamExt as CustomStreamExt};

impl RholangBackend {
    /// Spawns a reactive file watcher using stream operators
    ///
    /// This replaces the imperative `spawn_file_watcher` with a declarative
    /// reactive stream that:
    /// - Filters .rho files
    /// - Batches events with 100ms timeout
    /// - Processes batches concurrently
    /// - Automatically shuts down on signal
    pub(super) fn spawn_reactive_file_watcher(
        backend: RholangBackend,
        file_events: Arc<std::sync::Mutex<std::sync::mpsc::Receiver<notify::Result<notify::Event>>>>,
    ) {
        let mut shutdown_rx = backend.shutdown_tx.subscribe();

        tokio::spawn(async move {
            // Create file system event stream
            let file_stream = streams::file_system_stream(
                // Move receiver out of Arc<Mutex<>> for stream consumption
                Arc::try_unwrap(file_events)
                    .unwrap_or_else(|arc| {
                        // If Arc has multiple owners, we need to clone the receiver
                        // This shouldn't happen in practice
                        panic!("File events receiver has multiple owners")
                    })
                    .into_inner()
                    .expect("Mutex poisoned")
            );

            // Apply reactive operators
            let mut reactive_stream = Box::pin(
                file_stream
                    // Batch events with 100ms timeout
                    .chunk_timeout(10, Duration::from_millis(100))
                    // Flatten batches of batches into single batch
                    .map(|batches| {
                        batches.into_iter().flatten().collect::<Vec<PathBuf>>()
                    })
                    // Filter empty batches
                    .filter(|paths| futures::future::ready(!paths.is_empty()))
                    // Take until shutdown signal
                    .take_until(async move {
                        let _ = shutdown_rx.recv().await;
                        info!("Reactive file watcher received shutdown signal");
                    })
            );

            // Process stream
            while let Some(paths) = reactive_stream.next().await {
                info!("Processing batch of {} file changes", paths.len());

                // Process files concurrently
                let handles: Vec<_> = paths
                    .into_iter()
                    .map(|path| {
                        let backend = backend.clone();
                        tokio::spawn(async move {
                            backend.handle_file_change(path).await;
                        })
                    })
                    .collect();

                // Wait for all to complete
                for handle in handles {
                    let _ = handle.await;
                }
            }

            info!("Reactive file watcher task terminated");
        });
    }

    /// Spawns a reactive document debouncer using stream operators
    ///
    /// This replaces the imperative debouncer with a declarative stream that:
    /// - Groups events by URI
    /// - Debounces each URI independently with 300ms
    /// - Automatically cancels previous validations (via switch_map semantics)
    /// - Processes validations concurrently
    pub(super) fn spawn_reactive_document_debouncer(
        backend: RholangBackend,
        doc_change_rx: tokio::sync::mpsc::Receiver<DocumentChangeEvent>,
    ) {
        let mut shutdown_rx = backend.shutdown_tx.subscribe();

        tokio::spawn(async move {
            use std::collections::HashMap;

            // Create document change stream
            let doc_stream = ReceiverStream::new(doc_change_rx);

            // Apply reactive operators
            let mut reactive_stream = Box::pin(
                doc_stream
                    // Take until shutdown
                    .take_until(async move {
                        let _ = shutdown_rx.recv().await;
                        info!("Reactive document debouncer received shutdown signal");
                    })
            );

            // Per-URI debounce state
            let mut uri_debouncers: HashMap<tower_lsp::lsp_types::Url, tokio::time::Instant> =
                HashMap::new();
            let debounce_duration = Duration::from_millis(300);

            // Manual debounce implementation with per-URI tracking
            // (tokio-stream doesn't have group_by + debounce built-in)
            let mut pending_events: HashMap<tower_lsp::lsp_types::Url, DocumentChangeEvent> =
                HashMap::new();

            loop {
                tokio::select! {
                    Some(event) = reactive_stream.next() => {
                        // Store event and update timestamp
                        uri_debouncers.insert(event.uri.clone(), tokio::time::Instant::now());
                        pending_events.insert(event.uri.clone(), event);
                    }
                    _ = tokio::time::sleep(Duration::from_millis(50)) => {
                        // Check which URIs are ready to process
                        let now = tokio::time::Instant::now();
                        let mut ready_uris = Vec::new();

                        for (uri, timestamp) in &uri_debouncers {
                            if now.duration_since(*timestamp) >= debounce_duration {
                                ready_uris.push(uri.clone());
                            }
                        }

                        // Process ready events
                        for uri in ready_uris {
                            uri_debouncers.remove(&uri);
                            if let Some(event) = pending_events.remove(&uri) {
                                // Cancel previous validation for this URI
                                if let Some(cancel_tx) = backend.validation_cancel.lock().unwrap().remove(&uri) {
                                    let _ = cancel_tx.send(());
                                    trace!("Cancelled previous validation for {}", uri);
                                }

                                // Create new cancellation token
                                let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
                                backend.validation_cancel.lock().unwrap().insert(uri.clone(), cancel_tx);

                                // Spawn validation
                                let backend_clone = backend.clone();
                                let uri_clone = uri.clone();
                                let text_clone = event.text.clone();
                                tokio::spawn(async move {
                                    tokio::select! {
                                        result = backend_clone.validate(event.document, &text_clone, event.version) => {
                                            match result {
                                                Ok(_) => trace!("Validation completed for {}", uri_clone),
                                                Err(e) => error!("Validation failed for {}: {}", uri_clone, e),
                                            }
                                        }
                                        _ = cancel_rx => {
                                            debug!("Validation cancelled for {}", uri_clone);
                                        }
                                    }
                                });
                            }
                        }
                    }
                    else => break,
                }
            }

            info!("Reactive document debouncer task terminated");
        });
    }

    /// Spawns a reactive progressive indexer using stream operators
    ///
    /// This replaces the imperative indexer with a declarative stream that:
    /// - Batches tasks with priority-based sorting
    /// - Processes high-priority tasks first
    /// - Links symbols after each batch
    pub(super) fn spawn_reactive_progressive_indexer(
        backend: RholangBackend,
        indexing_rx: tokio::sync::mpsc::Receiver<IndexingTask>,
    ) {
        let mut shutdown_rx = backend.shutdown_tx.subscribe();

        tokio::spawn(async move {
            use std::collections::BinaryHeap;
            use std::cmp::Ordering;

            #[derive(Eq, PartialEq)]
            struct PrioritizedTask(u8, IndexingTask);

            impl Ord for PrioritizedTask {
                fn cmp(&self, other: &Self) -> Ordering {
                    self.0.cmp(&other.0).reverse() // Lower number = higher priority
                }
            }

            impl PartialOrd for PrioritizedTask {
                fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
                    Some(self.cmp(other))
                }
            }

            // Create indexing task stream
            let indexing_stream = ReceiverStream::new(indexing_rx);

            // Apply reactive operators
            let mut reactive_stream = Box::pin(
                indexing_stream
                    // Batch tasks with 200ms timeout
                    .chunk_timeout(10, Duration::from_millis(200))
                    // Take until shutdown
                    .take_until(async move {
                        let _ = shutdown_rx.recv().await;
                        info!("Reactive progressive indexer received shutdown signal");
                    })
            );

            // Process batches
            while let Some(tasks) = reactive_stream.next().await {
                // Sort by priority
                let mut queue: BinaryHeap<PrioritizedTask> = tasks
                    .into_iter()
                    .map(|task| PrioritizedTask(task.priority, task))
                    .collect();

                debug!("Processing indexing batch of {} tasks", queue.len());

                // Process each task
                while let Some(PrioritizedTask(_, task)) = queue.pop() {
                    if let Err(e) = backend.index_file(&task.uri, &task.text, 0, None).await {
                        error!("Failed to index {}: {}", task.uri, e);
                    }
                }

                // Link symbols after batch
                backend.link_symbols().await;
            }

            info!("Reactive progressive indexer task terminated");
        });
    }

    /// Creates a unified event stream pipeline that merges all backend event sources
    ///
    /// This is the ReactiveX Phase 2 unified pipeline that:
    /// - Merges file system, document, and indexing events
    /// - Handles priority-based event processing
    /// - Provides centralized event coordination
    /// - Automatically shuts down on signal
    ///
    /// Note: This is currently unused but demonstrates the pattern for future unified event handling
    #[allow(dead_code)]
    pub(super) fn spawn_unified_event_pipeline(
        backend: RholangBackend,
        doc_change_rx: tokio::sync::mpsc::Receiver<DocumentChangeEvent>,
        indexing_rx: tokio::sync::mpsc::Receiver<IndexingTask>,
        file_events: Arc<std::sync::Mutex<std::sync::mpsc::Receiver<notify::Result<notify::Event>>>>,
    ) {
        let mut shutdown_rx = backend.shutdown_tx.subscribe();

        tokio::spawn(async move {
            use futures::stream::{self, select_all};

            // Create individual event streams
            let doc_stream = ReceiverStream::new(doc_change_rx)
                .map(BackendEvent::DocumentChange);

            let indexing_stream = ReceiverStream::new(indexing_rx)
                .map(BackendEvent::IndexingTask);

            let file_stream = streams::file_system_stream(
                Arc::try_unwrap(file_events)
                    .unwrap_or_else(|_| panic!("File events receiver has multiple owners"))
                    .into_inner()
                    .expect("Mutex poisoned")
            ).map(BackendEvent::FileSystemChange);

            // Merge all streams into unified pipeline
            let mut unified_stream = Box::pin(
                select_all::<Vec<std::pin::Pin<Box<dyn futures::Stream<Item = BackendEvent> + Send>>>>(vec![
                    Box::pin(doc_stream),
                    Box::pin(indexing_stream),
                    Box::pin(file_stream),
                ])
                .take_until(async move {
                    let _ = shutdown_rx.recv().await;
                    info!("Unified event pipeline received shutdown signal");
                })
            );

            // Process unified event stream
            while let Some(event) = unified_stream.next().await {
                match event {
                    BackendEvent::DocumentChange(change_event) => {
                        debug!("Processing document change: {}", change_event.uri);
                        // Handle document change with debouncing
                        // (In practice, this would integrate with existing debouncer logic)
                    }
                    BackendEvent::IndexingTask(task) => {
                        debug!("Processing indexing task: {}", task.uri);
                        if let Err(e) = backend.index_file(&task.uri, &task.text, 0, None).await {
                            error!("Failed to index {}: {}", task.uri, e);
                        }
                    }
                    BackendEvent::FileSystemChange(paths) => {
                        debug!("Processing {} file system changes", paths.len());
                        for path in paths {
                            backend.handle_file_change(path).await;
                        }
                    }
                    BackendEvent::Shutdown => {
                        info!("Shutdown event received");
                        break;
                    }
                }
            }

            info!("Unified event pipeline task terminated");
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tests would go here to verify reactive behavior
    // For now, the integration tests in the main codebase cover this
}
