# Phase B-1: Incremental Workspace Indexing

**Status**: 📋 **PLANNING**
**Date Started**: 2025-11-13
**Expected Duration**: 1.5-2 weeks
**Prerequisites**: Phase A complete ✅

## 0. Context

After completing Phase A optimizations (2000x+ contract queries, 2.56x serialization, 5.9x indexing), Phase A-4 analytical review identified that the remaining performance bottleneck is **workspace re-indexing on file changes**.

**Problem**: When a user edits a single file, the language server currently re-indexes the entire workspace, even though only one file changed.

**Impact**: This affects the most common developer workflow (edit code → see diagnostics/completions).

**Phase B-1 Goal**: Implement incremental indexing to only re-index changed files and their dependents.

## 1. Problem Statement

### Current Behavior (Baseline)

**Scenario**: User edits `contract.rho` in a workspace with 100 `.rho` files

**Current Flow**:
1. User saves `contract.rho`
2. File watcher triggers `didChange` event
3. Language server re-parses `contract.rho` (5-10ms)
4. **Language server re-indexes ALL 100 workspace files** (~50ms)
5. Symbol tables rebuilt from scratch
6. Completion indices repopulated entirely
7. Diagnostics published

**Time Breakdown** (estimated):
- File parsing: ~5-10ms (unavoidable)
- **Full workspace re-indexing: ~50ms** ← TARGET FOR OPTIMIZATION
- Diagnostics generation: ~10-20ms
- **Total: ~65-80ms**

