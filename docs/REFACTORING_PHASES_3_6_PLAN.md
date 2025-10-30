# Rholang Symbol Indexing Refactoring - Phases 3-6 Implementation Plan

**Status**: Phases 1-4 Complete | Phases 5-6 Deferred
**Last Updated**: 2025-10-30
**Current Test Status**: 17/23 passing (Phase 4 implementation complete, no regressions)

---

## Completed Work (Phases 1-3)

### Phase 1: Position Simplification ✅
- Removed manual `byte: 0` normalizations from 3 locations
- Leveraged existing Position Hash/Eq (row, column only)
- No test regressions (17/23 still passing)

### Phase 2: RholangGlobalSymbols Structure ✅
- Created `src/lsp/rholang_global_symbols.rs` (596 lines)
- Lock-free unified storage with 10 passing tests
- Added to WorkspaceState as `rholang_symbols` field
- Enforces: 1 declaration + 0-1 definition + N references

### Phase 3: Direct Indexing Implementation ✅
**Completed**: 2025-10-30

#### 3.1: SymbolTableBuilder Parameter ✅
- Added `rholang_symbols: Option<Arc<RholangGlobalSymbols>>` parameter to struct
- Updated `new()` method signature
- Fixed 2 call sites in `src/lsp/backend/indexing.rs`

#### 3.2: Direct Contract Insertion ✅
- Modified `visit_contract()` in `src/ir/transforms/symbol_table_builder.rs:427-452`
- Top-level contracts now inserted directly into `rholang_symbols` during parsing
- Inserts both declaration and definition locations
- Works alongside existing symbol table for backward compatibility

#### 3.3: Variable Reference Tracking ✅
- Modified `visit_var()` in `src/ir/transforms/symbol_table_builder.rs:743-775`
- Adds references to `rholang_symbols` for same-file symbols
- Adds references for cross-file symbols
- Handles unbound references (forward refs)

#### 3.4: Call Site Updates ✅
- Updated `process_document_blocking()` signature to accept `rholang_symbols`
- Modified `process_document()` to pass `workspace.rholang_symbols`
- Modified `index_directory_parallel()` to pass `workspace.rholang_symbols`
- All code compiles successfully

**Result**: The new unified indexing system is now active and populating `rholang_symbols` in parallel with the old structures. Ready for Phase 4 cleanup.

---

## Phase 4: Cleanup and Simplification ✅
**Completed**: 2025-10-30

### 4.1: Simplified link_symbols Function ✅
**File**: `src/lsp/backend/symbols.rs`

Replaced the 6-phase algorithm with a simplified 4-phase sync:
- **Phase 1**: Sync rholang_symbols → global_symbols (contracts only)
- **Phase 2**: Build inverted index from rholang_symbols.references + per-document local indexes
- **Phase 3**: Populate global_inverted_index
- **Phase 4**: Broadcast workspace change event

**Key improvements**:
- Reduced from ~130 lines to ~100 lines
- Single source of truth (rholang_symbols) instead of multiple data collection phases
- Eliminated potential_global_refs resolution phase
- Eliminated cross-file reference resolution phase (now done during parsing)

### 4.2: Removed potential_global_refs ✅

**Removed from SymbolTableBuilder** (`src/ir/transforms/symbol_table_builder.rs`):
- Removed `potential_global_refs` field from struct (line 28)
- Removed from constructor initialization (line 53)
- Removed `get_potential_global_refs()` method (lines 64-67)
- Removed `resolve_local_potentials()` method (lines 69-89)
- Removed push statements in `visit_var()` at 2 locations (lines 728, 742)
- Added comments explaining Phase 4 changes

**Removed from CachedDocument** (`src/lsp/models.rs`):
- Removed `potential_global_refs` field (line 88)
- Added Phase 4 documentation comment

**Removed from indexing** (`src/lsp/backend/indexing.rs`):
- Removed `get_potential_global_refs()` calls (2 locations: lines 83, 212)
- Removed `resolve_local_potentials()` calls (2 locations: lines 91, 220)
- Removed `potential_global_refs` from CachedDocument initialization (3 locations: lines 144, 276, 454)
- Removed unused variable declaration (line 431)

