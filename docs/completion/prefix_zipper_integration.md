# PrefixZipper Integration Plan

## Overview

This document describes the integration of liblevenshtein's `PrefixZipper` trait into the Rholang Language Server for efficient pattern-aware code completion.

**Status**: Waiting for liblevenshtein implementation
**Blocking**: PrefixZipper trait + implementations (PathMap, DynamicDawg, DoubleArrayTrie)
**Timeline**: Implement in rholang-language-server after liblevenshtein PR merged

---

## Part 1: liblevenshtein Changes (To Be Implemented in liblevenshtein Repo)

### 1.1 PrefixZipper Trait Definition

**File**: `liblevenshtein-rust/src/dictionary/prefix_zipper.rs` (NEW)

**Purpose**: Generic trait for efficient prefix-based navigation across all dictionary backends

**Trait Interface**:

```rust
/// Extension trait for efficient prefix-based navigation in dictionaries
///
/// This trait enables O(k) navigation to a prefix in a trie-based dictionary,
/// followed by O(m) iteration over matching terms, where:
/// - k = prefix length (typically 2-5 characters)
/// - m = number of terms matching the prefix
///
/// This is significantly faster than O(n) iteration + `.starts_with()` filtering
/// when m << n (selective prefixes).
pub trait PrefixZipper: DictZipper {
    /// Navigate to the given prefix in the dictionary
    ///
    /// Returns a zipper positioned at the prefix node if any terms with this
    /// prefix exist, or None if no matching terms.
    ///
    /// # Arguments
    /// * `prefix` - Byte sequence to navigate to
    ///
    /// # Returns
    /// - `Some(zipper)` - Zipper positioned at prefix node
    /// - `None` - No terms with this prefix exist
    ///
    /// # Performance
    /// O(k) where k = prefix length
    fn descend_prefix(&self, prefix: &[Self::Unit]) -> Option<Self>;

    /// Iterate all complete terms under the current prefix position
    ///
    /// This method assumes the zipper is already positioned at a prefix
    /// (via `descend_prefix()` or manual navigation) and iterates only
    /// the terms under that prefix.
    ///
    /// # Returns
    /// Iterator over complete terms (as byte sequences)
    ///
    /// # Performance
    /// O(m) where m = number of terms under prefix
    fn iter_prefix(&self) -> Box<dyn Iterator<Item = Vec<Self::Unit>>>;
}

/// Extension of PrefixZipper for valued dictionaries
pub trait ValuedPrefixZipper: PrefixZipper + ValuedDictZipper {
    /// Iterate (term, value) pairs under the current prefix position
    ///
    /// Similar to `iter_prefix()` but also returns associated values.
    ///
    /// # Returns
    /// Iterator over (term, value) pairs
    ///
    /// # Performance
    /// O(m) where m = number of terms under prefix
    fn iter_prefix_with_values(&self) -> Box<dyn Iterator<Item = (Vec<Self::Unit>, Self::Value)>>;
}
```

### 1.2 Backend Implementations

#### PathMapZipper Implementation (~40 LOC)

**File**: `liblevenshtein-rust/src/dictionary/pathmap.rs` (MODIFY)

**Strategy**: Use PathMap's existing `read_zipper_at_path()` API

**Key Points**:
- PathMap already has efficient prefix navigation via ReadZipper
- `child_mask()` provides O(1) child enumeration bitmap
- `descend_to_existing()` handles partial path matching
- Zero-allocation zipper movement (Arc-based path sharing)

**Pseudo-implementation**:
```rust
impl PrefixZipper for PathMapZipper<V> {
    fn descend_prefix(&self, prefix: &[u8]) -> Option<Self> {
        let map = self.map.read();
        let mut zipper = map.read_zipper();

        // Navigate to prefix using existing API
        let traversed = zipper.descend_to_existing(prefix);

        if traversed == prefix.len() {
            Some(Self { map: self.map.clone(), /* zipper state */ })
        } else {
            None
        }
    }

    fn iter_prefix(&self) -> Box<dyn Iterator<Item = Vec<u8>>> {
        // Collect all paths from current zipper position
        // Use to_next_val() to iterate value nodes
        // ...
    }
}
```

**Performance**: O(k) navigation + O(m) iteration where k=prefix length, m=matches

#### DoubleArrayTrieZipper Implementation (~60 LOC)

**File**: `liblevenshtein-rust/src/dictionary/double_array_trie.rs` (MODIFY)

**Strategy**: Navigate to prefix state using base/check arrays, then iterate edges

