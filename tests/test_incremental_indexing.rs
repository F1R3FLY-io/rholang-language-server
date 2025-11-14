//! Integration tests for Phase B-1: Incremental Indexing
//!
//! This test suite validates the incremental re-indexing system implemented in Phase B-1.
//! It covers:
//! - Basic incremental flow (mark dirty → should_reindex → incremental_reindex)
//! - Transitive dependency tracking
//! - Edge cases (circular deps, file deletion, cache corruption)
//! - Performance characteristics
//!
//! ## Test Strategy
//!
//! These tests use unit-style testing of the incremental indexing components rather than
//! full LSP integration tests, because:
//! 1. The incremental system is designed to be testable in isolation
//! 2. Full LSP tests require RNode (heavyweight external dependency)
//! 3. Unit tests provide faster feedback and better isolation
//!
//! ## Running Tests
//!
//! ```bash
//! cargo test --test test_incremental_indexing
//! ```

use rholang_language_server::lsp::backend::dependency_graph::DependencyGraph;
use rholang_language_server::lsp::backend::dirty_tracker::{DirtyFileTracker, DirtyReason};
use rholang_language_server::lsp::backend::file_modification_tracker::FileModificationTracker;
use std::time::Duration;
use tower_lsp::lsp_types::Url;

/// Helper function to create test URIs
fn test_uri(name: &str) -> Url {
    Url::parse(&format!("file:///test/{}.rho", name)).unwrap()
}

// ============================================================================
// Phase B-1.1: File Modification Tracker Tests
// ============================================================================

#[tokio::test]
async fn test_file_modification_tracker_detects_changes() {
    // Create a temporary directory for test cache
    let temp_dir = tempfile::tempdir().unwrap();
    let cache_dir = temp_dir.path().to_path_buf();

    let tracker = FileModificationTracker::with_cache_dir(cache_dir.clone())
        .await
        .unwrap();

    // Create an actual test file
    let test_file = temp_dir.path().join("test_file.rho");
    tokio::fs::write(&test_file, "contract test = { Nil }").await.unwrap();
    let uri = Url::from_file_path(&test_file).unwrap();

    // First check: file should be considered changed (no cached timestamp)
    let has_changed = tracker.has_changed(&uri).await.unwrap();
    assert!(has_changed, "New file should be considered changed");

    // Mark as indexed
    tracker.mark_indexed(&uri).await.unwrap();

    // Immediately check again: should NOT be changed (timestamp matches)
    let has_changed = tracker.has_changed(&uri).await.unwrap();
    assert!(!has_changed, "Freshly indexed file should not be changed");
}

#[tokio::test]
async fn test_file_modification_tracker_persistence() {
    let temp_dir = tempfile::tempdir().unwrap();
    let cache_dir = temp_dir.path().to_path_buf();

    // Create an actual test file
    let test_file = temp_dir.path().join("persistent_file.rho");
    tokio::fs::write(&test_file, "contract test = { Nil }").await.unwrap();
    let uri = Url::from_file_path(&test_file).unwrap();

    // First instance: mark file as indexed
    {
        let tracker = FileModificationTracker::with_cache_dir(cache_dir.clone())
            .await
            .unwrap();
        tracker.mark_indexed(&uri).await.unwrap();
        tracker.persist().await.unwrap();
    }

    // Second instance: should load cached timestamp
    {
        let tracker = FileModificationTracker::with_cache_dir(cache_dir.clone())
            .await
            .unwrap();

        // File should not be considered changed (timestamp loaded from cache)
        let has_changed = tracker.has_changed(&uri).await.unwrap();
        assert!(!has_changed, "Timestamp should persist across tracker instances");
    }
}

// ============================================================================
// Phase B-1.2: Dependency Graph Tests
// ============================================================================

