# LSP Features Migration Plan: Semantic Layer Adoption

This document outlines the requirements and steps to migrate all existing LSP functionality from RholangNode-specific code to the language-agnostic semantic layer.

## Current State Analysis

### LSP Features to Migrate

1. **goto_definition** (`backend.rs:1428`)
   - Heavy RholangNode pattern matching
   - Special handling for Send/SendSync channels
   - Contract matching logic
   - Symbol table lookup (✅ already language-agnostic)

2. **references** (`backend.rs:1558`)
   - Contract reference finding
   - Send/SendSync pattern matching
   - Symbol table usage (✅ already language-agnostic)

3. **rename** (`backend.rs:1381`)
   - Inverted index usage (✅ already language-agnostic)
   - Relatively simple, mostly delegates to symbol table

4. **document_symbol** (`backend.rs:1667`)
   - Uses DocumentSymbolVisitor
   - Could benefit from GenericVisitor approach

### Rholang-Specific Dependencies

#### Helper Functions (in `rholang_node.rs`)
- ❌ `find_node_at_position_with_path()` - Position-based node finding
- ❌ `find_node_at_position()` - Simplified position finding
- ❌ `compute_absolute_positions()` - Position computation
- ❌ `collect_contracts()` - Contract collection
- ❌ `collect_calls()` - Send/SendSync collection
- ❌ `match_contract()` - Contract signature matching

#### Pattern Matching Locations
```rust
// Example from goto_definition (line 1462)
match &*parent {
    RholangNode::Send { channel, .. } | RholangNode::SendSync { channel, .. } =>
        Arc::ptr_eq(channel, &node),
    _ => false,
}

// Example from get_symbol_at_position (line 825)
match &*node {
    RholangNode::Var { name, .. } => { /* ... */ }
    RholangNode::Contract { name, .. } => { /* ... */ }
    RholangNode::Send { channel, inputs, .. } => { /* ... */ }
    // ... etc
}
```

## Critical Prerequisite: MORK Pattern Matching Integration

### Why MORK/PathMap First?

The current migration plan assumes manual pattern matching using semantic categories. However, for robust and maintainable pattern matching across languages, we should integrate **MORK** (pattern matching library) and **PathMap** (trie map library) first.

**MORK** was specifically designed for:
- MeTTa s-expression pattern matching
- Rholang process pattern matching
- Efficient tree structure matching

**PathMap** provides:
- Trie-based efficient pattern storage and lookup
- Foundation for MORK's matching algorithms

**Reference Implementation**: MeTTaTron already uses this architecture successfully.

### Phase 0: Integrate MORK and PathMap (NEW - PREREQUISITE)

**Estimated effort**: 12-16 hours

**📖 DETAILED GUIDE**: See `MORK_INTEGRATION_GUIDE.md` for complete implementation details with code examples extracted from MeTTaTron.

#### 0.1 Add Dependencies

Add to `Cargo.toml`:
```toml
[dependencies]
mork = { path = "../MORK/kernel", features = ["interning"] }
mork-expr = { path = "../MORK/expr" }
mork-frontend = { path = "../MORK/frontend" }
pathmap = { path = "../PathMap", features = ["jemalloc", "arena_compact"] }
```

#### 0.2 Study MeTTaTron Integration ✅ COMPLETED

**Reference locations in MeTTaTron** (studied and documented):
- ✅ `mork_convert.rs` (lines 1-281): Conversion between MettaValue and MORK Expr
- ✅ `eval.rs` (lines 879-946): Pattern matching with `query_multi()`
- ✅ `space.rs` (MORK kernel): Core Space API and pattern matching infrastructure
- ✅ `pathmap_par_integration.rs`: PathMap integration with Rholang Par types

**Key learnings extracted**:
1. **De Bruijn Variable Encoding**:
   - Variables use `NewVar` tag for first occurrence
   - Subsequent uses reference via `VarRef(index)`
   - `ConversionContext` tracks variable name → index mapping

2. **Pattern Structure**:
   - Processes/s-expressions encode as: `Arity(n) Symbol Child1 Child2 ...`
   - Symbols interned via `Space.sm` (SharedMapping)
   - Tags: `SymbolSize(len)` + symbol bytes

3. **Query Pattern**:
   ```rust
   // MeTTaTron creates pattern: (= <expr> $rhs)
   // For Rholang: (send <channel> $args)
   let pattern_str = format!("(= {} $rhs)", expr_serialized);
   ```

