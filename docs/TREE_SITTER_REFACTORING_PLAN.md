# Tree-Sitter Module Refactoring Plan

## Overview

The `src/tree_sitter.rs` file (1,493 lines) is the bridge between Tree-Sitter parsing and the RholangNode IR system. It's critical infrastructure for both Rholang and future virtual language support.

**Current Structure:**
- Public API functions (3 functions)
- Helper functions (4 functions)
- Main conversion function `convert_ts_node_to_ir()` (**865 lines** - 50+ match cases)
- Operator helper functions (2 functions)
- Test module (393 lines with 18 tests)

**Key Challenge:** The giant `convert_ts_node_to_ir()` function is a 50+ case match statement that's difficult to navigate and maintain.

## Why This Matters for Virtual Languages

This refactoring is **critical** for the virtual language architecture because:

1. **Template for Other Languages**: This pattern will be replicated for MeTTa, SQL, etc.
2. **Code Reuse**: Common conversion patterns can be extracted and shared
3. **Maintainability**: Clear structure makes it easier to add new language constructs
4. **Testing**: Smaller modules = better unit testing per construct type

## Proposed Module Structure

```
src/tree_sitter/
├── mod.rs                      # Public API + re-exports (60 lines)
├── parsing.rs                  # Tree-Sitter parsing (50 lines)
├── helpers.rs                  # Collection helpers (150 lines)
├── conversion/
│   ├── mod.rs                  # Conversion dispatcher (100 lines)
│   ├── processes.rs            # Send, SendSync, New, Input, etc. (300 lines)
│   ├── control_flow.rs         # IfElse, Match, Choice (180 lines)
│   ├── expressions.rs          # BinOp, UnaryOp, Method, Eval, Quote (200 lines)
│   ├── literals.rs             # Bool, Long, String, Uri, Nil (120 lines)
│   ├── collections.rs          # List, Set, Map, Tuple (180 lines)
│   ├── bindings.rs             # Contract, Let, LinearBind, etc. (250 lines)
│   └── patterns.rs             # Var, Wildcard, SimpleType, etc. (100 lines)
└── tests/
    ├── mod.rs                  # Test utilities (20 lines)
    ├── parsing_tests.rs        # Basic parsing tests (150 lines)
    ├── position_tests.rs       # Position tracking tests (120 lines)
    └── conversion_tests.rs     # Conversion tests (120 lines)
```

**Total estimated lines:** ~1,900 (vs 1,493 original)
- The increase is due to better organization and module headers

## Detailed Module Breakdown

### 1. `mod.rs` - Public API (60 lines)

**Purpose:** Entry point with re-exports for backward compatibility

```rust
// Tree-Sitter parsing and IR conversion for Rholang
//
// This module bridges Tree-Sitter's Concrete Syntax Tree (CST) with our
// RholangNode Intermediate Representation (IR).

pub mod parsing;
pub mod helpers;
pub mod conversion;

#[cfg(test)]
mod tests;

// Re-export public API
pub use parsing::{parse_code, parse_to_ir, update_tree};
pub use helpers::{safe_byte_slice, collect_named_descendants, collect_patterns, collect_linear_binds, is_comment};
pub use conversion::convert_ts_node_to_ir;
```

**Lines in original:** 1-20, module structure
**Risk:** Low - simple re-exports

---

### 2. `parsing.rs` - Tree-Sitter Parsing (50 lines)

**Purpose:** Direct Tree-Sitter API interaction

**Functions:**
- `parse_code()` - lines 29-33
- `parse_to_ir()` - lines 35-44
- `update_tree()` - lines 46-64

**Content:**
```rust
use tree_sitter::{Parser, Tree, InputEdit};
use ropey::Rope;
use std::sync::Arc;
use crate::ir::rholang_node::RholangNode;
use super::conversion::convert_ts_node_to_ir;

/// Parse Rholang code to Tree-Sitter tree
pub fn parse_code(code: &str) -> Tree {
    let mut parser = Parser::new();
    parser.set_language(&rholang_tree_sitter::language()).expect("Error loading Rholang grammar");
    parser.parse(code, None).expect("Error parsing code")
}

/// Parse Tree-Sitter tree to RholangNode IR
pub fn parse_to_ir(tree: &Tree, rope: &Rope) -> Arc<RholangNode> {
    let root_node = tree.root_node();
    let initial_position = Position { row: 0, column: 0, byte: 0 };
    let (ir, _end) = convert_ts_node_to_ir(root_node, rope, initial_position);
    ir
}

/// Update Tree-Sitter tree with incremental edit
pub fn update_tree(
    old_tree: &mut Tree,
    old_code: &str,
    new_code: &str,
    start_byte: usize,
    old_end_byte: usize,
    new_end_byte: usize,
    start_position: tree_sitter::Point,
    old_end_position: tree_sitter::Point,
    new_end_position: tree_sitter::Point,
) {
    // ... implementation from lines 46-64
}
```

