//! File modification timestamp tracking for incremental indexing (Phase B-1.1)
//!
//! This module provides `FileModificationTracker` which persists file modification timestamps
//! to detect which files have changed since the last indexing cycle. Unlike `DirtyFileTracker`
//! which tracks in-memory dirty state during a session, this tracker persists timestamps across
//! language server restarts.
//!
//! # Architecture
//!
//! - **In-memory cache**: DashMap for fast concurrent lookups (O(1))
//! - **Disk persistence**: Bincode-serialized HashMap at `~/.cache/f1r3fly-io/rholang-language-server/file_timestamps.bin`
//! - **Filesystem queries**: Uses `std::fs::metadata()` to check current modification time
//!
//! # Performance
//!
//! - **Check changed**: O(1) DashMap lookup + O(1) filesystem metadata query
//! - **Mark indexed**: O(1) DashMap insert
//! - **Persist to disk**: O(n) where n = number of tracked files (amortized across indexing cycles)
//! - **Memory overhead**: ~56 bytes per file (Url + SystemTime + DashMap overhead)
//!
//! # Usage
//!
//! ```ignore
//! use rholang_language_server::lsp::backend::file_modification_tracker::FileModificationTracker;
//!
//! let tracker = FileModificationTracker::new();
//!
//! // Check if file changed since last index
//! if tracker.has_changed(&uri).await? {
//!     // Re-index this file
//!     index_file(&uri).await?;
//!
//!     // Mark as indexed
//!     tracker.mark_indexed(&uri).await?;
//! }
//!
//! // Periodically persist to disk (e.g., after workspace indexing)
//! tracker.persist().await?;
//! ```
//!
//! # Disk Format
//!
//! The tracker uses bincode to serialize the timestamp cache to disk:
//! - **Location**: `~/.cache/f1r3fly-io/rholang-language-server/file_timestamps.bin`
//! - **Format**: Bincode-serialized `HashMap<String, SystemTime>`
//! - **Size**: ~40-80 bytes per file (depending on URI length)

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::fs;
use tower_lsp::lsp_types::Url;
use tracing::{debug, warn};

/// Tracks file modification timestamps for incremental indexing
///
/// Persists timestamps to disk across language server restarts to avoid full re-indexing.
#[derive(Clone, Debug)]
pub struct FileModificationTracker {
    /// In-memory cache: URI → last indexed modification time
    /// Uses DashMap for lock-free concurrent access
    timestamps: Arc<DashMap<Url, SystemTime>>,

    /// Path to persistent storage (bincode-serialized HashMap)
    /// Default: ~/.cache/f1r3fly-io/rholang-language-server/file_timestamps.bin
    cache_path: PathBuf,
}

/// Serializable format for disk persistence
#[derive(Debug, Serialize, Deserialize)]
struct TimestampCache {
    /// URI string → modification time
    /// String keys because Url doesn't implement Serialize
    timestamps: HashMap<String, SystemTime>,
}

impl FileModificationTracker {
    /// Create a new file modification tracker with default cache path
    ///
    /// Default cache location: `~/.cache/f1r3fly-io/rholang-language-server/file_timestamps.bin`
    ///
    /// # Errors
    /// Returns `io::Error` if cache directory cannot be created
    pub async fn new() -> io::Result<Self> {
        let cache_dir = if let Some(home) = dirs::home_dir() {
            home.join(".cache").join("f1r3fly-io").join("rholang-language-server")
        } else {
            PathBuf::from("/tmp/f1r3fly-io/rholang-language-server")
        };

        Self::with_cache_dir(cache_dir).await
    }

    /// Create a new file modification tracker with custom cache directory
    ///
    /// # Arguments
    /// * `cache_dir` - Directory to store `file_timestamps.bin`
    ///
    /// # Errors
    /// Returns `io::Error` if cache directory cannot be created
    pub async fn with_cache_dir(cache_dir: PathBuf) -> io::Result<Self> {
        // Create cache directory if it doesn't exist
        fs::create_dir_all(&cache_dir).await?;

        let cache_path = cache_dir.join("file_timestamps.bin");
        let timestamps = Arc::new(DashMap::new());

        let tracker = Self {
            timestamps,
            cache_path,
        };

        // Try to load from disk
        if let Err(e) = tracker.load().await {
            debug!(
                "Failed to load file timestamps from disk (starting fresh): {}",
                e
            );
        }

        Ok(tracker)
    }

