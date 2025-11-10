# Hierarchical Scope Filtering Design

**Date**: 2025-01-10
**Status**: Design Phase
**Goal**: Prioritize local symbols over global symbols in completion results

---

## Problem Statement

Currently, completion results are ranked by:
1. Distance (Levenshtein distance) - Primary
2. Reference count (usage frequency) - Secondary
3. Length (shorter names preferred) - Tertiary
4. Lexicographic order - Tie-breaker

**Missing**: Scope proximity - local variables should rank higher than global ones.

### Example Issue

```rholang
contract processData(@data) = {
  new result in {
    // Cursor here: typing "re"
    // Current behavior: All symbols starting with "re" ranked equally
    // Desired: "result" (local) should rank higher than "readFile" (global)
  }
}
```

---

## Proposed Solution

Add **scope depth** as the highest priority ranking criterion:

**New Ranking Order**:
1. **Scope depth** (lower is better - closer scopes rank higher) ← NEW
2. Distance (Levenshtein distance)
3. Reference count (usage frequency)
4. Length (shorter names preferred)
5. Lexicographic order (tie-breaker)

---

## Design

### 1. Scope Depth Calculation

**Definition**: Scope depth = number of parent scopes from symbol's declaration to cursor position.

```
Example:
Global scope (depth = ∞)
└─ File scope (depth = 3)
   └─ Contract scope (depth = 2)
      └─ New scope (depth = 1)
         └─ Match scope (depth = 0) ← Cursor here
```

**Calculation**:
- Symbols in current scope: depth = 0
- Symbols in parent scope: depth = 1
- Symbols in grandparent scope: depth = 2
- Global symbols (workspace-wide): depth = ∞ (or max value)

### 2. Symbol Table Structure (Already Exists)

The `SymbolTable` struct already has hierarchical structure:

```rust
pub struct SymbolTable {
    pub symbols: Arc<DashMap<String, Arc<Symbol>, FxBuildHasher>>,
    pattern_index: Arc<DashMap<...>>,
    parent: Option<Arc<SymbolTable>>,  // ← Already supports hierarchy
}
```

**Methods to add**:
- `get_scope_depth()` - Calculate depth from root
- `collect_symbols_with_depth()` - Collect symbols from all scopes with depth

### 3. CompletionSymbol Extension

Add scope depth to `CompletionSymbol`:

```rust
pub struct CompletionSymbol {
    pub metadata: SymbolMetadata,
    pub distance: usize,  // Levenshtein distance
    pub scope_depth: usize,  // ← NEW: Scope proximity
}
```

### 4. RankingCriteria Extension

Add scope depth weight to `RankingCriteria`:

```rust
pub struct RankingCriteria {
    pub scope_depth_weight: f64,  // ← NEW (default: 10.0)
    pub distance_weight: f64,      // default: 1.0
    pub reference_count_weight: f64,  // default: 0.1
    pub length_weight: f64,        // default: 0.01
    pub max_results: usize,
}
```

**Why scope_depth_weight = 10.0?**
- Must dominate distance to ensure local symbols always rank first
- Example: `result` (scope_depth=0, distance=5) should beat `readFile` (scope_depth=3, distance=0)
  - `result` score: 0 * 10 + 5 * 1 = 5
  - `readFile` score: 3 * 10 + 0 * 1 = 30
  - `result` wins ✓

### 5. Ranking Function Update

```rust
fn calculate_score(symbol: &CompletionSymbol, criteria: &RankingCriteria) -> f64 {
    let scope_score = symbol.scope_depth as f64 * criteria.scope_depth_weight;
    let distance_score = symbol.distance as f64 * criteria.distance_weight;
    let reference_score = if symbol.metadata.reference_count > 0 {
        -1.0 * (symbol.metadata.reference_count as f64) * criteria.reference_count_weight
    } else {
        0.0
    };
    let length_score = symbol.metadata.name.len() as f64 * criteria.length_weight;

    scope_score + distance_score + reference_score + length_score
}
```

---

## Implementation Plan

### Phase 1: Extend Symbol Table (1 hour)

**File**: `src/ir/symbol_table.rs`

1. Add `get_scope_depth(&self) -> usize` method:
   ```rust
   impl SymbolTable {
       pub fn get_scope_depth(&self) -> usize {
           match &self.parent {
               None => 0,  // Root scope
               Some(parent) => 1 + parent.get_scope_depth(),
           }
       }
   }
   ```

