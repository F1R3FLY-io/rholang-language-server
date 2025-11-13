//! Space Object Pooling for MORK Serialization
//!
//! This module implements object pooling for MORK `Space` instances to avoid
//! the allocation overhead of creating new `Space` objects for each pattern
//! serialization operation.
//!
//! ## Performance Impact (Phase A-3)
//!
//! Baseline measurements show that `Space::new()` costs ~2.5 µs per call,
//! representing 83% of total pattern serialization time. Object pooling
//! provides a **2.56x speedup** for pattern serialization and **5.9x faster**
//! workspace indexing for 1000 contracts.
//!
//! **Benchmark Results**:
//! - Creating new Space each time: 9.20 µs (for 3 patterns)
//! - Reusing pooled Space: 3.59 µs (for 3 patterns)
//! - **Speedup**: 2.56x
//!
//! See `docs/optimization/ledger/phase-a-3-space-object-pooling.md` for full analysis.
//!
//! ## Usage
//!
//! ```rust
//! use rholang_language_server::ir::space_pool::SpacePool;
//!
//! // Create pool with capacity for 16 Space objects
//! let pool = SpacePool::new(16);
//!
//! // Acquire Space from pool (creates new if pool empty)
//! {
//!     let mut space = pool.acquire();
//!
//!     // Use space for MORK serialization
//!     let mork_bytes = pattern_to_mork_bytes(&pattern, &space)?;
//!
//!     // Space automatically returned to pool when dropped
//! }
//!
//! // Space is now available for reuse
//! let space2 = pool.acquire();
//! ```
//!
//! ## Thread Safety
//!
//! `SpacePool` is thread-safe via `Arc<Mutex<>>` and can be shared across threads.
//! Contention is minimal since MORK serialization operations are fast (~1-3 µs).

use mork::space::Space;
use pathmap::PathMap;
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex};

/// Pool of reusable MORK `Space` objects
///
/// Manages a collection of pre-allocated `Space` objects to avoid allocation
/// overhead during pattern serialization. Uses RAII guards (`PooledSpace`)
/// to automatically return objects to the pool when dropped.
///
/// ## Thread Safety
///
/// `SpacePool` is `Send + Sync` and can be safely shared across threads via
/// `Arc<SpacePool>`. Internal state is protected by a mutex.
///
/// ## Performance Characteristics
///
/// - **Acquire**: O(1) - pops from Vec or creates new Space (~2.5 µs if pool empty)
/// - **Release**: O(1) - pushes to Vec after reset
/// - **Reset cost**: ~100ns (clear PathMap + HashMap)
/// - **Total overhead**: Negligible compared to MORK serialization (~1-3 µs)
///
/// ## Pool Size Guidelines
///
/// - **Small workspaces** (<100 contracts): 8-16 objects
/// - **Medium workspaces** (100-1000 contracts): 16-32 objects
/// - **Large workspaces** (>1000 contracts): 32-64 objects
///
/// Pool size should be tuned based on concurrent serialization operations.
/// For single-threaded indexing, a small pool (8-16) is sufficient.
#[derive(Clone)]
pub struct SpacePool {
    /// Vector of available Space objects
    pool: Arc<Mutex<Vec<Space>>>,

    /// Maximum pool size (soft limit)
    max_size: usize,
}

impl SpacePool {
    /// Create a new SpacePool with the specified maximum size
    ///
    /// # Arguments
    ///
    /// * `max_size` - Maximum number of Space objects to keep in the pool.
    ///   When the pool exceeds this size, extra objects are dropped instead
    ///   of being returned to the pool.
    ///
    /// # Returns
    ///
    /// A new `SpacePool` instance. The pool starts empty and creates Space
    /// objects on-demand as they are acquired.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rholang_language_server::ir::space_pool::SpacePool;
    ///
    /// // Create pool for typical workspace (16 concurrent operations)
    /// let pool = SpacePool::new(16);
    /// ```
    pub fn new(max_size: usize) -> Self {
        SpacePool {
            pool: Arc::new(Mutex::new(Vec::with_capacity(max_size))),
            max_size,
        }
    }