    /// Check if a file has changed since it was last indexed
    ///
    /// Compares current filesystem modification time against cached timestamp.
    ///
    /// # Arguments
    /// * `uri` - File URI to check
    ///
    /// # Returns
    /// - `Ok(true)` if file has changed or is not tracked
    /// - `Ok(false)` if file is unchanged since last index
    /// - `Err` if filesystem metadata cannot be read
    ///
    /// # Performance
    /// O(1) DashMap lookup + O(1) filesystem stat
    pub async fn has_changed(&self, uri: &Url) -> io::Result<bool> {
        // Get current filesystem modification time
        let current_mtime = Self::get_modification_time(uri).await?;

        // Check if we have a cached timestamp
        if let Some(cached) = self.timestamps.get(uri) {
            // File changed if current time > cached time
            Ok(current_mtime > *cached)
        } else {
            // Not tracked → treat as changed
            Ok(true)
        }
    }

    /// Mark a file as indexed with current modification time
    ///
    /// Updates the in-memory cache with the file's current modification time.
    /// Call `persist()` periodically to save to disk.
    ///
    /// # Arguments
    /// * `uri` - File URI that was just indexed
    ///
    /// # Errors
    /// Returns `io::Error` if filesystem metadata cannot be read
    ///
    /// # Performance
    /// O(1) DashMap insert + O(1) filesystem stat
    pub async fn mark_indexed(&self, uri: &Url) -> io::Result<()> {
        let mtime = Self::get_modification_time(uri).await?;
        self.timestamps.insert(uri.clone(), mtime);
        Ok(())
    }

    /// Get the cached modification time for a file
    ///
    /// # Arguments
    /// * `uri` - File URI
    ///
    /// # Returns
    /// `Some(SystemTime)` if file is tracked, `None` otherwise
    ///
    /// # Performance
    /// O(1) DashMap lookup
    pub fn get_cached_time(&self, uri: &Url) -> Option<SystemTime> {
        self.timestamps.get(uri).map(|entry| *entry)
    }

    /// Remove a file from tracking
    ///
    /// Used when a file is deleted or should no longer be tracked.
    ///
    /// # Arguments
    /// * `uri` - File URI to remove
    ///
    /// # Performance
    /// O(1) DashMap remove
    pub fn remove(&self, uri: &Url) {
        self.timestamps.remove(uri);
    }

    /// Get the number of tracked files
    ///
    /// # Returns
    /// Count of tracked files
    ///
    /// # Performance
    /// O(1) - DashMap maintains length atomically
    pub fn len(&self) -> usize {
        self.timestamps.len()
    }

    /// Check if any files are tracked
    ///
    /// # Returns
    /// `true` if no files are tracked, `false` otherwise
    ///
    /// # Performance
    /// O(1)
    pub fn is_empty(&self) -> bool {
        self.timestamps.is_empty()
    }

    /// Clear all tracked timestamps
    ///
    /// Removes all timestamps from memory (does not delete disk cache).
    ///
    /// # Performance
    /// O(n) where n = number of tracked files
    pub fn clear(&self) {
        self.timestamps.clear();
    }

    /// Persist timestamps to disk
    ///
    /// Serializes the in-memory cache to disk using bincode.
    /// Should be called:
    /// - After workspace indexing completes
    /// - Periodically (e.g., every 5 minutes)
    /// - On language server shutdown
    ///
    /// # Errors
    /// Returns `io::Error` if serialization or file write fails
    ///
    /// # Performance
    /// O(n) where n = number of tracked files
    pub async fn persist(&self) -> io::Result<()> {
        // Convert DashMap to HashMap with String keys
        let cache = TimestampCache {
            timestamps: self
                .timestamps
                .iter()
                .map(|entry| (entry.key().to_string(), *entry.value()))
                .collect(),
        };

        // Serialize with bincode
        let data = bincode::serialize(&cache).map_err(|e| {
            io::Error::new(
                io::ErrorKind::Other,
                format!("Failed to serialize timestamps: {}", e),
            )
        })?;

        // Write to disk atomically (write to temp file, then rename)
        let temp_path = self.cache_path.with_extension("tmp");
        fs::write(&temp_path, &data).await?;
        fs::rename(&temp_path, &self.cache_path).await?;

        debug!(
            "Persisted {} file timestamps to {:?}",
            cache.timestamps.len(),
            self.cache_path
        );

        Ok(())
    }

