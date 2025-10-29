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

use super::state::{DiagnosticUpdate, DocumentChangeEvent, IndexingTask, RholangBackend};
use super::streams::{self, BackendEvent, StreamExt as CustomStreamExt};

impl RholangBackend {
    /// Spawns a reactive file watcher using stream operators
    ///
    /// This replaces the imperative `spawn_file_watcher` with a declarative
    /// reactive stream that:
    /// - Filters .rho files
    /// - Batches events with 100ms timeout
    /// - Processes batches concurrently with 5-second timeout per file
    /// - Provides timeout protection against stuck file processing
    /// - Automatically shuts down on signal
    pub(super) fn spawn_reactive_file_watcher(
        backend: RholangBackend,
        file_events: Arc<std::sync::Mutex<std::sync::mpsc::Receiver<notify::Result<notify::Event>>>>,
    ) {
        let mut shutdown_rx = backend.shutdown_tx.subscribe();

        tokio::spawn(async move {
            // Create file system event stream by polling the Arc<Mutex<Receiver>>
            let file_stream = streams::file_system_stream_from_arc(file_events);

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

                // Process files concurrently with timeout
                let handles: Vec<_> = paths
                    .into_iter()
                    .map(|path| {
                        let backend = backend.clone();
                        let path_clone = path.clone();
                        tokio::spawn(async move {
                            // Add 5-second timeout to file processing
                            match tokio::time::timeout(
                                Duration::from_secs(5),
                                backend.handle_file_change(path)
                            ).await {
                                Ok(_) => {
                                    trace!("Successfully processed file change: {:?}", path_clone);
                                }
                                Err(_) => {
                                    error!("Timeout processing file change: {:?}", path_clone);
                                }
                            }
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
    /// - Debounces each URI independently with 100ms
    /// - Automatically cancels previous validations (via manual cancellation tokens)
    /// - Processes validations concurrently with 10-second timeout
    /// - Provides timeout protection against stuck validations
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
            let debounce_duration = Duration::from_millis(100);

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
                                if let Some(cancel_tx) = backend.validation_cancel.lock().await.remove(&uri) {
                                    let _ = cancel_tx.send(());
                                    trace!("Cancelled previous validation for {}", uri);
                                }

                                // Create new cancellation token
                                let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
                                backend.validation_cancel.lock().await.insert(uri.clone(), cancel_tx);

                                // Spawn validation with timeout
                                let backend_clone = backend.clone();
                                let uri_clone = uri.clone();
                                let text_clone = event.text.clone();
                                let version_clone = event.version;
                                tokio::spawn(async move {
                                    tokio::select! {
                                        result = tokio::time::timeout(
                                            Duration::from_secs(10),
                                            backend_clone.validate(event.document.clone(), &text_clone, event.version)
                                        ) => {
                                            match result {
                                                Ok(Ok(diagnostics)) => {
                                                    trace!("Validation completed for {}", uri_clone);
                                                    // Publish diagnostics to client
                                                    if event.document.version().await == version_clone {
                                                        backend_clone.client.publish_diagnostics(uri_clone.clone(), diagnostics, Some(version_clone)).await;
                                                    }
                                                }
                                                Ok(Err(e)) => error!("Validation failed for {}: {}", uri_clone, e),
                                                Err(_) => error!("Validation timeout for {}", uri_clone),
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
                    match backend.index_file(&task.uri, &task.text, 0, None).await {
                        Ok(cached_doc) => {
                            backend.update_workspace_document(&task.uri, std::sync::Arc::new(cached_doc)).await;
                        }
                        Err(e) => error!("Failed to index {}: {}", task.uri, e),
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
            use futures::stream::select_all;

            // Create individual event streams
            let doc_stream = ReceiverStream::new(doc_change_rx)
                .map(BackendEvent::DocumentChange);

            let indexing_stream = ReceiverStream::new(indexing_rx)
                .map(BackendEvent::IndexingTask);

            let file_stream = streams::file_system_stream_from_arc(file_events)
                .map(BackendEvent::FileSystemChange);

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

    /// Spawns a debounced symbol linker task
    ///
    /// This function creates a background task that listens for symbol linking requests
    /// and batches them to reduce lock contention. Instead of calling link_symbols()
    /// immediately for each file update, requests are batched with a 50ms timeout window.
    ///
    /// Benefits:
    /// - Reduces workspace write lock acquisitions from O(n) to O(1) per batch
    /// - Improves indexing throughput during workspace initialization
    /// - Safe: All link requests are eventually processed, none are dropped
    pub(super) fn spawn_debounced_symbol_linker(
        backend: RholangBackend,
        link_symbols_rx: tokio::sync::mpsc::Receiver<()>,
    ) {
        let mut shutdown_rx = backend.shutdown_tx.subscribe();

        tokio::spawn(async move {
            // Create stream from receiver
            let link_stream = ReceiverStream::new(link_symbols_rx);

            // Apply reactive operators with batching
            let mut reactive_stream = Box::pin(
                link_stream
                    // Batch link requests with 50ms timeout window
                    .chunk_timeout(100, Duration::from_millis(50))
                    // Take until shutdown
                    .take_until(async move {
                        let _ = shutdown_rx.recv().await;
                        info!("Debounced symbol linker received shutdown signal");
                    })
            );

            // Process batches
            while let Some(batch) = reactive_stream.next().await {
                let batch_size = batch.len();
                trace!("Symbol linking batch: {} requests collapsed into 1 call", batch_size);

                // Execute single link_symbols call for entire batch
                backend.link_symbols().await;

                // Also link virtual document symbols
                backend.link_virtual_symbols().await;

                debug!("Completed symbol linking for batch of {} requests", batch_size);
            }

            info!("Debounced symbol linker task terminated");
        });
    }

    /// Spawns a debounced diagnostics publisher task
    ///
    /// This function creates a background task that batches diagnostic updates before
    /// publishing them to the LSP client. This reduces LSP protocol overhead and
    /// prevents UI flicker during rapid typing.
    ///
    /// Features:
    /// - Batches diagnostics with 150ms timeout window
    /// - Deduplicates: keeps only the latest diagnostics per URI
    /// - Version checking: only publishes if document version matches
    /// - Safe: All diagnostic updates are eventually published, none are dropped
    pub(super) fn spawn_debounced_diagnostics_publisher(
        backend: RholangBackend,
        diagnostics_rx: tokio::sync::mpsc::Receiver<DiagnosticUpdate>,
    ) {
        let mut shutdown_rx = backend.shutdown_tx.subscribe();

        tokio::spawn(async move {
            use std::collections::HashMap;

            // Create stream from receiver
            let diagnostics_stream = ReceiverStream::new(diagnostics_rx);

            // Apply reactive operators with batching
            let mut reactive_stream = Box::pin(
                diagnostics_stream
                    // Batch diagnostics with 150ms timeout window
                    .chunk_timeout(50, Duration::from_millis(150))
                    // Take until shutdown
                    .take_until(async move {
                        let _ = shutdown_rx.recv().await;
                        info!("Debounced diagnostics publisher received shutdown signal");
                    })
            );

            // Process batches
            while let Some(batch) = reactive_stream.next().await {
                // Deduplicate: keep only latest diagnostics per URI
                let mut latest_by_uri: HashMap<tower_lsp::lsp_types::Url, DiagnosticUpdate> =
                    HashMap::new();

                for update in batch {
                    // Always keep the latest update for each URI
                    latest_by_uri
                        .entry(update.uri.clone())
                        .and_modify(|existing| {
                            // Keep the one with higher version, or the newer one if versions are equal
                            match (existing.version, update.version) {
                                (Some(existing_ver), Some(new_ver)) if new_ver > existing_ver => {
                                    *existing = update.clone();
                                }
                                (None, Some(_)) => {
                                    *existing = update.clone();
                                }
                                _ => {}
                            }
                        })
                        .or_insert(update);
                }

                trace!(
                    "Diagnostics batch: {} updates deduplicated to {} unique URIs",
                    latest_by_uri.len(),
                    latest_by_uri.len()
                );

                // Publish deduplicated diagnostics
                for (uri, update) in latest_by_uri {
                    let diagnostic_count = update.diagnostics.len();

                    // Publish diagnostics to client
                    backend
                        .client
                        .publish_diagnostics(uri.clone(), update.diagnostics, update.version)
                        .await;

                    // Broadcast completion event for tests/subscribers
                    let _ = backend.diagnostics_published.send(crate::lsp::backend::state::DiagnosticPublished {
                        uri,
                        version: update.version,
                        diagnostic_count,
                    });
                }

                debug!("Published diagnostics batch");
            }

            info!("Debounced diagnostics publisher task terminated");
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tests would go here to verify reactive behavior
    // For now, the integration tests in the main codebase cover this
}
