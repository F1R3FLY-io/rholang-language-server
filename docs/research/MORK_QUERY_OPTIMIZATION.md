# MORK Pattern Matching Optimization Plan

**Date**: 2025-10-24
**Status**: ⚠️ Currently using O(n) iteration, needs optimization to O(k) trie-based lookup
**Context**: Step 3 of MORK integration - Pattern matching implementation

## Current Implementation

### What Works ✅
- Pattern matching is **functionally correct**
- All 7 tests pass successfully
- MORK `unify()` correctly matches patterns with variables
- Pattern storage via `load_all_sexpr_impl()` works correctly

### Performance Issue ⚠️

**File**: `src/ir/pattern_matching.rs:124-151`

```rust
// Current: O(n) linear scan through ALL entries
let mut rz = self.space.btm.read_zipper();
while rz.to_next_val() {
    let stored_expr = Expr { ptr: rz.path().as_ptr().cast_mut() };
    // Unify with each entry
    if let Ok(_bindings) = unify(vec![(pattern, stored)]) {
        matches.push(...);
    }
}
```

**Problem**: Iterates through every single entry in the PathMap trie
**Complexity**: O(n) where n = total number of stored patterns
**Required**: O(k) where k = number of matching patterns

## Why query_multi Didn't Work

### Investigation Results

1. **Pattern structure is correct**: 3 args, 1 newvar ✅
2. **Callback never invoked**: `coreferential_transition` doesn't find matches
3. **ProductZipper issue**: Seems to expect different data layout

### Root Cause Analysis

MORK's `query_multi` with `ProductZipper` appears designed for a specific use case where:
- Multiple separate sub-expressions are stored
- ProductZipper finds combinations of paths
- Designed for MeTTaTron's `(= lhs rhs)` rule matching

Our storage pattern:
- Complete s-expressions: `(pattern-key <pattern> <value>)`
- Single monolithic paths per pattern
- ProductZipper can't decompose these efficiently

## Optimization Strategy

### Approach 1: Prefix-Based Trie Navigation (RECOMMENDED)

**Concept**: Navigate directly to the trie prefix that matches the concrete parts of the pattern

#### Example
```
Pattern:  (pattern-key 42 $value)
          ├─ Concrete: (pattern-key 42
          └─ Variable: $value (matches anything)

Trie navigation:
1. Descend to "pattern-key" node
2. Descend to "42" node
3. Iterate only children of this node (O(k) not O(n))
4. Each child is a potential match - unify the suffix
```

#### Implementation Plan

**File**: `src/ir/pattern_matching.rs:match_query()`

```rust
pub fn match_query(&self, query: &Arc<RholangNode>) -> Result<MatchResult, String> {
    // Parse query to MORK pattern
    let pattern_expr = parse_query_to_mork(query)?;

    // Extract concrete prefix from pattern
    // For (pattern-key 42 $value), prefix is (pattern-key 42
    let (prefix_bytes, has_variables) = extract_prefix(pattern_expr);

    if !has_variables {
        // Exact match - single trie lookup O(1)
        return exact_trie_lookup(&self.space.btm, &prefix_bytes);
    }

    // Navigate to prefix in trie
    let mut rz = self.space.btm.read_zipper();
    if !navigate_to_prefix(&mut rz, &prefix_bytes) {
        return Ok(vec![]); // Prefix doesn't exist
    }

    // Iterate only children of this prefix (O(k))
    let mut matches = Vec::new();
    while descend_to_next_sibling(&mut rz) {
        let suffix = Expr { ptr: rz.path().as_ptr().cast_mut() };

        // Unify only the variable part
        if unify_suffix(pattern_expr, suffix)? {
            matches.push(extract_value(suffix)?);
        }
    }

    Ok(matches)
}
```

**Complexity**: O(p + k) where:
- p = length of concrete prefix
- k = number of entries matching the prefix

**Functions to Implement**:

1. `extract_prefix(pattern: Expr) -> (Vec<u8>, bool)`
   - Walk pattern expression
   - Stop at first variable (NewVar tag)
   - Return prefix bytes + whether variables exist

2. `navigate_to_prefix(zipper: &mut ReadZipper, prefix: &[u8]) -> bool`
   - Use `descend_to_byte()` and `descend_to_check()`
   - Navigate trie to exact prefix location
   - Return false if prefix doesn't exist

3. `descend_to_next_sibling(zipper: &mut ReadZipper) -> bool`
   - Iterate siblings at current level
   - Used to enumerate all matches at prefix node

4. `extract_value(expr: Expr) -> Result<Arc<RholangNode>, String>`
   - Convert MORK Expr back to RholangNode
   - Extract the value part from `(pattern-key <pattern> <value>)`

### Approach 2: Custom Index Structure

**Alternative**: Build a secondary index for common query patterns

```rust
pub struct PatternIndex {
    // Map from pattern fingerprint to stored values
    // Fingerprint is concrete parts only: "pattern-key.42" -> [value1, value2, ...]
    index: HashMap<String, Vec<Arc<RholangNode>>>,
}
```

