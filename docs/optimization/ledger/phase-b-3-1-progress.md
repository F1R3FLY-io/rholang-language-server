# Phase B-3.1: Basic Serialization - Progress Report

**Date**: 2025-11-14
**Status**: In Progress
**Objective**: Create SerializableCachedDocument and add Serialize/Deserialize to core types

## Completed ✅

### 1. Dependencies Added
- ✅ **zstd 0.13**: Compression library for persistent cache
- ✅ **dirs 5.0**: Platform-specific cache directories
- ✅ **bincode 1.3**: Already present (used in Phase B-1.1)
- ✅ **serde**: Already present

### 2. Module Structure Created
- ✅ **src/lsp/backend/persistent_cache.rs**: New module (311 lines)
  - `SerializableCachedDocument` struct definition
  - Conversion methods: `from_cached_document()`, `to_cached_document()`
  - Cache validation: `is_valid()` with mtime checking
  - Cache directory management: `get_workspace_cache_dir()`
  - Cache metadata: `CacheMetadata` struct
  - Cache versioning: `CACHE_VERSION` constant
  - Unit tests: 3 basic tests for version/directory/compatibility

- ✅ **Module registration**: Added to `src/lsp/backend.rs`

### 3. Serialization Strategy Documented
From planning document (Solution 1: Partial Serialization):

**Fields to Serialize**:
- `ir: Arc<RholangNode>` - Primary semantic tree
- `document_ir: Option<Arc<DocumentIR>>` - With comment channel
- `position_index: Arc<PositionIndex>` - O(log n) lookups
- `symbol_table: Arc<SymbolTable>` - Symbol resolution
- `inverted_index: HashMap<...>` - Find references
- `symbol_index: Arc<SymbolIndex>` - Suffix array search
- `positions: Arc<HashMap<...>>` - Position mappings
- `version: i32` - Document version
- `content_hash: u64` - Change detection
- `language: DocumentLanguage` - Language type
- `uri: Url` - File location (for reconstruction)
- `modified_at: SystemTime` - Cache validation

**Fields to Skip** (reconstructed on load):
- `tree: Arc<Tree>` - Tree-sitter tree (reconstruct from file)
- `text: Rope` - Document text (read from file)
- `unified_ir: Arc<dyn SemanticNode>` - Reconstruct from IR
- `completion_state: Option<...>` - Rebuild on first use
- `metta_ir: Option<...>` - Skipped for now

## In Progress 🚧

### 4. Serialize/Deserialize Implementation

**Current Blocker**: Core IR types don't implement `Serialize/Deserialize`

**Types Requiring Serialize/Deserialize**:
1. ✅ **DocumentLanguage** - Added (simple enum)
2. ⏳ **RholangNode** - Large enum, complex structure
3. ⏳ **DocumentIR** - Contains RholangNode + comment channel
4. ⏳ **PositionIndex** - BTreeMap-based index
5. ⏳ **SymbolTable** - Hierarchical symbol table
6. ⏳ **SymbolIndex** - Suffix array + sorted symbols
7. ⏳ **Position** - Simple struct (should be easy)
8. ⏳ **NodeBase** - Part of RholangNode

**Compilation Errors** (current state):
```
error[E0277]: the trait bound `Arc<RholangNode>: Serialize` is not satisfied
error[E0277]: the trait bound `Arc<DocumentIR>: Serialize` is not satisfied
error[E0277]: the trait bound `Arc<PositionIndex>: Serialize` is not satisfied
error[E0277]: the trait bound `Arc<SymbolTable>: Serialize` is not satisfied
error[E0277]: the trait bound `Arc<SymbolIndex>: Serialize` is not satisfied
... (similar for Deserialize)
```

## Next Steps (Remaining for B-3.1) 📋

### Step 1: Add Serialize/Deserialize to Core Types

**Priority Order** (simple → complex):
1. **Position** (`src/ir/semantic_node.rs`):
   - Simple struct: `{ row, column, byte }`
   - Estimated: 5 minutes

2. **NodeBase** (`src/ir/rholang_node.rs`):
   - Contains Position + metadata
   - Estimated: 10 minutes

3. **DocumentLanguage** (`src/lsp/models.rs`):
   - ✅ Already completed

4. **SymbolTable** (`src/ir/symbol_table.rs`):
   - HashMap-based structure
   - May need custom serialization for parent reference
   - Estimated: 30 minutes

5. **Position Index** (`src/lsp/position_index.rs`):
   - BTreeMap-based
   - Straightforward if Position is serializable
   - Estimated: 15 minutes

6. **SymbolIndex** (`src/lsp/symbol_index.rs`):
   - Suffix array + sorted symbols
   - Needs careful handling of internal structure
   - Estimated: 45 minutes

7. **DocumentIR** (`src/ir/mod.rs`):
   - Contains RholangNode + comment channel
   - Depends on RholangNode
   - Estimated: 20 minutes

8. **RholangNode** (`src/ir/rholang_node.rs`):
   - **MOST COMPLEX**: Large enum with ~40 variants
   - Recursive structure (nodes contain nodes)
   - May need custom serialization for Some variants
   - Estimated: 2-3 hours

**Total Estimated Time**: 4-5 hours

### Step 2: Handle Arc<T> Serialization