    /// Load timestamps from disk
    ///
    /// Deserializes the on-disk cache and populates the in-memory DashMap.
    /// Called automatically during `new()` and `with_cache_dir()`.
    ///
    /// # Errors
    /// Returns `io::Error` if file read or deserialization fails
    ///
    /// # Performance
    /// O(n) where n = number of persisted timestamps
    async fn load(&self) -> io::Result<()> {
        // Read from disk
        let data = fs::read(&self.cache_path).await?;

        // Deserialize
        let cache: TimestampCache = bincode::deserialize(&data).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Failed to deserialize timestamps: {}", e),
            )
        })?;

        // Populate DashMap
        for (uri_str, mtime) in cache.timestamps {
            if let Ok(uri) = Url::parse(&uri_str) {
                self.timestamps.insert(uri, mtime);
            } else {
                warn!("Skipping invalid URI in timestamp cache: {}", uri_str);
            }
        }

        debug!(
            "Loaded {} file timestamps from {:?}",
            self.timestamps.len(),
            self.cache_path
        );

        Ok(())
    }

    /// Get the current filesystem modification time for a file
    ///
    /// # Arguments
    /// * `uri` - File URI
    ///
    /// # Returns
    /// Filesystem modification time
    ///
    /// # Errors
    /// Returns `io::Error` if:
    /// - URI is not a file:// scheme
    /// - File does not exist
    /// - Metadata cannot be read
    ///
    /// # Performance
    /// O(1) filesystem stat
    async fn get_modification_time(uri: &Url) -> io::Result<SystemTime> {
        // Convert URI to filesystem path
        let path = uri.to_file_path().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Invalid file URI: {}", uri),
            )
        })?;

        // Get filesystem metadata
        let metadata = fs::metadata(&path).await?;
        let mtime = metadata.modified()?;

        Ok(mtime)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tempfile::TempDir;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_has_changed_new_file() {
        let temp_dir = TempDir::new().unwrap();
        let tracker = FileModificationTracker::with_cache_dir(temp_dir.path().to_path_buf())
            .await
            .unwrap();

        // Create test file
        let test_file = temp_dir.path().join("test.rho");
        fs::write(&test_file, "contract test = { Nil }").await.unwrap();
        let uri = Url::from_file_path(&test_file).unwrap();

        // New file should be reported as changed
        assert!(tracker.has_changed(&uri).await.unwrap());
    }

    #[tokio::test]
    async fn test_mark_indexed() {
        let temp_dir = TempDir::new().unwrap();
        let tracker = FileModificationTracker::with_cache_dir(temp_dir.path().to_path_buf())
            .await
            .unwrap();

        let test_file = temp_dir.path().join("test.rho");
        fs::write(&test_file, "contract test = { Nil }").await.unwrap();
        let uri = Url::from_file_path(&test_file).unwrap();

        // Mark as indexed
        tracker.mark_indexed(&uri).await.unwrap();

        // Should not be reported as changed (same mtime)
        assert!(!tracker.has_changed(&uri).await.unwrap());
    }

    #[tokio::test]
    async fn test_has_changed_after_modification() {
        let temp_dir = TempDir::new().unwrap();
        let tracker = FileModificationTracker::with_cache_dir(temp_dir.path().to_path_buf())
            .await
            .unwrap();

        let test_file = temp_dir.path().join("test.rho");
        fs::write(&test_file, "contract test = { Nil }").await.unwrap();
        let uri = Url::from_file_path(&test_file).unwrap();

        // Mark as indexed
        tracker.mark_indexed(&uri).await.unwrap();

        // Wait to ensure mtime changes (some filesystems have 1-second granularity)
        sleep(Duration::from_millis(1100)).await;

        // Modify file
        fs::write(&test_file, "contract test = { Nil }  // modified")
            .await
            .unwrap();

        // Should be reported as changed
        assert!(tracker.has_changed(&uri).await.unwrap());
    }

    #[tokio::test]
    async fn test_persist_and_load() {
        let temp_dir = TempDir::new().unwrap();

        // Create test file
        let test_file = temp_dir.path().join("test.rho");
        fs::write(&test_file, "contract test = { Nil }").await.unwrap();
        let uri = Url::from_file_path(&test_file).unwrap();

        // Create tracker and mark file
        {
            let tracker = FileModificationTracker::with_cache_dir(temp_dir.path().to_path_buf())
                .await
                .unwrap();
            tracker.mark_indexed(&uri).await.unwrap();

            assert_eq!(tracker.len(), 1);

            // Persist to disk
            tracker.persist().await.unwrap();
        }

        // Create new tracker (loads from disk)
        {
            let tracker = FileModificationTracker::with_cache_dir(temp_dir.path().to_path_buf())
                .await
                .unwrap();

            // Should have loaded the timestamp
            assert_eq!(tracker.len(), 1);
            assert!(tracker.get_cached_time(&uri).is_some());

            // Should not report as changed (loaded mtime matches filesystem)
            assert!(!tracker.has_changed(&uri).await.unwrap());
        }
    }

    #[tokio::test]
    async fn test_remove() {
        let temp_dir = TempDir::new().unwrap();
        let tracker = FileModificationTracker::with_cache_dir(temp_dir.path().to_path_buf())
            .await
            .unwrap();

        let test_file = temp_dir.path().join("test.rho");
        fs::write(&test_file, "contract test = { Nil }").await.unwrap();
        let uri = Url::from_file_path(&test_file).unwrap();

        tracker.mark_indexed(&uri).await.unwrap();
        assert_eq!(tracker.len(), 1);

        tracker.remove(&uri);
        assert_eq!(tracker.len(), 0);
        assert!(tracker.get_cached_time(&uri).is_none());
    }

    #[tokio::test]
    async fn test_clear() {
        let temp_dir = TempDir::new().unwrap();
        let tracker = FileModificationTracker::with_cache_dir(temp_dir.path().to_path_buf())
            .await
            .unwrap();

        // Create multiple test files
        for i in 0..10 {
            let test_file = temp_dir.path().join(format!("test{}.rho", i));
            fs::write(&test_file, "contract test = { Nil }").await.unwrap();
            let uri = Url::from_file_path(&test_file).unwrap();
            tracker.mark_indexed(&uri).await.unwrap();
        }

        assert_eq!(tracker.len(), 10);

        tracker.clear();
        assert_eq!(tracker.len(), 0);
        assert!(tracker.is_empty());
    }

    #[tokio::test]
    async fn test_concurrent_marking() {
        use std::sync::Arc;

        let temp_dir = TempDir::new().unwrap();
        let tracker = Arc::new(
            FileModificationTracker::with_cache_dir(temp_dir.path().to_path_buf())
                .await
                .unwrap(),
        );

        let mut handles = vec![];

        // Spawn 10 tasks, each marking 10 files
        for task_id in 0..10 {
            let tracker_clone = Arc::clone(&tracker);
            let temp_path = temp_dir.path().to_path_buf();

            let handle = tokio::spawn(async move {
                for i in 0..10 {
                    let test_file = temp_path.join(format!("test_{}_{}.rho", task_id, i));
                    fs::write(&test_file, "contract test = { Nil }")
                        .await
                        .unwrap();
                    let uri = Url::from_file_path(&test_file).unwrap();
                    tracker_clone.mark_indexed(&uri).await.unwrap();
                }
            });
            handles.push(handle);
        }

        // Wait for all tasks
        for handle in handles {
            handle.await.unwrap();
        }

        // Should have 100 tracked files (10 tasks × 10 files)
        assert_eq!(tracker.len(), 100);
    }
}
