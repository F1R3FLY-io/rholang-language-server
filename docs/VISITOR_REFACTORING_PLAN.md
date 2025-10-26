# Visitor Module Refactoring Plan

## Overview

The `src/ir/visitor.rs` file (1,601 lines) implements the Visitor pattern for traversing and transforming the RholangNode IR tree. It's a single trait with **42 visitor methods** - one for each RholangNode variant.

**Current Structure:**
- Single `Visitor` trait (1,601 lines)
- 1 dispatcher method: `visit_node()` (50-line match statement)
- 42 visitor methods (average ~35 lines each)
- All methods follow consistent pattern: visit children → check changes → reconstruct or return original

**Key Challenge:** While the file is already well-structured, 1,601 lines makes it hard to navigate. The pattern is highly repetitive but difficult to reduce.

## Pattern Analysis

Every visitor method follows this pattern:

```rust
fn visit_foo(
    &self,
    node: &Arc<RholangNode>,
    base: &NodeBase,
    child1: &Arc<RholangNode>,
    child2: &Vector<Arc<RholangNode>, ArcK>,
    metadata: &Option<Arc<Metadata>>,
) -> Arc<RholangNode> {
    // 1. Visit all children
    let new_child1 = self.visit_node(child1);
    let new_child2 = child2.iter().map(|c| self.visit_node(c)).collect();

    // 2. Check if any child changed (using Arc::ptr_eq)
    if Arc::ptr_eq(child1, &new_child1) &&
       child2.iter().zip(new_child2.iter()).all(|(a, b)| Arc::ptr_eq(a, b)) {
        Arc::clone(node)  // No changes - return original
    } else {
        // 3. Reconstruct node with new children
        Arc::new(RholangNode::Foo {
            base: base.clone(),
            child1: new_child1,
            child2: new_child2,
            metadata: metadata.clone(),
        })
    }
}
```

## Why This Matters for Virtual Languages

This refactoring is **critical** for the virtual language architecture because:

1. **Template for Other Languages**: MeTTa, SQL, etc. will need similar visitor traits (MettaVisitor, SqlVisitor)
2. **Shared Pattern**: The visit-check-reconstruct pattern is universal across languages
3. **Transform Pipeline**: Visitors are the foundation of the IR pipeline system
4. **Code Generation**: We might generate visitor code from language schemas

## Proposed Module Structure

```
src/ir/visitor/
├── mod.rs                          # Trait definition + dispatcher (80 lines)
├── processes.rs                    # Process visitor methods (400 lines)
├── control_flow.rs                 # Control flow visitor methods (250 lines)
├── expressions.rs                  # Expression visitor methods (300 lines)
├── literals.rs                     # Literal visitor methods (200 lines)
├── collections.rs                  # Collection visitor methods (250 lines)
├── bindings.rs                     # Binding visitor methods (320 lines)
└── patterns.rs                     # Pattern visitor methods (200 lines)
```

**Total estimated lines:** ~2,000 (vs 1,601 original)
- The increase is due to module headers and trait partial implementations

## Detailed Module Breakdown

### 1. `mod.rs` - Trait Definition + Dispatcher (80 lines)

**Purpose:** Define the core trait and dispatch to specialized modules

```rust
//! Visitor pattern for Rholang IR traversal and transformation
//!
//! The Visitor trait provides methods for visiting each RholangNode variant,
//! enabling tree transformations while preserving structural sharing.

use std::sync::Arc;
use rpds::Vector;
use archery::ArcK;
use super::rholang_node::{RholangNode, RholangNodeVector, Metadata, ...};
use super::semantic_node::{NodeBase, RelativePosition};

// Re-export visitor method implementations
mod processes;
mod control_flow;
mod expressions;
mod literals;
mod collections;
mod bindings;
mod patterns;

/// Visitor trait for traversing and transforming RholangNode IR trees.
///
/// All methods have default implementations that preserve the tree structure.
/// Override specific methods to implement custom transformations.
///
/// # Pattern
/// Each visitor method:
/// 1. Visits all child nodes recursively
/// 2. Checks if any children changed using Arc::ptr_eq
/// 3. Returns original node if unchanged, new node if changed
///
/// This pattern enables efficient structural sharing via Arc.
pub trait Visitor:
    processes::ProcessVisitor +
    control_flow::ControlFlowVisitor +
    expressions::ExpressionVisitor +
    literals::LiteralVisitor +
    collections::CollectionVisitor +
    bindings::BindingVisitor +
    patterns::PatternVisitor
{
    /// Entry point for visiting an IR node, dispatching to the appropriate method.
    ///
    /// # Arguments
    /// * node - The node to visit
    ///
    /// # Returns
    /// The transformed node, or the original if unchanged
    fn visit_node(&self, node: &Arc<RholangNode>) -> Arc<RholangNode> {
        match &**node {
            RholangNode::Par { base, left: Some(left), right: Some(right), processes: None, metadata } =>
                self.visit_par(node, base, left, right, metadata),
            RholangNode::Par { base, processes: Some(procs), metadata, .. } =>
                self.visit_par_nary(node, base, procs, metadata),
            RholangNode::Par { .. } => Arc::clone(node),

            RholangNode::SendSync { base, channel, inputs, cont, metadata } =>
                self.visit_send_sync(node, base, channel, inputs, cont, metadata),
            RholangNode::Send { base, channel, send_type, send_type_delta, inputs, metadata } =>
                self.visit_send(node, base, channel, send_type, send_type_delta, inputs, metadata),
            // ... all 42 variants
        }
    }
}
```

