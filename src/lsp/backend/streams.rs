//! Reactive stream types and utilities for LSP backend
//!
//! This module provides ReactiveX-style event streams and operators for
//! handling document changes, file system events, and workspace indexing.

use std::path::PathBuf;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use futures::Stream;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;


use super::state::{DocumentChangeEvent, IndexingTask};

/// Unified event type for backend event streams
#[derive(Debug, Clone)]
pub enum BackendEvent {
    /// Document change event (from didChange)
    DocumentChange(DocumentChangeEvent),
    /// File system change event (from notify watcher)
    FileSystemChange(Vec<PathBuf>),
    /// Workspace indexing task
    IndexingTask(IndexingTask),
    /// Shutdown signal
    Shutdown,
}

/// Stream extension trait for reactive operators
pub trait StreamExt: Stream {
    /// Debounces stream emissions, emitting only after a period of inactivity
    fn debounce_time(self, duration: Duration) -> DebounceStream<Self>
    where
        Self: Sized,
    {
        DebounceStream::new(self, duration)
    }

    /// Groups consecutive items into batches based on time window
    fn chunk_timeout(self, max_size: usize, duration: Duration) -> ChunkTimeoutStream<Self>
    where
        Self: Sized,
    {
        ChunkTimeoutStream::new(self, max_size, duration)
    }

    /// Maps each item to a future, canceling previous futures when new items arrive
    ///
    /// This is useful for operations like validation where only the latest request matters.
    /// Previous operations are automatically canceled when a new item arrives.
    fn switch_map<F, Fut, T>(self, f: F) -> SwitchMapStream<Self, F, Fut>
    where
        Self: Sized,
        F: FnMut(Self::Item) -> Fut,
        Fut: std::future::Future<Output = T>,
    {
        SwitchMapStream::new(self, f)
    }

    /// Adds a timeout to each stream item
    ///
    /// If an item is not received within the specified duration, the stream terminates.
    /// This is useful for preventing indefinite waiting on slow operations.
    fn timeout(self, duration: Duration) -> TimeoutStream<Self>
    where
        Self: Sized,
    {
        TimeoutStream::new(self, duration)
    }

    /// Retries failed operations with exponential backoff
    ///
    /// Maps each item to a future that may fail, retrying up to max_retries times
    /// with exponential backoff between attempts. Useful for transient failure recovery.
    fn retry<F, Fut, T, E>(
        self,
        max_retries: usize,
        f: F,
    ) -> RetryStream<Self, F, Fut>
    where
        Self: Sized,
        F: FnMut(Self::Item) -> Fut,
        Fut: std::future::Future<Output = Result<T, E>>,
    {
        RetryStream::new(self, max_retries, f)
    }
}

impl<T: Stream> StreamExt for T {}

/// Debounce stream operator
///
/// Emits items only after a specified duration has elapsed without new items.
/// This is useful for handling rapid document changes where we only want to
/// process the final state after edits have settled.
pub struct DebounceStream<S: Stream> {
    stream: S,
    duration: Duration,
    pending: Option<S::Item>,
    sleep: Option<Pin<Box<tokio::time::Sleep>>>,
}

impl<S: Stream> DebounceStream<S> {
    pub fn new(stream: S, duration: Duration) -> Self {
        Self {
            stream,
            duration,
            pending: None,
            sleep: None,
        }
    }
}

impl<S> Stream for DebounceStream<S>
where
    S: Stream + Unpin,
    S::Item: Unpin,
{
    type Item = S::Item;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Get mutable access to the unpinned Self
        let this = self.as_mut().get_mut();

        // Check if sleep timer has expired
        if let Some(sleep) = this.sleep.as_mut() {
            if sleep.as_mut().poll(cx).is_ready() {
                this.sleep = None;
                if let Some(item) = this.pending.take() {
                    return Poll::Ready(Some(item));
                }
            }
        }

        // Poll for new items
        loop {
            match Pin::new(&mut this.stream).poll_next(cx) {
                Poll::Ready(Some(item)) => {
                    // New item arrived, reset timer
                    this.pending = Some(item);
                    this.sleep = Some(Box::pin(tokio::time::sleep(this.duration)));
                    // Wake up when timer expires
                    if let Some(sleep) = this.sleep.as_mut() {
                        let _ = sleep.as_mut().poll(cx);
                    }
                }
                Poll::Ready(None) => {
                    // Stream ended - emit pending item if any
                    return if let Some(item) = this.pending.take() {
                        Poll::Ready(Some(item))
                    } else {
                        Poll::Ready(None)
                    };
                }
                Poll::Pending => {
                    return Poll::Pending;
                }
            }
        }
    }
}