4. **Match Execution**:
   ```rust
   Space::query_multi(&space.btm, pattern_expr, |result, _matched| {
       if let Err(bindings) = result {
           // Convert bindings: BTreeMap<(u8,u8), ExprEnv> → HashMap<String, Value>
           let our_bindings = mork_bindings_to_our_format(&bindings, &ctx);
       }
       true // Continue for all matches
   });
   ```

5. **Fallback Strategy**: Always maintain iterative fallback if MORK conversion fails

#### 0.3 Create Conversion Module

**New file**: `src/ir/mork_convert.rs`

**Key components** (see MORK_INTEGRATION_GUIDE.md for full code):

1. **ConversionContext**: Tracks variable De Bruijn indices
2. **rholang_to_mork_bytes()**: Convert RholangNode → MORK Expr bytes
3. **write_rholang_node()**: Recursive encoding of node types
4. **write_symbol()**: Symbol interning via Space
5. **mork_bindings_to_rholang()**: Convert match results back
6. **mork_expr_to_rholang()**: Reverse conversion

**Node Encoding Examples**:
```rust
// Var: NewVar or VarRef
RholangNode::Var { name } → Tag::NewVar (first) or Tag::VarRef(idx) (subsequent)

// Send: (send <channel> <inputs...>)
RholangNode::Send { channel, inputs } → Arity(2+len) "send" channel inputs...

// Contract: (contract <name> <formals...> <body>)
RholangNode::Contract { name, formals, body } → Arity(3+len) "contract" name formals... body

// New: (new <bindings...> <body>)
RholangNode::New { bindings, body } → Arity(2+len) "new" bindings... body
```

#### 0.4 Create Pattern Matching Module

**New file**: `src/ir/pattern_matching.rs`

**Key components** (see MORK_INTEGRATION_GUIDE.md for full code):

1. **RholangPatternMatcher**: High-level pattern matching API
   ```rust
   pub struct RholangPatternMatcher {
       space: Space,  // MORK Space for pattern storage
   }
   ```

2. **add_pattern()**: Store pattern-value pairs
   ```rust
   matcher.add_pattern(pattern, value)?;
   // Stores as: (pattern-key <pattern-bytes> <value-bytes>)
   ```

3. **match_query()**: Find all matches using `query_multi()`
   ```rust
   let matches = matcher.match_query(query)?;
   // Returns: Vec<(value, bindings)>
   ```

4. **find_contract_invocations()**: Specialized LSP helper
   ```rust
   matcher.find_contract_invocations("myContract", &["x", "y"])?;
   // Finds: (send (contract "myContract") (42 100))
   // Returns bindings: {"x": 42, "y": 100}
   ```

#### 0.5 Replace match_contract() as Proof of Concept

**Target**: `backend.rs` `match_contract()` function

**Before** (manual pattern matching):
```rust
fn match_contract(
    contract_name: &str,
    parent: &RholangNode,
    inputs: &[Arc<RholangNode>],
    formals: &[String],
) -> bool {
    match &*parent {
        RholangNode::Send { channel, inputs: send_inputs, .. } => {
            // Lots of nested conditionals...
        }
        _ => false,
    }
}
```

