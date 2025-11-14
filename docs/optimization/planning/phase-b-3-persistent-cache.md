# Phase B-3: Persistent Document IR Cache

**Date**: 2025-11-13
**Status**: Planning
**Objective**: Serialize document cache to disk for faster LSP server startup

## Problem Statement

### Current Behavior (Phase B-2)

**Cold Start Scenario** (LSP server restart):
1. Server starts with empty cache
2. Workspace indexing begins (parse all `.rho` files)
3. **Time**: 182.63ms per file × N files
4. **Example**: 100 files = 18.2 seconds to full index

**Pain Point**: Every server restart requires full workspace re-indexing

**Impact**:
- Slow startup experience (especially for large projects)
- Wasted CPU cycles (re-parsing unchanged files)
- Poor developer experience (waiting for LSP features)

### Desired Behavior (Phase B-3)

**Warm Start Scenario** (LSP server restart with persistent cache):
1. Server starts, loads cache from disk
2. **Time**: ~100-500ms (deserialize cache)
3. Validate cached entries against file modifications
4. Only re-index changed files
5. **Result**: Near-instant LSP readiness

**Expected Improvement**: **18-36x faster startup** for 100-file projects

## Goals

1. **Fast Startup**: Reduce cold start time from 18s → ~0.5s (100 files)
2. **Correctness**: Detect file changes and invalidate stale cache entries
3. **Minimal Overhead**: Serialization time < 200ms on shutdown
4. **Disk Efficiency**: Compress cached data (2-3x size reduction)
5. **Backward Compatibility**: Gracefully handle cache format changes

## Non-Goals

1. **Distributed Cache**: Not syncing cache across machines
2. **Cloud Storage**: Not storing cache in cloud (local disk only)
3. **Incremental Updates**: Not updating cache on every file change (only on shutdown)

## Architecture

### Cache Storage Location

**Proposed Directory Structure**:

```
~/.cache/rholang-language-server/
├── v1/                              # Cache format version
│   ├── workspace-{hash}/            # Per-workspace cache
│   │   ├── metadata.json            # Workspace metadata
│   │   ├── documents.bincode        # Serialized documents
│   │   └── index.bincode            # Symbol index
│   └── workspace-{hash2}/
│       └── ...
└── cache.lock                       # Lock file for concurrent access
```

**Workspace Hash**: Blake3 hash of workspace root path
- Ensures separate caches for different projects
- Avoids path conflicts

**Example**:
```
Workspace: /home/user/myproject
Hash: blake3("/home/user/myproject") = "a1b2c3d4..."
Cache Dir: ~/.cache/rholang-language-server/v1/workspace-a1b2c3d4/
```

### Serialization Format

**Option 1: bincode (Recommended)**
- **Pros**: Fast, compact, Rust-native
- **Cons**: Not human-readable, Rust-specific
- **Use Case**: Performance-critical, Rust-only codebase

**Option 2: MessagePack**
- **Pros**: Compact, language-agnostic
- **Cons**: Slower than bincode
- **Use Case**: Cross-language compatibility

**Option 3: Protocol Buffers**
- **Pros**: Schema evolution, backward-compatible
- **Cons**: Verbose, slower
- **Use Case**: Long-term stability

**Decision**: **bincode** for Phase B-3 (can migrate to Protocol Buffers in Phase C)

### Cache Invalidation Strategy

**File Modification Detection**:

```rust
pub struct CacheEntry {
    uri: Url,
    content_hash: ContentHash,        // Blake3 of file content
    modified_at: SystemTime,          // File mtime
    cached_document: Arc<CachedDocument>,
}

impl CacheEntry {
    fn is_valid(&self) -> Result<bool, std::io::Error> {
        let path = self.uri.to_file_path().unwrap();
        let metadata = std::fs::metadata(&path)?;
        let current_mtime = metadata.modified()?;

        // Invalidate if file modified after cache entry
        Ok(current_mtime <= self.modified_at)
    }
}
```

**Three-Level Validation**:
1. **mtime check**: Fast, catches most changes
2. **Content hash**: Detects modifications with same mtime (rare)
3. **Fallback**: Re-parse on hash mismatch

### Serialization Workflow

**On Shutdown** (`didExit` LSP notification):