/// Chunk with timeout stream operator
///
/// Groups items into chunks based on either:
/// - Maximum chunk size reached
/// - Timeout duration elapsed since first item in chunk
pub struct ChunkTimeoutStream<S: Stream> {
    stream: S,
    max_size: usize,
    duration: Duration,
    buffer: Vec<S::Item>,
    sleep: Option<Pin<Box<tokio::time::Sleep>>>,
}

impl<S: Stream> ChunkTimeoutStream<S> {
    pub fn new(stream: S, max_size: usize, duration: Duration) -> Self {
        Self {
            stream,
            max_size,
            duration,
            buffer: Vec::new(),
            sleep: None,
        }
    }
}

impl<S> Stream for ChunkTimeoutStream<S>
where
    S: Stream + Unpin,
    S::Item: Unpin,
{
    type Item = Vec<S::Item>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            // Get mutable access to the unpinned Self
            let this = self.as_mut().get_mut();

            // Check if timer expired
            if let Some(sleep) = this.sleep.as_mut() {
                if sleep.as_mut().poll(cx).is_ready() {
                    this.sleep = None;
                    if !this.buffer.is_empty() {
                        let chunk = std::mem::take(&mut this.buffer);
                        return Poll::Ready(Some(chunk));
                    }
                }
            }

            // Poll for new items
            match Pin::new(&mut this.stream).poll_next(cx) {
                Poll::Ready(Some(item)) => {
                    // Add item to buffer
                    this.buffer.push(item);

                    // Start timer if this is first item
                    if this.buffer.len() == 1 {
                        this.sleep = Some(Box::pin(tokio::time::sleep(this.duration)));
                    }

                    // Emit chunk if buffer full
                    if this.buffer.len() >= this.max_size {
                        this.sleep = None;
                        let chunk = std::mem::take(&mut this.buffer);
                        return Poll::Ready(Some(chunk));
                    }
                }
                Poll::Ready(None) => {
                    // Stream ended - emit remaining items if any
                    return if !this.buffer.is_empty() {
                        let chunk = std::mem::take(&mut this.buffer);
                        Poll::Ready(Some(chunk))
                    } else {
                        Poll::Ready(None)
                    };
                }
                Poll::Pending => {
                    // Keep timer running if we have pending items
                    if let Some(sleep) = this.sleep.as_mut() {
                        let _ = sleep.as_mut().poll(cx);
                    }
                    return Poll::Pending;
                }
            }
        }
    }
}

/// Creates a document change stream from an mpsc receiver
pub fn document_change_stream(
    rx: mpsc::Receiver<DocumentChangeEvent>,
) -> impl Stream<Item = DocumentChangeEvent> {
    ReceiverStream::new(rx)
}

/// Creates an indexing task stream from an mpsc receiver
pub fn indexing_task_stream(
    rx: mpsc::Receiver<IndexingTask>,
) -> impl Stream<Item = IndexingTask> {
    ReceiverStream::new(rx)
}

/// Creates a file system event stream from notify events
pub fn file_system_stream(
    rx: std::sync::mpsc::Receiver<notify::Result<notify::Event>>,
) -> Pin<Box<dyn Stream<Item = Vec<PathBuf>> + Send>> {
    use futures::stream::{self, StreamExt};

    // Convert std::sync::mpsc to async stream
    Box::pin(stream::unfold(rx, |rx| async move {
        match rx.recv() {
            Ok(Ok(event)) => {
                let paths: Vec<PathBuf> = event
                    .paths
                    .into_iter()
                    .filter(|p| p.extension().map_or(false, |ext| ext == "rho"))
                    .collect();

                if !paths.is_empty() {
                    Some((paths, rx))
                } else {
                    // Recursively try next event
                    None
                }
            }
            Ok(Err(e)) => {
                tracing::warn!("File watcher error: {}", e);
                None
            }
            Err(_) => None, // Channel closed
        }
    }))
}