    /// Acquire a Space object from the pool
    ///
    /// If the pool has available objects, returns one immediately (O(1)).
    /// Otherwise, creates a new Space object (~2.5 µs).
    ///
    /// The returned `PooledSpace` automatically returns the object to the pool
    /// when dropped (RAII pattern).
    ///
    /// # Returns
    ///
    /// A `PooledSpace` guard that dereferences to `&Space` and `&mut Space`.
    ///
    /// # Panics
    ///
    /// Panics if the mutex is poisoned (indicates a panic occurred while
    /// holding the lock, which should never happen in normal operation).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rholang_language_server::ir::space_pool::SpacePool;
    ///
    /// let pool = SpacePool::new(16);
    ///
    /// {
    ///     let space = pool.acquire();
    ///     // Use space for serialization
    ///     // Space automatically returned to pool when scope ends
    /// }
    ///
    /// // Space now available for reuse
    /// let space2 = pool.acquire();
    /// ```
    pub fn acquire(&self) -> PooledSpace {
        let mut pool = self.pool.lock().unwrap();

        // Try to reuse existing Space from pool
        let space = pool.pop().unwrap_or_else(|| {
            // Pool empty - create new Space
            Space::new()
        });

        PooledSpace {
            space: Some(space),
            pool: self.pool.clone(),
            max_size: self.max_size,
        }
    }

    /// Get current pool size (number of available objects)
    ///
    /// This is primarily useful for testing and diagnostics.
    ///
    /// # Returns
    ///
    /// Number of Space objects currently available in the pool.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rholang_language_server::ir::space_pool::SpacePool;
    ///
    /// let pool = SpacePool::new(16);
    /// assert_eq!(pool.size(), 0); // Pool starts empty
    ///
    /// {
    ///     let _space = pool.acquire();
    ///     assert_eq!(pool.size(), 0); // Space in use
    /// }
    ///
    /// assert_eq!(pool.size(), 1); // Space returned to pool
    /// ```
    pub fn size(&self) -> usize {
        self.pool.lock().unwrap().len()
    }

    /// Get maximum pool size
    ///
    /// # Returns
    ///
    /// The maximum number of Space objects that will be retained in the pool.
    /// Objects beyond this limit are dropped instead of being returned.
    pub fn max_size(&self) -> usize {
        self.max_size
    }
}

/// RAII guard for pooled Space objects
///
/// Automatically returns the Space to the pool when dropped. The Space is
/// reset (PathMap and HashMap cleared) before being returned to ensure
/// clean state for reuse.
///
/// Implements `Deref` and `DerefMut` for transparent access to the underlying
/// `Space` object.
pub struct PooledSpace {
    /// The Space object (Some when active, None when released)
    space: Option<Space>,

    /// Reference to the pool for return on drop
    pool: Arc<Mutex<Vec<Space>>>,

    /// Maximum pool size (to enforce limit on return)
    max_size: usize,
}

impl Drop for PooledSpace {
    /// Return Space to pool when dropped
    ///
    /// Resets the Space state (clears PathMap and HashMap) and returns it
    /// to the pool if the pool is below max_size. Otherwise, drops the Space.
    fn drop(&mut self) {
        if let Some(mut space) = self.space.take() {
            // Reset Space state for reuse
            space.btm = PathMap::new();
            space.mmaps.clear();

            // Return to pool if below max size
            let mut pool = self.pool.lock().unwrap();
            if pool.len() < self.max_size {
                pool.push(space);
            }
            // Otherwise, Space is dropped (pool at capacity)
        }
    }
}

impl Deref for PooledSpace {
    type Target = Space;

    /// Dereference to &Space for read access
    fn deref(&self) -> &Self::Target {
        self.space.as_ref().expect("PooledSpace already released")
    }
}