**Trait Composition Strategy:**
Instead of one giant trait, compose multiple smaller traits:
- `ProcessVisitor` - Process-related methods
- `ControlFlowVisitor` - Control flow methods
- `ExpressionVisitor` - Expression methods
- `LiteralVisitor` - Literal methods
- `CollectionVisitor` - Collection methods
- `BindingVisitor` - Binding methods
- `PatternVisitor` - Pattern methods

The main `Visitor` trait inherits all of them.

**Risk:** Medium - Trait composition can be tricky, but well-defined

---

### 2. `processes.rs` - Process Visitor Methods (400 lines)

**Purpose:** Visitor methods for process constructs

```rust
use std::sync::Arc;
use rpds::Vector;
use archery::ArcK;
use super::super::rholang_node::{RholangNode, ...};
use super::super::semantic_node::{NodeBase, RelativePosition};

/// Visitor methods for Rholang process constructs
pub trait ProcessVisitor {
    /// Main dispatcher (required for recursion)
    fn visit_node(&self, node: &Arc<RholangNode>) -> Arc<RholangNode>;

    /// Visits a parallel composition node (Par)
    fn visit_par(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        left: &Arc<RholangNode>,
        right: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        // Implementation (lines 87-109 from original)
    }

    /// Visits an n-ary parallel composition node
    fn visit_par_nary(...) -> Arc<RholangNode> {
        // Implementation (lines 124-152)
    }

    /// Visits a synchronous send node (SendSync)
    fn visit_send_sync(...) -> Arc<RholangNode> {
        // Implementation (lines 169-194)
    }

    /// Visits an asynchronous send node (Send)
    fn visit_send(...) -> Arc<RholangNode> {
        // Implementation (lines 212-237)
    }

    /// Visits a new name declaration node (New)
    fn visit_new(...) -> Arc<RholangNode> {
        // Implementation (lines 253-274)
    }

    /// Visits an input/receive node (Input)
    fn visit_input(...) -> Arc<RholangNode> {
        // Implementation (lines 521-558)
    }

    /// Visits a block node (Block)
    fn visit_block(...) -> Arc<RholangNode> {
        // Implementation (lines 558-590)
    }
}
```

**Methods included:**
- `visit_par()` - lines 87-109
- `visit_par_nary()` - lines 124-152
- `visit_send_sync()` - lines 169-194
- `visit_send()` - lines 212-237
- `visit_new()` - lines 253-274
- `visit_input()` - lines 521-558
- `visit_block()` - lines 558-590

**Estimated lines:** ~400
**Risk:** Low - clear process grouping

---

### 3. `control_flow.rs` - Control Flow Visitor Methods (250 lines)

**Purpose:** Visitor methods for control flow constructs

```rust
pub trait ControlFlowVisitor {
    fn visit_node(&self, node: &Arc<RholangNode>) -> Arc<RholangNode>;

    /// Visits a conditional node (IfElse)
    fn visit_ifelse(...) -> Arc<RholangNode> {
        // Implementation (lines 291-331)
    }

    /// Visits a pattern matching node (Match)
    fn visit_match(...) -> Arc<RholangNode> {
        // Implementation (lines 403-438)
    }

    /// Visits a choice node (Choice)
    fn visit_choice(...) -> Arc<RholangNode> {
        // Implementation (lines 438-476)
    }

    /// Visits a bundle node (Bundle)
    fn visit_bundle(...) -> Arc<RholangNode> {
        // Implementation (lines 368-403)
    }
}
```