```rust
async fn serialize_cache(
    workspace_root: &Path,
    cache: &DocumentCache,
) -> Result<(), Error> {
    // 1. Create workspace cache directory
    let cache_dir = get_workspace_cache_dir(workspace_root)?;
    std::fs::create_dir_all(&cache_dir)?;

    // 2. Collect cache entries
    let entries: Vec<SerializedCacheEntry> = cache.iter()
        .map(|(uri, entry)| serialize_entry(uri, entry))
        .collect()?;

    // 3. Compress with zstd (level 3 = fast compression)
    let compressed = zstd::encode_all(&bincode::serialize(&entries)?, 3)?;

    // 4. Write to disk atomically (tmp file + rename)
    let tmp_path = cache_dir.join("documents.bincode.tmp");
    let final_path = cache_dir.join("documents.bincode");

    std::fs::write(&tmp_path, compressed)?;
    std::fs::rename(tmp_path, final_path)?;

    // 5. Write metadata
    let metadata = CacheMetadata {
        version: CACHE_VERSION,
        created_at: SystemTime::now(),
        entry_count: entries.len(),
    };
    std::fs::write(
        cache_dir.join("metadata.json"),
        serde_json::to_string_pretty(&metadata)?
    )?;

    Ok(())
}
```

**On Startup** (`initialize` LSP request):

```rust
async fn deserialize_cache(
    workspace_root: &Path,
) -> Result<DocumentCache, Error> {
    let cache_dir = get_workspace_cache_dir(workspace_root)?;
    let cache_path = cache_dir.join("documents.bincode");

    // 1. Check if cache exists
    if !cache_path.exists() {
        return Ok(DocumentCache::new()); // Cold start
    }

    // 2. Read and decompress
    let compressed = std::fs::read(&cache_path)?;
    let decompressed = zstd::decode_all(&compressed[..])?;
    let entries: Vec<SerializedCacheEntry> = bincode::deserialize(&decompressed)?;

    // 3. Validate entries and populate cache
    let cache = DocumentCache::with_capacity(entries.len());

    for entry in entries {
        // Validate entry (mtime + content hash)
        if entry.is_valid()? {
            cache.insert(
                entry.uri,
                entry.content_hash,
                Arc::new(entry.cached_document),
                entry.modified_at,
            );
        } else {
            // Stale entry: skip (will be re-indexed)
            debug!("Skipping stale cache entry: {}", entry.uri);
        }
    }

    info!("Loaded {} documents from persistent cache", cache.len());
    Ok(cache)
}
```

## Serializable Types

### Current Challenge: Non-Serializable Fields

**Problem**: `CachedDocument` contains non-serializable types:

```rust
pub struct CachedDocument {
    pub ir: Arc<RholangNode>,               // ✅ Serializable (enum)
    pub tree: Arc<tree_sitter::Tree>,       // ❌ NOT serializable
    pub symbol_table: Arc<SymbolTable>,     // ✅ Serializable
    pub text: Rope,                         // ❌ NOT serializable (ropey::Rope)
    // ... other fields
}
```

### Solution 1: Partial Serialization (Recommended for Phase B-3)

**Strategy**: Serialize only essential fields, reconstruct others on load

```rust
#[derive(Serialize, Deserialize)]
pub struct SerializableCachedDocument {
    // Essential fields (serialize)
    pub ir: Arc<RholangNode>,
    pub symbol_table: Arc<SymbolTable>,
    pub inverted_index: HashMap<...>,
    pub content_hash: u64,
    pub version: i32,

    // Reconstructible fields (skip)
    #[serde(skip)]
    pub tree: Arc<Tree>,  // Reconstruct from text
    #[serde(skip)]
    pub text: Rope,       // Reconstruct from disk

    // Metadata for reconstruction
    pub uri: Url,         // Needed to read file from disk
    pub modified_at: SystemTime,
}

impl SerializableCachedDocument {
    fn reconstruct(self) -> Result<CachedDocument, Error> {
        // Read file from disk
        let path = self.uri.to_file_path()?;
        let text_content = std::fs::read_to_string(&path)?;

        // Reconstruct Rope
        let rope = Rope::from_str(&text_content);

        // Reconstruct Tree-sitter tree
        let tree = Arc::new(parse_code(&text_content));

        Ok(CachedDocument {
            ir: self.ir,
            tree,
            symbol_table: self.symbol_table,
            text: rope,
            // ... other fields
        })
    }
}
```

**Trade-off**:
- **Pro**: Faster serialization (no need to serialize Rope/Tree)
- **Con**: Requires disk I/O on load (read file to reconstruct Rope)
- **Impact**: ~1-2ms per document on load (acceptable for cold start)

### Solution 2: Full Serialization (Phase C)

**Strategy**: Implement `Serialize/Deserialize` for all types