### 4.3: Retained inverted_index ✅

**Decision**: Keep `inverted_index` in CachedDocument and SymbolTableBuilder

**Rationale**:
- Still needed for local (non-global) symbols like variables within functions
- Used by simplified link_symbols to merge local references
- rholang_symbols currently only stores global contracts
- Future enhancement: Extend rholang_symbols to include local symbols, then remove inverted_index

**Current usage**:
- SymbolTableBuilder: Tracks same-file variable usages
- link_symbols: Merges local inverted_index into global_inverted_index (lines 122-137 in symbols.rs)

### Test Results ✅
**Status**: 17/23 passing (same as before refactoring - no regressions!)

Passing tests verify:
- goto-definition for same-file symbols
- References for local variables
- Document symbols extraction
- Workspace symbols search
- Hover information
- And 12 more...

Failing tests (pre-existing issues, not caused by refactoring):
- test_document_highlight_contract
- test_goto_definition_contract_on_name
- test_goto_definition_loop_param
- test_goto_definition_quoted_contract_cross_file
- test_references_contract_with_new
- test_rename

All 6 failures involve contracts with `new` bindings - a known limitation requiring special handling.

---

## Remaining Work (Deferred)

### Phase 5: Update Consumers to Use rholang_symbols Directly

**Goal**: Delete redundant storage now that `rholang_symbols` is fully operational.

#### Current Problems
1. **potential_global_refs hack** (lines 25, 38, 49-73 in symbol_table_builder.rs)
   - Collects unresolved symbol references
   - Requires post-processing in `resolve_local_potentials()`
   - Then requires 6-phase `link_symbols` algorithm

2. **Separate inverted index** (line 24)
   - Built locally, then merged globally
   - Redundant with `rholang_symbols.references`

3. **No direct global insertion**
   - Global contracts not added to `rholang_symbols` during parsing
   - Must wait for separate linking phase

#### Implementation Steps

**3.1: Add RholangGlobalSymbols Parameter**

```rust
// In src/ir/transforms/symbol_table_builder.rs

pub struct SymbolTableBuilder {
    root: Arc<RholangNode>,
    current_uri: Url,
    current_table: RwLock<Arc<SymbolTable>>,
    // REMOVE: inverted_index: RwLock<InvertedIndex>,
    // REMOVE: potential_global_refs: RwLock<Vec<(String, Position)>>,
    global_table: Arc<SymbolTable>,
    // NEW: Direct access to global symbol storage
    rholang_symbols: Arc<crate::lsp::rholang_global_symbols::RholangGlobalSymbols>,
}

impl SymbolTableBuilder {
    pub fn new(
        root: Arc<RholangNode>,
        uri: Url,
        global_table: Arc<SymbolTable>,
        rholang_symbols: Arc<crate::lsp::rholang_global_symbols::RholangGlobalSymbols>,
    ) -> Self {
        let local_table = Arc::new(SymbolTable::new(Some(global_table.clone())));
        Self {
            root,
            current_uri: uri,
            current_table: RwLock::new(local_table),
            global_table,
            rholang_symbols,
        }
    }

    // REMOVE: get_inverted_index()
    // REMOVE: get_potential_global_refs()
    // REMOVE: resolve_local_potentials()
}
```

**3.2: Update Contract Handling**

When visiting Contract nodes (around line 450):

```rust
fn visit_contract(&self, node: Arc<RholangNode>) -> Arc<RholangNode> {
    // ... existing contract parsing ...

    // NEW: Directly insert into rholang_symbols
    use crate::lsp::rholang_global_symbols::SymbolLocation;

    let decl_location = SymbolLocation::new(
        self.current_uri.clone(),
        contract_name_position,
    );

    // Insert declaration (ignore errors - may already exist from forward ref)
    let _ = self.rholang_symbols.insert_declaration(
        contract_name.clone(),
        SymbolType::Contract,
        decl_location.clone(),
    );

    // Contract body is the definition location
    if contract_body_position != contract_name_position {
        let _ = self.rholang_symbols.set_definition(
            &contract_name,
            SymbolLocation::new(self.current_uri.clone(), contract_body_position),
        );
    }

    // Continue with existing local symbol table insertion...
}
```