**Methods included:**
- `visit_ifelse()` - lines 291-331
- `visit_match()` - lines 403-438
- `visit_choice()` - lines 438-476
- `visit_bundle()` - lines 368-403

**Estimated lines:** ~250
**Risk:** Low - isolated control flow logic

---

### 4. `expressions.rs` - Expression Visitor Methods (300 lines)

**Purpose:** Visitor methods for expression constructs

```rust
pub trait ExpressionVisitor {
    fn visit_node(&self, node: &Arc<RholangNode>) -> Arc<RholangNode>;

    /// Visits a binary operation node (BinOp)
    fn visit_binop(...) -> Arc<RholangNode> {
        // Implementation (lines 624-662)
    }

    /// Visits a unary operation node (UnaryOp)
    fn visit_unaryop(...) -> Arc<RholangNode> {
        // Implementation (lines 662-698)
    }

    /// Visits a method call node (Method)
    fn visit_method(...) -> Arc<RholangNode> {
        // Implementation (lines 698-735)
    }

    /// Visits an eval node (Eval)
    fn visit_eval(...) -> Arc<RholangNode> {
        // Implementation (lines 735-767)
    }

    /// Visits a quote node (Quote)
    fn visit_quote(...) -> Arc<RholangNode> {
        // Implementation (lines 767-800)
    }

    /// Visits a variable reference node (VarRef)
    fn visit_varref(...) -> Arc<RholangNode> {
        // Implementation (lines 800-834)
    }

    /// Visits a parenthesized expression (Parenthesized)
    fn visit_parenthesized(...) -> Arc<RholangNode> {
        // Implementation (lines 590-624)
    }

    /// Visits a disjunction pattern node (Disjunction)
    fn visit_disjunction(...) -> Arc<RholangNode> {
        // Implementation (lines 1491-1511)
    }

    /// Visits a conjunction pattern node (Conjunction)
    fn visit_conjunction(...) -> Arc<RholangNode> {
        // Implementation (lines 1527-1547)
    }

    /// Visits a negation pattern node (Negation)
    fn visit_negation(...) -> Arc<RholangNode> {
        // Implementation (lines 1562-1579)
    }
}
```

**Methods included:**
- `visit_binop()` - lines 624-662
- `visit_unaryop()` - lines 662-698
- `visit_method()` - lines 698-735
- `visit_eval()` - lines 735-767
- `visit_quote()` - lines 767-800
- `visit_varref()` - lines 800-834
- `visit_parenthesized()` - lines 590-624
- `visit_disjunction()` - lines 1491-1511
- `visit_conjunction()` - lines 1527-1547
- `visit_negation()` - lines 1562-1579

**Estimated lines:** ~300
**Risk:** Low - expression logic is self-contained

---

### 5. `literals.rs` - Literal Visitor Methods (200 lines)

**Purpose:** Visitor methods for literal value constructs

```rust
pub trait LiteralVisitor {
    fn visit_node(&self, node: &Arc<RholangNode>) -> Arc<RholangNode>;

    /// Visits a boolean literal node (BoolLiteral)
    fn visit_bool_literal(...) -> Arc<RholangNode> {
        // Implementation (lines 834-857)
    }

    /// Visits a long integer literal node (LongLiteral)
    fn visit_long_literal(...) -> Arc<RholangNode> {
        // Implementation (lines 857-880)
    }

    /// Visits a string literal node (StringLiteral)
    fn visit_string_literal(...) -> Arc<RholangNode> {
        // Implementation (lines 880-903)
    }

    /// Visits a URI literal node (UriLiteral)
    fn visit_uri_literal(...) -> Arc<RholangNode> {
        // Implementation (lines 903-925)
    }

    /// Visits a nil node (Nil)
    fn visit_nil(...) -> Arc<RholangNode> {
        // Implementation (lines 925-948)
    }

    /// Visits a unit value node (Unit)
    fn visit_unit(...) -> Arc<RholangNode> {
        // Implementation (lines 1593-1600)
    }
}
```

**Methods included:**
- `visit_bool_literal()` - lines 834-857
- `visit_long_literal()` - lines 857-880
- `visit_string_literal()` - lines 880-903
- `visit_uri_literal()` - lines 903-925
- `visit_nil()` - lines 925-948
- `visit_unit()` - lines 1593-1600

**Estimated lines:** ~200
**Risk:** Low - simple leaf node visitors

---