**Why This Is Slow**:
- 99% of files unchanged, but all are re-processed
- No tracking of file modification timestamps
- No dependency graph (don't know which files reference which symbols)
- Brute-force approach: "file changed → re-index everything"

### Desired Behavior (Post-Phase B-1)

**Scenario**: Same as above (user edits `contract.rho`)

**New Flow**:
1. User saves `contract.rho`
2. File watcher triggers `didChange` event
3. **Check file modification timestamp** (new)
4. Language server re-parses `contract.rho` (5-10ms)
5. **Language server queries dependency graph** (new)
   - Finds 2 files that reference symbols from `contract.rho`
   - Only re-indexes: `contract.rho` + 2 dependent files (~5-10ms) ← **10x FASTER**
6. **Incrementally updates** symbol tables and completion indices (new)
7. Diagnostics published

**Time Breakdown** (predicted):
- File parsing: ~5-10ms (unchanged)
- **Incremental indexing: ~5-10ms** (10x improvement)
- Diagnostics generation: ~10-20ms (unchanged)
- **Total: ~20-40ms** (2-3x overall improvement)

**Predicted Speedup**: **5-10x** for workspace re-indexing operations

## 2. Hypothesis

### Scientific Hypothesis

**Claim**: Implementing incremental indexing with file change tracking and dependency graph will reduce workspace re-indexing time by **5-10x** for typical file change operations.

**Rationale**:
- In typical development, only 1-2 files change per edit
- Most files have localized dependencies (not cross-referencing entire workspace)
- Average file references 1-5 other files (not 100)
- Therefore: Re-indexing 1-5 files is 20-100x faster than re-indexing 100 files

**Theoretical Complexity**:
- **Before**: O(n) where n = total workspace files (always re-index all)
- **After**: O(k + d) where k = changed files, d = dependent files (typically k+d ≪ n)

**Example**:
- Workspace: 100 files
- Change: 1 file with 2 dependents
- Before: Re-index 100 files (~50ms)
- After: Re-index 3 files (~1.5ms)
- **Speedup**: 50ms / 1.5ms ≈ **33x faster**

### Predicted Impact on LSP Responsiveness

**Current**: ~65-80ms from file save to diagnostics
**Predicted**: ~20-40ms from file save to diagnostics
**Improvement**: **2-3x faster overall** (user-perceived)

This brings file change latency well below the 50ms threshold for "instant" UX.

## 3. Baseline Measurements

### Measurement Plan

To validate our hypothesis, we need to measure:

1. **Workspace Indexing Time** (current full re-index):
   - Benchmark: `benches/indexing_performance.rs`
   - Scenarios: 10, 50, 100, 500, 1000 files
   - Metric: Time from workspace initialization to all symbols indexed

2. **File Change Latency** (current):
   - Scenario: Edit single file in 100-file workspace
   - Measure: Time from `didChange` to diagnostics published
   - Breakdown: Parsing time vs indexing time vs diagnostics time

3. **Dependency Graph Characteristics** (for prediction):
   - Analyze typical Rholang workspace:
     - How many files does each contract reference? (avg/median/p95)
     - What is the dependency graph depth? (imports, contract calls)
     - What percentage of files are isolated? (no cross-file references)

4. **Symbol Table Size** (memory impact):
   - Current memory usage for 100-file workspace
   - Projected memory usage with incremental indexing (dependency graph overhead)

### Running Baselines

**Status**: ⚠️ **BLOCKED** - liblevenshtein compilation errors

**Issue**: Attempted to run `benches/indexing_performance.rs` but encountered compilation errors in liblevenshtein dependency:
```
error[E0061]: this method takes 5 arguments but 3 arguments were supplied
   --> liblevenshtein-rust/src/transducer/generalized/automaton.rs:319:45
    |
319 |             if let Some(next_state) = state.transition(&self.operations, &bit_vector, i + 1) {
    |                                             ^^^^^^^^^^
```

This appears to be from recent Phase 2d subsumption changes in liblevenshtein (generalized/subsumption.rs).

**Workaround Options**:
1. Fix liblevenshtein compilation errors (requires modifying external dependency)
2. Use existing Phase A benchmark results as proxy (workspace indexing from `benches/space_object_pooling_baseline.rs`)
3. Proceed with implementation based on architectural analysis (defer baseline to post-implementation)

**Decision**: Option 3 - Proceed with architectural-based estimates

**Rationale**:
- Phase A-4 analytical review already established baseline understanding
- Compilation errors are in external dependency (not our codebase)
- Phase B-1 implementation can proceed based on well-founded architectural analysis
- Baseline measurements can be completed once liblevenshtein is fixed

### Expected Baseline Results (Architectural Analysis)

Based on Phase A measurements, existing benchmarks, and architectural analysis:

**Workspace Indexing** (from Phase A-3 benchmarks):
- 10 files: ~5ms (estimated)
- 50 files: ~25ms (estimated)
- 100 files: ~50ms (estimated from Phase A-4 analysis)
- 500 files: ~250ms (linear extrapolation)
- 1000 files: ~500ms (from Phase A-3: 0.53ms/contract × 1000 ≈ 530ms)
- **Scaling**: Linear O(n)

**File Change Latency** (architectural estimate):
- Parsing: ~5-10ms (Tree-Sitter - unavoidable)
- **Indexing: ~50ms (full re-index of 100 files)** ← TARGET FOR OPTIMIZATION
- Diagnostics: ~10-20ms (from Phase A-4 analysis)
- **Total**: ~65-80ms

**Dependency Graph** (predictions from Rholang architecture):
- Average dependencies per file: 2-5 (contract calls + channel references)
- Median dependencies: 1-3 (most files are self-contained)
- p95 dependencies: 10-20 (complex orchestration files)
- Isolated files: 30-50% (many contracts are standalone)

## 4. Architecture Design

### High-Level Components

Phase B-1 requires four new subsystems:

1. **File Modification Tracker**
   - Tracks last-known modification timestamp for each file
   - Detects which files changed since last indexing
   - Invalidates cached IRs for changed files

2. **Dependency Graph**
   - Directed graph: file A depends on file B if A references symbols from B
   - Edge types: `import`, `contract_call`, `channel_reference`
   - Supports queries: "What files depend on X?" (reverse lookup)

3. **Incremental Symbol Index**
   - Supports partial updates (add/remove symbols for single file)
   - Maintains global symbol table consistency
   - Updates completion indices incrementally

4. **Cache Invalidation Strategy**
   - When file A changes:
     - Invalidate A's cached IR
     - Invalidate symbol tables for A
     - Invalidate symbol tables for all files that depend on A
     - Re-index A + dependents

### Data Structures

#### FileModificationTracker

```rust
pub struct FileModificationTracker {
    /// Maps file URI → last known modification time
    timestamps: HashMap<Url, SystemTime>,
}

impl FileModificationTracker {
    /// Check if file changed since last indexing
    pub fn has_changed(&self, uri: &Url) -> Result<bool, io::Error> {
        let current_mtime = fs::metadata(uri.path())?.modified()?;
        Ok(self.timestamps.get(uri).map_or(true, |&prev| current_mtime > prev))
    }

    /// Update timestamp after indexing
    pub fn mark_indexed(&mut self, uri: &Url) -> Result<(), io::Error> {
        let mtime = fs::metadata(uri.path())?.modified()?;
        self.timestamps.insert(uri.clone(), mtime);
        Ok(())
    }
}
```

#### DependencyGraph

```rust
pub struct DependencyGraph {
    /// Forward edges: file A → files that A imports/references
    forward: HashMap<Url, HashSet<Url>>,

    /// Reverse edges: file B → files that depend on B (for invalidation)
    reverse: HashMap<Url, HashSet<Url>>,
}

impl DependencyGraph {
    /// Add dependency: `source` depends on `target`
    pub fn add_dependency(&mut self, source: Url, target: Url) {
        self.forward.entry(source.clone()).or_default().insert(target.clone());
        self.reverse.entry(target).or_default().insert(source);
    }

    /// Get all files that depend on `file` (directly or transitively)
    pub fn get_dependents(&self, file: &Url) -> HashSet<Url> {
        let mut dependents = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(file.clone());

        while let Some(current) = queue.pop_front() {
            if let Some(direct_dependents) = self.reverse.get(&current) {
                for dep in direct_dependents {
                    if dependents.insert(dep.clone()) {
                        queue.push_back(dep.clone());  // Transitive
                    }
                }
            }
        }

        dependents
    }

    /// Remove file from graph (when deleted)
    pub fn remove_file(&mut self, file: &Url) {
        self.forward.remove(file);
        self.reverse.remove(file);

        // Clean up references to this file
        for edges in self.forward.values_mut() {
            edges.remove(file);
        }
        for edges in self.reverse.values_mut() {
            edges.remove(file);
        }
    }
}
```

#### IncrementalSymbolIndex

```rust
pub struct IncrementalSymbolIndex {
    /// Global symbol table (unchanged from current implementation)
    global_symbols: Arc<RwLock<GlobalSymbolIndex>>,

    /// Per-file symbol tables (for incremental updates)
    file_symbols: HashMap<Url, Arc<SymbolTable>>,

    /// Completion index (supports incremental updates)
    completion_index: WorkspaceCompletionIndex,
}

impl IncrementalSymbolIndex {
    /// Remove symbols for a specific file
    pub fn remove_file_symbols(&mut self, uri: &Url) {
        if let Some(symbol_table) = self.file_symbols.remove(uri) {
            // Remove from global index
            let mut global = self.global_symbols.write().unwrap();
            for symbol in symbol_table.all_symbols() {
                global.remove_symbol(&symbol.name, uri);
            }

            // Remove from completion index
            for symbol in symbol_table.all_symbols() {
                self.completion_index.remove_symbol(&symbol.name, uri);
            }
        }
    }

    /// Add symbols for a specific file
    pub fn add_file_symbols(&mut self, uri: &Url, symbol_table: Arc<SymbolTable>) {
        // Add to global index
        let mut global = self.global_symbols.write().unwrap();
        for symbol in symbol_table.all_symbols() {
            global.add_symbol(symbol.clone(), uri.clone());
        }

        // Add to completion index
        for symbol in symbol_table.all_symbols() {
            self.completion_index.add_symbol(symbol.clone(), uri.clone());
        }

        self.file_symbols.insert(uri.clone(), symbol_table);
    }

    /// Incrementally update: remove old, add new
    pub fn update_file_symbols(&mut self, uri: &Url, new_symbols: Arc<SymbolTable>) {
        self.remove_file_symbols(uri);
        self.add_file_symbols(uri, new_symbols);
    }
}
```

### Integration Points

**Workspace Initialization** (`src/lsp/backend/indexing.rs`):
- Build initial dependency graph while indexing files
- Populate `FileModificationTracker` with initial timestamps

**File Change Handler** (`src/lsp/backend.rs::did_change`):
```rust
async fn did_change(&self, params: DidChangeTextDocumentParams) {
    let uri = params.text_document.uri;

    // 1. Check if file actually changed (timestamp)
    if !self.modification_tracker.has_changed(&uri)? {
        return;  // No-op if unchanged (rare but possible)
    }

    // 2. Re-parse changed file
    let new_ir = parse_file(&uri).await?;

    // 3. Query dependency graph for files that depend on this one
    let dependents = self.dependency_graph.get_dependents(&uri);

    // 4. Incrementally update symbol index
    self.incremental_index.update_file_symbols(&uri, build_symbols(&new_ir));

    // 5. Re-index dependent files (if any)
    for dependent_uri in dependents {
        let dep_ir = parse_file(&dependent_uri).await?;
        self.incremental_index.update_file_symbols(&dependent_uri, build_symbols(&dep_ir));
    }

    // 6. Update modification timestamp
    self.modification_tracker.mark_indexed(&uri)?;

    // 7. Publish diagnostics
    self.publish_diagnostics(&uri).await;
}
```

### Dependency Detection

**How to build the dependency graph** (during initial indexing):

For each file, analyze its IR to find:

1. **Import statements** (if Rholang supports imports):
   - Extract imported file paths
   - Add edge: `current_file` → `imported_file`

2. **Contract calls** (`Send` nodes):
   - Extract contract name
   - Look up contract definition location (via `GlobalSymbolIndex`)
   - If definition in another file: Add edge: `current_file` → `definition_file`

3. **Channel references** (quoted variables):
   - Extract channel name
   - Look up channel definition location
   - If definition in another file: Add edge

**Example**:
```rholang
// File: consumer.rho
new ret in {
  process!("start", *ret)  // Depends on file defining `process` contract
}
```

Dependency graph edge: `consumer.rho` → `process.rho`

**Implementation** (`src/ir/transforms/dependency_builder.rs` - new file):
```rust
pub struct DependencyGraphBuilder {
    current_file: Url,
    global_index: Arc<RwLock<GlobalSymbolIndex>>,
    dependencies: HashSet<Url>,
}

impl Visitor<RholangNode> for DependencyGraphBuilder {
    fn visit_send(&mut self, node: &RholangNode) -> Arc<RholangNode> {
        if let RholangNode::Send { channel, .. } = node {
            // Extract contract name
            if let Some(name) = extract_contract_name(channel) {
                // Look up definition in global index
                if let Some(locations) = self.global_index.read().unwrap().get_definitions(&name) {
                    for location in locations {
                        if location.uri != self.current_file {
                            self.dependencies.insert(location.uri.clone());
                        }
                    }
                }
            }
        }
        node.clone()  // No transformation, just analysis
    }
}
```

## 5. Implementation Plan

### Phase B-1.1: File Modification Tracking (2-3 days)

**Goal**: Detect which files changed since last indexing

**Tasks**:
1. Create `FileModificationTracker` struct
2. Integrate with workspace initialization
3. Update `did_change` handler to check timestamps
4. Add unit tests (10+ test cases)

**Acceptance Criteria**:
- ✅ Correctly detects file changes via filesystem timestamps
- ✅ Handles edge cases (file deletion, external modification)
- ✅ All tests passing

### Phase B-1.2: Dependency Graph Construction (3-4 days)

**Goal**: Build dependency graph during workspace indexing

**Tasks**:
1. Create `DependencyGraph` struct with forward/reverse edges
2. Implement `DependencyGraphBuilder` visitor
3. Integrate with workspace indexing pipeline
4. Add transitive dependency resolution
5. Add unit tests (15+ test cases)
6. Create integration test with real Rholang files

**Acceptance Criteria**:
- ✅ Correctly identifies contract call dependencies
- ✅ Handles circular dependencies gracefully
- ✅ Transitive dependency resolution works
- ✅ All tests passing

### Phase B-1.3: Incremental Symbol Index (3-4 days)

**Goal**: Support adding/removing symbols for individual files

**Tasks**:
1. Create `IncrementalSymbolIndex` wrapper
2. Implement `remove_file_symbols()` and `add_file_symbols()`
3. Update `GlobalSymbolIndex` to support removal (if not already)
4. Update `WorkspaceCompletionIndex` to support removal (Phase 10 added this)
5. Add unit tests (20+ test cases)
6. Integration test: Add file → Remove file → Re-add file

**Acceptance Criteria**:
- ✅ Removing symbols doesn't break global index consistency
- ✅ Adding symbols updates completion index correctly
- ✅ No memory leaks (symbols properly cleaned up)
- ✅ All tests passing

### Phase B-1.4: Incremental Re-indexing Logic (2-3 days)

**Goal**: Re-index only changed files + dependents

**Tasks**:
1. Update `did_change` handler with incremental logic
2. Query dependency graph for dependents
3. Re-index changed file + dependents only
4. Update modification timestamps
5. Add integration tests (10+ test cases)
6. Create benchmark for file change latency

**Acceptance Criteria**:
- ✅ File change triggers re-indexing of changed file + dependents only
- ✅ No full workspace re-indexing for single file change
- ✅ Diagnostics published after incremental update
- ✅ All tests passing

### Phase B-1.5: Testing and Validation (2-3 days)

**Goal**: Validate 5-10x speedup hypothesis

**Tasks**:
1. Run file change latency benchmark (before vs after)
2. Test with real Rholang workspaces (10, 50, 100, 500 files)
3. Measure memory overhead of dependency graph
4. Edge case testing:
   - Circular dependencies
   - File deletion
   - External file modification (outside LSP)
   - Workspace with no cross-file dependencies
5. Create regression tests

**Acceptance Criteria**:
- ✅ File change latency reduced by ≥5x (hypothesis confirmed)
- ✅ No correctness regressions (all existing tests pass)
- ✅ Memory overhead <10% increase
- ✅ Edge cases handled gracefully

### Phase B-1.6: Documentation (1-2 days)

**Goal**: Document implementation and results

**Tasks**:
1. Update this file with implementation details
2. Document baseline vs post-optimization benchmarks
3. Create architecture diagram (dependency graph + incremental indexing)
4. Update CLAUDE.md with new architecture components
5. Add code comments to new modules

**Acceptance Criteria**:
- ✅ Comprehensive documentation in optimization ledger
- ✅ Architecture diagram clearly shows data flow
- ✅ CLAUDE.md updated with incremental indexing architecture
- ✅ Code comments explain non-obvious design decisions

## 6. Timeline Estimate

**Conservative** (recommended):
- B-1.1: File Modification Tracking - 3 days
- B-1.2: Dependency Graph Construction - 4 days
- B-1.3: Incremental Symbol Index - 4 days
- B-1.4: Incremental Re-indexing Logic - 3 days
- B-1.5: Testing and Validation - 3 days
- B-1.6: Documentation - 2 days
- **Total**: ~19 days (~4 weeks with buffer)

**Optimistic**:
- B-1.1: 2 days
- B-1.2: 3 days
- B-1.3: 3 days
- B-1.4: 2 days
- B-1.5: 2 days
- B-1.6: 1 day
- **Total**: ~13 days (~2.5 weeks)

**Realistic** (with 20% buffer):
- **Total**: ~16 days (~3 weeks)

## 7. Risk Assessment

### Medium Risk: Cache Invalidation Bugs

**Risk**: Stale symbols in global index if invalidation logic has bugs

**Likelihood**: Medium (cache invalidation is notoriously tricky)

**Impact**: High (incorrect goto-definition, missing completions)

**Mitigation**:
- Comprehensive integration tests with file modification scenarios
- Fallback mechanism: Detect staleness via timestamp check, trigger full re-index
- Validation: Compare incremental index with full re-index periodically (in tests)

### Low Risk: Dependency Graph Correctness

**Risk**: Missing dependencies leads to incomplete re-indexing

**Likelihood**: Low (contract call detection is straightforward in Rholang IR)

**Impact**: Medium (some symbols not updated after file change)

**Mitigation**:
- Conservative approach: Over-approximate dependencies (safe but less optimal)
- Validation: Manual inspection of dependency graph for test workspaces
- Monitoring: Log dependency graph size and edge count for anomaly detection

### Low Risk: Memory Overhead

**Risk**: Dependency graph and per-file symbol tables increase memory usage

**Likelihood**: Low (dependency graph is small, symbol tables already exist)

**Impact**: Low (acceptable tradeoff for 5-10x speedup)

**Mitigation**:
- Measure memory usage before/after in benchmarks
- Limit: If overhead >10%, investigate optimization (e.g., compact representation)

## 8. Success Metrics

### Quantitative Metrics

- **File Change Latency**: <10ms (from baseline ~50ms) ✅
- **Workspace Indexing**: <5ms for single file change + 2 dependents ✅
- **Memory Overhead**: <10% increase ✅
- **Dependency Graph Size**: O(n) edges where n = total files (not O(n²)) ✅

### Qualitative Metrics

- **User Feedback**: Perceived instant response to file edits ✅
- **Bug Reports**: No increase in symbol resolution bugs ✅
- **Maintainability**: Code remains understandable and testable ✅

### Phase B-1 Completion Criteria

- ✅ File change latency reduced by ≥5x (hypothesis confirmed)
- ✅ All quantitative metrics met
- ✅ No regressions in existing tests (all 412+ tests passing)
- ✅ Comprehensive documentation in optimization ledger
- ✅ Benchmarks validate predicted speedup

## 9. Next Steps

### Status: ⚠️ PLANNING COMPLETE, BLOCKED ON BASELINE MEASUREMENTS

**Blocking Issue**: liblevenshtein compilation errors prevent running `benches/indexing_performance.rs`

**Options**:
1. **Fix liblevenshtein** (requires modifying external dependency - outside project scope)
2. **Defer baselines** - Proceed with implementation based on architectural analysis
3. **Alternative benchmarks** - Create standalone benchmark without liblevenshtein dependency

**Recommended**: Option 2 - Proceed with implementation

**Rationale**:
- Phase A-4 analytical review provides sufficient baseline understanding
- Architecture-based estimates are well-founded (50ms workspace re-indexing for 100 files)
- Implementation can proceed independently of benchmark measurements
- Baselines can be collected post-implementation for validation

### This Week (if proceeding with implementation)

1. **Resolve Blocking Issue** (Decision Required):
   - User decides: Fix liblevenshtein OR defer baselines?

2. **Finalize Architecture Design** (if proceeding):
   - Review data structures with architectural constraints
   - Identify integration points in existing codebase
   - Plan backward compatibility (feature flag for incremental indexing)

3. **Start Implementation** (if unblocked):
   - Phase B-1.1: File Modification Tracking (2-3 days)

### Next Week (contingent on unblocking)

- Continue implementation: Phase B-1.2 (Dependency Graph Construction)

---

**Ledger Entry**: Phase B-1: Incremental Workspace Indexing
**Author**: Claude (via user dylon)
**Date**: 2025-11-13
**Status**: 📋 **PLANNING COMPLETE** - Ready for baseline measurements
**Predicted Speedup**: **5-10x** for file change operations
**Implementation Time**: ~3-4 weeks (conservative)

**Hardware Specifications** (for future benchmarking):
- **CPU**: Intel Xeon E5-2699 v3 @ 2.30GHz (36 physical cores, 72 threads)
- **RAM**: 252 GB DDR4-2133 ECC (8× 32GB DIMMs)
- **Storage**: Samsung SSD 990 PRO 4TB (NVMe 2.0, PCIe)
- **OS**: Linux 6.17.7-arch1-1
- **Rust**: Edition 2024

See `.claude/CLAUDE.md` for complete hardware specifications.