**3.3: Update Variable References**

When visiting Var nodes (around line 700):

```rust
fn visit_var(&self, node: Arc<RholangNode>) -> Arc<RholangNode> {
    // ... get var_name and var_position ...

    let current_table = self.current_table.read().unwrap().clone();

    // Try to resolve locally first
    if let Some(symbol) = current_table.lookup(&var_name) {
        // Local resolution - add to references
        if symbol.declaration_uri == self.current_uri {
            // Same file - add reference
            let ref_location = SymbolLocation::new(
                self.current_uri.clone(),
                var_position,
            );
            let _ = self.rholang_symbols.add_reference(&var_name, ref_location);
        }
        // TODO: Cross-file references handled in Phase 4
    } else {
        // Not in local scope - try global
        if let Some(global_symbol) = self.rholang_symbols.lookup(&var_name) {
            // Found in global symbols - add reference
            let ref_location = SymbolLocation::new(
                self.current_uri.clone(),
                var_position,
            );
            let _ = self.rholang_symbols.add_reference(&var_name, ref_location);
        }
        // If not found anywhere, it's truly unresolved (error)
    }

    // Continue...
}
```

**3.4: Update Call Sites**

Update all places that create SymbolTableBuilder:

```rust
// In src/lsp/backend/symbols.rs (or wherever builders are created)

let builder = SymbolTableBuilder::new(
    ir.clone(),
    uri.clone(),
    workspace.global_table.clone(),
    workspace.rholang_symbols.clone(),  // NEW parameter
);
```

**3.5: Remove Obsolete Methods**

Delete:
- `get_inverted_index()`
- `get_potential_global_refs()`
- `resolve_local_potentials()`

---

### Phase 4: Remove Old Structures and link_symbols Algorithm

**Goal**: Delete redundant storage and the 6-phase linking algorithm.

#### 4.1: Remove from WorkspaceState

In `src/lsp/models.rs`:

```rust
pub struct WorkspaceState {
    pub documents: Arc<DashMap<Url, Arc<CachedDocument>>>,

    // KEEP (used by other parts still):
    // pub global_symbols: Arc<DashMap<String, (Url, IrPosition)>>,  // TODO: Deprecate later
    // pub global_table: Arc<tokio::sync::RwLock<SymbolTable>>,       // TODO: Deprecate later

    // REMOVE (fully replaced):
    // pub global_inverted_index: Arc<DashMap<(Url, IrPosition), Vec<(Url, IrPosition)>>>,

    // REMOVE (unused):
    // pub global_contracts: Arc<DashMap<Url, Vec<Arc<RholangNode>>>>,
    // pub global_calls: Arc<DashMap<Url, Vec<Arc<RholangNode>>>>,

    pub global_index: Arc<std::sync::RwLock<GlobalSymbolIndex>>,
    pub global_virtual_symbols: Arc<DashMap<...>>,
    pub rholang_symbols: Arc<RholangGlobalSymbols>,  // NEW (Phase 2)
    pub indexing_state: Arc<tokio::sync::RwLock<IndexingState>>,
}
```

**Note**: Keep `global_symbols` and `global_table` temporarily during migration. Remove in final cleanup once all references are updated.

#### 4.2: Delete link_symbols Function

In `src/lsp/backend/symbols.rs`:

```rust
// DELETE entire link_symbols() function (lines ~50-190)
// This 6-phase algorithm is replaced by direct insertion in SymbolTableBuilder
```

#### 4.3: Update Workspace Indexing

Replace the link_symbols call with a simpler approach:

