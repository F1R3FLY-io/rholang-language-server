//! Dirty file tracking for incremental workspace indexing (Phase 11.1)
//!
//! This module provides the `DirtyFileTracker` which tracks files that have changed
//! since the last indexing cycle. Files are batched within a debounce window (default 100ms)
//! and processed in priority order (open files before workspace files).
//!
//! # Performance
//!
//! - **Mark dirty**: O(1) lock-free insert into DashMap
//! - **Drain dirty**: O(k log k) where k = number of dirty files (sorting by priority)
//! - **Memory overhead**: ~48 bytes per dirty file + DashMap overhead
//!
//! # Usage
//!
//! ```ignore
//! use rholang_language_server::lsp::backend::dirty_tracker::{DirtyFileTracker, DirtyReason};
//!
//! let tracker = DirtyFileTracker::new();
//!
//! // Mark file as dirty (from didChange handler)
//! tracker.mark_dirty(
//!     uri.clone(),
//!     0,  // High priority (open file)
//!     DirtyReason::DidChange,
//! );
//!
//! // Background task periodically checks and drains
//! if tracker.should_flush() {
//!     let dirty_files = tracker.drain_dirty();
//!     // Process dirty files incrementally...
//! }
//! ```

use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tower_lsp::lsp_types::Url;
use parking_lot::RwLock;

/// Tracks dirty files and batches them for incremental indexing
///
/// Files are considered "dirty" when they've been modified since the last indexing cycle.
/// The tracker batches changes within a debounce window to avoid thrashing on rapid edits.
#[derive(Clone, Debug)]
pub struct DirtyFileTracker {
    /// Files marked dirty since last indexing cycle
    /// Uses DashMap for lock-free concurrent access
    dirty_files: Arc<DashMap<Url, DirtyFileMetadata>>,

    /// Debouncing: batch changes within this window
    /// Default: 100ms (reasonable for typing cadence)
    debounce_window: Duration,

    /// Last indexing cycle completion time
    /// Used to calculate debounce window expiration
    last_cycle: Arc<RwLock<Instant>>,
}

/// Metadata about a dirty file
#[derive(Debug, Clone)]
pub struct DirtyFileMetadata {
    /// Priority: 0 = high (open file), 1 = normal (workspace file)
    /// Lower numbers are processed first
    pub priority: u8,

    /// When this file was marked dirty
    pub marked_at: Instant,

    /// Reason for being dirty (for debugging/telemetry)
    pub reason: DirtyReason,
}

/// Reason why a file was marked dirty
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirtyReason {
    /// User edited in editor (didChange)
    DidChange,

    /// User saved file (didSave)
    DidSave,

    /// External file system change (file watcher)
    FileWatcher,

    /// File opened for first time (didOpen)
    DidOpen,
}

impl DirtyFileTracker {
    /// Create a new dirty file tracker with default debounce window (100ms)
    pub fn new() -> Self {
        Self::with_debounce(Duration::from_millis(100))
    }

    /// Create a new dirty file tracker with custom debounce window
    ///
    /// # Arguments
    /// * `debounce_window` - How long to wait before flushing dirty files
    ///
    /// # Recommendations
    /// - For typing: 100-200ms (balance between responsiveness and batching)
    /// - For saves: 50-100ms (users expect quick feedback)
    /// - For file watching: 200-500ms (external tools may batch changes)
    pub fn with_debounce(debounce_window: Duration) -> Self {
        Self {
            dirty_files: Arc::new(DashMap::new()),
            debounce_window,
            last_cycle: Arc::new(RwLock::new(Instant::now())),
        }
    }

    /// Mark a file as dirty
    ///
    /// If the file is already dirty, updates its metadata to reflect the most recent change.
    /// This ensures priority and reason are current.
    ///
    /// # Arguments
    /// * `uri` - File URI
    /// * `priority` - 0 = high (open files), 1 = normal (workspace files)
    /// * `reason` - Why the file is dirty
    ///
    /// # Performance
    /// O(1) lock-free insert
    pub fn mark_dirty(&self, uri: Url, priority: u8, reason: DirtyReason) {
        self.dirty_files.insert(
            uri,
            DirtyFileMetadata {
                priority,
                marked_at: Instant::now(),
                reason,
            },
        );
    }