#[test]
fn test_dependency_graph_transitive_dependents() {
    let graph = DependencyGraph::new();

    // Setup dependency chain: a → b → c → d
    let uri_a = test_uri("a");
    let uri_b = test_uri("b");
    let uri_c = test_uri("c");
    let uri_d = test_uri("d");

    graph.add_dependency(uri_b.clone(), uri_a.clone()); // b depends on a
    graph.add_dependency(uri_c.clone(), uri_b.clone()); // c depends on b
    graph.add_dependency(uri_d.clone(), uri_c.clone()); // d depends on c

    // Query transitive dependents of a
    let dependents = graph.get_dependents(&uri_a);

    // Should find all files that transitively depend on a
    assert_eq!(dependents.len(), 3, "Should find 3 transitive dependents");
    assert!(dependents.contains(&uri_b), "b depends on a");
    assert!(dependents.contains(&uri_c), "c transitively depends on a via b");
    assert!(dependents.contains(&uri_d), "d transitively depends on a via c");
}

#[test]
fn test_dependency_graph_diamond_dependencies() {
    let graph = DependencyGraph::new();

    // Setup diamond: a → b → d, a → c → d
    let uri_a = test_uri("a");
    let uri_b = test_uri("b");
    let uri_c = test_uri("c");
    let uri_d = test_uri("d");

    graph.add_dependency(uri_b.clone(), uri_a.clone()); // b depends on a
    graph.add_dependency(uri_c.clone(), uri_a.clone()); // c depends on a
    graph.add_dependency(uri_d.clone(), uri_b.clone()); // d depends on b
    graph.add_dependency(uri_d.clone(), uri_c.clone()); // d depends on c

    // Query transitive dependents of a
    let dependents = graph.get_dependents(&uri_a);

    // Should find b, c, d (with d counted only once despite two paths)
    assert_eq!(dependents.len(), 3, "Diamond should deduplicate d");
    assert!(dependents.contains(&uri_b));
    assert!(dependents.contains(&uri_c));
    assert!(dependents.contains(&uri_d));
}

#[test]
fn test_dependency_graph_circular_dependencies() {
    let graph = DependencyGraph::new();

    // Setup cycle: a → b → c → a
    let uri_a = test_uri("a");
    let uri_b = test_uri("b");
    let uri_c = test_uri("c");

    graph.add_dependency(uri_b.clone(), uri_a.clone()); // b depends on a
    graph.add_dependency(uri_c.clone(), uri_b.clone()); // c depends on b
    graph.add_dependency(uri_a.clone(), uri_c.clone()); // a depends on c (cycle!)

    // Query transitive dependents of a
    let dependents = graph.get_dependents(&uri_a);

    // Should handle cycle gracefully (visited set prevents infinite loop)
    // Note: get_dependents() excludes the queried file itself (see test_self_dependency in dependency_graph.rs)
    assert_eq!(dependents.len(), 2, "Cycle should include other files in loop");
    assert!(!dependents.contains(&uri_a), "Queried file is excluded from dependents");
    assert!(dependents.contains(&uri_b));
    assert!(dependents.contains(&uri_c));
}

// ============================================================================
// Phase B-1.4: Dirty File Tracker Tests
// ============================================================================

#[tokio::test]
async fn test_dirty_tracker_basic_flow() {
    let tracker = DirtyFileTracker::with_debounce(Duration::from_millis(100));
    let uri = test_uri("dirty_file");

    // Mark file as dirty
    tracker.mark_dirty(uri.clone(), 0, DirtyReason::DidChange);

    // Should not be ready immediately (debounce window)
    assert!(!tracker.should_flush(), "Should not flush before debounce window");

    // Wait for debounce window
    tokio::time::sleep(Duration::from_millis(150)).await;

    // Now should be ready
    assert!(tracker.should_flush(), "Should flush after debounce window");

    // Drain dirty files
    let dirty = tracker.drain_dirty();
    assert_eq!(dirty.len(), 1, "Should have 1 dirty file");
    assert!(dirty.iter().any(|(u, _)| u == &uri));

    // After draining, should not be ready
    assert!(!tracker.should_flush(), "Should not flush after draining");
}