### 6. `collections.rs` - Collection Visitor Methods (250 lines)

**Purpose:** Visitor methods for collection types

```rust
pub trait CollectionVisitor {
    fn visit_node(&self, node: &Arc<RholangNode>) -> Arc<RholangNode>;

    /// Visits a list node (List)
    fn visit_list(...) -> Arc<RholangNode> {
        // Implementation (lines 948-985)
    }

    /// Visits a set node (Set)
    fn visit_set(...) -> Arc<RholangNode> {
        // Implementation (lines 985-1022)
    }

    /// Visits a map node (Map)
    fn visit_map(...) -> Arc<RholangNode> {
        // Implementation (lines 1022-1058)
    }

    /// Visits a tuple node (Tuple)
    fn visit_tuple(...) -> Arc<RholangNode> {
        // Implementation (lines 1058-1090)
    }
}
```

**Methods included:**
- `visit_list()` - lines 948-985
- `visit_set()` - lines 985-1022
- `visit_map()` - lines 1022-1058
- `visit_tuple()` - lines 1058-1090

**Estimated lines:** ~250
**Risk:** Low - collection logic is isolated

---

### 7. `bindings.rs` - Binding Visitor Methods (320 lines)

**Purpose:** Visitor methods for binding and declaration constructs

```rust
pub trait BindingVisitor {
    fn visit_node(&self, node: &Arc<RholangNode>) -> Arc<RholangNode>;

    /// Visits a contract node (Contract)
    fn visit_contract(...) -> Arc<RholangNode> {
        // Implementation (lines 476-521)
    }

    /// Visits a let binding node (Let)
    fn visit_let(...) -> Arc<RholangNode> {
        // Implementation (lines 331-368)
    }

    /// Visits a name declaration node (NameDecl)
    fn visit_name_decl(...) -> Arc<RholangNode> {
        // Implementation (lines 1114-1157)
    }

    /// Visits a declaration node (Decl)
    fn visit_decl(...) -> Arc<RholangNode> {
        // Implementation (lines 1157-1199)
    }

    /// Visits a linear bind node (LinearBind)
    fn visit_linear_bind(...) -> Arc<RholangNode> {
        // Implementation (lines 1199-1241)
    }

    /// Visits a repeated bind node (RepeatedBind)
    fn visit_repeated_bind(...) -> Arc<RholangNode> {
        // Implementation (lines 1241-1283)
    }

    /// Visits a peek bind node (PeekBind)
    fn visit_peek_bind(...) -> Arc<RholangNode> {
        // Implementation (lines 1283-1323)
    }

    /// Visits a receive-send source node (ReceiveSendSource)
    fn visit_receive_send_source(...) -> Arc<RholangNode> {
        // Implementation (lines 1390-1423)
    }

    /// Visits a send-receive source node (SendReceiveSource)
    fn visit_send_receive_source(...) -> Arc<RholangNode> {
        // Implementation (lines 1423-1458)
    }
}
```

**Methods included:**
- `visit_contract()` - lines 476-521
- `visit_let()` - lines 331-368
- `visit_name_decl()` - lines 1114-1157
- `visit_decl()` - lines 1157-1199
- `visit_linear_bind()` - lines 1199-1241
- `visit_repeated_bind()` - lines 1241-1283
- `visit_peek_bind()` - lines 1283-1323
- `visit_receive_send_source()` - lines 1390-1423
- `visit_send_receive_source()` - lines 1423-1458

**Estimated lines:** ~320
**Risk:** Low - binding logic is well-defined

---

### 8. `patterns.rs` - Pattern Visitor Methods (200 lines)

**Purpose:** Visitor methods for pattern constructs

```rust
pub trait PatternVisitor {
    fn visit_node(&self, node: &Arc<RholangNode>) -> Arc<RholangNode>;

    /// Visits a variable node (Var)
    fn visit_var(...) -> Arc<RholangNode> {
        // Implementation (lines 1090-1114)
    }

    /// Visits a wildcard node (Wildcard)
    fn visit_wildcard(...) -> Arc<RholangNode> {
        // Implementation (lines 1345-1367)
    }

    /// Visits a simple type node (SimpleType)
    fn visit_simple_type(...) -> Arc<RholangNode> {
        // Implementation (lines 1367-1390)
    }

    /// Visits a comment node (Comment)
    fn visit_comment(...) -> Arc<RholangNode> {
        // Implementation (lines 1323-1345)
    }

    /// Visits an error node (Error)
    fn visit_error(...) -> Arc<RholangNode> {
        // Implementation (lines 1458-1491)
    }
}
```