**After** (MORK-based):
```rust
use crate::ir::pattern_matching::RholangPatternMatcher;

fn find_contract_references(
    contract_name: &str,
    formals: &[String],
    ir: &Arc<RholangNode>,
) -> Vec<Arc<RholangNode>> {
    let matcher = RholangPatternMatcher::new();
    match matcher.find_contract_invocations(contract_name, formals) {
        Ok(matches) => matches.into_iter().map(|(node, _)| node).collect(),
        Err(e) => {
            log::warn!("Pattern matching failed: {}", e);
            Vec::new()
        }
    }
}

#### 0.6 Testing and Validation

**Test Strategy**:
1. Unit tests for each node type conversion
2. Integration tests matching MeTTaTron's test patterns
3. Performance benchmarks (O(n) iterative vs O(k) MORK)
4. Fallback behavior verification

**Success Criteria**:
- All node types convert to/from MORK successfully
- `match_contract()` replacement works correctly
- 10x+ performance improvement on large codebases
- Zero test regressions

#### 0.7 Performance Measurement

**Benchmark Setup**:
- Large codebase (100+ contracts, 1000+ send operations)
- Measure: Pattern compilation time, query time, memory usage
- Compare: Iterative O(n) vs MORK O(k)

**Expected Results** (based on MeTTaTron experience):
- 10-100x speedup for contract reference finding
- Sub-millisecond query_multi performance
- One-time compilation cost amortized across queries

### Why MORK/PathMap First?

**Benefits** (validated by MeTTaTron):

1. **Performance**: `query_multi()` is O(k) where k = matches (vs O(n) iteration)
2. **Correctness**: MORK's unification handles complex patterns correctly
3. **De Bruijn Encoding**: Consistent variable handling across languages
4. **Proven Architecture**: MeTTaTron demonstrates viability
5. **Symbol Interning**: SharedMapping reduces memory footprint
6. **Fallback Support**: Can fall back to iteration if conversion fails

**Real Performance** (from MeTTaTron):
- MeTTaTron uses `query_multi()` for rule matching (eval.rs:931)
- Iterative fallback only for conversion failures (eval.rs:948)
- Pattern storage in PathMap trie enables prefix sharing

## Migration Requirements (Updated)

### Phase 0: MORK/PathMap Integration (NEW - 12-16 hours)

See detailed breakdown above. This becomes the **critical prerequisite** for all subsequent phases.

### Phase 1: Core Semantic Helpers (✅ COMPLETE)

Already implemented in `src/lsp/semantic_features.rs`:
- ✅ `find_semantic_node_at_position()` - Language-agnostic node finding
- ✅ `extract_variable_name()` - Name extraction using semantic categories
- ✅ `get_symbol_table_for_node()` - Symbol table access
- ✅ `is_binding_node()` - Category-based checks
- ✅ `is_invocation_node()` - Category-based checks

### Phase 2: Additional Semantic Helpers (TODO)

Need to implement in `semantic_features.rs`:

#### 2.1 Position Computation
```rust
/// Compute absolute positions for all nodes in a semantic tree
pub fn compute_semantic_positions(
    root: &dyn SemanticNode
) -> HashMap<NodeId, (Position, Position)> {
    // Language-agnostic version of compute_absolute_positions
    // Track parent positions during traversal
    // Works with any SemanticNode
}
```

#### 2.2 Contract/Invocation Collection
```rust
/// Collect all contract definitions (bindings with specific semantics)
pub fn collect_semantic_contracts(
    root: &dyn SemanticNode
) -> Vec<Arc<dyn SemanticNode>> {
    // Use SemanticCategory::Binding
    // Check node metadata for contract type
    // Language-agnostic approach
}

/// Collect all invocations (calls, sends, etc.)
pub fn collect_semantic_invocations(
    root: &dyn SemanticNode
) -> Vec<Arc<dyn SemanticNode>> {
    // Use SemanticCategory::Invocation or LanguageSpecific
    // Works across languages
}
```

#### 2.3 Semantic Matching
```rust
/// Match an invocation against a contract signature
///
/// This is language-specific in implementation but generic in interface
pub trait InvocationMatcher {
    fn matches(
        &self,
        invocation: &dyn SemanticNode,
        contract: &dyn SemanticNode
    ) -> bool;
}

/// Rholang-specific implementation
pub struct RholangInvocationMatcher;

impl InvocationMatcher for RholangInvocationMatcher {
    fn matches(&self, invocation: &dyn SemanticNode, contract: &dyn SemanticNode) -> bool {
        // Downcast to RholangNode and use existing match_contract logic
        // Or implement new semantic-based matching
    }
}
```

#### 2.4 Node Relationship Helpers
```rust
/// Check if a node is a child of a specific parent in a path
pub fn is_child_of_category(
    node: &dyn SemanticNode,
    path: &[&dyn SemanticNode],
    parent_category: SemanticCategory
) -> bool {
    // Generic version of checking "is this Var the name of a Contract?"
}

/// Extract channel from invocation node
pub fn extract_invocation_target(
    node: &dyn SemanticNode
) -> Option<&dyn SemanticNode> {
    // Generic extraction of invocation target
    // For Send: the channel
    // For function call: the function name
}

/// Extract arguments from invocation
pub fn extract_invocation_args(
    node: &dyn SemanticNode
) -> Vec<&dyn SemanticNode> {
    // Generic argument extraction
}
```

### Phase 3: Migrate LSP Features (TODO)

#### 3.1 Migrate `goto_definition`

**Current complexity**: HIGH (100+ lines, heavy pattern matching)

**Migration steps**:
1. Replace `find_node_at_position_with_path` with `find_semantic_node_at_position`
2. Replace RholangNode pattern matching with semantic category checks
3. Use `extract_variable_name` instead of `name` field access
4. Implement semantic contract matching using InvocationMatcher trait
5. Keep symbol table lookup (already language-agnostic)

**Estimated effort**: 4-6 hours

**Example transformation**:
```rust
// BEFORE (Rholang-specific)
match &*parent {
    RholangNode::Send { channel, .. } | RholangNode::SendSync { channel, .. } => {
        Arc::ptr_eq(channel, &node)
    }
    _ => false,
}

