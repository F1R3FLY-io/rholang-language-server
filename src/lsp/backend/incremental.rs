//! Incremental workspace re-indexing (Phase B-1.4)
//!
//! This module implements incremental workspace re-indexing to minimize file change overhead.
//! Instead of re-indexing all 100+ workspace files on every change (~50ms), we only re-index:
//! 1. The changed file itself
//! 2. Files that transitively depend on it (via dependency graph)
//!
//! # Performance Target
//!
//! - **Baseline (full re-index)**: ~50ms for 100 files
//! - **Incremental (Phase B-1)**: ~5-10ms for 1-5 changed files + dependents
//! - **Expected speedup**: 5-10x faster
//!
//! # Architecture
//!
//! ```text
//! didChange/didSave
//!     ↓
//! Mark file dirty (FileModificationTracker)
//!     ↓
//! Debounce window (100ms) - batch rapid changes
//!     ↓
//! Query changed files (FileModificationTracker)
//!     ↓
//! Query transitive dependents (DependencyGraph)
//!     ↓
//! Re-index: changed files + dependents ONLY
//!     ↓
//! Update completion dictionaries incrementally
//!     ↓
//! Persist timestamps + dictionaries
//! ```
//!
//! # Usage
//!
//! ```ignore
//! // In did_change handler:
//! backend.mark_file_dirty(uri.clone()).await;
//!
//! // Background task periodically checks:
//! if backend.should_reindex().await {
//!     backend.incremental_reindex().await;
//! }
//! ```

use std::sync::Arc;
use std::collections::HashSet;

use tracing::{debug, error, info, warn};
use tower_lsp::lsp_types::Url;
use dashmap::DashSet;

use super::state::RholangBackend;
use crate::lsp::backend::dirty_tracker::{DirtyFileTracker, DirtyReason};

impl RholangBackend {
    /// Mark a file as dirty for incremental re-indexing
    ///
    /// This should be called from `did_change`, `did_save`, and file watcher handlers.
    ///
    /// # Arguments
    /// * `uri` - File URI that changed
    /// * `priority` - 0 = high (open files), 1 = normal (workspace files)
    /// * `reason` - Why the file is dirty (for debugging/telemetry)
    ///
    /// # Performance
    /// O(1) lock-free insert into DashMap
    pub async fn mark_file_dirty(&self, uri: Url, priority: u8, reason: DirtyReason) {
        self.dirty_tracker.mark_dirty(uri, priority, reason);
    }

    /// Check if we should trigger incremental re-indexing
    ///
    /// Returns true if:
    /// 1. There are dirty files, AND
    /// 2. The oldest dirty file has been waiting >= debounce_window (default: 100ms)
    ///
    /// # Returns
    /// `true` if dirty files should be re-indexed, `false` otherwise
    ///
    /// # Performance
    /// O(k) where k = number of dirty files (finding minimum timestamp)
    pub async fn should_reindex(&self) -> bool {
        self.dirty_tracker.should_flush()
    }

    /// Perform incremental workspace re-indexing
    ///
    /// This is the core Phase B-1.4 algorithm:
    /// 1. Query FileModificationTracker for changed files
    /// 2. Query DependencyGraph for transitive dependents
    /// 3. Re-index changed files + dependents ONLY (not entire workspace)
    /// 4. Update completion dictionaries incrementally
    /// 5. Persist timestamps + dictionaries
    ///
    /// # Expected Performance
    /// - **Baseline (full re-index)**: ~50ms for 100 files
    /// - **Incremental**: ~5-10ms for 1-5 changed files + dependents
    /// - **Speedup**: 5-10x faster
    ///
    /// # Returns
    /// Number of files re-indexed
    pub async fn incremental_reindex(&self) -> usize {
        let start = std::time::Instant::now();

        // Step 1: Get all dirty files (sorted by priority)
        let dirty_files = self.dirty_tracker.drain_dirty();

        if dirty_files.is_empty() {
            return 0;
        }

        debug!(
            "Incremental re-indexing: {} dirty files (Phase B-1.4)",
            dirty_files.len()
        );

        // Step 2: Compute transitive closure of dependents
        let mut files_to_reindex = DashSet::new();

        for (uri, metadata) in &dirty_files {
            // Add the dirty file itself
            files_to_reindex.insert(uri.clone());

            // Query transitive dependents (files that depend on this file)
            let dependents = self.workspace.dependency_graph.get_dependents(uri);

            debug!(
                "File {} has {} transitive dependents (reason: {:?})",
                uri,
                dependents.len(),
                metadata.reason
            );

            // Add all dependents to re-index set
            for dependent_uri_ref in dependents.iter() {
                let dependent_uri: &Url = dependent_uri_ref.key();
                files_to_reindex.insert(dependent_uri.clone());
            }
        }

        let total_files = files_to_reindex.len();

        info!(
            "Incremental re-index: {} dirty files expanded to {} files (including dependents)",
            dirty_files.len(),
            total_files
        );

        // Step 3: Re-index each file
        let mut reindexed_count = 0;
        let mut failed_count = 0;

        for file_uri_ref in files_to_reindex.iter() {
            let file_uri = file_uri_ref.key().clone();

            // Read file content from disk
            match file_uri.to_file_path() {
                Ok(path) => {
                    match tokio::fs::read_to_string(&path).await {
                        Ok(text) => {
                            // Re-index the file
                            match self.index_file(&file_uri, &text, 0, None).await {
                                Ok(cached_doc) => {
                                    // Update completion index incrementally
                                    self.workspace.completion_index.remove_document_symbols(&file_uri);
                                    crate::lsp::features::completion::populate_from_symbol_table_with_tracking(
                                        &self.workspace.completion_index,
                                        &cached_doc.symbol_table,
                                        &file_uri,
                                    );

                                    // Update workspace document cache
                                    self.update_workspace_document(&file_uri, Arc::new(cached_doc)).await;

                                    // Mark as successfully indexed in FileModificationTracker
                                    if let Err(e) = self.workspace.file_modification_tracker.mark_indexed(&file_uri).await {
                                        warn!("Failed to mark {} as indexed: {}", file_uri, e);
                                    }

                                    reindexed_count += 1;
                                }
                                Err(e) => {
                                    error!("Failed to re-index {}: {}", file_uri, e);
                                    failed_count += 1;
                                }
                            }
                        }
                        Err(e) => {
                            warn!("Failed to read {}: {}", file_uri, e);
                            failed_count += 1;
                        }
                    }
                }
                Err(_) => {
                    warn!("Invalid file path for URI: {}", file_uri);
                    failed_count += 1;
                }
            }
        }

        // Step 4: Link symbols (single batched operation)
        self.link_symbols().await;

        // Step 5: Persist file modification timestamps
        if let Err(e) = self.workspace.file_modification_tracker.persist().await {
            error!("Failed to persist file modification timestamps: {}", e);
        }

        // Step 6: Persist completion dictionaries (Phase B-1.3)
        if let Err(e) = self.persist_completion_index().await {
            error!("Failed to persist completion index: {}", e);
        }

        let elapsed = start.elapsed();

        info!(
            "Incremental re-index complete: {}/{} files succeeded, {} failed ({:.2}ms)",
            reindexed_count,
            total_files,
            failed_count,
            elapsed.as_secs_f64() * 1000.0
        );

        reindexed_count
    }