**Key Points**:
- DoubleArrayTrie already 25-132x faster than DynamicDawg for prefix queries
- State-based navigation: `state = base[state] + byte`
- Edge list at each state enables efficient child enumeration
- Precomputed arrays avoid runtime allocation

**Pseudo-implementation**:
```rust
impl PrefixZipper for DoubleArrayTrieZipper {
    fn descend_prefix(&self, prefix: &[u8]) -> Option<Self> {
        let mut state = self.state;

        for &byte in prefix {
            let next_state = self.trie.base[state] + byte as usize;
            if self.trie.check[next_state] != state {
                return None; // Prefix doesn't exist
            }
            state = next_state;
        }

        Some(Self { trie: self.trie.clone(), state, path: self.path.clone() })
    }

    fn iter_prefix(&self) -> Box<dyn Iterator<Item = Vec<u8>>> {
        // DFS from current state to collect all complete terms
        // Use base/check arrays for traversal
        // ...
    }
}
```

**Performance**: O(k) navigation + O(m) iteration where k=prefix length, m=matches

#### DynamicDawgZipper Implementation (~60 LOC)

**File**: `liblevenshtein-rust/src/dictionary/dynamic_dawg.rs` (MODIFY)

**Strategy**: Node-based traversal to prefix node, then edge collection

**Key Points**:
- Node graph representation with RwLock-based concurrency
- Edge list per node for child enumeration
- Requires traversal through node indices

**Pseudo-implementation**:
```rust
impl PrefixZipper for DynamicDawgZipper {
    fn descend_prefix(&self, prefix: &[u8]) -> Option<Self> {
        let dawg = self.dawg.read();
        let mut node_idx = self.node_idx;

        for &byte in prefix {
            let node = &dawg.nodes[node_idx];
            match node.edges.iter().find(|e| e.label == byte) {
                Some(edge) => node_idx = edge.target,
                None => return None,
            }
        }

        Some(Self { dawg: self.dawg.clone(), node_idx, path: self.path.clone() })
    }

    fn iter_prefix(&self) -> Box<dyn Iterator<Item = Vec<u8>>> {
        // DFS from current node to collect all complete terms
        // Use edge lists for traversal
        // ...
    }
}
```

**Performance**: O(k * avg_edges) navigation + O(m) iteration

### 1.3 Testing Strategy (~200 LOC)

**File**: `liblevenshtein-rust/tests/prefix_zipper.rs` (NEW)

**Test Categories**:

1. **Per-Backend Basic Tests**:
   - `test_pathmap_descend_prefix_exists()` - Verify prefix navigation
   - `test_pathmap_descend_prefix_not_exists()` - Handle missing prefix
   - `test_pathmap_iter_prefix_empty()` - Empty result set
   - `test_pathmap_iter_prefix_single()` - Single match
   - `test_pathmap_iter_prefix_multiple()` - Multiple matches
   - Repeat for DoubleArrayTrie and DynamicDawg

2. **Integration Tests**:
   - Verify all backends return same results for same input
   - Test with Unicode (via Char variants)
   - Test with valued dictionaries

3. **Performance Tests**:
   - Measure prefix navigation time (target: <10µs)
   - Measure iteration time per match (target: <1µs/match)
   - Compare against `.starts_with()` filtering baseline

**Example Test**:
```rust
#[test]
fn test_pathmap_prefix_navigation() {
    let terms = vec!["process", "processUser", "produce", "product"];
    let dict = PathMapDictionary::from_terms(terms.iter());

    // Navigate to "proc" prefix
    let zipper = dict.zipper();
    let prefix_zipper = zipper.descend_prefix(b"proc").unwrap();

    // Collect matching terms
    let results: Vec<String> = prefix_zipper
        .iter_prefix()
        .map(|bytes| String::from_utf8(bytes).unwrap())
        .collect();

    assert_eq!(results.len(), 2);
    assert!(results.contains(&"process".to_string()));
    assert!(results.contains(&"processUser".to_string()));

    // "produce" and "product" should NOT be included
    assert!(!results.contains(&"produce".to_string()));
}
```

### 1.4 Documentation

**Files to Update**:
- `liblevenshtein-rust/README.md` - Add PrefixZipper to feature list
- `liblevenshtein-rust/docs/zippers.md` - Document PrefixZipper API
- Inline rustdoc comments in trait definition

**Key Documentation Points**:
- When to use PrefixZipper vs Transducer
- Performance characteristics per backend
- Example usage in completion engines
- Comparison to manual `.starts_with()` filtering