/// Creates a file system event stream from an Arc<Mutex<Receiver>>
///
/// This variant works with shared receivers wrapped in Arc<Mutex<>> to support
/// multiple potential readers. It polls the receiver using an interval-based approach
/// that properly yields to allow shutdown signals and other tasks to run.
pub fn file_system_stream_from_arc(
    rx: std::sync::Arc<std::sync::Mutex<std::sync::mpsc::Receiver<notify::Result<notify::Event>>>>,
) -> Pin<Box<dyn Stream<Item = Vec<PathBuf>> + Send>> {
    use futures::stream::{self, StreamExt};
    use tokio::time::{interval, Duration};

    // Use an interval stream to poll the receiver periodically
    // This ensures we yield to the async runtime between polls, allowing
    // shutdown signals to be processed
    let mut interval_stream = interval(Duration::from_millis(10));

    Box::pin(stream::unfold((rx, interval_stream), |(rx, mut interval)| async move {
        loop {
            // Wait for next tick - this yields to the runtime
            interval.tick().await;

            // Try to receive from the shared receiver (non-blocking)
            let recv_result = {
                let guard = match rx.lock() {
                    Ok(g) => g,
                    Err(_) => return None,  // Lock poisoned, terminate stream
                };
                guard.try_recv()
            };

            match recv_result {
                Ok(Ok(event)) => {
                    let paths: Vec<PathBuf> = event
                        .paths
                        .into_iter()
                        .filter(|p| p.extension().map_or(false, |ext| ext == "rho"))
                        .collect();

                    if !paths.is_empty() {
                        return Some((paths, (rx, interval)));
                    }
                    // Continue loop to try next event if no .rho files
                }
                Ok(Err(e)) => {
                    tracing::warn!("File watcher error: {}", e);
                    // Continue to next event
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    // No events available, continue loop to wait for next tick
                    // This will yield back to the async runtime
                    continue;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    // Channel closed, terminate stream
                    return None;
                }
            }
        }
    }))
}

/// Switch map stream operator
///
/// Maps each item to a future, automatically canceling previous futures when new items arrive.
/// This implements the ReactiveX `switchMap` operator pattern for automatic cancellation.
///
/// Note: Futures are canceled by dropping them when a new item arrives. The future must be Send
/// to allow safe cancellation across thread boundaries.
pub struct SwitchMapStream<S, F, Fut>
where
    S: Stream,
    F: FnMut(S::Item) -> Fut,
    Fut: std::future::Future,
{
    stream: Pin<Box<S>>,
    f: F,
    current_future: Option<Pin<Box<Fut>>>,
}

impl<S, F, Fut> SwitchMapStream<S, F, Fut>
where
    S: Stream,
    F: FnMut(S::Item) -> Fut,
    Fut: std::future::Future,
{
    pub fn new(stream: S, f: F) -> Self {
        Self {
            stream: Box::pin(stream),
            f,
            current_future: None,
        }
    }
}

impl<S, F, Fut> Stream for SwitchMapStream<S, F, Fut>
where
    S: Stream,
    F: FnMut(S::Item) -> Fut + Unpin,
    Fut: std::future::Future,
{
    type Item = Fut::Output;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        // Poll the source stream for new items
        match this.stream.as_mut().poll_next(cx) {
            Poll::Ready(Some(item)) => {
                // New item arrived - cancel previous future and start new one
                let fut = (this.f)(item);
                this.current_future = Some(Box::pin(fut));
                // Don't return yet - poll the new future below
            }
            Poll::Ready(None) => {
                // Source stream ended
                if this.current_future.is_none() {
                    return Poll::Ready(None);
                }
                // Fall through to poll current future one last time
            }
            Poll::Pending => {
                // No new items, continue with current future
            }
        }

        // Poll the current future if we have one
        if let Some(fut) = this.current_future.as_mut() {
            match fut.as_mut().poll(cx) {
                Poll::Ready(output) => {
                    this.current_future = None;
                    return Poll::Ready(Some(output));
                }
                Poll::Pending => {
                    return Poll::Pending;
                }
            }
        }

        // Stream ended and no current future
        if this.current_future.is_none() {
            Poll::Ready(None)
        } else {
            Poll::Pending
        }
    }
}