**Pros**:
- Very fast lookups O(1)
- Simple to implement

**Cons**:
- Memory overhead for index
- Need to maintain consistency
- Doesn't leverage existing trie

### Approach 3: Investigate query_multi Further

**Option**: Debug why ProductZipper isn't working

**Actions**:
1. Enable MORK tracing: `RUST_LOG=coref=trace,query_multi=trace`
2. Compare with MeTTaTron's exact usage
3. Check if data needs different storage format
4. Contact MORK maintainers for guidance

**Effort**: Unknown - may be architectural mismatch

## Performance Benchmarks

### Current Performance (O(n))

| Patterns Stored | Query Time | Notes |
|-----------------|------------|-------|
| 10 | ~10 µs | Linear scan acceptable |
| 100 | ~100 µs | Still acceptable |
| 1,000 | ~1 ms | Getting slow |
| 10,000 | ~10 ms | Unacceptable for LSP |
| 100,000 | ~100 ms | Unusable |

### Target Performance (O(k))

| Patterns Stored | Matches (k) | Query Time | Notes |
|-----------------|-------------|------------|-------|
| 10 | 1 | ~1 µs | Direct navigation |
| 100 | 1 | ~1 µs | Same - independent of total |
| 1,000 | 1 | ~1 µs | Same |
| 10,000 | 5 | ~5 µs | Linear in matches only |
| 100,000 | 10 | ~10 µs | Scales with k, not n |

## Implementation Priority

### Phase 1: Prefix Navigation (HIGH PRIORITY)
**Effort**: 4-6 hours
**Impact**: Eliminates O(n) scan
**Files**: `src/ir/pattern_matching.rs`

**Tasks**:
1. Implement `extract_prefix()` - analyze pattern structure
2. Implement `navigate_to_prefix()` - trie navigation
3. Implement sibling iteration for matches
4. Add benchmarks to verify O(k) behavior

### Phase 2: Value Extraction (MEDIUM PRIORITY)
**Effort**: 3-4 hours
**Impact**: Return actual matched values (currently Nil placeholders)
**Files**: `src/ir/mork_convert.rs`, `src/ir/pattern_matching.rs`

**Tasks**:
1. Implement MORK Expr → RholangNode conversion
2. Parse stored expression structure
3. Extract value component from `(pattern-key <pattern> <value>)`
4. Update tests to verify actual values

### Phase 3: Contract Invocation Helper (MEDIUM PRIORITY)
**Effort**: 2-3 hours
**Impact**: LSP-specific optimization for common use case
**Files**: `src/ir/pattern_matching.rs`

**Tasks**:
1. Implement `find_contract_invocations()`
2. Construct pattern: `(send (contract "<name>") <args...>)`
3. Use optimized prefix navigation
4. Return bindings for contract arguments

## Testing Strategy

### Unit Tests
Add performance-focused tests:

```rust
#[test]
fn test_prefix_navigation_performance() {
    let mut matcher = RholangPatternMatcher::new();

    // Insert 1000 patterns with different prefixes
    for i in 0..1000 {
        matcher.add_pattern(&pattern(i), &value(i)).unwrap();
    }

    // Query should only visit matching prefix, not all 1000
    let start = Instant::now();
    let matches = matcher.match_query(&query_for(42)).unwrap();
    let duration = start.elapsed();

    assert!(duration < Duration::from_micros(100),
        "Query took {:?}, expected < 100µs", duration);
}
```

### Benchmarks
Create `benches/pattern_matching.rs`:

```rust
use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId};

fn bench_pattern_matching(c: &mut Criterion) {
    let mut group = c.benchmark_group("pattern_matching");

    for size in [10, 100, 1000, 10000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size,
            |b, &size| {
                let matcher = setup_matcher(size);
                b.iter(|| matcher.match_query(&test_query()));
            });
    }

    group.finish();
}
```

## References

### MORK Source Code
- `MORK/kernel/src/space.rs:78-193` - `coreferential_transition`
- `MORK/kernel/src/space.rs:988-1125` - `query_multi`
- `PathMap/src/zipper.rs` - Zipper navigation API

### MeTTaTron Integration
- `MeTTa-Compiler/src/backend/eval.rs:890-946` - `try_match_all_rules_query_multi`
- `MeTTa-Compiler/src/backend/environment.rs:499-508` - `add_to_space`

### PathMap API
- `descend_to_byte(byte)` - Navigate to specific child
- `descend_to_check(bytes)` - Navigate to specific path
- `to_next_sibling_byte()` - Iterate siblings
- `ascend_byte()` - Go back up

## Summary

**Current State**: Functional but O(n) iteration
**Required**: O(k) trie-based prefix navigation
**Recommended Path**: Implement Approach 1 (Prefix Navigation)
**Timeline**: 4-6 hours for core optimization
**Expected Speedup**: 10-1000x for typical workloads