#[tokio::test]
async fn test_dirty_tracker_batching() {
    let tracker = DirtyFileTracker::with_debounce(Duration::from_millis(100));

    // Mark multiple files dirty rapidly
    let uri1 = test_uri("file1");
    let uri2 = test_uri("file2");
    let uri3 = test_uri("file3");

    tracker.mark_dirty(uri1.clone(), 0, DirtyReason::DidChange);
    tokio::time::sleep(Duration::from_millis(20)).await;
    tracker.mark_dirty(uri2.clone(), 0, DirtyReason::DidSave);
    tokio::time::sleep(Duration::from_millis(20)).await;
    tracker.mark_dirty(uri3.clone(), 1, DirtyReason::FileWatcher);

    // Should not be ready yet (debounce batches changes)
    assert!(!tracker.should_flush(), "Should batch rapid changes");

    // Wait for debounce window from first file
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Now should be ready with all 3 files batched
    assert!(tracker.should_flush(), "Should be ready after debounce");

    let dirty = tracker.drain_dirty();
    assert_eq!(dirty.len(), 3, "Should batch all 3 files");
}

#[tokio::test]
async fn test_dirty_tracker_priority_ordering() {
    let tracker = DirtyFileTracker::with_debounce(Duration::from_millis(50));

    // Mark files with different priorities
    let high_priority = test_uri("open_file");
    let low_priority = test_uri("workspace_file");

    tracker.mark_dirty(low_priority.clone(), 1, DirtyReason::FileWatcher); // priority 1 (normal)
    tracker.mark_dirty(high_priority.clone(), 0, DirtyReason::DidChange);  // priority 0 (high)

    tokio::time::sleep(Duration::from_millis(100)).await;

    let dirty = tracker.drain_dirty();

    // drain_dirty() returns files sorted by priority (high priority first)
    assert_eq!(dirty.len(), 2);
    assert_eq!(&dirty[0].0, &high_priority, "High priority file should be first");
    assert_eq!(dirty[0].1.priority, 0, "First file should have priority 0");
    assert_eq!(&dirty[1].0, &low_priority, "Low priority file should be second");
    assert_eq!(dirty[1].1.priority, 1, "Second file should have priority 1");
}

// ============================================================================
// Integration Tests: Full Incremental Flow
// ============================================================================

#[tokio::test]
async fn test_incremental_flow_no_dependents() {
    // Test the full incremental flow when a changed file has no dependents
    let graph = DependencyGraph::new();
    let tracker = DirtyFileTracker::with_debounce(Duration::from_millis(50));

    let uri = test_uri("standalone_file");

    // Mark file as dirty
    tracker.mark_dirty(uri.clone(), 0, DirtyReason::DidChange);

    // Wait for debounce
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(tracker.should_flush());

    // Drain dirty files
    let dirty = tracker.drain_dirty();
    assert_eq!(dirty.len(), 1);

    // Compute files to re-index (changed file + dependents)
    let mut files_to_reindex: std::collections::HashSet<Url> = std::collections::HashSet::new();
    for (uri, _) in &dirty {
        files_to_reindex.insert(uri.clone());

        let dependents = graph.get_dependents(uri);
        for dep_ref in dependents.iter() {
            files_to_reindex.insert(dep_ref.key().clone());
        }
    }

    // Should only re-index the changed file
    assert_eq!(files_to_reindex.len(), 1, "No dependents means only 1 file to re-index");
    assert!(files_to_reindex.contains(&uri));
}