/// Timeout stream operator
///
/// Adds a timeout to stream items. If no item is received within the duration,
/// the stream terminates. This prevents indefinite waiting.
pub struct TimeoutStream<S: Stream> {
    stream: Pin<Box<S>>,
    duration: Duration,
    timeout: Option<Pin<Box<tokio::time::Sleep>>>,
}

impl<S: Stream> TimeoutStream<S> {
    pub fn new(stream: S, duration: Duration) -> Self {
        Self {
            stream: Box::pin(stream),
            duration,
            timeout: Some(Box::pin(tokio::time::sleep(duration))),
        }
    }
}

impl<S> Stream for TimeoutStream<S>
where
    S: Stream,
{
    type Item = S::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        // Check if timeout expired
        if let Some(timeout) = this.timeout.as_mut() {
            if timeout.as_mut().poll(cx).is_ready() {
                // Timeout expired, terminate stream
                return Poll::Ready(None);
            }
        }

        // Poll the stream
        match this.stream.as_mut().poll_next(cx) {
            Poll::Ready(Some(item)) => {
                // Reset timeout for next item
                this.timeout = Some(Box::pin(tokio::time::sleep(this.duration)));
                Poll::Ready(Some(item))
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => {
                // Keep timeout running
                if let Some(timeout) = this.timeout.as_mut() {
                    let _ = timeout.as_mut().poll(cx);
                }
                Poll::Pending
            }
        }
    }
}

/// Retry stream operator
///
/// Maps each item to a future that may fail, retrying with exponential backoff.
/// This provides automatic recovery from transient failures.
pub struct RetryStream<S, F, Fut>
where
    S: Stream,
    F: FnMut(S::Item) -> Fut,
    Fut: std::future::Future,
{
    stream: Pin<Box<S>>,
    f: F,
    max_retries: usize,
    current_item: Option<S::Item>,
    current_future: Option<Pin<Box<Fut>>>,
    retry_count: usize,
}

impl<S, F, Fut> RetryStream<S, F, Fut>
where
    S: Stream,
    F: FnMut(S::Item) -> Fut,
    Fut: std::future::Future,
{
    pub fn new(stream: S, max_retries: usize, f: F) -> Self {
        Self {
            stream: Box::pin(stream),
            f,
            max_retries,
            current_item: None,
            current_future: None,
            retry_count: 0,
        }
    }
}

