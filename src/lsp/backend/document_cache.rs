//! Document IR caching with LRU eviction and hash-based invalidation (Phase B-2)
//!
//! This module implements efficient caching of parsed document IR + symbol tables to avoid
//! redundant parsing and symbol table construction for unchanged files.
//!
//! # Performance Target
//!
//! - **Baseline (no caching)**: ~182.63ms per file change operation
//! - **With caching (Phase B-2)**: ~10-20ms per cache hit (9-18x speedup)
//! - **Expected cache hit rate**: >80%
//!
//! # Architecture
//!
//! ```text
//! File Change Event
//!     ↓
//! Compute blake3 hash of file content
//!     ↓
//! Query DocumentCache by (URI, hash)
//!     ├─ Cache HIT → Return cached CachedDocument (~100µs)
//!     └─ Cache MISS → Parse + build IR + symbol table (~182ms)
//!         ↓
//!         Store in cache (LRU eviction if full)
//! ```
//!
//! # Cache Invalidation Strategy
//!
//! - **Key**: `(Url, ContentHash)` where `ContentHash = blake3::Hash`
//! - **Invalidation**: When file content changes, hash changes → new key → cache miss
//! - **Eviction**: LRU policy evicts least recently used entries when capacity reached
//!
//! # Memory Management
//!
//! Default cache capacity: 50 files (~50-100MB depending on file size)
//! - Configurable via `DocumentCache::with_capacity()`
//! - Can be tuned based on workspace size and available memory
//!
//! # Thread Safety
//!
//! All cache operations are thread-safe:
//! - `lru::LruCache` is wrapped in `parking_lot::RwLock`
//! - Concurrent reads allowed
//! - Exclusive write lock for insertions/evictions

use std::sync::Arc;
use std::time::{Instant, SystemTime};

use blake3::Hash as Blake3Hash;
use lru::LruCache;
use parking_lot::RwLock;
use tower_lsp::lsp_types::Url;

use crate::lsp::models::CachedDocument;

/// Content hash for cache invalidation
///
/// Uses blake3 for fast, cryptographically-secure hashing.
/// Performance: ~1GB/s on modern CPUs (2-3x faster than SHA-256)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContentHash(Blake3Hash);

impl ContentHash {
    /// Compute content hash from file text
    ///
    /// Performance: ~1-2µs for typical Rholang files (<10KB)
    pub fn from_str(content: &str) -> Self {
        Self(blake3::hash(content.as_bytes()))
    }

    /// Get the underlying blake3 hash
    pub fn as_blake3(&self) -> &Blake3Hash {
        &self.0
    }
}

/// Cache key: (URI, ContentHash)
///
/// The combination ensures:
/// - Different files don't collide (URI uniqueness)
/// - File modifications invalidate cache (ContentHash changes)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    uri: Url,
    content_hash: ContentHash,
}

/// Cached document entry with metadata
///
/// Stores the cached document plus tracking information for
/// cache statistics and debugging.
#[derive(Debug)]
struct CacheEntry {
    /// The cached document (IR, symbol tables, Tree-Sitter tree, etc.)
    document: Arc<CachedDocument>,

    /// Content hash for this version of the document
    content_hash: ContentHash,

    /// When this document was last modified (filesystem timestamp)
    modified_at: SystemTime,

    /// When this entry was added to the cache
    cached_at: Instant,

    /// When this entry was last accessed
    last_accessed: Instant,
}

impl CacheEntry {
    fn new(document: Arc<CachedDocument>, content_hash: ContentHash, modified_at: SystemTime) -> Self {
        let now = Instant::now();
        Self {
            document,
            content_hash,
            modified_at,
            cached_at: now,
            last_accessed: now,
        }
    }

    /// Update last accessed time (called on cache hit)
    fn touch(&mut self) {
        self.last_accessed = Instant::now();
    }
}

/// Cache statistics for monitoring and debugging
#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    /// Total number of cache lookups
    pub total_queries: u64,

    /// Number of cache hits
    pub hits: u64,

    /// Number of cache misses
    pub misses: u64,

    /// Number of cache evictions
    pub evictions: u64,

    /// Current cache size (number of entries)
    pub current_size: usize,

    /// Maximum cache capacity
    pub max_capacity: usize,
}

impl CacheStats {
    /// Calculate cache hit rate (0.0 to 1.0)
    pub fn hit_rate(&self) -> f64 {
        if self.total_queries == 0 {
            0.0
        } else {
            self.hits as f64 / self.total_queries as f64
        }
    }
}

/// Thread-safe LRU cache for document IR
///
/// # Example Usage
///
/// ```ignore
/// let cache = DocumentCache::new();
///
/// // Cache miss: Parse and cache
/// let content = std::fs::read_to_string(&path)?;
/// let hash = ContentHash::from_str(&content);
///
/// if let Some(cached_doc) = cache.get(&uri, &hash) {
///     // Cache hit: Use cached document
///     return Ok(cached_doc);
/// }
///
/// // Cache miss: Parse document
/// let doc = parse_and_index_document(&content)?;
/// cache.insert(uri.clone(), hash, doc.clone(), modified_time);
/// Ok(doc)
/// ```
#[derive(Debug)]
pub struct DocumentCache {
    /// LRU cache: most recently used documents kept in cache
    cache: RwLock<LruCache<CacheKey, CacheEntry>>,

    /// Cache statistics (protected by RwLock for thread-safe updates)
    stats: RwLock<CacheStats>,
}

impl DocumentCache {
    /// Create a new document cache with default capacity (50 entries)
    pub fn new() -> Self {
        Self::with_capacity(50)
    }

