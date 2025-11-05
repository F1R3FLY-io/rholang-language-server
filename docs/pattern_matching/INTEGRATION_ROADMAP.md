# MORK/PathMap Integration Roadmap

**Last Updated**: 2025-11-04
**Current Status**: Steps 1-2D Complete ✅
**Next Step**: Step 3 - LSP Backend Integration

---

## Overview

This document outlines what remains to be done to complete the MORK+PathMap pattern matching integration for Rholang contract goto-definition with parameter-aware matching and overload resolution.

---

## Completed Work (Steps 1-2D)

### ✅ Step 1: MORK Canonical Serialization

**File**: `src/ir/mork_canonical.rs` (~1,678 lines)

**Completed**:
- `MorkForm` enum covering all Rholang constructs
- `to_mork_bytes()` serialization (100% working, 9/16 tests passing)
- Pattern-specific variants (MapPattern, ListPattern, etc.)
- Deterministic serialization for reliable pattern matching

**Deferred** (intentionally):
- MORK deserialization via `from_mork_bytes()` (7 tests marked as `#[ignore]`)
- Not needed for pattern matching use case (only serialization required)

---

### ✅ Step 2A: PathMap API Integration

**File**: `src/ir/rholang_pattern_index.rs` (756 lines)

**Completed**:
- Correct PathMap zipper creation patterns (`map.write_zipper()`, `map.read_zipper()`)
- Required trait imports (`ZipperMoving`, `ZipperValues`, `ZipperWriting`)
- Position serialization support (`Position` now derives `Serialize`/`Deserialize`)
- PathMap storage structure: `["contract", <name>, <param0_bytes>, <param1_bytes>, ...]`

---

### ✅ Step 2B: Pattern Extraction Helpers

**File**: `src/ir/rholang_pattern_index.rs` (lines 252-667)

**Completed**:
- `extract_contract_signature()` - Extract name and parameters from contracts
- `rholang_node_to_mork()` - Convert RholangNode to MorkForm (237 lines)
- `rholang_pattern_to_mork()` - Pattern-specific MorkForm conversion (95 lines)
- `pattern_to_mork_bytes()` / `node_to_mork_bytes()` - Wrapper functions
- `extract_param_names()` - Optional parameter name extraction

**Coverage**: All major RholangNode variants (literals, variables, collections, processes)

---

### ✅ Step 2C: Basic Unit Tests

**Files**: `src/ir/rholang_pattern_index.rs`, `src/ir/global_index.rs`

**Completed**:
- 6 tests in `rholang_pattern_index.rs`:
  - MORK serialization round-trips
  - Deterministic serialization
  - Index creation
- 8 tests in `global_index.rs`:
  - Index creation and clearing
  - Contract definition storage
  - Map key pattern matching

**Status**: 14/14 tests passing ✅

---

### ✅ Step 2D: GlobalSymbolIndex Integration

**File**: `src/ir/global_index.rs` (+135 lines)

**Completed**:
- Added `pattern_index: RholangPatternIndex` field to `GlobalSymbolIndex`
- Wrapper methods:
  - `add_contract_with_pattern_index(&contract_node, location)`
  - `query_contract_by_pattern(name, &arguments)`
- LSP ↔ IR type conversions (SymbolLocation)
- Manual Debug implementation for `RholangPatternIndex`

**Status**: Zero regressions, all existing tests still passing ✅

---

## Remaining Work

### 🔲 Step 3: LSP Backend Integration (NEXT)

**Estimated Time**: 2-3 hours

**Objective**: Wire up the pattern index in the goto-definition handler to enable parameter-aware contract lookups.

#### 3.1 Identify Goto-Definition Handler Location

**Files to check**:
- `src/lsp/backend.rs`
- `src/lsp/features/goto_definition.rs`
- `src/lsp/backend/handlers.rs`

**Action**: Find where goto-definition for contract calls is currently implemented.

#### 3.2 Extract Contract Call Information