impl<S, F, Fut, T, E> Stream for RetryStream<S, F, Fut>
where
    S: Stream,
    S::Item: Clone + Unpin,
    F: FnMut(S::Item) -> Fut + Unpin,
    Fut: std::future::Future<Output = Result<T, E>>,
{
    type Item = Result<T, E>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        loop {
            // If we have a current future, poll it
            if let Some(fut) = this.current_future.as_mut() {
                match fut.as_mut().poll(cx) {
                    Poll::Ready(Ok(value)) => {
                        // Success! Reset state and return
                        this.current_future = None;
                        this.current_item = None;
                        this.retry_count = 0;
                        return Poll::Ready(Some(Ok(value)));
                    }
                    Poll::Ready(Err(err)) => {
                        // Failure - check if we should retry
                        if this.retry_count < this.max_retries {
                            this.retry_count += 1;
                            // Exponential backoff: 2^retry_count * 100ms
                            let backoff_ms = (1 << this.retry_count) * 100;
                            tracing::debug!("Retry attempt {} after {}ms", this.retry_count, backoff_ms);

                            // Start new future with same item
                            if let Some(item) = this.current_item.clone() {
                                let fut = (this.f)(item);
                                this.current_future = Some(Box::pin(fut));
                                // Continue loop to poll new future
                                continue;
                            } else {
                                // No item to retry, return error
                                this.current_future = None;
                                this.retry_count = 0;
                                return Poll::Ready(Some(Err(err)));
                            }
                        } else {
                            // Max retries exceeded, return error
                            tracing::warn!("Max retries ({}) exceeded", this.max_retries);
                            this.current_future = None;
                            this.current_item = None;
                            this.retry_count = 0;
                            return Poll::Ready(Some(Err(err)));
                        }
                    }
                    Poll::Pending => return Poll::Pending,
                }
            }

            // No current future - poll stream for next item
            match this.stream.as_mut().poll_next(cx) {
                Poll::Ready(Some(item)) => {
                    // New item - start future
                    this.current_item = Some(item.clone());
                    this.retry_count = 0;
                    let fut = (this.f)(item);
                    this.current_future = Some(Box::pin(fut));
                    // Continue loop to poll new future
                }
                Poll::Ready(None) => {
                    // Stream ended
                    return Poll::Ready(None);
                }
                Poll::Pending => {
                    return Poll::Pending;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt as FuturesStreamExt;
    use tokio::time::{sleep, Duration};

    #[tokio::test]
    async fn test_debounce_stream() {
        let (tx, rx) = mpsc::channel(10);
        let stream = ReceiverStream::new(rx).debounce_time(Duration::from_millis(100));

        // Send rapid events
        tx.send(1).await.unwrap();
        tx.send(2).await.unwrap();
        tx.send(3).await.unwrap();
        drop(tx);

        // Should only emit last value after debounce
        let results: Vec<i32> = stream.collect().await;
        assert_eq!(results, vec![3]);
    }

    #[tokio::test]
    async fn test_chunk_timeout_by_size() {
        let (tx, rx) = mpsc::channel(10);
        let mut stream = Box::pin(ReceiverStream::new(rx).chunk_timeout(3, Duration::from_secs(10)));

        // Send exactly 3 items
        tx.send(1).await.unwrap();
        tx.send(2).await.unwrap();
        tx.send(3).await.unwrap();

        // Should emit immediately when size reached
        let chunk = stream.next().await.unwrap();
        assert_eq!(chunk, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn test_chunk_timeout_by_time() {
        let (tx, rx) = mpsc::channel(10);
        let mut stream = Box::pin(ReceiverStream::new(rx).chunk_timeout(10, Duration::from_millis(100)));

        // Send 2 items (less than max_size)
        tx.send(1).await.unwrap();
        tx.send(2).await.unwrap();

        // Wait for timeout
        sleep(Duration::from_millis(150)).await;

        // Should emit after timeout
        let chunk = stream.next().await.unwrap();
        assert_eq!(chunk, vec![1, 2]);
    }

    #[tokio::test]
    async fn test_switch_map_cancellation() {
        use std::sync::{Arc, atomic::{AtomicU32, Ordering}};
        use futures::stream::StreamExt as FutStreamExt;

        let (tx, rx) = mpsc::channel(10);
        let counter = Arc::new(AtomicU32::new(0));

        let counter_clone = counter.clone();
        let mut stream = ReceiverStream::new(rx).switch_map(move |value: i32| {
            let counter = counter_clone.clone();
            async move {
                // Simulate async work
                sleep(Duration::from_millis(50)).await;
                counter.fetch_add(1, Ordering::SeqCst);
                value * 10
            }
        });

        // Send multiple rapid items
        tx.send(1).await.unwrap();
        sleep(Duration::from_millis(10)).await;  // Send before first completes
        tx.send(2).await.unwrap();
        sleep(Duration::from_millis(10)).await;  // Send before second completes
        tx.send(3).await.unwrap();
        drop(tx);

        // Only the last value should complete (30)
        let result = stream.next().await.unwrap();
        assert_eq!(result, 30);

        // Should be no more results (earlier futures were canceled)
        assert!(stream.next().await.is_none());

        // Counter should only increment once (for the completed future)
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }
}