---

## Part 2: rholang-language-server Integration (This Repo - After liblevenshtein Changes)

### 2.1 Update WorkspaceCompletionIndex

**File**: `src/lsp/features/completion/dictionary.rs`

**Current Implementation** (lines 311-353):
```rust
pub fn query_prefix(&self, prefix: &str) -> Vec<CompletionSymbol> {
    // SUBOPTIMAL: Manual iteration + .starts_with() filtering
    for keyword in RHOLANG_KEYWORDS.iter() {
        if keyword.starts_with(prefix) && self.static_dict.contains(keyword) {
            // ...
        }
    }

    // SUBOPTIMAL: HashMap iteration + .starts_with() filtering
    for (name, metadata) in map.iter() {
        if name.starts_with(prefix) {
            // ...
        }
    }
}
```

**New Implementation** (using PrefixZipper):
```rust
pub fn query_prefix(&self, prefix: &str) -> Vec<CompletionSymbol> {
    let mut results = Vec::new();

    // Use PrefixZipper for static keywords (DoubleArrayTrie backend)
    if let Some(zipper) = self.static_dict.zipper().descend_prefix(prefix.as_bytes()) {
        for term_bytes in zipper.iter_prefix() {
            if let Ok(term) = String::from_utf8(term_bytes) {
                results.push(CompletionSymbol {
                    metadata: SymbolMetadata {
                        name: term.clone(),
                        kind: CompletionItemKind::KEYWORD,
                        documentation: None,
                        signature: None,
                        reference_count: 0,
                    },
                    distance: 0,
                    scope_depth: usize::MAX,
                });
            }
        }
    }

    // Use PrefixZipper for dynamic symbols (DynamicDawg backend)
    let dict = self.dynamic_dict.read();
    if let Some(zipper) = dict.zipper().descend_prefix(prefix.as_bytes()) {
        let map = self.metadata_map.read();

        for term_bytes in zipper.iter_prefix() {
            if let Ok(term) = String::from_utf8(term_bytes) {
                if let Some(metadata) = map.get(&term) {
                    results.push(CompletionSymbol {
                        metadata: metadata.clone(),
                        distance: 0,
                        scope_depth: usize::MAX,
                    });
                }
            }
        }
    }

    // Sort by name length (shorter = more likely to be relevant)
    results.sort_by_key(|s| s.metadata.name.len());
    results
}
```

**Changes**: ~40 LOC modified
**Performance Improvement**: 50-90% faster for typical prefixes (3-4 chars)

### 2.2 Implement query_contracts_by_pattern

**File**: `src/lsp/features/completion/pattern_aware.rs`

**Current Placeholder** (lines 304-311):
```rust
pub fn query_contracts_by_pattern(
    _global_index: &Arc<RwLock<GlobalSymbolIndex>>,
    _pattern_ctx: &QuotedPatternContext,
) -> Vec<CompletionSymbol> {
    // TODO: Implement in Phase 3
    debug!("query_contracts_by_pattern called - implementation pending (Phase 3)");
    vec![]
}
```