    /// Get all dirty files and clear the tracker
    ///
    /// Returns files sorted by priority (high-priority first), then by marked time (oldest first).
    /// Clears the dirty set after draining.
    ///
    /// # Returns
    /// Vector of (URI, metadata) tuples sorted by priority
    ///
    /// # Performance
    /// O(k log k) where k = number of dirty files (sorting overhead)
    pub fn drain_dirty(&self) -> Vec<(Url, DirtyFileMetadata)> {
        // Collect all dirty files
        let mut files: Vec<_> = self
            .dirty_files
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect();

        // Clear dirty set
        self.dirty_files.clear();

        // Update last cycle timestamp
        *self.last_cycle.write() = Instant::now();

        // Sort by priority (0 = high comes first), then by marked time (oldest first)
        files.sort_by(|(_, meta_a), (_, meta_b)| {
            meta_a
                .priority
                .cmp(&meta_b.priority)
                .then_with(|| meta_a.marked_at.cmp(&meta_b.marked_at))
        });

        files
    }

    /// Check if we should flush based on debounce window
    ///
    /// Returns true if:
    /// 1. There are dirty files, AND
    /// 2. The oldest dirty file has been waiting >= debounce_window
    ///
    /// # Returns
    /// `true` if dirty files should be flushed, `false` otherwise
    ///
    /// # Performance
    /// O(k) where k = number of dirty files (finding minimum timestamp)
    pub fn should_flush(&self) -> bool {
        if self.dirty_files.is_empty() {
            return false;
        }

        // Find oldest dirty file
        let oldest = self
            .dirty_files
            .iter()
            .map(|entry| entry.value().marked_at)
            .min();

        if let Some(oldest_time) = oldest {
            oldest_time.elapsed() >= self.debounce_window
        } else {
            false
        }
    }

    /// Get the number of dirty files currently tracked
    ///
    /// # Returns
    /// Count of dirty files
    ///
    /// # Performance
    /// O(1) - DashMap maintains length atomically
    pub fn len(&self) -> usize {
        self.dirty_files.len()
    }

    /// Check if there are any dirty files
    ///
    /// # Returns
    /// `true` if no dirty files, `false` otherwise
    ///
    /// # Performance
    /// O(1)
    pub fn is_empty(&self) -> bool {
        self.dirty_files.is_empty()
    }

    /// Get the debounce window duration
    ///
    /// # Returns
    /// Debounce window duration
    pub fn debounce_window(&self) -> Duration {
        self.debounce_window
    }

    /// Clear all dirty files without processing
    ///
    /// Useful for testing or when cancelling an indexing cycle.
    ///
    /// # Performance
    /// O(k) where k = number of dirty files
    pub fn clear(&self) {
        self.dirty_files.clear();
        *self.last_cycle.write() = Instant::now();
    }
}