    /// Persist completion index to disk (Phase B-1.3 integration)
    ///
    /// Saves the completion dictionaries to `~/.cache/f1r3fly-io/rholang-language-server/completion_index.bin`
    /// for fast startup on next LSP restart.
    ///
    /// # Performance
    /// - Serialization: ~1-10ms for 100-1000 symbols
    /// - File size: ~10KB per 100 symbols
    ///
    /// # Errors
    /// Returns error if serialization or file I/O fails
    async fn persist_completion_index(&self) -> std::io::Result<()> {
        use std::path::PathBuf;

        // Get cache directory (same pattern as FileModificationTracker)
        let cache_dir = dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("rholang-language-server");

        // Create cache directory if it doesn't exist
        tokio::fs::create_dir_all(&cache_dir).await?;

        let cache_path = cache_dir.join("completion_index.bin");

        // Serialize completion index (Phase B-1.3)
        self.workspace.completion_index.serialize_to_file(&cache_path)?;

        debug!("Persisted completion index to {:?}", cache_path);

        Ok(())
    }

    /// Load completion index from disk on startup (Phase B-1.3 integration)
    ///
    /// Attempts to load cached completion dictionaries from
    /// `~/.cache/f1r3fly-io/rholang-language-server/completion_index.bin`.
    ///
    /// If cache doesn't exist or deserialization fails, returns None and will rebuild from scratch.
    ///
    /// # Performance
    /// - Deserialization: ~1-10ms for 100-1000 symbols
    /// - Speedup: 10-100ms faster than rebuilding from scratch
    ///
    /// # Returns
    /// `Some(index)` if cache loaded successfully, `None` if cache unavailable
    pub async fn load_completion_index() -> std::io::Result<Option<crate::lsp::features::completion::WorkspaceCompletionIndex>> {
        use std::path::PathBuf;

        let cache_dir = dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("rholang-language-server");

        let cache_path = cache_dir.join("completion_index.bin");

        // Deserialize completion index (Phase B-1.3)
        match crate::lsp::features::completion::WorkspaceCompletionIndex::deserialize_from_file(&cache_path)? {
            Some(index) => {
                info!("Loaded completion index from cache: {:?}", cache_path);
                Ok(Some(index))
            }
            None => {
                debug!("No completion index cache found at {:?}", cache_path);
                Ok(None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp::lsp_types::Url;

    // Note: Integration tests will be added in Phase B-1.5
    // These tests require a full RholangBackend instance which is complex to set up in unit tests.
    // For now, we rely on the existing integration test framework.

    #[tokio::test]
    async fn test_mark_file_dirty() {
        // This test would require initializing a full RholangBackend
        // which is too heavyweight for a unit test.
        // See tests/test_incremental_indexing.rs for integration tests.
    }

    #[tokio::test]
    async fn test_incremental_reindex_no_dirty_files() {
        // This test would require initializing a full RholangBackend
        // which is too heavyweight for a unit test.
        // See tests/test_incremental_indexing.rs for integration tests.
    }
}