**New Implementation**:
```rust
/// Query contracts matching a quoted pattern for code completion
///
/// This function handles pattern-aware completion for contract identifiers
/// that use quoted processes (e.g., @"myContract", @{key: value}).
///
/// # Arguments
/// * `global_index` - Workspace-wide symbol index with contract definitions
/// * `pattern_ctx` - Context describing the pattern being typed at cursor
///
/// # Returns
/// Vector of matching contracts as CompletionSymbols
///
/// # Implementation Notes
/// - String patterns: Use prefix matching via GlobalSymbolIndex.definitions
/// - Map/List/Tuple/Set patterns: Deferred to Phase 2 (complex MORK unification)
pub fn query_contracts_by_pattern(
    global_index: &Arc<RwLock<GlobalSymbolIndex>>,
    pattern_ctx: &QuotedPatternContext,
) -> Vec<CompletionSymbol> {
    match pattern_ctx.pattern_type {
        QuotedPatternType::String => {
            // String literal pattern: @"prefix|"
            // Use prefix matching on contract names
            query_contracts_by_name_prefix(global_index, &pattern_ctx.partial_text)
        }

        // Complex patterns deferred to Phase 2
        QuotedPatternType::Map => {
            debug!("Map pattern completion deferred to Phase 2 (requires MORK unification)");
            vec![]
        }
        QuotedPatternType::List => {
            debug!("List pattern completion deferred to Phase 2 (requires MORK unification)");
            vec![]
        }
        QuotedPatternType::Tuple => {
            debug!("Tuple pattern completion deferred to Phase 2 (requires MORK unification)");
            vec![]
        }
        QuotedPatternType::Set => {
            debug!("Set pattern completion deferred to Phase 2 (requires MORK unification)");
            vec![]
        }
    }
}

/// Query contracts by name prefix using GlobalSymbolIndex
///
/// This is a helper function that iterates the global definitions HashMap
/// and filters for contracts whose names start with the given prefix.
///
/// # Arguments
/// * `global_index` - Global symbol index containing all workspace contracts
/// * `prefix` - String prefix to match against contract names
///
/// # Returns
/// Vector of matching contracts as CompletionSymbols
///
/// # Performance
/// O(n) where n = total symbols in workspace (typically 500-1000)
/// Acceptable for LSP response times (<100ms)
fn query_contracts_by_name_prefix(
    global_index: &Arc<RwLock<GlobalSymbolIndex>>,
    prefix: &str,
) -> Vec<CompletionSymbol> {
    let index = match global_index.read() {
        Ok(guard) => guard,
        Err(_) => return vec![],
    };

    let mut results = Vec::new();

    // Iterate all definitions, filter for contracts with matching prefix
    for (symbol_id, location) in index.definitions.iter() {
        // Filter for contracts only
        if location.kind != SymbolKind::Contract {
            continue;
        }

        // Filter by name prefix
        if !symbol_id.name.starts_with(prefix) {
            continue;
        }

        // Convert to CompletionSymbol
        results.push(CompletionSymbol {
            metadata: SymbolMetadata {
                name: symbol_id.name.clone(),
                kind: CompletionItemKind::FUNCTION, // Contracts complete as functions
                documentation: location.documentation.clone(),
                signature: location.signature.clone(),
                reference_count: 0, // Could be enriched from references index
            },
            distance: 0, // Exact prefix match
            scope_depth: usize::MAX, // Global scope
        });
    }

    // Sort by name length (shorter = more likely to be relevant)
    results.sort_by_key(|s| s.metadata.name.len());

    results
}
```

**Changes**: ~80 LOC added
**Performance**: O(n) where n = total symbols (acceptable for <1000 contracts)

### 2.3 Testing Strategy

**File**: `tests/lsp_features.rs` (MODIFY)

**New Tests**:

1. **test_completion_quoted_string_prefix**:
   ```rust
   #[tokio::test]
   async fn test_completion_quoted_string_prefix() {
       // Setup: Create contract @"processUser"
       // Query: Type @"proc|" at cursor
       // Assert: Completion suggests @"processUser"
   }
   ```

2. **test_completion_quoted_string_no_match**:
   ```rust
   #[tokio::test]
   async fn test_completion_quoted_string_no_match() {
       // Setup: Create contract @"processUser"
       // Query: Type @"xyz|" at cursor
       // Assert: No completions returned
   }
   ```

3. **test_completion_quoted_string_multiple_matches**:
   ```rust
   #[tokio::test]
   async fn test_completion_quoted_string_multiple_matches() {
       // Setup: Create contracts @"process", @"processUser", @"product"
       // Query: Type @"proc|" at cursor
       // Assert: Returns @"process" and @"processUser", not @"product"
   }
   ```

4. **test_completion_performance_regression**:
   ```rust
   #[tokio::test]
   async fn test_completion_performance_regression() {
       // Setup: Index 1000 symbols
       // Query: Prefix completion 100 times
       // Assert: Average response time <50ms (well within LSP 200ms target)
   }
   ```

**Expected Results**: All existing tests pass + 4 new tests pass

### 2.4 Documentation Updates

**Files to Update**:

1. **docs/completion/pattern_aware_completion_phase1.md** (MODIFY):
   - Add "Implementation Complete" status for Phase 3
   - Document PrefixZipper integration
   - Update performance benchmarks

2. **CLAUDE.md** (MODIFY):
   - Update completion module documentation
   - Add PrefixZipper to architecture diagram
   - Document prefix query performance characteristics

3. **src/lsp/features/completion/mod.rs** (MODIFY):
   - Update module-level documentation
   - Add usage examples for pattern_aware completion

---

## Performance Targets