2. Add `collect_symbols_with_depth(&self, prefix: &str) -> Vec<(Arc<Symbol>, usize)>`:
   ```rust
   impl SymbolTable {
       pub fn collect_symbols_with_depth(&self, prefix: &str) -> Vec<(Arc<Symbol>, usize)> {
           let mut results = Vec::new();
           let current_depth = 0;
           self.collect_symbols_with_depth_helper(prefix, current_depth, &mut results);
           results
       }

       fn collect_symbols_with_depth_helper(
           &self,
           prefix: &str,
           current_depth: usize,
           results: &mut Vec<(Arc<Symbol>, usize)>
       ) {
           // Collect from current scope
           for entry in self.symbols.iter() {
               if entry.key().starts_with(prefix) {
                   results.push((entry.value().clone(), current_depth));
               }
           }

           // Recursively collect from parent scopes
           if let Some(parent) = &self.parent {
               parent.collect_symbols_with_depth_helper(prefix, current_depth + 1, results);
           }
       }
   }
   ```

### Phase 2: Extend CompletionSymbol (30 minutes)

**File**: `src/lsp/features/completion/dictionary.rs`

1. Add `scope_depth` field to `CompletionSymbol`:
   ```rust
   pub struct CompletionSymbol {
       pub metadata: SymbolMetadata,
       pub distance: usize,
       pub scope_depth: usize,  // ← NEW
   }
   ```

2. Update constructors to accept `scope_depth` parameter

### Phase 3: Extend RankingCriteria (30 minutes)

**File**: `src/lsp/features/completion/ranking.rs`

1. Add `scope_depth_weight` field to `RankingCriteria`:
   ```rust
   pub struct RankingCriteria {
       pub scope_depth_weight: f64,  // ← NEW
       pub distance_weight: f64,
       pub reference_count_weight: f64,
       pub length_weight: f64,
       pub max_results: usize,
   }
   ```

2. Update `default()`, `exact_prefix()`, and `fuzzy()` methods:
   ```rust
   pub fn default() -> Self {
       Self {
           scope_depth_weight: 10.0,  // ← NEW
           distance_weight: 1.0,
           reference_count_weight: 0.1,
           length_weight: 0.01,
           max_results: 50,
       }
   }
   ```

3. Update `calculate_score()` to include scope depth:
   ```rust
   fn calculate_score(symbol: &CompletionSymbol, criteria: &RankingCriteria) -> f64 {
       let scope_score = symbol.scope_depth as f64 * criteria.scope_depth_weight;
       // ... rest unchanged
   }
   ```

### Phase 4: Update Completion Query (1 hour)

**File**: `src/lsp/features/completion/incremental.rs` or main completion handler

1. When querying completions, use `collect_symbols_with_depth()` instead of flat lookup
2. Pass scope depth to `CompletionSymbol` constructors
3. For workspace-wide symbols (global), use `usize::MAX` as scope depth

### Phase 5: Add Tests (1 hour)

**File**: `tests/test_completion.rs` or new test file

Test cases:
1. Local variable ranks higher than global variable (same prefix)
2. Nested scope priority (innermost > middle > outermost)
3. Global symbol still appears when no local matches
4. Multiple local symbols ranked by distance within same scope

**Example test**:
```rust
#[test]
fn test_local_symbol_priority() {
    let code = indoc! {r#"
        contract globalProcess(@x) = { Nil }

        new result in {
            contract process(@data) = {
                new processLocal in {
                    // Cursor here: "proc"
                    // Expected order: processLocal (depth=0), process (depth=1), globalProcess (depth=∞)
                }
            }
        }
    "#};
    // ... test assertion
}
```

---

## Expected Behavior

### Before Hierarchical Filtering

```
Query: "re"
Results:
1. readFile (distance=0, global)
2. result (distance=0, local)
3. remoteCall (distance=1, global)
```

**Issue**: Global symbols dominate results even when local symbols are more relevant.

### After Hierarchical Filtering

```
Query: "re"
Results:
1. result (scope_depth=0, distance=0) ← Local, exact match
2. readFile (scope_depth=∞, distance=0) ← Global, exact match
3. remoteCall (scope_depth=∞, distance=1) ← Global, fuzzy match
```

**Improvement**: Local symbols always appear first, regardless of other factors.

---

## Performance Impact

### Additional Computation

1. **Scope depth calculation**: O(d) where d = depth (typically <5)
2. **Symbol collection with depth**: O(n * d) where n = symbols per scope
3. **Ranking**: O(n log n) → unchanged, just one more field in score

**Total overhead**: ~1-2µs per completion request (negligible)

