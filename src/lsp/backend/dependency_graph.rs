//! Dependency graph for incremental workspace indexing (Phase B-1.2)
//!
//! This module provides `DependencyGraph` which tracks cross-file dependencies for
//! incremental re-indexing. When a file changes, only that file and its transitive
//! dependents need to be re-indexed, not the entire workspace.
//!
//! # Architecture
//!
//! - **Forward edges**: `file A → files that A imports/references`
//! - **Reverse edges**: `file B → files that depend on B` (for invalidation)
//! - **Transitive closure**: BFS to find all transitive dependents
//!
//! # Performance
//!
//! - **Add dependency**: O(1) DashMap insert (both forward + reverse)
//! - **Get dependents**: O(k) where k = number of transitive dependents (BFS)
//! - **Remove file**: O(d) where d = number of direct dependencies
//! - **Memory overhead**: ~96 bytes per dependency edge + DashMap overhead
//!
//! # Dependency Types Tracked
//!
//! In Rholang, files can depend on each other through:
//! 1. **Contract calls**: File A calls a contract defined in file B
//! 2. **Symbol references**: File A references a variable/constant from file B
//! 3. **Imports**: File A imports definitions from file B (future feature)
//!
//! # Usage
//!
//! ```ignore
//! use rholang_language_server::lsp::backend::dependency_graph::DependencyGraph;
//!
//! let graph = DependencyGraph::new();
//!
//! // Build dependency edges during indexing
//! for file in workspace_files {
//!     let ir = parse_file(&file)?;
//!     let dependencies = extract_dependencies(&ir);
//!
//!     for dep in dependencies {
//!         graph.add_dependency(file.clone(), dep);
//!     }
//! }
//!
//! // When file changes, get all files that need re-indexing
//! let changed_file = Url::parse("file:///contract.rho")?;
//! let to_reindex = graph.get_dependents(&changed_file);
//!
//! // Re-index changed file + all dependents
//! for file in to_reindex {
//!     reindex_file(&file)?;
//! }
//! ```

use dashmap::{DashMap, DashSet};
use std::collections::VecDeque;
use std::sync::Arc;
use tower_lsp::lsp_types::Url;
use tracing::debug;

/// Tracks cross-file dependencies for incremental indexing
///
/// Maintains bidirectional dependency graph to efficiently find transitive dependents
/// when a file changes. Uses DashMap for lock-free concurrent access.
#[derive(Clone, Debug)]
pub struct DependencyGraph {
    /// Forward edges: file → set of files it depends on
    /// Example: contract.rho → [utils.rho, types.rho]
    forward: Arc<DashMap<Url, Arc<DashSet<Url>>>>,

    /// Reverse edges: file → set of files that depend on it
    /// Example: utils.rho → [contract.rho, main.rho]
    /// Used for invalidation: when utils.rho changes, re-index all reverse dependents
    reverse: Arc<DashMap<Url, Arc<DashSet<Url>>>>,
}

impl DependencyGraph {
    /// Create a new empty dependency graph
    pub fn new() -> Self {
        Self {
            forward: Arc::new(DashMap::new()),
            reverse: Arc::new(DashMap::new()),
        }
    }

    /// Add a dependency edge: `dependent → dependency`
    ///
    /// Records that `dependent` file depends on `dependency` file. Updates both
    /// forward and reverse edges for efficient bidirectional traversal.
    ///
    /// # Arguments
    /// * `dependent` - File that depends on another
    /// * `dependency` - File being depended upon
    ///
    /// # Examples
    /// ```ignore
    /// // contract.rho calls a function defined in utils.rho
    /// graph.add_dependency(
    ///     Url::parse("file:///contract.rho")?,
    ///     Url::parse("file:///utils.rho")?
    /// );
    /// ```
    ///
    /// # Performance
    /// O(1) - two DashSet inserts
    pub fn add_dependency(&self, dependent: Url, dependency: Url) {
        // Forward edge: dependent → dependency
        self.forward
            .entry(dependent.clone())
            .or_insert_with(|| Arc::new(DashSet::new()))
            .insert(dependency.clone());

        // Reverse edge: dependency → dependent
        self.reverse
            .entry(dependency.clone())
            .or_insert_with(|| Arc::new(DashSet::new()))
            .insert(dependent.clone());

        debug!(
            "Added dependency: {} → {}",
            dependent.as_str(),
            dependency.as_str()
        );
    }