```rust
// In src/lsp/backend.rs or symbols.rs (workspace indexing)

pub async fn index_workspace(&self) {
    // Clear old data
    self.workspace.rholang_symbols.clear();

    // Parse all documents
    for entry in self.workspace.documents.iter() {
        let (uri, doc) = (entry.key(), entry.value());

        // SymbolTableBuilder now populates rholang_symbols directly
        let builder = SymbolTableBuilder::new(
            doc.ir.clone(),
            uri.clone(),
            self.workspace.global_table.read().await.clone(),
            self.workspace.rholang_symbols.clone(),
        );

        // Build (this now inserts into rholang_symbols)
        let transformed_ir = builder.visit(doc.ir.clone());

        // No need for resolve_local_potentials or link_symbols!
    }

    debug!("Workspace indexing complete: {} symbols", self.workspace.rholang_symbols.len());
}
```

#### 4.4: Update CachedDocument

In `src/lsp/document.rs`:

```rust
pub struct CachedDocument {
    pub ir: Arc<RholangNode>,
    pub symbol_table: Arc<SymbolTable>,
    // REMOVE: pub inverted_index: InvertedIndex,
    // REMOVE: pub potential_global_refs: Vec<(String, Position)>,
    // ... other fields ...
}
```

---

### Phase 5: Update RholangSymbolResolver

**Goal**: Use `rholang_symbols` for lookups instead of old structures.

#### 5.1: Update resolve_symbol Implementation

In `src/lsp/features/adapters/rholang.rs`:

```rust
impl SymbolResolver for RholangSymbolResolver {
    fn resolve_symbol(
        &self,
        symbol_name: &str,
        position: &Position,
        context: &ResolutionContext,
    ) -> Vec<SymbolLocation> {
        // Two-tier lookup:
        // 1. Local scope (via symbol_table)
        // 2. Global scope (via rholang_symbols)

        if let Some(symbol) = self.symbol_table.lookup(symbol_name) {
            // Found in local scope - return declaration + definition
            let mut locations = vec![
                SymbolLocation {
                    uri: symbol.declaration_uri.clone(),
                    range: position_to_range(symbol.declaration_location),
                    kind: map_symbol_kind(symbol.symbol_type),
                    confidence: ResolutionConfidence::Exact,
                    metadata: None,
                }
            ];

            if let Some(def_pos) = symbol.definition_location {
                if def_pos != symbol.declaration_location {
                    locations.push(SymbolLocation {
                        uri: symbol.declaration_uri.clone(),
                        range: position_to_range(def_pos),
                        kind: map_symbol_kind(symbol.symbol_type),
                        confidence: ResolutionConfidence::Exact,
                        metadata: None,
                    });
                }
            }

            locations
        } else {
            // Not in local scope - check global via workspace
            if let Some(workspace) = &self.workspace {
                workspace.rholang_symbols.get_definition_locations(symbol_name)
            } else {
                Vec::new()
            }
        }
    }
}
```

**Note**: Need to add workspace field to RholangSymbolResolver:

```rust
pub struct RholangSymbolResolver {
    symbol_table: Arc<SymbolTable>,
    workspace: Option<Arc<WorkspaceState>>,  // NEW for global lookups
}
```

#### 5.2: Update find-references

Replace inverted index lookups with `rholang_symbols`:

```rust
// In src/lsp/features/references.rs

pub async fn find_references(...) -> Vec<Location> {
    // Get symbol at position
    let symbol_name = get_symbol_at_position(...);

    // Use rholang_symbols.get_references()
    let refs = workspace.rholang_symbols.get_references(&symbol_name);

    refs.into_iter()
        .map(|sym_loc| Location {
            uri: sym_loc.uri,
            range: sym_loc.range,
        })
        .collect()
}
```

---

### Phase 6: Test and Verify

**Goal**: Ensure all 23 tests pass with the new architecture.

#### 6.1: Run Full Test Suite

```bash
RUST_LOG=error timeout 90 cargo test --test lsp_features
```

**Expected**: All 23 tests should pass (currently 17/23).

#### 6.2: Debug Failing Tests

The 6 timeout tests are likely related to:
- Contracts with `new` bindings
- Possibly forward reference handling
- May need special handling in Phase 3 for contract parameters