**Risk:** Low - straightforward extraction

---

### 3. `helpers.rs` - Collection and Utility Helpers (150 lines)

**Purpose:** Common helper functions for node collection and processing

**Functions:**
- `safe_byte_slice()` - lines 21-27
- `collect_named_descendants()` - lines 66-81
- `collect_patterns()` - lines 83-173
- `collect_linear_binds()` - lines 175-191
- `is_comment()` - lines 193-208

**Content:**
```rust
use tree_sitter::Node as TSNode;
use ropey::Rope;
use std::sync::Arc;
use rpds::Vector;
use archery::ArcK;
use crate::ir::rholang_node::{RholangNode, Position};
use super::conversion::convert_ts_node_to_ir;

/// Safely extract a byte slice from rope as a String
pub(crate) fn safe_byte_slice(rope: &Rope, start_byte: usize, end_byte: usize) -> String {
    // ... implementation
}

/// Collect all named descendants of a Tree-Sitter node
pub fn collect_named_descendants(
    ts_node: TSNode,
    rope: &Rope,
    prev_end: Position,
) -> (Vector<Arc<RholangNode>, ArcK>, Position) {
    // ... implementation
}

/// Collect patterns from a Tree-Sitter node (for formals/names)
pub fn collect_patterns(
    ts_node: TSNode,
    rope: &Rope,
    prev_end: Position,
) -> (Vector<Arc<RholangNode>, ArcK>, Option<Arc<RholangNode>>, Position) {
    // ... implementation (90 lines)
}

/// Collect linear binds from a Tree-Sitter branch node
pub fn collect_linear_binds(
    branch_node: TSNode,
    rope: &Rope,
    prev_end: Position,
) -> (Vector<Arc<RholangNode>, ArcK>, Position) {
    // ... implementation
}

/// Check if a Tree-Sitter node is a comment
pub fn is_comment(kind_id: u16) -> bool {
    // ... implementation
}
```

**Risk:** Low - these are already isolated functions

---

### 4. `conversion/mod.rs` - Conversion Dispatcher (100 lines)

**Purpose:** Main entry point for CST → IR conversion, delegates to specialized modules

**Key Function:**
- `convert_ts_node_to_ir()` - Dispatcher with **50+ case match statement**

**Strategy:** Keep the main match statement here, but delegate complex cases to submodules:

```rust
use tree_sitter::Node as TSNode;
use ropey::Rope;
use std::sync::Arc;
use std::collections::HashMap;
use std::any::Any;
use crate::ir::rholang_node::{RholangNode, NodeBase, Position, RelativePosition};
use super::helpers::{is_comment, collect_named_descendants};

mod processes;
mod control_flow;
mod expressions;
mod literals;
mod collections;
mod bindings;
mod patterns;

use processes::*;
use control_flow::*;
use expressions::*;
use literals::*;
use collections::*;
use bindings::*;
use patterns::*;

/// Converts Tree-Sitter nodes to IR nodes with accurate relative positions.
pub fn convert_ts_node_to_ir(
    ts_node: TSNode,
    rope: &Rope,
    prev_end: Position,
) -> (Arc<RholangNode>, Position) {
    // Common position calculation (lines 212-256)
    let absolute_start = Position { ... };
    let relative_start = RelativePosition { ... };
    let base = NodeBase::new(...);
    let metadata = Some(Arc::new(data));

    match ts_node.kind() {
        // Structural nodes (keep here - simple)
        "source_file" => convert_source_file(ts_node, rope, absolute_start, absolute_end, base, metadata),
        "collection" => { /* simple passthrough */ }
        "par" => convert_par(ts_node, rope, absolute_start, base, metadata),

        // Delegate to specialized modules
        "send" | "send_sync" => processes::convert_send(ts_node, rope, absolute_start, base, metadata),
        "new" => processes::convert_new(ts_node, rope, absolute_start, base, metadata),
        "input" => processes::convert_input(ts_node, rope, absolute_start, base, metadata),

        "ifElse" => control_flow::convert_if_else(ts_node, rope, absolute_start, base, metadata),
        "match" => control_flow::convert_match(ts_node, rope, absolute_start, base, metadata),
        "choice" => control_flow::convert_choice(ts_node, rope, absolute_start, base, metadata),

        "or" | "and" | "add" | "mult" | ... => expressions::convert_binary_op(...),
        "not" | "neg" => expressions::convert_unary_op(...),
        "method" => expressions::convert_method(ts_node, rope, absolute_start, base, metadata),
        "quote" => expressions::convert_quote(ts_node, rope, absolute_start, base, metadata),

        "bool_literal" => literals::convert_bool(ts_node, rope, absolute_end, base, metadata),
        "long_literal" => literals::convert_long(ts_node, rope, absolute_end, base, metadata),
        "string_literal" => literals::convert_string(ts_node, rope, absolute_end, base, metadata),

        "list" | "set" | "map" | "tuple" => collections::convert_collection(ts_node, rope, absolute_start, absolute_end, base, metadata),

        "contract" => bindings::convert_contract(ts_node, rope, absolute_start, base, metadata),
        "let" => bindings::convert_let(ts_node, rope, absolute_start, base, metadata),
        "linear_bind" => bindings::convert_linear_bind(ts_node, rope, absolute_start, base, metadata),

        "var" => patterns::convert_var(ts_node, rope, absolute_end, base, metadata),
        "wildcard" => patterns::convert_wildcard(absolute_end, base, metadata),

        // Error handling (keep here)
        "ERROR" => { /* ... */ }
        _ => { /* ... */ }
    }
}
```