impl DerefMut for PooledSpace {
    /// Dereference to &mut Space for write access
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.space.as_mut().expect("PooledSpace already released")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_space_pool_creation() {
        let pool = SpacePool::new(16);
        assert_eq!(pool.size(), 0);
        assert_eq!(pool.max_size(), 16);
    }

    #[test]
    fn test_space_pool_acquire_and_release() {
        let pool = SpacePool::new(16);

        // Initially empty
        assert_eq!(pool.size(), 0);

        // Acquire space (creates new)
        {
            let _space = pool.acquire();
            assert_eq!(pool.size(), 0); // Space in use
        }

        // Space returned to pool after drop
        assert_eq!(pool.size(), 1);
    }

    #[test]
    fn test_space_pool_reuse() {
        let pool = SpacePool::new(16);

        // First acquire creates new Space
        {
            let _space = pool.acquire();
        }
        assert_eq!(pool.size(), 1);

        // Second acquire reuses pooled Space
        {
            let _space = pool.acquire();
            assert_eq!(pool.size(), 0); // Pool now empty
        }
        assert_eq!(pool.size(), 1); // Space returned again
    }

    #[test]
    fn test_space_pool_max_size() {
        let pool = SpacePool::new(2);

        // Acquire and release 3 spaces
        {
            let _s1 = pool.acquire();
            let _s2 = pool.acquire();
            let _s3 = pool.acquire();
        }

        // Pool size should be capped at max_size (2)
        assert_eq!(pool.size(), 2);
    }

    #[test]
    fn test_space_pool_deref() {
        let pool = SpacePool::new(16);
        let mut space = pool.acquire();

        // Test Deref - read access to btm field
        // We can't check len() since PathMap doesn't have that method,
        // but we can verify access works
        let _ = &space.btm;
        let _ = &space.mmaps;

        // Test DerefMut - write access (replace PathMap)
        space.btm = PathMap::new();

        // Verify we can access Space fields through deref (no panic = success)
        assert!(true);
    }

    #[test]
    fn test_space_pool_state_reset() {
        let pool = SpacePool::new(16);

        // Acquire space and verify initial state
        {
            let space = pool.acquire();
            // Verify HashMap is empty after creation
            assert_eq!(space.mmaps.len(), 0);
        }

        // Acquire again and verify state is clean
        {
            let space = pool.acquire();
            // State should be reset (PathMap recreated, mmaps cleared)
            assert_eq!(space.mmaps.len(), 0);
        }
    }

    #[test]
    fn test_space_pool_concurrent_access() {
        use std::sync::Arc;
        use std::thread;

        let pool = Arc::new(SpacePool::new(32));
        let mut handles = vec![];

        // Spawn 10 threads that each acquire and release a Space
        for _ in 0..10 {
            let pool_clone = pool.clone();
            let handle = thread::spawn(move || {
                let _space = pool_clone.acquire();
                // Just acquire and release - Space returned on drop
            });
            handles.push(handle);
        }

        // Wait for all threads to complete
        for handle in handles {
            handle.join().unwrap();
        }

        // All spaces should be returned to pool (up to max_size)
        assert!(pool.size() <= 32);
        assert!(pool.size() > 0); // At least some returned
    }

    #[test]
    fn test_space_pool_clone() {
        let pool1 = SpacePool::new(16);

        // Acquire space from pool1
        {
            let _space = pool1.acquire();
        }
        assert_eq!(pool1.size(), 1);

        // Clone pool (shares underlying storage)
        let pool2 = pool1.clone();
        assert_eq!(pool2.size(), 1); // Same pool

        // Acquire from pool2
        {
            let _space = pool2.acquire();
            assert_eq!(pool1.size(), 0); // pool1 sees the change
        }
        assert_eq!(pool1.size(), 1); // Space returned to shared pool
        assert_eq!(pool2.size(), 1);
    }
}