**Options**:
1. **serde_with attribute**: Use `#[serde(with = "...")]` to serialize inner value
2. **Transparent wrapper**: Serialize the inner `T`, wrap in `Arc` on deserialization
3. **Manual implementation**: Custom `Serialize/Deserialize` implementations

**Recommended**: Option 2 (transparent wrapper) is cleanest for our use case

### Step 3: Unit Tests

**Test Cases Needed**:
1. Round-trip serialization test:
   ```rust
   #[test]
   fn test_serialize_deserialize_cached_document() {
       let doc = create_test_cached_document();
       let serializable = SerializableCachedDocument::from_cached_document(&doc, uri).unwrap();
       let serialized = bincode::serialize(&serializable).unwrap();
       let deserialized: SerializableCachedDocument = bincode::deserialize(&serialized).unwrap();
       let reconstructed = deserialized.to_cached_document().unwrap();
       assert_eq!(doc.content_hash, reconstructed.content_hash);
   }
   ```

2. Cache invalidation test:
   ```rust
   #[test]
   fn test_cache_entry_invalidation() {
       let entry = create_serializable_entry();
       assert!(entry.is_valid().unwrap());

       // Modify file
       std::fs::write(&path, "modified content").unwrap();

       assert!(!entry.is_valid().unwrap());
   }
   ```

3. Version compatibility test:
   ```rust
   #[test]
   fn test_version_mismatch_detection() {
       let old_metadata = CacheMetadata { version: 0, ... };
       assert!(!is_cache_compatible(&old_metadata));
   }
   ```

## Architecture Notes

### Cache Directory Structure
```
~/.cache/rholang-language-server/
├── v1/                              # Cache format version
│   ├── workspace-{hash}/            # Per-workspace cache
│   │   ├── metadata.json            # Workspace metadata
│   │   ├── documents.bincode        # Serialized documents
│   │   └── index.bincode            # Symbol index (future)
│   └── workspace-{hash2}/
│       └── ...
└── cache.lock                       # Lock file (future)
```

### Workspace Hash
- Uses blake3 hash of workspace root path
- Ensures separate caches for different projects
- Example: `/home/user/myproject` → `workspace-a1b2c3d4...`

### Cache Validation Strategy
**Three-Level Validation** (from planning):
1. **mtime check** ✅ Implemented: Fast, catches most changes
2. **Content hash** ⏳ Future (B-3.3): Detects modifications with same mtime
3. **Fallback** ✅ Implemented: Re-parse on validation failure

## Performance Expectations

### Serialization (per document)
- bincode serialize: ~50µs
- zstd compress (level 3): ~100-200µs
- **Total**: ~150-250µs

### Deserialization (per document)
- Read from disk: ~10-50ms (for all docs)
- zstd decompress: ~50-100ms
- bincode deserialize: ~50µs per document
- Validation (mtime): ~1ms per file
- Reconstruct Rope/Tree: ~1-2ms per document (on demand)
- **Total for 100 files**: **100-300ms** (vs 18.2 seconds without cache)

### Expected Speedup
- **Cold start (100 files)**: 18.2 seconds
- **Warm start (100 files)**: 0.1-0.3 seconds
- **Improvement**: **60-180x faster**

## Risks and Mitigations

| Risk | Impact | Likelihood | Mitigation |
|------|--------|-----------|-----------|
| Serialization too slow | High | Low | Benchmark and optimize critical paths |
| Large cache size | Medium | Medium | Use zstd compression (3x reduction) |
| Version incompatibility | Medium | Medium | Version checking + graceful fallback |
| RholangNode serialization complexity | High | Medium | Incremental testing per variant |

## Testing Strategy

### Unit Tests (Phase B-3.1)
- ✅ Cache version constant
- ✅ Cache directory structure
- ✅ Cache compatibility check
- ⏳ Round-trip serialization (blocked on Serialize impl)
- ⏳ Cache validation with mtime
- ⏳ Reconstruction correctness

### Integration Tests (Phase B-3.3)
- ⏳ Cold start vs warm start performance
- ⏳ Cache persistence across restarts
- ⏳ Cache invalidation on file change
- ⏳ Graceful fallback on cache corruption

## Success Criteria for B-3.1

- ✅ SerializableCachedDocument struct created
- ✅ Conversion methods implemented
- ✅ Cache directory management implemented
- ⏳ All core types implement Serialize/Deserialize
- ⏳ Unit tests pass (round-trip serialization)
- ⏳ No compilation errors

## Current Status Summary

**Progress**: 40% complete
- ✅ Module structure and architecture defined
- ✅ Conversion logic implemented
- ✅ Cache validation (mtime) implemented
- ⏳ **Blocked on**: Serialize/Deserialize implementation for core IR types
- ⏳ **Next task**: Add Serialize/Deserialize to Position, NodeBase, then RholangNode

**Files Modified**:
- `Cargo.toml` - Added zstd, dirs dependencies
- `src/lsp/backend.rs` - Registered persistent_cache module
- `src/lsp/backend/persistent_cache.rs` - New module (311 lines)
- `src/lsp/models.rs` - Added Serialize/Deserialize to DocumentLanguage

**Time Invested**: ~2 hours
**Estimated Remaining**: 4-5 hours for Serialize/Deserialize implementation

---

**Last Updated**: 2025-11-14
**Phase**: B-3.1 (Basic Serialization)
**Next Milestone**: Complete Serialize/Deserialize for all core types