impl Default for DirtyFileTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    #[test]
    fn test_mark_and_drain_single_file() {
        let tracker = DirtyFileTracker::new();
        let uri = Url::parse("file:///test.rho").unwrap();

        // Mark file as dirty
        tracker.mark_dirty(uri.clone(), 0, DirtyReason::DidChange);

        assert_eq!(tracker.len(), 1);
        assert!(!tracker.is_empty());

        // Drain dirty files
        let dirty = tracker.drain_dirty();

        assert_eq!(dirty.len(), 1);
        assert_eq!(dirty[0].0, uri);
        assert_eq!(dirty[0].1.priority, 0);
        assert_eq!(dirty[0].1.reason, DirtyReason::DidChange);

        // Tracker should be empty after drain
        assert_eq!(tracker.len(), 0);
        assert!(tracker.is_empty());
    }

    #[test]
    fn test_mark_multiple_files() {
        let tracker = DirtyFileTracker::new();

        let uri1 = Url::parse("file:///test1.rho").unwrap();
        let uri2 = Url::parse("file:///test2.rho").unwrap();
        let uri3 = Url::parse("file:///test3.rho").unwrap();

        tracker.mark_dirty(uri1.clone(), 0, DirtyReason::DidChange);
        tracker.mark_dirty(uri2.clone(), 1, DirtyReason::FileWatcher);
        tracker.mark_dirty(uri3.clone(), 0, DirtyReason::DidSave);

        assert_eq!(tracker.len(), 3);

        let dirty = tracker.drain_dirty();
        assert_eq!(dirty.len(), 3);

        // Check priority ordering: 0 (high) comes before 1 (normal)
        assert_eq!(dirty[0].1.priority, 0); // uri1 or uri3
        assert_eq!(dirty[1].1.priority, 0); // uri1 or uri3
        assert_eq!(dirty[2].1.priority, 1); // uri2
    }

    #[test]
    fn test_priority_ordering() {
        let tracker = DirtyFileTracker::new();

        // Add files with different priorities
        let high1 = Url::parse("file:///high1.rho").unwrap();
        let high2 = Url::parse("file:///high2.rho").unwrap();
        let normal1 = Url::parse("file:///normal1.rho").unwrap();
        let normal2 = Url::parse("file:///normal2.rho").unwrap();

        tracker.mark_dirty(normal1.clone(), 1, DirtyReason::FileWatcher);
        tracker.mark_dirty(high1.clone(), 0, DirtyReason::DidChange);
        tracker.mark_dirty(normal2.clone(), 1, DirtyReason::FileWatcher);
        tracker.mark_dirty(high2.clone(), 0, DirtyReason::DidSave);

        let dirty = tracker.drain_dirty();

        // All high-priority (0) should come before normal-priority (1)
        assert_eq!(dirty[0].1.priority, 0);
        assert_eq!(dirty[1].1.priority, 0);
        assert_eq!(dirty[2].1.priority, 1);
        assert_eq!(dirty[3].1.priority, 1);
    }

    #[test]
    fn test_debounce_window() {
        let tracker = DirtyFileTracker::with_debounce(Duration::from_millis(50));
        let uri = Url::parse("file:///test.rho").unwrap();

        // Initially should not flush (no dirty files)
        assert!(!tracker.should_flush());

        // Mark file dirty
        tracker.mark_dirty(uri.clone(), 0, DirtyReason::DidChange);

        // Should not flush immediately
        assert!(!tracker.should_flush());

        // Wait for debounce window to expire
        sleep(Duration::from_millis(60));

        // Now should flush
        assert!(tracker.should_flush());
    }

    #[test]
    fn test_update_dirty_file() {
        let tracker = DirtyFileTracker::new();
        let uri = Url::parse("file:///test.rho").unwrap();

        // Mark as low priority file watcher event
        tracker.mark_dirty(uri.clone(), 1, DirtyReason::FileWatcher);

        // Update to high priority didChange
        tracker.mark_dirty(uri.clone(), 0, DirtyReason::DidChange);

        let dirty = tracker.drain_dirty();

        // Should have updated metadata (only 1 entry)
        assert_eq!(dirty.len(), 1);
        assert_eq!(dirty[0].1.priority, 0);
        assert_eq!(dirty[0].1.reason, DirtyReason::DidChange);
    }

    #[test]
    fn test_clear() {
        let tracker = DirtyFileTracker::new();

        for i in 0..10 {
            let uri = Url::parse(&format!("file:///test{}.rho", i)).unwrap();
            tracker.mark_dirty(uri, 0, DirtyReason::DidChange);
        }

        assert_eq!(tracker.len(), 10);

        tracker.clear();

        assert_eq!(tracker.len(), 0);
        assert!(tracker.is_empty());
    }

    #[test]
    fn test_concurrent_marking() {
        use std::thread;

        let tracker = Arc::new(DirtyFileTracker::new());
        let mut handles = vec![];

        // Spawn 10 threads, each marking 10 files
        for thread_id in 0..10 {
            let tracker_clone = Arc::clone(&tracker);
            let handle = thread::spawn(move || {
                for i in 0..10 {
                    let uri =
                        Url::parse(&format!("file:///test_{}_{}.rho", thread_id, i)).unwrap();
                    tracker_clone.mark_dirty(uri, 0, DirtyReason::DidChange);
                }
            });
            handles.push(handle);
        }

        // Wait for all threads
        for handle in handles {
            handle.join().unwrap();
        }

        // Should have 100 dirty files (10 threads × 10 files)
        assert_eq!(tracker.len(), 100);
    }
}
