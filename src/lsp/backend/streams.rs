//! Reactive stream types and utilities for LSP backend
//!
//! This module provides ReactiveX-style event streams and operators for
//! handling document changes, file system events, and workspace indexing.

use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use futures::Stream;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tower_lsp::lsp_types::Url;

use crate::lsp::models::LspDocument;

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
}