**Implementation**:
```rust
// In goto-definition handler, when positioned on a contract call:

// Extract contract name from the call site
let contract_name = match node {
    RholangNode::Send { chan, .. } => {
        // Extract name from channel (e.g., "echo" from echo!(...))
        extract_contract_name(chan)?
    }
    _ => return None,
};

// Extract arguments from the send node
let arguments = match node {
    RholangNode::Send { data, .. } => {
        // Get the list of arguments
        data.as_slice()
    }
    _ => return None,
};
```

**Helper Function Needed**:
```rust
fn extract_contract_name(chan: &Arc<RholangNode>) -> Option<String> {
    match chan.as_ref() {
        RholangNode::Var(name) => Some(name.clone()),
        RholangNode::Quote(process) => {
            // Handle @"contractName" pattern
            if let RholangNode::Ground(Ground::String(s)) = process.as_ref() {
                Some(s.clone())
            } else {
                None
            }
        }
        _ => None,
    }
}
```

#### 3.3 Query Pattern Index

**Implementation**:
```rust
// In goto-definition handler

use crate::ir::global_index::GlobalSymbolIndex;

// Query the pattern index
let matches = self.workspace.global_index
    .query_contract_by_pattern(&contract_name, arguments)?;

if !matches.is_empty() {
    // Convert IR SymbolLocations to LSP Locations
    return Ok(Some(GotoDefinitionResponse::Array(
        matches.into_iter()
            .map(|sym_loc| lsp_types::Location {
                uri: Url::parse(&sym_loc.uri).unwrap(),
                range: lsp_types::Range {
                    start: lsp_types::Position {
                        line: sym_loc.start.row,
                        character: sym_loc.start.column,
                    },
                    end: lsp_types::Position {
                        line: sym_loc.end.row,
                        character: sym_loc.end.column,
                    },
                },
            })
            .collect()
    )));
}
```

#### 3.4 Add Fallback Logic

**Implementation**:
```rust
// If pattern-based lookup finds nothing, fall back to name-only lookup
if matches.is_empty() {
    // Existing name-based symbol table lookup
    matches = self.workspace.global_index.get_contract_by_name(&contract_name)?;
}
```

**Rationale**: Ensures backward compatibility and handles cases where pattern matching isn't applicable.

#### 3.5 Update Contract Indexing

**Location**: Wherever contracts are currently indexed (likely in `src/lsp/backend.rs` during workspace indexing)

**Implementation**:
```rust
// When indexing a document with contracts:

for contract in contracts {
    // Convert LSP Position to IR Position
    let ir_location = SymbolLocation {
        uri: document_uri.clone(),
        start: Position {
            row: lsp_start.line,
            column: lsp_start.character,
            byte: /* calculate or use 0 */,
        },
        end: Position {
            row: lsp_end.line,
            column: lsp_end.character,
            byte: /* calculate or use 0 */,
        },
    };

    // Add to pattern index
    global_index.add_contract_with_pattern_index(&contract_node, ir_location)?;
}
```

#### 3.6 Testing

**Create integration test**: `tests/test_pattern_matching_goto_definition.rs`

```rust
use test_utils::with_lsp_client;
use test_utils::lsp::client::{CommType, LspClient};
use tower_lsp::lsp_types::Position;

with_lsp_client!(test_pattern_matching_exact_arity, CommType::Stdio, |client: &LspClient| {
    let source = r#"
        new echo1, echo2, stdout(`rho:io:stdout`) in {
            // Define two overloaded contracts
            contract echo1(@x) = { stdout!(x) }
            contract echo1(@x, @y) = { stdout!((x, y)) }

            // Call with one argument - should match first definition
            echo1!(42)
        }
    "#;

    let doc = client.open_document("/test/overload.rho", source)
        .expect("Failed to open document");

    let _diagnostics = client.await_diagnostics(&doc)
        .expect("Failed to receive diagnostics");

    // Position on "echo1" in the call `echo1!(42)`
    let call_position = Position { line: 6, character: 12 };

    let locations = client.definition_all(&doc.uri(), call_position)
        .expect("goto_definition failed");

    assert_eq!(locations.len(), 1, "Should find exactly one match");
    assert_eq!(locations[0].range.start.line, 3, "Should match echo1(@x) not echo1(@x, @y)");

    client.close_document(&doc).expect("Failed to close document");
});
```