```rust
// Custom serialization for Rope
impl Serialize for Rope {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Serialize as string
        self.to_string().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Rope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(Rope::from_str(&s))
    }
}
```

**Trade-off**:
- **Pro**: Complete cache, no disk I/O on load
- **Con**: Larger cache size, slower serialization
- **Impact**: 2-3x larger cache files

**Decision for B-3**: Use **Solution 1** (partial serialization)

## Performance Expectations

### Serialization Performance

**Baseline** (from Phase B-2):
- Parse + Index: 182.63ms per file
- 100 files: 18.2 seconds

**Serialization** (expected):
- bincode serialize: ~50µs per document
- zstd compress (level 3): ~100-200µs per document
- Total: ~150-250µs per document
- 100 files: **15-25ms total**

**Disk I/O**:
- Write compressed cache: ~10-50ms (depends on disk speed)
- Total shutdown overhead: **25-75ms** (acceptable)

### Deserialization Performance

**Expected**:
- Read from disk: ~10-50ms
- zstd decompress: ~50-100ms
- bincode deserialize: ~50µs per document
- Validation (mtime check): ~1ms per file
- Reconstruct Rope/Tree: ~1-2ms per document (on demand)

**Total for 100 files**: **100-300ms** (cold start)

**Comparison**:
- **Without cache**: 18.2 seconds
- **With persistent cache**: 0.1-0.3 seconds
- **Speedup**: **60-180x faster startup**

## Disk Space Requirements

### Per-Document Size Estimation

| Component | Size (uncompressed) | Size (compressed, zstd) |
|-----------|---------------------|-------------------------|
| IR (AST) | 500 KB - 1 MB | 150-300 KB |
| Symbol Table | 50-200 KB | 20-60 KB |
| Metadata | 10-20 KB | 5-10 KB |
| **Total** | **~600 KB - 1.2 MB** | **~180-370 KB** |

### Workspace Cache Size

| Workspace Size | Uncompressed | Compressed (zstd) |
|----------------|--------------|-------------------|
| 50 files | 30-60 MB | 9-18 MB |
| 100 files | 60-120 MB | 18-37 MB |
| 500 files | 300-600 MB | 90-185 MB |
| 1000 files | 600 MB - 1.2 GB | 180-370 MB |

**Compression Ratio**: ~3x reduction with zstd level 3

## Cache Versioning and Migration

### Cache Format Versioning

```rust
const CACHE_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
pub struct CacheMetadata {
    version: u32,
    created_at: SystemTime,
    entry_count: usize,
    language_server_version: String,  // e.g., "0.1.0"
}

fn is_cache_compatible(metadata: &CacheMetadata) -> bool {
    // Invalidate cache if version mismatch
    metadata.version == CACHE_VERSION
}
```

**Version Mismatch Strategy**:
1. Detect version mismatch
2. Delete old cache
3. Fallback to cold start (re-index workspace)
4. Create new cache with current version

### Migration Path (Future)

**Phase C**: Support backward-compatible migrations

```rust
fn migrate_cache(old_version: u32, new_version: u32) -> Result<(), Error> {
    match (old_version, new_version) {
        (1, 2) => migrate_v1_to_v2()?,
        (2, 3) => migrate_v2_to_v3()?,
        _ => return Err(Error::IncompatibleVersion),
    }
    Ok(())
}
```

## Error Handling

### Cache Load Failures

**Strategy**: Graceful degradation

```rust
match deserialize_cache(workspace_root).await {
    Ok(cache) => {
        info!("Loaded persistent cache with {} entries", cache.len());
        cache
    }
    Err(e) => {
        warn!("Failed to load cache: {}. Starting with empty cache.", e);
        // Delete corrupted cache
        let _ = std::fs::remove_dir_all(get_workspace_cache_dir(workspace_root)?);
        DocumentCache::new()  // Fallback to cold start
    }
}
```

**Failure Scenarios**:
1. **Corrupted cache file**: Delete and cold start
2. **Version mismatch**: Delete old cache and cold start
3. **Disk full**: Skip serialization, log warning
4. **Permission denied**: Skip serialization, log warning

## Configuration

### User-Configurable Options (Phase C)

**Proposed Configuration** (LSP settings):

```json
{
  "rholang.cache.persistent": {
    "enabled": true,
    "location": "~/.cache/rholang-language-server",
    "compressionLevel": 3,
    "maxSizeMB": 1000,
    "cleanupOlderThanDays": 30
  }
}
```

## Testing Strategy

### Unit Tests