| Operation | Current | With PrefixZipper | Target Improvement |
|-----------|---------|-------------------|-------------------|
| Static keyword completion (16 keywords) | ~20µs | ~5µs | 4x faster |
| Dynamic symbol completion (500 symbols) | ~100µs | ~20µs | 5x faster |
| Contract name prefix (1000 contracts) | ~200µs | ~40µs | 5x faster |
| Overall completion query | ~320µs | ~65µs | 5x faster |

**LSP Response Target**: <200ms (well within bounds even at current performance)

**Justification**: Optimization provides headroom for future features and improves user experience with instant completions.

---

## Implementation Timeline

### Phase 1: liblevenshtein (External - User Will Implement)
- PrefixZipper trait definition: 2-3 hours
- PathMapZipper implementation: 2 hours
- DoubleArrayTrieZipper implementation: 2-3 hours
- DynamicDawgZipper implementation: 2-3 hours
- Testing: 3-4 hours
- Documentation: 1-2 hours
- **Total: 12-17 hours**

### Phase 2: rholang-language-server (After liblevenshtein PR Merged)
- Update WorkspaceCompletionIndex: 1-2 hours
- Implement query_contracts_by_pattern: 2-3 hours
- Testing: 2-3 hours
- Documentation: 1 hour
- **Total: 6-9 hours**

### Phase 3: Validation & Benchmarking
- Performance benchmarks: 1-2 hours
- Manual testing: 1 hour
- Documentation updates: 1 hour
- **Total: 3-4 hours**

**Overall Total: 21-30 hours** (split between liblevenshtein and rholang-language-server)

---

## Dependencies

### Blocking
- liblevenshtein PrefixZipper trait implementation
- liblevenshtein 0.x.x release with PrefixZipper support

### Non-Blocking
- All existing completion infrastructure already in place
- Pattern-aware context detection complete (Phase 1)
- Integration points ready in handlers.rs (Phase 3)

---

## Rollout Strategy

1. **Wait for liblevenshtein PR**: User implements PrefixZipper in liblevenshtein repo
2. **Update dependency**: Bump liblevenshtein version in Cargo.toml
3. **Implement WorkspaceCompletionIndex changes**: Low-risk, backward compatible
4. **Implement pattern_aware completion**: Already has placeholder, drop-in replacement
5. **Test thoroughly**: Run full test suite + manual testing
6. **Benchmark**: Measure improvement, document in completion_performance.md
7. **Commit**: Single atomic commit with all changes
8. **Document**: Update CLAUDE.md and phase documentation

---

## Alternatives Considered

### Alternative 1: Direct PathMap Usage (Without liblevenshtein)
- **Pros**: Simpler, no dependency on liblevenshtein changes
- **Cons**: Doesn't fix static keyword iteration, tightly couples to PathMap
- **Verdict**: Rejected - misses opportunity for generic abstraction

### Alternative 2: Keep Current HashMap Iteration
- **Pros**: Zero implementation cost, simple to understand
- **Cons**: 5x slower, doesn't scale well, misses optimization opportunity
- **Verdict**: Acceptable fallback if PrefixZipper proves too complex

### Alternative 3: Add query_by_name_prefix to RholangPatternIndex
- **Pros**: Localized change, no liblevenshtein dependency
- **Cons**: Doesn't fix WorkspaceCompletionIndex, pattern index uses MORK bytes (not friendly for string prefix)
- **Verdict**: Complementary approach, may be useful in Phase 2

---

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|-----------|
| PrefixZipper implementation delays | Medium | Low | Current HashMap approach works fine |
| Performance doesn't meet targets | Low | Low | 5x improvement is conservative estimate |
| API changes in liblevenshtein | Low | Medium | Version pin, update integration as needed |
| Regression in existing tests | Low | High | Thorough testing before commit |

---

## Success Criteria

1. ✅ PrefixZipper trait defined in liblevenshtein
2. ✅ All three backends (PathMap, DynamicDawg, DoubleArrayTrie) implement PrefixZipper
3. ✅ WorkspaceCompletionIndex uses PrefixZipper for prefix queries
4. ✅ query_contracts_by_pattern implemented for string patterns
5. ✅ All existing tests pass (no regressions)
6. ✅ 4 new tests pass for pattern-aware completion
7. ✅ 5x performance improvement measured and documented
8. ✅ User can type `@"proc|"` and get contract completions

---

## Contact & Questions

**Implementation Owner**: User (for liblevenshtein), Claude (for rholang-language-server)
**Documentation**: This file + inline code comments
**Related Work**: Pattern matching (Phases 1-5), Completion infrastructure (Phases 1-4)