**Test cases to cover**:
1. Exact arity matching (1-arg call matches 1-param contract)
2. Overload resolution (distinguish `echo(@x)` from `echo(@x, @y)`)
3. Complex patterns (map patterns, list patterns)
4. Fallback to name-only lookup when pattern matching fails

---

### 🔲 Step 4: Advanced Pattern Matching (Optional Future Work)

**Estimated Time**: 5-8 hours

**Not required for basic functionality**, but would enable more sophisticated matching:

#### 4.1 MORK Unification

**Objective**: Support variable and wildcard pattern matching

**Example**:
```rholang
contract process(@x, @y) = { ... }  // Pattern with variables

// These calls should all match:
process!(42, "hello")    // Literal arguments unify with variables
process!(@z, @w)         // Variables unify with variables
process!(_, "hello")     // Wildcard unifies with anything
```

**Implementation**: Requires extending `RholangPatternIndex::query_call_site()` to use MORK's unification capabilities instead of exact byte matching.

#### 4.2 Remainder Patterns

**Objective**: Support `...rest` style patterns

**Example**:
```rholang
contract process(@first, ...@rest) = { ... }

// Should match:
process!(1)           // first=1, rest=[]
process!(1, 2, 3)     // first=1, rest=[2, 3]
```

**Implementation**: Extend `MorkForm` enum with `Remainder` variant, update serialization logic.

#### 4.3 Map Key Navigation (commented-out test)

**Objective**: Enable goto-definition for map literal keys in contract invocations

**Example**:
```rholang
contract processComplex(@{
  user: {name: n, email: e},  // <-- "email:" pattern key
  ...
}, ret)

// Clicking "email" here should jump to "email: e" above
processComplex!({
  "user": {"name": "Bob", "email": "bob@example.com"},
  ...
})
```

**Test Location**: `tests/test_complex_quote_patterns.rs:277-353` (currently commented out)

**Implementation**: Requires building a PathMap path for map keys and using structural analysis to link literal keys to pattern keys.

---

## Timeline Estimates

| Step | Description | Estimated Time | Priority |
|------|-------------|----------------|----------|
| **3.1** | Find goto-definition handler | 15 min | 🔴 Critical |
| **3.2** | Extract contract call info | 30 min | 🔴 Critical |
| **3.3** | Query pattern index | 30 min | 🔴 Critical |
| **3.4** | Add fallback logic | 15 min | 🔴 Critical |
| **3.5** | Update contract indexing | 45 min | 🔴 Critical |
| **3.6** | Integration tests | 60 min | 🔴 Critical |
| **Total Step 3** | | **3 hours** | |
| | | | |
| **4.1** | MORK unification | 3-4 hours | 🟡 Optional |
| **4.2** | Remainder patterns | 2-3 hours | 🟡 Optional |
| **4.3** | Map key navigation | 3-4 hours | 🟡 Optional |
| **Total Step 4** | | **8-11 hours** | |

---

## Success Criteria

### Step 3 (LSP Integration) - Required

- ✅ Goto-definition works for contract calls with exact arity matching
- ✅ Overloaded contracts are disambiguated by parameter count
- ✅ Complex patterns (maps, lists, tuples) are matched correctly
- ✅ Fallback to name-only lookup works when pattern matching fails
- ✅ All existing goto-definition tests still pass (zero regressions)
- ✅ New integration tests demonstrate pattern-aware matching

### Step 4 (Advanced Patterns) - Optional

- Variable/wildcard unification works across patterns
- Remainder patterns are recognized and matched
- Map literal keys can navigate to pattern keys
- Performance remains acceptable (<100ms for typical lookups)

---

## Files Requiring Changes

### Step 3 (Required)

| File | Changes Needed | Estimated Lines |
|------|----------------|-----------------|
| `src/lsp/features/goto_definition.rs` | Add pattern-based lookup | +50-80 |
| `src/lsp/backend.rs` | Update contract indexing | +20-30 |
| `tests/test_pattern_matching_goto_definition.rs` | New integration tests | +200-300 |

**Total**: ~270-410 new lines

### Step 4 (Optional)