    /// Get all files that depend on the given file (transitively)
    ///
    /// Uses BFS to find all transitive dependents via reverse edges. Returns the
    /// complete set of files that must be re-indexed when `file` changes.
    ///
    /// # Arguments
    /// * `file` - File to find dependents for
    ///
    /// # Returns
    /// Set of all files (URIs) that transitively depend on `file`
    ///
    /// # Examples
    /// ```ignore
    /// // utils.rho changed
    /// let dependents = graph.get_dependents(&utils_uri);
    ///
    /// // Re-index utils.rho + all dependents
    /// for file in dependents.iter() {
    ///     reindex_file(file)?;
    /// }
    /// ```
    ///
    /// # Performance
    /// O(k) where k = number of transitive dependents (BFS traversal)
    pub fn get_dependents(&self, file: &Url) -> DashSet<Url> {
        let mut dependents = DashSet::new();
        let mut visited = DashSet::new();
        let mut queue = VecDeque::new();

        // Mark the file itself as visited so it won't be included in results
        visited.insert(file.clone());
        queue.push_back(file.clone());

        while let Some(current) = queue.pop_front() {
            // Get direct dependents of current file
            if let Some(direct_dependents) = self.reverse.get(&current) {
                for dep in direct_dependents.iter() {
                    let dep_url = dep.key().clone();

                    // If not already visited, add to result and queue for transitive exploration
                    if visited.insert(dep_url.clone()) {
                        dependents.insert(dep_url.clone());
                        queue.push_back(dep_url);
                    }
                }
            }
        }

        debug!(
            "Found {} transitive dependents for {}",
            dependents.len(),
            file.as_str()
        );

        dependents
    }

    /// Get direct dependencies of a file (non-transitive)
    ///
    /// Returns only the immediate dependencies, not transitive closure.
    ///
    /// # Arguments
    /// * `file` - File to get dependencies for
    ///
    /// # Returns
    /// Set of files that `file` directly depends on, or empty set if no dependencies
    ///
    /// # Performance
    /// O(1) DashMap lookup
    pub fn get_dependencies(&self, file: &Url) -> DashSet<Url> {
        if let Some(deps) = self.forward.get(file) {
            // Clone the DashSet to return owned value
            let result = DashSet::new();
            for dep in deps.iter() {
                result.insert(dep.key().clone());
            }
            result
        } else {
            DashSet::new()
        }
    }

    /// Remove a file from the dependency graph
    ///
    /// Removes all edges involving this file (both forward and reverse). Used when
    /// a file is deleted or should no longer be tracked.
    ///
    /// # Arguments
    /// * `file` - File to remove from graph
    ///
    /// # Performance
    /// O(d) where d = number of direct dependencies
    pub fn remove_file(&self, file: &Url) {
        // Remove forward edges (file → deps)
        if let Some((_, deps)) = self.forward.remove(file) {
            // Remove reverse edges (dep → file)
            for dep in deps.iter() {
                if let Some(reverse_deps) = self.reverse.get(dep.key()) {
                    reverse_deps.remove(file);
                    // Don't clean up empty sets - files should remain in graph
                }
            }
        }

        // Remove reverse edges (dependents → file)
        if let Some((_, dependents)) = self.reverse.remove(file) {
            // Remove forward edges (dependent → file)
            for dependent in dependents.iter() {
                if let Some(forward_deps) = self.forward.get(dependent.key()) {
                    forward_deps.remove(file);
                    // Don't clean up empty sets - files should remain in graph
                }
            }
        }

        debug!("Removed file from dependency graph: {}", file.as_str());
    }

    /// Check if a file has any dependencies
    ///
    /// # Arguments
    /// * `file` - File to check
    ///
    /// # Returns
    /// `true` if file has at least one dependency, `false` otherwise
    ///
    /// # Performance
    /// O(1)
    pub fn has_dependencies(&self, file: &Url) -> bool {
        self.forward
            .get(file)
            .map(|deps| !deps.is_empty())
            .unwrap_or(false)
    }

    /// Check if a file has any dependents
    ///
    /// # Arguments
    /// * `file` - File to check
    ///
    /// # Returns
    /// `true` if at least one file depends on this file, `false` otherwise
    ///
    /// # Performance
    /// O(1)
    pub fn has_dependents(&self, file: &Url) -> bool {
        self.reverse
            .get(file)
            .map(|deps| !deps.is_empty())
            .unwrap_or(false)
    }