**Common issues to check**:
1. Forward references: Symbol used before declared in same file
2. Contract parameters: `new` bindings in contract formals
3. Scoping: Ensure local symbols don't leak to global
4. Position mapping: Ensure ranges are computed correctly

#### 6.3: Verify No Regressions

Run specific tests that were passing before:

```bash
cargo test test_goto_definition_same_file
cargo test test_references_local
cargo test test_document_symbols
# ... etc for all 17 passing tests
```

#### 6.4: Performance Verification

Compare before/after:

```bash
# Before refactoring (Phase 1-2 baseline)
cargo bench --bench lsp_operations_benchmark -- --save-baseline before-phase3

# After Phases 3-6
cargo bench --bench lsp_operations_benchmark -- --save-baseline after-phase6

# Compare
cargo bench --bench lsp_operations_benchmark -- --baseline before-phase3
```

**Expected**: Improved performance due to:
- Lock-free rholang_symbols (no RwLock contention)
- Single-pass parsing (no 6-phase linking)
- Direct indexing (no potential_global_refs resolution)

---

## Success Criteria

- ✅ All 23 LSP feature tests passing
- ✅ No performance regressions (ideally improvements)
- ✅ Code compiles without warnings
- ✅ Documentation updated in SYMBOL_INDEXING_AND_RESOLUTION.md
- ✅ Architecture thoroughly documented
- ✅ Old structures fully removed (no dead code)

---

## Risk Mitigation

### Incremental Testing

After each sub-phase:
1. Run `cargo check` - ensure it compiles
2. Run subset of tests - ensure no new failures
3. Commit changes with descriptive message
4. Document any issues encountered

### Rollback Plan

If issues arise:
1. Each phase is independent - can revert one at a time
2. Phase 1-2 changes are complete and tested - safe baseline
3. Git history documents all changes

### Known Risks

1. **Forward References**: May need special handling
2. **Contract Parameters**: Complex scoping with `new` bindings
3. **Cross-File References**: Must be added in Var visitor (Phase 3.3)
4. **Test Timeouts**: May indicate deeper issues with contract handling

---

## Documentation Updates Required

After completion:

1. **SYMBOL_INDEXING_AND_RESOLUTION.md**:
   - Add Phase 3-6 completion entries
   - Update "Current Implementation" sections
   - Remove "Pending Refactoring" section

2. **.claude/CLAUDE.md**:
   - Update symbol table architecture description
   - Document new direct indexing approach
   - Remove references to link_symbols algorithm

3. **Code Comments**:
   - Update SymbolTableBuilder documentation
   - Add comments explaining two-tier lookup
   - Document RholangGlobalSymbols usage patterns

---

## Estimated Effort

- **Phase 3**: 2-3 hours (most complex)
- **Phase 4**: 1-2 hours (mostly deletions)
- **Phase 5**: 1 hour (straightforward refactor)
- **Phase 6**: 2-3 hours (testing and debugging)

**Total**: 6-9 hours for complete implementation

---

## Questions to Resolve

1. **Forward References**: How to handle symbols used before declaration in same file?
   - Current: potential_global_refs + resolve_local_potentials
   - Proposed: Two-pass or deferred insertion?

2. **Contract Parameters**: Should `new` bindings in contract formals be global?
   - Need to check Rholang semantics

3. **Cross-File References**: When should they be resolved?
   - Option A: During parsing (if global symbol exists)
   - Option B: Lazily during LSP requests

4. **Concurrent Indexing**: How to handle multiple files parsing simultaneously?
   - RholangGlobalSymbols is lock-free (good)
   - But need to ensure consistent state

---

## Next Steps

1. Start Phase 3.1: Add rholang_symbols parameter to SymbolTableBuilder
2. Update all SymbolTableBuilder::new() call sites
3. Implement Phase 3.2: Direct contract insertion
4. Test compilation after each step
5. Continue sequentially through phases

**Priority**: Complete Phase 3 first - it's the foundation for Phases 4-6.