| File | Changes Needed | Estimated Lines |
|------|----------------|-----------------|
| `src/ir/rholang_pattern_index.rs` | MORK unification logic | +100-150 |
| `src/ir/mork_canonical.rs` | Remainder pattern support | +50-80 |
| `src/lsp/features/goto_definition.rs` | Map key navigation | +80-120 |

**Total**: ~230-350 additional lines

---

## Testing Strategy

### Unit Tests (Existing ✅)

- `src/ir/rholang_pattern_index.rs` - 6 tests (pattern serialization, index creation)
- `src/ir/global_index.rs` - 8 tests (index operations, type conversions)

### Integration Tests (New - Required for Step 3)

**File**: `tests/test_pattern_matching_goto_definition.rs`

Test cases:
1. `test_exact_arity_matching` - 1-arg call matches 1-param contract
2. `test_overload_resolution` - Distinguish contracts by arity
3. `test_map_pattern_matching` - Match map patterns exactly
4. `test_list_pattern_matching` - Match list patterns exactly
5. `test_tuple_pattern_matching` - Match tuple patterns exactly
6. `test_fallback_to_name_lookup` - Name-only lookup when patterns differ

### Integration Tests (Optional - Step 4)

**File**: `tests/test_advanced_pattern_matching.rs`

Test cases:
1. `test_variable_unification` - Variables match literals
2. `test_wildcard_matching` - Wildcards match anything
3. `test_remainder_patterns` - `...rest` patterns work
4. `test_map_key_navigation` - Uncomment and fix `test_pathmap_pattern_goto_definition`

---

## Risk Mitigation

### Performance Concerns

**Risk**: Pattern matching adds overhead to goto-definition

**Mitigation**:
- PathMap lookups are O(path_length), not O(n contracts)
- MORK serialization is cached (deterministic bytes)
- Fallback ensures existing name-based lookup still works
- Benchmark before/after to verify <100ms for typical cases

### Backward Compatibility

**Risk**: Breaking existing goto-definition behavior

**Mitigation**:
- All existing tests must pass (zero regressions)
- Fallback to name-only lookup preserves old behavior
- Pattern matching is additive, not replacing existing logic

### Edge Cases

**Risk**: Unexpected pattern structures break matching

**Mitigation**:
- Comprehensive test coverage for all node types
- Defensive programming (return None on unrecognized patterns)
- Error logging for debugging failures

---

## Documentation Updates Needed

After Step 3 completion:

1. **`docs/pattern_matching/README.md`**:
   - Update "Implementation Status" to mark Step 3 complete
   - Add "LSP Integration" section with examples
   - Update "Next Steps" to remove Step 3

2. **`docs/pattern_matching/implementation/05_lsp_integration.md`**:
   - Create new document describing Step 3 implementation
   - Include code snippets and design decisions

3. **`docs/pattern_matching/DOCUMENTATION_STATUS.md`**:
   - Add verification entry for Step 3
   - Update "Next Review" date

4. **Update project README.md**:
   - Add note about pattern-aware goto-definition feature
   - Include example demonstrating overload resolution

---

## Getting Started

To begin Step 3 implementation:

```bash
# 1. Ensure current state is clean
cargo test --all
# Expected: 547 passed, 9 skipped

# 2. Find goto-definition handler
rg "goto_definition" src/lsp --type rust
rg "GotoDefinitionResponse" src/lsp --type rust

# 3. Create test file
touch tests/test_pattern_matching_goto_definition.rs

# 4. Start with simplest test case
# Add test for exact arity matching (1 param vs 2 params)

# 5. Implement minimal integration
# Just wire up query_contract_by_pattern() call

# 6. Iterate until tests pass
cargo test --test test_pattern_matching_goto_definition
```

---

## Questions? Issues?

- **Documentation**: See `docs/pattern_matching/guides/mork_and_pathmap_integration.md` for API details
- **Implementation examples**: Check `src/ir/rholang_pattern_index.rs` for existing patterns
- **Test examples**: See `tests/test_complex_quote_patterns.rs` for LSP test structure

---

**Status**: Ready for Step 3 implementation ✅
**Next Action**: Implement LSP backend integration (estimated 3 hours)
**Last Updated**: 2025-11-04