**Risk:** Medium - Main dispatcher, but logic stays the same

---

### 5. `conversion/processes.rs` - Process Constructs (300 lines)

**Purpose:** Rholang-specific process constructs

**Convert functions for:**
- `send` (lines 441-487)
- `send_sync` (lines 394-422)
- `new` (lines 489-495)
- `input` (lines 597-610)
- Related helpers for continuation handling

**Estimated lines:** ~300
**Risk:** Low - well-defined conversions

---

### 6. `conversion/control_flow.rs` - Control Flow (180 lines)

**Purpose:** Control flow constructs

**Convert functions for:**
- `ifElse` (lines 497-516)
- `match` (lines 548-565)
- `choice` (lines 567-581)
- `bundle` (lines 526-546)

**Estimated lines:** ~180
**Risk:** Low - isolated logic

---

### 7. `conversion/expressions.rs` - Expressions (200 lines)

**Purpose:** Expression constructs

**Convert functions for:**
- Binary operators (using `binary_op()` helper - lines 692-710, 1078-1088)
- Unary operators (using `unary_op()` helper - lines 709-711, 771, 1090-1098)
- `method` (lines 711-731)
- `eval` (lines 733-737)
- `quote` (lines 739-751)
- `var_ref` (lines 753-767)

**Includes:**
- `binary_op()` helper (lines 1078-1088)
- `unary_op()` helper (lines 1090-1098)

**Estimated lines:** ~200
**Risk:** Low - clear operator patterns

---

### 8. `conversion/literals.rs` - Literal Values (120 lines)

**Purpose:** Ground literal types

**Convert functions for:**
- `bool_literal` (lines 776-780)
- `long_literal` (lines 782-813) - includes validation
- `string_literal` (lines 815-827) - includes escape handling
- `uri_literal` (lines 829-840)
- `nil` (lines 842-844)
- `unit` (lines 1051-1053)

**Estimated lines:** ~120
**Risk:** Low - straightforward conversions

---

### 9. `conversion/collections.rs` - Collections (180 lines)

**Purpose:** Collection types (list, set, map, tuple)

**Convert functions for:**
- `list` (lines 846-866)
- `set` (lines 868-887)
- `map` (lines 889-911)
- `tuple` (lines 913-923)

**Pattern:** All collections handle:
- Element iteration
- Remainder handling (`_proc_remainder`)
- Position tracking

**Estimated lines:** ~180
**Risk:** Low - similar patterns

---

### 10. `conversion/bindings.rs` - Bindings and Declarations (250 lines)

**Purpose:** Binding constructs (contract, let, decl, binds)

**Convert functions for:**
- `contract` (lines 583-595)
- `let` (lines 518-524)
- `decl` (lines 941-947)
- `name_decl` (lines 930-939)
- `linear_bind` (lines 949-968)
- `repeated_bind` (lines 970-989)
- `peek_bind` (lines 991-1010)
- `simple_source` (lines 1012-1014)
- `receive_send_source` (lines 1016-1020)
- `send_receive_source` (lines 1022-1035)