    /// Get the total number of files in the graph
    ///
    /// Counts all files that either have dependencies or are depended upon.
    ///
    /// # Returns
    /// Number of unique files in the graph
    ///
    /// # Performance
    /// O(1) - DashMap maintains length atomically
    pub fn file_count(&self) -> usize {
        // Use a set to avoid double-counting files that appear in both maps
        let mut files = DashSet::new();
        for entry in self.forward.iter() {
            files.insert(entry.key().clone());
        }
        for entry in self.reverse.iter() {
            files.insert(entry.key().clone());
        }
        files.len()
    }

    /// Get the total number of dependency edges in the graph
    ///
    /// # Returns
    /// Total number of dependency relationships
    ///
    /// # Performance
    /// O(n) where n = number of files with dependencies
    pub fn edge_count(&self) -> usize {
        self.forward
            .iter()
            .map(|entry| entry.value().len())
            .sum()
    }

    /// Clear all dependencies from the graph
    ///
    /// Removes all edges. Useful for resetting during full workspace re-indexing.
    ///
    /// # Performance
    /// O(n) where n = total number of files
    pub fn clear(&self) {
        self.forward.clear();
        self.reverse.clear();
        debug!("Cleared dependency graph");
    }

    /// Check if the graph is empty
    ///
    /// # Returns
    /// `true` if no dependencies exist, `false` otherwise
    ///
    /// # Performance
    /// O(1)
    pub fn is_empty(&self) -> bool {
        self.forward.is_empty() && self.reverse.is_empty()
    }
}

impl Default for DependencyGraph {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_single_dependency() {
        let graph = DependencyGraph::new();
        let file_a = Url::parse("file:///a.rho").unwrap();
        let file_b = Url::parse("file:///b.rho").unwrap();

        graph.add_dependency(file_a.clone(), file_b.clone());

        // Check forward edge
        assert!(graph.has_dependencies(&file_a));
        assert!(!graph.has_dependencies(&file_b));

        // Check reverse edge
        assert!(graph.has_dependents(&file_b));
        assert!(!graph.has_dependents(&file_a));
    }

    #[test]
    fn test_get_direct_dependencies() {
        let graph = DependencyGraph::new();
        let main = Url::parse("file:///main.rho").unwrap();
        let utils = Url::parse("file:///utils.rho").unwrap();
        let types = Url::parse("file:///types.rho").unwrap();

        graph.add_dependency(main.clone(), utils.clone());
        graph.add_dependency(main.clone(), types.clone());

        let deps = graph.get_dependencies(&main);
        assert_eq!(deps.len(), 2);
        assert!(deps.contains(&utils));
        assert!(deps.contains(&types));
    }

    #[test]
    fn test_get_transitive_dependents() {
        let graph = DependencyGraph::new();

        // Build dependency chain: a → b → c → d
        let a = Url::parse("file:///a.rho").unwrap();
        let b = Url::parse("file:///b.rho").unwrap();
        let c = Url::parse("file:///c.rho").unwrap();
        let d = Url::parse("file:///d.rho").unwrap();

        graph.add_dependency(b.clone(), a.clone()); // b depends on a
        graph.add_dependency(c.clone(), b.clone()); // c depends on b
        graph.add_dependency(d.clone(), c.clone()); // d depends on c

        // If 'a' changes, all of {b, c, d} must be re-indexed
        let dependents = graph.get_dependents(&a);
        assert_eq!(dependents.len(), 3);
        assert!(dependents.contains(&b));
        assert!(dependents.contains(&c));
        assert!(dependents.contains(&d));
    }

    #[test]
    fn test_diamond_dependency() {
        let graph = DependencyGraph::new();

        // Diamond pattern:
        //     a
        //    / \
        //   b   c
        //    \ /
        //     d
        let a = Url::parse("file:///a.rho").unwrap();
        let b = Url::parse("file:///b.rho").unwrap();
        let c = Url::parse("file:///c.rho").unwrap();
        let d = Url::parse("file:///d.rho").unwrap();

        graph.add_dependency(b.clone(), a.clone());
        graph.add_dependency(c.clone(), a.clone());
        graph.add_dependency(d.clone(), b.clone());
        graph.add_dependency(d.clone(), c.clone());

        // If 'a' changes, {b, c, d} must be re-indexed
        let dependents = graph.get_dependents(&a);
        assert_eq!(dependents.len(), 3);
        assert!(dependents.contains(&b));
        assert!(dependents.contains(&c));
        assert!(dependents.contains(&d));
    }