#[tokio::test]
async fn test_incremental_flow_with_dependents() {
    // Test the full incremental flow when a changed file has dependents
    let graph = DependencyGraph::new();
    let tracker = DirtyFileTracker::with_debounce(Duration::from_millis(50));

    // Setup: base → dependent1, base → dependent2
    let base = test_uri("base");
    let dep1 = test_uri("dependent1");
    let dep2 = test_uri("dependent2");

    graph.add_dependency(dep1.clone(), base.clone());
    graph.add_dependency(dep2.clone(), base.clone());

    // Mark base file as dirty
    tracker.mark_dirty(base.clone(), 0, DirtyReason::DidChange);

    // Wait for debounce
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(tracker.should_flush());

    // Drain and compute re-index set
    let dirty = tracker.drain_dirty();
    let mut files_to_reindex: std::collections::HashSet<Url> = std::collections::HashSet::new();

    for (uri, _) in &dirty {
        files_to_reindex.insert(uri.clone());

        let dependents = graph.get_dependents(uri);
        for dep_ref in dependents.iter() {
            files_to_reindex.insert(dep_ref.key().clone());
        }
    }

    // Should re-index base + both dependents
    assert_eq!(files_to_reindex.len(), 3, "Should re-index changed file + dependents");
    assert!(files_to_reindex.contains(&base));
    assert!(files_to_reindex.contains(&dep1));
    assert!(files_to_reindex.contains(&dep2));
}

#[tokio::test]
async fn test_incremental_flow_multiple_changed_files() {
    // Test when multiple files change (should batch and expand all dependencies)
    let graph = DependencyGraph::new();
    let tracker = DirtyFileTracker::with_debounce(Duration::from_millis(50));

    // Setup: base1 → dep1, base2 → dep2
    let base1 = test_uri("base1");
    let base2 = test_uri("base2");
    let dep1 = test_uri("dep1");
    let dep2 = test_uri("dep2");

    graph.add_dependency(dep1.clone(), base1.clone());
    graph.add_dependency(dep2.clone(), base2.clone());

    // Mark both base files as dirty
    tracker.mark_dirty(base1.clone(), 0, DirtyReason::DidChange);
    tracker.mark_dirty(base2.clone(), 0, DirtyReason::DidChange);

    // Wait for debounce
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Drain and compute re-index set
    let dirty = tracker.drain_dirty();
    let mut files_to_reindex: std::collections::HashSet<Url> = std::collections::HashSet::new();

    for (uri, _) in &dirty {
        files_to_reindex.insert(uri.clone());

        let dependents = graph.get_dependents(uri);
        for dep_ref in dependents.iter() {
            files_to_reindex.insert(dep_ref.key().clone());
        }
    }

    // Should re-index both bases + both dependents
    assert_eq!(files_to_reindex.len(), 4, "Should handle multiple changed files");
    assert!(files_to_reindex.contains(&base1));
    assert!(files_to_reindex.contains(&base2));
    assert!(files_to_reindex.contains(&dep1));
    assert!(files_to_reindex.contains(&dep2));
}

// ============================================================================
// Edge Case Tests
// ============================================================================

#[test]
fn test_dependency_graph_remove_file() {
    let graph = DependencyGraph::new();

    // Setup: a → b → c
    let uri_a = test_uri("a");
    let uri_b = test_uri("b");
    let uri_c = test_uri("c");

    graph.add_dependency(uri_b.clone(), uri_a.clone());
    graph.add_dependency(uri_c.clone(), uri_b.clone());

    // Remove b from graph
    graph.remove_file(&uri_b);

    // Query dependents of a should no longer include b or c
    let dependents = graph.get_dependents(&uri_a);
    assert_eq!(dependents.len(), 0, "Removing b should break the chain");
}

#[tokio::test]
async fn test_dirty_tracker_same_file_multiple_times() {
    // Marking the same file dirty multiple times should only track it once
    let tracker = DirtyFileTracker::with_debounce(Duration::from_millis(50));
    let uri = test_uri("repeated_file");

    tracker.mark_dirty(uri.clone(), 0, DirtyReason::DidChange);
    tracker.mark_dirty(uri.clone(), 0, DirtyReason::DidSave);
    tracker.mark_dirty(uri.clone(), 1, DirtyReason::FileWatcher);

    tokio::time::sleep(Duration::from_millis(100)).await;

    let dirty = tracker.drain_dirty();
    assert_eq!(dirty.len(), 1, "Same file marked multiple times should only appear once");

    // Most recent reason should be recorded
    assert_eq!(&dirty[0].0, &uri);
    assert!(matches!(dirty[0].1.reason, DirtyReason::FileWatcher), "Should use most recent reason");
}