**Estimated lines:** ~250
**Risk:** Low - well-structured binding logic

---

### 11. `conversion/patterns.rs` - Pattern Constructs (100 lines)

**Purpose:** Pattern matching and variable constructs

**Convert functions for:**
- `var` (lines 925-928)
- `wildcard` (lines 1037-1039)
- `simple_type` (lines 1041-1044)
- `block` (lines 612-678)
- `_parenthesized` (lines 680-684)
- `_name_remainder` (lines 686-690)
- `_ground_expression` (lines 772-774)

**Estimated lines:** ~100
**Risk:** Low - simple conversions

---

### 12. Test Refactoring

**Current:** 393 lines in single test module (lines 1100-1493)

**Proposed Split:**

#### `tests/mod.rs` (20 lines)
```rust
mod parsing_tests;
mod position_tests;
mod conversion_tests;
```

#### `tests/parsing_tests.rs` (150 lines)
- `test_parse_send`
- `test_parse_new_nested`
- `test_parse_name_remainder`
- `test_parse_sync_send_empty_cont`
- `test_parse_sync_send_non_empty_cont`
- `test_parse_invalid_long_literal`
- `test_parse_valid_long_literal`
- `test_parse_string_literal_with_escapes`
- `test_parse_invalid_string_literal`
- `test_parse_uri_literal`
- `test_parse_parenthesized`
- `test_tree_sitter_extras_access`

#### `tests/position_tests.rs` (120 lines)
- `test_parse_par_position`
- `test_position_consistency` (QuickCheck property test)
- `test_debug_block_positions`

#### `tests/conversion_tests.rs` (120 lines)
- Future: tests for individual conversion functions
- Can add targeted tests per module

**Risk:** Low - tests remain unchanged, just reorganized

---

## Implementation Strategy

### Phase 1: Extract Helpers and Parsing (LOW RISK)
**Goal:** Extract already-isolated functions

1. Create `src/tree_sitter/` directory
2. Create `parsing.rs` with `parse_code()`, `parse_to_ir()`, `update_tree()`
3. Create `helpers.rs` with all helper functions
4. Create `mod.rs` with re-exports
5. Update imports in `src/lib.rs` and tests
6. **Verify:** Run `cargo test` - should pass immediately

**Estimated time:** 15 minutes
**Lines affected:** ~200 lines extracted

---

### Phase 2: Extract Test Modules (LOW RISK)
**Goal:** Organize tests before refactoring conversion logic

7. Create `tests/` directory under `src/tree_sitter/`
8. Create `mod.rs`, `parsing_tests.rs`, `position_tests.rs`
9. Move tests to appropriate files
10. **Verify:** Run `cargo test` - all tests should still pass

**Estimated time:** 10 minutes
**Lines affected:** 393 lines reorganized

---

### Phase 3: Create Conversion Modules (MEDIUM RISK)
**Goal:** Split the giant `convert_ts_node_to_ir()` match statement

11. Create `conversion/` directory
12. Create all conversion submodules (processes.rs, control_flow.rs, etc.)
13. Extract match cases to appropriate modules
14. Create public conversion functions in each module
15. Keep position calculation logic in main dispatcher
16. **Verify after EACH module:** Run targeted tests

**Estimated time:** 45 minutes
**Lines affected:** 865 lines split into 7 modules

---

### Phase 4: Update Main Dispatcher (MEDIUM RISK)
**Goal:** Refactor `convert_ts_node_to_ir()` to delegate

17. Create `conversion/mod.rs` with main dispatcher
18. Update match statement to call submodule functions
19. Move helpers (`binary_op`, `unary_op`) to `expressions.rs`
20. **Verify:** Run full test suite

**Estimated time:** 20 minutes
**Lines affected:** Main conversion function (100 lines in dispatcher)

---

### Phase 5: Final Integration (LOW RISK)
**Goal:** Clean up and verify

21. Update all imports across codebase
22. Remove backup files
23. Update documentation in module headers
24. Run full test suite
25. Git commit with detailed message

**Estimated time:** 10 minutes

---

## Total Estimated Effort

- **Total time:** ~2 hours
- **Complexity:** Medium
- **Risk level:** Medium (large refactor but well-structured)

---

## Testing Strategy