    #[test]
    fn test_remove_file() {
        let graph = DependencyGraph::new();
        let a = Url::parse("file:///a.rho").unwrap();
        let b = Url::parse("file:///b.rho").unwrap();
        let c = Url::parse("file:///c.rho").unwrap();

        graph.add_dependency(b.clone(), a.clone());
        graph.add_dependency(c.clone(), b.clone());

        assert_eq!(graph.file_count(), 3);
        assert_eq!(graph.edge_count(), 2);

        // Remove middle file
        graph.remove_file(&b);

        assert_eq!(graph.file_count(), 2);
        assert_eq!(graph.edge_count(), 0);
        assert!(!graph.has_dependencies(&b));
        assert!(!graph.has_dependents(&b));
    }

    #[test]
    fn test_clear() {
        let graph = DependencyGraph::new();
        for i in 0..10 {
            let file = Url::parse(&format!("file:///file{}.rho", i)).unwrap();
            let dep = Url::parse(&format!("file:///dep{}.rho", i)).unwrap();
            graph.add_dependency(file, dep);
        }

        assert_eq!(graph.file_count(), 20);
        assert_eq!(graph.edge_count(), 10);

        graph.clear();

        assert_eq!(graph.file_count(), 0);
        assert_eq!(graph.edge_count(), 0);
        assert!(graph.is_empty());
    }

    #[test]
    fn test_file_and_edge_counts() {
        let graph = DependencyGraph::new();
        let a = Url::parse("file:///a.rho").unwrap();
        let b = Url::parse("file:///b.rho").unwrap();
        let c = Url::parse("file:///c.rho").unwrap();

        // a → b, a → c (2 edges, 3 files)
        graph.add_dependency(a.clone(), b.clone());
        graph.add_dependency(a.clone(), c.clone());

        assert_eq!(graph.file_count(), 3);
        assert_eq!(graph.edge_count(), 2);
    }

    #[test]
    fn test_no_dependents_for_leaf() {
        let graph = DependencyGraph::new();
        let a = Url::parse("file:///a.rho").unwrap();
        let b = Url::parse("file:///b.rho").unwrap();

        graph.add_dependency(b.clone(), a.clone());

        // 'b' is a leaf (no one depends on it)
        let dependents = graph.get_dependents(&b);
        assert_eq!(dependents.len(), 0);
    }

    #[test]
    fn test_concurrent_access() {
        use std::sync::Arc;
        use std::thread;

        let graph = Arc::new(DependencyGraph::new());
        let mut handles = vec![];

        // Spawn 10 threads, each adding 10 dependencies
        for thread_id in 0..10 {
            let graph_clone = Arc::clone(&graph);
            let handle = thread::spawn(move || {
                for i in 0..10 {
                    let dependent =
                        Url::parse(&format!("file:///thread_{}_file_{}.rho", thread_id, i))
                            .unwrap();
                    let dependency =
                        Url::parse(&format!("file:///thread_{}_dep_{}.rho", thread_id, i)).unwrap();
                    graph_clone.add_dependency(dependent, dependency);
                }
            });
            handles.push(handle);
        }

        // Wait for all threads
        for handle in handles {
            handle.join().unwrap();
        }

        // Should have 100 edges (10 threads × 10 dependencies)
        assert_eq!(graph.edge_count(), 100);
    }

    #[test]
    fn test_self_dependency() {
        let graph = DependencyGraph::new();
        let a = Url::parse("file:///a.rho").unwrap();

        // File depends on itself (edge case)
        graph.add_dependency(a.clone(), a.clone());

        assert!(graph.has_dependencies(&a));
        assert!(graph.has_dependents(&a));

        let deps = graph.get_dependencies(&a);
        assert_eq!(deps.len(), 1);
        assert!(deps.contains(&a));

        // Dependents should not include self in transitive closure
        let dependents = graph.get_dependents(&a);
        // The BFS will visit 'a' as its own dependent, forming a cycle
        // Our implementation will detect the cycle and stop
        assert_eq!(dependents.len(), 0); // 'a' is already in the visited set
    }

    #[test]
    fn test_cyclic_dependencies() {
        let graph = DependencyGraph::new();
        let a = Url::parse("file:///a.rho").unwrap();
        let b = Url::parse("file:///b.rho").unwrap();
        let c = Url::parse("file:///c.rho").unwrap();

        // Create cycle: a → b → c → a
        graph.add_dependency(b.clone(), a.clone());
        graph.add_dependency(c.clone(), b.clone());
        graph.add_dependency(a.clone(), c.clone());

        // Get dependents should handle cycle gracefully (visited set prevents infinite loop)
        let dependents = graph.get_dependents(&a);

        // All three files depend on each other transitively
        assert!(!dependents.contains(&a));
        assert!(dependents.contains(&b));
        assert!(dependents.contains(&c));
    }
}