1. **Serialization Round-Trip**:
   ```rust
   #[test]
   fn test_serialize_deserialize_cached_document() {
       let doc = create_test_cached_document();
       let serialized = bincode::serialize(&doc).unwrap();
       let deserialized: CachedDocument = bincode::deserialize(&serialized).unwrap();
       assert_eq!(doc.content_hash, deserialized.content_hash);
   }
   ```

2. **Cache Invalidation**:
   ```rust
   #[test]
   fn test_cache_invalidation_on_file_change() {
       let cache_entry = create_cache_entry();
       assert!(cache_entry.is_valid().unwrap());

       // Modify file
       std::fs::write(&path, "modified content").unwrap();

       assert!(!cache_entry.is_valid().unwrap());
   }
   ```

3. **Version Compatibility**:
   ```rust
   #[test]
   fn test_version_mismatch_fallback() {
       let old_metadata = CacheMetadata { version: 0, ... };
       assert!(!is_cache_compatible(&old_metadata));
   }
   ```

### Integration Tests

1. **Cold Start vs Warm Start**:
   ```rust
   #[tokio::test]
   async fn test_warm_start_performance() {
       // Cold start
       let cold_start = time_workspace_indexing_cold().await;

       // Warm start (with persistent cache)
       let warm_start = time_workspace_indexing_warm().await;

       assert!(warm_start < cold_start / 10);  // 10x faster minimum
   }
   ```

2. **Cache Persistence Across Restarts**:
   ```rust
   #[tokio::test]
   async fn test_cache_survives_restart() {
       let backend = create_backend().await;
       // Index workspace
       backend.index_workspace().await;
       // Serialize cache
       backend.shutdown().await;

       // Restart
       let backend2 = create_backend().await;
       let cache_size = backend2.workspace.document_cache.len();
       assert!(cache_size > 0);  // Cache loaded
   }
   ```

## Rollout Plan

### Phase B-3.1: Basic Serialization (Week 1)
- [x] Planning document
- [ ] Implement `SerializableCachedDocument` struct
- [ ] Implement bincode serialization
- [ ] Unit tests for round-trip serialization

### Phase B-3.2: Disk I/O (Week 2)
- [ ] Implement workspace cache directory management
- [ ] Implement atomic write (tmp + rename)
- [ ] Implement zstd compression
- [ ] Error handling and fallback logic

### Phase B-3.3: Cache Validation (Week 3)
- [ ] Implement mtime-based invalidation
- [ ] Implement content hash verification
- [ ] Integration tests for invalidation

### Phase B-3.4: LSP Integration (Week 4)
- [ ] Hook serialization to `didExit` notification
- [ ] Hook deserialization to `initialize` request
- [ ] Benchmark warm start vs cold start
- [ ] Documentation

## Success Criteria

1. **Performance**: Warm start < 500ms for 100-file project (vs 18s cold start)
2. **Correctness**: No stale cache entries (100% accuracy on file change detection)
3. **Reliability**: Graceful degradation on cache failures (no crashes)
4. **Disk Usage**: Compressed cache < 200 KB per file
5. **Test Coverage**: >90% coverage for cache serialization code

## Risks and Mitigations

| Risk | Impact | Likelihood | Mitigation |
|------|--------|-----------|-----------|
| Corrupted cache causes crash | High | Low | Graceful fallback to cold start |
| Version incompatibility | Medium | Medium | Cache versioning + migration |
| Disk space exhaustion | Medium | Low | Cache size limits + cleanup |
| Slow deserialization | High | Low | Benchmarking + optimization |
| False cache hits (stale data) | High | Low | Multi-level validation (mtime + hash) |

## Future Enhancements (Phase C+)

1. **Incremental Cache Updates**: Update cache on every file change (not just shutdown)
2. **Distributed Cache**: Share cache across team members (CI/CD)
3. **Cloud Storage**: Store cache in cloud for remote development
4. **Smart Prefetching**: Predict and preload likely-accessed files
5. **Compression Tuning**: Auto-tune compression level based on CPU/disk speed

## References

- [Phase B-2 Implementation](../ledger/phase-b-2-implementation-complete.md)
- [Cache Capacity Tuning Guide](../cache-capacity-tuning-guide.md)
- [bincode Documentation](https://docs.rs/bincode/)
- [zstd Documentation](https://docs.rs/zstd/)

---

**Last Updated**: 2025-11-13
**Maintainer**: F1R3FLY.io
**Status**: Planning (Phase B-3)
**Timeline**: 4 weeks
**Priority**: High (major performance improvement)