**Methods included:**
- `visit_var()` - lines 1090-1114
- `visit_wildcard()` - lines 1345-1367
- `visit_simple_type()` - lines 1367-1390
- `visit_comment()` - lines 1323-1345
- `visit_error()` - lines 1458-1491

**Estimated lines:** ~200
**Risk:** Low - simple pattern methods

---

## Implementation Strategy

### Phase 1: Create Module Structure (LOW RISK)
**Goal:** Set up directory without breaking existing code

1. Create `src/ir/visitor/` directory
2. Keep original `visitor.rs` as backup
3. Create `mod.rs` with trait composition skeleton
4. **Verify:** `cargo check` should still work with existing visitor.rs

**Estimated time:** 5 minutes

---

### Phase 2: Extract First Module - Literals (LOW RISK)
**Goal:** Prove the pattern works with simplest module

5. Create `literals.rs` with `LiteralVisitor` trait
6. Extract 6 literal visitor methods
7. Add `mod literals;` to mod.rs
8. Add `LiteralVisitor` to main Visitor trait composition
9. **Verify:** `cargo test` - should pass

**Estimated time:** 10 minutes
**Lines extracted:** ~200

---

### Phase 3: Extract Collections (LOW RISK)
**Goal:** Build confidence with second module

10. Create `collections.rs` with `CollectionVisitor` trait
11. Extract 4 collection visitor methods
12. Add to mod.rs and trait composition
13. **Verify:** `cargo test` - should pass

**Estimated time:** 10 minutes
**Lines extracted:** ~250

---

### Phase 4: Extract Patterns (LOW RISK)
**Goal:** Continue with another simple module

14. Create `patterns.rs` with `PatternVisitor` trait
15. Extract 5 pattern visitor methods
16. Add to mod.rs and trait composition
17. **Verify:** `cargo test` - should pass

**Estimated time:** 10 minutes
**Lines extracted:** ~200

---

### Phase 5: Extract Expressions (MEDIUM RISK)
**Goal:** Handle more complex expression methods

18. Create `expressions.rs` with `ExpressionVisitor` trait
19. Extract 10 expression visitor methods
20. Add to mod.rs and trait composition
21. **Verify:** `cargo test` - should pass

**Estimated time:** 15 minutes
**Lines extracted:** ~300

---

### Phase 6: Extract Control Flow (MEDIUM RISK)
**Goal:** Handle control flow constructs

22. Create `control_flow.rs` with `ControlFlowVisitor` trait
23. Extract 4 control flow visitor methods
24. Add to mod.rs and trait composition
25. **Verify:** `cargo test` - should pass

**Estimated time:** 10 minutes
**Lines extracted:** ~250

---

### Phase 7: Extract Bindings (MEDIUM RISK)
**Goal:** Handle binding constructs

26. Create `bindings.rs` with `BindingVisitor` trait
27. Extract 9 binding visitor methods
28. Add to mod.rs and trait composition
29. **Verify:** `cargo test` - should pass

**Estimated time:** 15 minutes
**Lines extracted:** ~320

---

### Phase 8: Extract Processes (MEDIUM RISK)
**Goal:** Handle process constructs (largest module)

30. Create `processes.rs` with `ProcessVisitor` trait
31. Extract 7 process visitor methods
32. Add to mod.rs and trait composition
33. **Verify:** `cargo test` - should pass

**Estimated time:** 15 minutes
**Lines extracted:** ~400

---

### Phase 9: Finalize and Clean Up (LOW RISK)
**Goal:** Complete the refactoring

34. Update mod.rs with final trait composition
35. Remove backup visitor.rs
36. Update documentation in module headers
37. Run full test suite
38. Git commit with detailed message

**Estimated time:** 10 minutes

---

## Total Estimated Effort

- **Total time:** ~2 hours
- **Complexity:** Medium
- **Risk level:** Medium (trait composition complexity)

---

## Testing Strategy

### After Each Phase:
```bash
# Quick validation
cargo check

# Run visitor-related tests
cargo test visitor

# Run transform tests (use visitors)
cargo test transform

# Run all tests
cargo test
```

### Critical Tests to Watch:
- All transform tests (symbol table, document symbols, pretty printer)
- IR pipeline tests
- Position tracking tests

---

## Risks and Mitigations

### Risk 1: Trait Composition Complexity
**Impact:** High - multiple trait inheritance can cause issues
**Mitigation:**
- Extract one module at a time
- Test after each extraction
- Use clear trait bounds
- Each sub-trait requires `visit_node()` for recursion