### Memory Impact

- **Per CompletionSymbol**: +8 bytes (`usize` for `scope_depth`)
- **Per completion request**: ~50 symbols * 8 bytes = 400 bytes (negligible)

**Verdict**: Performance impact is negligible (<1% overhead).

---

## Alternative Designs Considered

### Alternative 1: Separate Local/Global Queries

**Approach**: Query local symbols first, then global symbols if needed.

**Pros**:
- Simpler implementation
- Potentially faster (early termination if enough local results)

**Cons**:
- Binary decision (all local OR all global, no mixing)
- Poor UX if user wants to see both local and global
- Harder to tune threshold

**Verdict**: Rejected - less flexible than weighted ranking.

### Alternative 2: Scope-Based Filtering (Boolean)

**Approach**: Add checkbox "Show only local symbols"

**Pros**:
- User control
- Simple implementation

**Cons**:
- Requires user action
- Binary decision (no gradual ranking)
- Extra UI complexity

**Verdict**: Rejected - prefer automatic ranking without user intervention.

### Alternative 3: Context-Aware Scope Detection

**Approach**: Detect what user is likely trying to reference (local variable, global contract, etc.) and filter accordingly.

**Pros**:
- Very smart, adapts to user intent
- Could provide even better results

**Cons**:
- Much more complex (requires ML or heuristics)
- Harder to debug when wrong
- Overkill for current needs

**Verdict**: Deferred - consider for future enhancement if weighted ranking proves insufficient.

---

## Future Enhancements

### 1. Scope-Aware Fuzzy Matching

Currently, fuzzy matching is applied equally to all scopes. Could optimize by:
- Higher edit distance threshold for local symbols (more forgiving)
- Lower edit distance threshold for global symbols (more strict)

**Example**:
- Local: "reslt" matches "result" (distance=1, allowed)
- Global: "reslt" doesn't match "result" (distance=1, too far)

### 2. Symbol Visibility Rules

Some symbols should be invisible outside their scope:
- Private functions (not yet supported in Rholang)
- Module-local symbols (when modules are added)

Could extend to filter by visibility in addition to ranking by scope depth.

### 3. Import-Aware Ranking

When Rholang supports imports/modules:
- Symbols from current file rank higher than imported symbols
- Symbols from nearby files rank higher than distant files

---

## Success Criteria

### Functional Requirements

1. ✅ Local variables rank higher than global variables (same prefix)
2. ✅ Nested scopes work correctly (innermost first)
3. ✅ Global symbols still accessible when no local matches
4. ✅ Backward compatible (existing tests still pass)

### Performance Requirements

1. ✅ Overhead < 5µs per completion request
2. ✅ Memory overhead < 1KB per request
3. ✅ All existing performance targets still met

### User Experience Requirements

1. ✅ More relevant completions appear first
2. ✅ No configuration required (works out of the box)
3. ✅ Predictable behavior (users can understand ranking)

---

## Implementation Checklist

- [ ] Phase 1: Extend SymbolTable (1 hour)
  - [ ] Add `get_scope_depth()` method
  - [ ] Add `collect_symbols_with_depth()` method
  - [ ] Add unit tests for scope traversal

- [ ] Phase 2: Extend CompletionSymbol (30 min)
  - [ ] Add `scope_depth` field
  - [ ] Update constructors
  - [ ] Update serialization if needed

- [ ] Phase 3: Extend RankingCriteria (30 min)
  - [ ] Add `scope_depth_weight` field
  - [ ] Update default values
  - [ ] Update `calculate_score()` function
  - [ ] Add unit tests for scope-based ranking

- [ ] Phase 4: Update Completion Query (1 hour)
  - [ ] Integrate `collect_symbols_with_depth()`
  - [ ] Pass scope depth to CompletionSymbol
  - [ ] Handle global symbols (scope_depth = MAX)

- [ ] Phase 5: Add Integration Tests (1 hour)
  - [ ] Test local > global priority
  - [ ] Test nested scope priority
  - [ ] Test global fallback
  - [ ] Test multiple locals ranked by distance

**Total Estimated Time**: 4 hours

---

## Conclusion

Hierarchical scope filtering will significantly improve completion relevance by prioritizing local symbols over global ones. The design is:

- **Simple**: Adds one field to existing ranking system
- **Efficient**: Negligible performance impact (<5µs overhead)
- **Effective**: Solves the "local vs global" ranking problem
- **Extensible**: Foundation for future scope-aware features

**Recommended**: Proceed with implementation.