### After Each Phase:
```bash
# Quick validation
cargo check

# Run tree_sitter tests specifically
cargo test tree_sitter

# Run all tests
cargo test
```

### Critical Tests to Watch:
- `test_parse_send` - Basic conversion
- `test_parse_par_position` - Position tracking
- `test_position_consistency` - QuickCheck (100 cases)
- `test_parse_new_nested` - Nested structure
- All LSP integration tests (backend depends on tree_sitter)

---

## Risks and Mitigations

### Risk 1: Breaking Position Tracking
**Impact:** High - positions critical for LSP features
**Mitigation:**
- Keep position calculation in main dispatcher (don't duplicate)
- Run position tests after each phase
- Use `test_position_consistency` QuickCheck test (100 random cases)

### Risk 2: Breaking LSP Backend
**Impact:** High - backend depends on `parse_to_ir()`
**Mitigation:**
- Public API remains unchanged
- Run full test suite after each phase
- Test virtual language support explicitly

### Risk 3: Import Chain Complexity
**Impact:** Medium - circular dependencies
**Mitigation:**
- Use `pub(crate)` for internal helpers
- Keep clear import hierarchy: `mod.rs` → submodules
- Avoid cross-module dependencies between conversion modules

### Risk 4: Performance Regression
**Impact:** Low - parsing is hot path
**Mitigation:**
- Keep conversion logic identical (just reorganized)
- No new allocations or abstractions
- Run performance tests if available

---

## Success Criteria

1. ✅ All 204+ tests passing
2. ✅ No public API changes (backward compatible)
3. ✅ Largest file reduced from 1,493 lines to ~150 lines (dispatcher)
4. ✅ Clear module boundaries for adding new constructs
5. ✅ Position tracking still accurate
6. ✅ LSP backend still functional
7. ✅ Virtual language support unaffected

---

## Future Benefits

### For Virtual Language Architecture:

1. **Template Pattern:** Each virtual language (MeTTa, SQL) can follow this structure:
   ```
   src/parsers/
   ├── metta_parser/
   │   ├── mod.rs
   │   ├── parsing.rs
   │   ├── helpers.rs
   │   └── conversion/
   │       ├── mod.rs
   │       ├── expressions.rs
   │       ├── definitions.rs
   │       └── types.rs
   ├── sql_parser/
   │   └── ... (same structure)
   ```

2. **Shared Utilities:** Extract common patterns:
   - Position tracking logic
   - Collection helpers
   - Error handling

3. **Better Testing:** Each language construct can have targeted tests

4. **Easier Debugging:** Stack traces point to specific modules, not line 847 of tree_sitter.rs

5. **Parallel Development:** Multiple developers can work on different modules

---

## Alternative Approaches Considered

### Alternative 1: Keep as Single File
**Pros:** Simple, no import complexity
**Cons:** Already 1,493 lines, will grow with virtual languages
**Decision:** Rejected - not scalable

### Alternative 2: Split by Node Type (Process vs Expression)
**Pros:** Two big modules instead of seven small ones
**Cons:** Still 400-500 lines per module, less focused
**Decision:** Rejected - not granular enough

### Alternative 3: One File Per Match Case (50+ files)
**Pros:** Maximum granularity
**Cons:** Too many small files, import nightmare
**Decision:** Rejected - over-engineering

### Alternative 4: Current Proposal (7 conversion modules)
**Pros:**
- Natural groupings by language construct type
- ~100-300 lines per module (digestible)
- Clear boundaries
- Easy to navigate
**Decision:** ✅ **Selected**

---

## Next Steps

After this refactoring is complete:

1. **Apply pattern to `visitor.rs`** (Task B - next in sequence)
2. **Refactor `backend.rs`** (Task C - big integration)
3. **Use as template for MeTTa parser**
4. **Extract shared utilities for all parsers**

---

## Questions for Review

1. ✅ Module structure makes sense?
2. ✅ Groupings are logical (processes, control_flow, expressions, etc.)?
3. ✅ Phase order is safe (helpers first, conversion last)?
4. ✅ Risk mitigation is adequate?
5. ✅ Test strategy is comprehensive?

---

## Conclusion

This refactoring will:
- ✅ Reduce main file from 1,493 to ~150 lines (90% reduction)
- ✅ Create clear module boundaries
- ✅ Serve as template for virtual language parsers
- ✅ Maintain backward compatibility
- ✅ Keep all tests passing

**Recommendation:** Proceed with implementation following the phased approach.