### Risk 2: Method Signature Changes
**Impact:** High - existing visitors depend on exact signatures
**Mitigation:**
- Keep all signatures identical
- Use `pub use` to re-export from sub-modules
- Run full test suite after each phase

### Risk 3: Import Chain Complexity
**Impact:** Medium - circular dependencies between sub-traits
**Mitigation:**
- Each sub-trait is independent
- All imports come from `super::super::rholang_node`
- No cross-dependencies between sub-traits

### Risk 4: Existing Visitor Implementations Break
**Impact:** High - PrettyPrinter, SymbolTableBuilder, etc.
**Mitigation:**
- Keep backward compatibility
- Test each existing visitor after each phase
- Trait composition should be transparent to implementors

---

## Alternative Approaches Considered

### Alternative 1: Keep as Single File
**Pros:** Simple, no trait composition complexity
**Cons:** Already 1,601 lines, hard to navigate
**Decision:** Rejected - not sustainable

### Alternative 2: Split by Node Category (3-4 traits)
**Pros:** Fewer traits, less composition complexity
**Cons:** Still 400-500 lines per trait
**Decision:** Rejected - not granular enough

### Alternative 3: One File Per Method (42 files)
**Pros:** Maximum granularity
**Cons:** Too many small files, impossible trait composition
**Decision:** Rejected - over-engineering

### Alternative 4: Current Proposal (7 sub-traits)
**Pros:**
- Natural groupings matching tree_sitter refactoring
- ~200-400 lines per trait (digestible)
- Clear boundaries
- Consistent with conversion module structure
**Decision:** ✅ **Selected**

---

## Success Criteria

1. ✅ All 204+ tests passing
2. ✅ Existing visitors (PrettyPrinter, SymbolTableBuilder) still work
3. ✅ Trait composition transparent to implementors
4. ✅ Largest file reduced from 1,601 lines to ~80 lines (main trait)
5. ✅ Clear module boundaries matching conversion modules
6. ✅ No performance regression

---

## Future Benefits

### For Virtual Language Architecture:

1. **Template Pattern:** Each virtual language can have a similar visitor structure:
   ```
   src/ir/
   ├── rholang_visitor/
   │   ├── mod.rs
   │   ├── processes.rs
   │   ├── expressions.rs
   │   └── ...
   ├── metta_visitor/
   │   ├── mod.rs
   │   ├── expressions.rs
   │   ├── definitions.rs
   │   └── types.rs
   ├── sql_visitor/
   │   └── ... (same structure)
   ```

2. **Shared Pattern**: The visit-check-reconstruct pattern is universal

3. **Cross-Language Transforms**: Unified IR visitor can handle all languages

4. **Code Generation**: Generate visitor traits from language schemas

5. **Easier Testing**: Each visitor category can have targeted tests

---

## Consistency with tree_sitter Refactoring

The module structure **mirrors the tree_sitter conversion modules**:

| tree_sitter/conversion/ | visitor/              |
|-------------------------|-----------------------|
| processes.rs            | processes.rs          |
| control_flow.rs         | control_flow.rs       |
| expressions.rs          | expressions.rs        |
| literals.rs             | literals.rs           |
| collections.rs          | collections.rs        |
| bindings.rs             | bindings.rs           |
| patterns.rs             | patterns.rs           |

This consistency makes the codebase easier to navigate:
- Converting `Send` → processes/conversion → processes/visitor
- Converting `Match` → control_flow/conversion → control_flow/visitor
- etc.

---

## Next Steps

After this refactoring is complete:

1. **Implement backend.rs refactoring** (Task C - final integration)
2. **Create MettaVisitor using same pattern**
3. **Create SqlVisitor using same pattern**
4. **Extract shared visitor utilities**

---

## Questions for Review

1. ✅ Trait composition strategy makes sense?
2. ✅ Module groupings match tree_sitter structure?
3. ✅ Phase order is safe (simple modules first)?
4. ✅ Risk mitigation is adequate?
5. ✅ Backward compatibility preserved?

---

## Conclusion

This refactoring will:
- ✅ Reduce main file from 1,601 to ~80 lines (95% reduction)
- ✅ Create clear module boundaries matching conversion structure
- ✅ Serve as template for virtual language visitors
- ✅ Maintain backward compatibility
- ✅ Keep all tests passing

**Recommendation:** Proceed with implementation following the phased approach.