// ============================================================================
// Performance Characteristic Tests
// ============================================================================

#[test]
fn test_dependency_graph_scalability() {
    // Test that dependency graph can handle 100+ files efficiently
    let graph = DependencyGraph::new();

    // Create a fan-out: base → 100 dependents
    let base = test_uri("base");
    let mut dependents = Vec::new();

    for i in 0..100 {
        let dep = test_uri(&format!("dep_{}", i));
        graph.add_dependency(dep.clone(), base.clone());
        dependents.push(dep);
    }

    // Query should complete quickly
    let start = std::time::Instant::now();
    let result = graph.get_dependents(&base);
    let elapsed = start.elapsed();

    assert_eq!(result.len(), 100, "Should find all 100 dependents");
    assert!(elapsed.as_micros() < 1000, "Query should complete in < 1ms (was {:?})", elapsed);
}

#[tokio::test]
async fn test_dirty_tracker_scalability() {
    // Test that dirty tracker can handle 1000 files efficiently
    let tracker = DirtyFileTracker::with_debounce(Duration::from_millis(50));

    // Mark 1000 files dirty
    let start = std::time::Instant::now();
    for i in 0..1000 {
        let uri = test_uri(&format!("file_{}", i));
        tracker.mark_dirty(uri, (i % 2) as u8, DirtyReason::DidChange); // Mix priorities
    }
    let elapsed = start.elapsed();

    assert!(elapsed.as_millis() < 100, "Marking 1000 files should be < 100ms (was {:?})", elapsed);

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Drain should also be fast
    let start = std::time::Instant::now();
    let dirty = tracker.drain_dirty();
    let elapsed = start.elapsed();

    assert_eq!(dirty.len(), 1000);
    assert!(elapsed.as_millis() < 50, "Draining 1000 files should be < 50ms (was {:?})", elapsed);
}

#[cfg(test)]
mod completion_index_serialization {
    use super::*;
    use rholang_language_server::lsp::features::completion::dictionary::{
        SymbolMetadata, WorkspaceCompletionIndex,
    };
    use std::path::PathBuf;
    use tower_lsp::lsp_types::CompletionItemKind;

    #[test]
    fn test_completion_index_serialize_deserialize() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_path = temp_dir.path().join("completion_test.bin");

        // Create index with some symbols
        let index = WorkspaceCompletionIndex::new();

        index.insert("myContract".to_string(), SymbolMetadata {
            name: "myContract".to_string(),
            kind: CompletionItemKind::FUNCTION,
            documentation: Some("Test contract".to_string()),
            signature: Some("contract myContract(@x) = {...}".to_string()),
            reference_count: 5,
        });

        index.insert("myVariable".to_string(), SymbolMetadata {
            name: "myVariable".to_string(),
            kind: CompletionItemKind::VARIABLE,
            documentation: None,
            signature: None,
            reference_count: 2,
        });

        // Serialize
        index.serialize_to_file(&cache_path).unwrap();

        // Deserialize
        let loaded = WorkspaceCompletionIndex::deserialize_from_file(&cache_path)
            .unwrap()
            .expect("Should load cached index");

        // Verify symbols are present
        assert!(loaded.contains("myContract"), "Should contain myContract");
        assert!(loaded.contains("myVariable"), "Should contain myVariable");

        let metadata = loaded.get_metadata("myContract").unwrap();
        assert_eq!(metadata.name, "myContract");
        assert_eq!(metadata.kind, CompletionItemKind::FUNCTION);
        assert_eq!(metadata.reference_count, 5);
    }

    #[test]
    fn test_completion_index_deserialize_missing_file() {
        let cache_path = PathBuf::from("/nonexistent/path/completion.bin");

        // Should return None when file doesn't exist
        let result = WorkspaceCompletionIndex::deserialize_from_file(&cache_path).unwrap();
        assert!(result.is_none(), "Should return None for missing file");
    }
}