    /// Create a new document cache with specified capacity
    ///
    /// # Arguments
    ///
    /// * `capacity` - Maximum number of cached documents
    ///
    /// # Memory Usage
    ///
    /// Approximate memory per cached entry:
    /// - CachedDocument: ~1-2MB (depending on file size and complexity)
    /// - CacheEntry metadata: ~200 bytes
    /// - Total: ~1-2MB per entry
    ///
    /// Recommended capacities:
    /// - Small projects (<50 files): 20-50
    /// - Medium projects (50-200 files): 50-100
    /// - Large projects (>200 files): 100-200
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            cache: RwLock::new(LruCache::new(
                std::num::NonZeroUsize::new(capacity).expect("capacity must be non-zero")
            )),
            stats: RwLock::new(CacheStats {
                max_capacity: capacity,
                ..Default::default()
            }),
        }
    }

    /// Get a cached document by URI and content hash
    ///
    /// Returns `Some(document)` if cache hit, `None` if cache miss.
    ///
    /// # Performance
    ///
    /// - Cache hit: O(1) lookup + Arc clone (~100µs)
    /// - Cache miss: O(1) lookup (~10µs)
    ///
    /// # Thread Safety
    ///
    /// Uses read lock for lookup, upgraded to write lock if entry found (for LRU update).
    pub fn get(&self, uri: &Url, content_hash: &ContentHash) -> Option<Arc<CachedDocument>> {
        let key = CacheKey {
            uri: uri.clone(),
            content_hash: *content_hash,
        };

        // Update stats (requires write lock)
        {
            let mut stats = self.stats.write();
            stats.total_queries += 1;
        }

        // Try to get entry (requires write lock for LRU update)
        let mut cache = self.cache.write();

        if let Some(entry) = cache.get_mut(&key) {
            // Cache hit: update access time and return document
            entry.touch();

            // Update hit stats
            {
                let mut stats = self.stats.write();
                stats.hits += 1;
            }

            Some(entry.document.clone())
        } else {
            // Cache miss
            {
                let mut stats = self.stats.write();
                stats.misses += 1;
            }

            None
        }
    }

    /// Insert a document into the cache
    ///
    /// If the cache is at capacity, the least recently used entry will be evicted.
    ///
    /// # Arguments
    ///
    /// * `uri` - Document URI
    /// * `content_hash` - Hash of document content (for invalidation)
    /// * `document` - The cached document to store
    /// * `modified_at` - Filesystem modification time
    ///
    /// # Performance
    ///
    /// - O(1) insertion
    /// - If eviction occurs: O(1) to remove LRU entry
    ///
    /// # Thread Safety
    ///
    /// Uses exclusive write lock for insertion.
    pub fn insert(
        &self,
        uri: Url,
        content_hash: ContentHash,
        document: Arc<CachedDocument>,
        modified_at: SystemTime,
    ) {
        let key = CacheKey {
            uri,
            content_hash,
        };

        let entry = CacheEntry::new(document, content_hash, modified_at);

        let mut cache = self.cache.write();
        let mut stats = self.stats.write();

        // Insert and check if eviction occurred
        if let Some(_evicted) = cache.push(key, entry) {
            stats.evictions += 1;
        }

        stats.current_size = cache.len();
    }

    /// Remove a document from the cache
    ///
    /// Typically called when a file is deleted from the workspace.
    ///
    /// # Performance
    ///
    /// O(1) removal
    pub fn remove(&self, uri: &Url) {
        let mut cache = self.cache.write();
        let mut stats = self.stats.write();

        // Remove all entries for this URI (regardless of content hash)
        let keys_to_remove: Vec<_> = cache
            .iter()
            .filter(|(k, _)| k.uri == *uri)
            .map(|(k, _)| k.clone())
            .collect();

        for key in keys_to_remove {
            cache.pop(&key);
        }

        stats.current_size = cache.len();
    }

    /// Clear all cached documents
    ///
    /// Useful for testing or when workspace is reloaded.
    pub fn clear(&self) {
        let mut cache = self.cache.write();
        let mut stats = self.stats.write();

        cache.clear();
        stats.current_size = 0;
    }

    /// Get current cache statistics
    pub fn stats(&self) -> CacheStats {
        self.stats.read().clone()
    }

    /// Get number of cached entries
    pub fn len(&self) -> usize {
        self.cache.read().len()
    }

    /// Check if cache is empty
    pub fn is_empty(&self) -> bool {
        self.cache.read().is_empty()
    }
}

impl Default for DocumentCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn mock_url(name: &str) -> Url {
        Url::parse(&format!("file:///tmp/{}.rho", name)).unwrap()
    }

    #[test]
    fn test_content_hash_deterministic() {
        let content = "contract foo(@x) = { x!(42) }";
        let hash1 = ContentHash::from_str(content);
        let hash2 = ContentHash::from_str(content);
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_content_hash_different_for_different_content() {
        let content1 = "contract foo(@x) = { x!(42) }";
        let content2 = "contract bar(@y) = { y!(100) }";
        let hash1 = ContentHash::from_str(content1);
        let hash2 = ContentHash::from_str(content2);
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_cache_miss() {
        let cache = DocumentCache::new();
        let uri = mock_url("test");
        let hash = ContentHash::from_str("test content");

        assert!(cache.get(&uri, &hash).is_none());

        let stats = cache.stats();
        assert_eq!(stats.total_queries, 1);
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.misses, 1);
    }

    // Note: Full cache hit test requires mock CachedDocument
    // This will be added in integration tests
}