// AFTER (semantic)
match parent.semantic_category() {
    SemanticCategory::Invocation | SemanticCategory::LanguageSpecific => {
        if let Some(target) = extract_invocation_target(parent) {
            std::ptr::eq(target as *const _, node as *const _)
        } else {
            false
        }
    }
    _ => false,
}
```

#### 3.2 Migrate `references`

**Current complexity**: MEDIUM (50+ lines)

**Migration steps**:
1. Replace `find_node_at_position_with_path` with semantic equivalent
2. Use semantic contract/invocation collectors
3. Use InvocationMatcher for contract matching
4. Symbol table already language-agnostic

**Estimated effort**: 2-3 hours

#### 3.3 Migrate `rename`

**Current complexity**: LOW (mostly delegates to symbol table)

**Migration steps**:
1. Minimal changes needed
2. Inverted index is already language-agnostic
3. Just replace position finding

**Estimated effort**: 30 minutes

#### 3.4 Migrate `document_symbol`

**Current complexity**: LOW (uses visitor pattern)

**Migration steps**:
1. Already uses DocumentSymbolVisitor
2. Could create GenericDocumentSymbolVisitor
3. Use semantic categories instead of variant matching

**Estimated effort**: 1-2 hours

### Phase 4: Update Helper Functions (TODO)

#### 4.1 Migrate `get_symbol_at_position`

**Current state**: Heavy RholangNode pattern matching (lines 813-1050)

**Migration steps**:
1. Use `find_semantic_node_at_position`
2. Replace all pattern matching with semantic category checks
3. Use `extract_variable_name` helper
4. Use `is_child_of_category` for parent checking

**Estimated effort**: 3-4 hours

### Phase 5: Testing and Validation (TODO)

#### 5.1 Create Migration Tests
- Test each migrated feature with Rholang code
- Test with UnifiedIR to verify language-agnosticism
- Ensure all existing LSP tests still pass

**Estimated effort**: 2-3 hours per feature

#### 5.2 Performance Validation
- Compare performance of semantic vs. RholangNode approach
- Optimize hot paths if needed
- Ensure position computation is efficient

**Estimated effort**: 2-4 hours

## Implementation Strategy (Updated with MORK)

### Recommended Approach: MORK-First Gradual Migration

Instead of a big-bang rewrite, follow this incremental approach:

1. **Week 1-2: Phase 0 - MORK/PathMap Integration**
   - Study MeTTaTron's pattern matching architecture
   - Integrate MORK and PathMap dependencies
   - Implement `ToMorkPattern` trait for RholangNode
   - Create `SemanticPatternMatcher` infrastructure
   - Build pattern cache system
   - **Critical**: Comprehensive testing of pattern conversion and matching
   - **Deliverable**: Pattern matching working for basic contracts

2. **Week 2-3: Complete Phase 2 helpers (simplified with MORK)**
   - Implement remaining semantic helper functions
   - Use MORK for complex matching operations
   - Write tests using pattern-based approach
   - Ensure compatibility with Rholang, MeTTa, and UnifiedIR

3. **Week 3: Migrate simplest feature (rename)**
   - Validate migration approach with minimal MORK usage
   - Establish patterns for other features
   - Build confidence in migration process

4. **Week 4: Migrate medium features (references, document_symbol)**
   - Apply pattern matching for contract lookups
   - Use MORK for invocation matching
   - Refine semantic helpers as needed

5. **Week 4-5: Migrate complex feature (goto_definition)**
   - Leverage MORK's declarative patterns
   - Replace complex conditionals with pattern queries
   - By now, pattern matching is battle-tested

6. **Week 5: Testing and optimization**
   - Test pattern cache performance
   - Benchmark MORK vs manual matching
   - Optimize pattern compilation
   - Documentation with MORK examples

### Alternative Approach: Parallel Implementation

Keep existing Rholang-specific code, add semantic versions alongside:

```rust
async fn goto_definition(&self, params: GotoDefinitionParams) -> LspResult<...> {
    // Try semantic approach first (works for all languages)
    if let Some(result) = self.goto_definition_semantic(params.clone()).await? {
        return Ok(Some(result));
    }

    // Fallback to Rholang-specific (deprecated, but keeps working)
    self.goto_definition_rholang_legacy(params).await
}
```

**Benefits**:
- Zero risk of breaking existing functionality
- Can validate semantic approach gradually
- Easy rollback if issues found
- Can deprecate legacy code after validation

## Effort Estimation (Updated with MORK)

### Total Implementation Time

| Phase | Estimated Time |
|-------|---------------|
| **Phase 0: MORK/PathMap integration** | **12-16 hours** |
| Phase 1: Core helpers (✅ COMPLETE) | 0 hours |
| Phase 2: Additional semantic helpers | 6-8 hours (reduced with MORK) |
| Phase 3.1: goto_definition | 3-4 hours (simplified with MORK) |
| Phase 3.2: references | 2-3 hours |
| Phase 3.3: rename | 0.5 hours |
| Phase 3.4: document_symbol | 1-2 hours |
| Phase 4: get_symbol_at_position | 2-3 hours (simplified) |
| Phase 5: Testing | 10-14 hours (more patterns to test) |
| **Total** | **37-54 hours** |

**Note**: Total hours increased due to Phase 0, but individual phase complexity decreased due to MORK's declarative patterns replacing manual matching.

### Resource Requirements

- **Developer**: 1 senior Rust developer familiar with LSP and the codebase
- **MeTTaTron Reference**: Access to MeTTaTron codebase for MORK integration patterns
- **Timeline**: 1.5-2.5 weeks development (spread over 4-5 calendar weeks)
- **Review**: 3-6 hours of code review time (more due to MORK integration)

## Risk Assessment

### Low Risk Items
✅ Symbol table (already language-agnostic)
✅ Inverted index (already language-agnostic)
✅ Rename feature (simple migration)

### Medium Risk Items
⚠️ Position computation (needs careful testing)
⚠️ Contract matching (language-specific semantics)

### High Risk Items
🔴 goto_definition (complex logic, many edge cases)
🔴 Performance (semantic layer adds indirection)

### Mitigation Strategies

1. **Comprehensive Testing**: Test each migrated feature extensively
2. **Parallel Implementation**: Keep legacy code as fallback
3. **Performance Monitoring**: Benchmark before/after migration
4. **Gradual Rollout**: Start with non-critical features
5. **User Feedback**: Beta test with real users before full deployment

## Success Criteria

### Must Have
- ✅ All existing LSP tests pass
- ✅ Features work with RholangNode (backward compatibility)
- ✅ Features work with UnifiedIR (forward compatibility)
- ✅ No performance regression (< 10% slowdown)

### Should Have
- ✅ Code is more maintainable (less duplication)
- ✅ Clear migration path for MeTTa support
- ✅ Comprehensive test coverage (> 90%)

### Nice to Have
- ✅ Performance improvement (semantic layer can enable caching)
- ✅ Cross-language goto-definition (Rholang → MeTTa)
- ✅ Better error messages using semantic information

## Conclusion

The migration is **feasible and highly worthwhile**, especially with MORK/PathMap integration. The semantic layer foundation (Steps 1-6) is complete and solid. The remaining work now includes:

1. **Phase 0: MORK/PathMap integration** (12-16 hours) - NEW PREREQUISITE
   - Study MeTTaTron's proven approach
   - Implement pattern abstraction layer
   - Build pattern cache infrastructure

2. **Semantic helper functions** (6-8 hours, simplified by MORK)
   - Less manual matching needed
   - Declarative pattern definitions
   - More maintainable code

3. **LSP feature migration** (8-13 hours, simplified by MORK)
   - Replace complex conditionals with pattern queries
   - Leverage PathMap for O(log n) lookups
   - Cleaner, more expressive code

4. **Thorough testing** (10-14 hours, comprehensive pattern testing)
   - Test pattern conversion correctness
   - Validate matching performance
   - Ensure cross-language compatibility

**Total effort**: 37-54 hours (vs 27-40 without MORK)

### Key Advantages of MORK Integration

1. **Unified Pattern Language**: Same patterns work for MeTTa and Rholang
2. **Performance**: O(log n) PathMap lookups vs O(n) iteration
3. **Maintainability**: Declarative patterns vs nested conditionals
4. **Proven Technology**: Already battle-tested in MeTTaTron
5. **Extensibility**: Easy to add new pattern types for future languages

### Recommendation

Use the **MORK-first gradual migration approach** with **parallel implementation**:

1. **Start with Phase 0**: Integrate MORK/PathMap before migrating LSP features
2. **Reference MeTTaTron**: Study existing patterns and adapt for Rholang
3. **Parallel Implementation**: Keep legacy code as fallback during migration
4. **Incremental Validation**: Test each migrated feature thoroughly
5. **Performance Monitoring**: Benchmark pattern matching vs manual checks

The upfront investment in MORK integration (12-16 hours) pays dividends by:
- Simplifying subsequent migration phases
- Providing a unified pattern system for multiple languages
- Enabling future cross-language features (MeTTa ↔ Rholang)
- Reducing long-term maintenance burden
