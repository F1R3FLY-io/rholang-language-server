# Phase B: Medium Complexity Optimizations - PLANNING

**Status**: 📋 **PLANNING**
**Date Started**: 2025-11-13
**Expected Duration**: 1-2 weeks per optimization
**Prerequisites**: Phase A complete ✅

## Overview

Phase B focuses on **medium complexity optimizations** - improvements requiring significant architectural changes but with proven >2x speedup potential. These optimizations target the 60-80% of execution time currently dominated by I/O and parsing operations.

**Phase A Achievements** (baseline for Phase B):
- Contract queries: 2000x+ faster (85µs → 41ns)
- Pattern serialization: 2.56x faster (9.2µs → 3.59µs)
- Workspace indexing: 5.9x faster (3.15ms → 0.53ms)
- All LSP operations <200ms ✅

**Phase B Goals**:
- Target the remaining 60-80% of execution time (I/O + parsing)
- Maintain LSP responsiveness <200ms for all operations
- Focus on user-impacting scenarios (file changes, workspace initialization)

## Candidate Optimizations

Based on Phase A-4 analytical review, we identified three primary Phase B candidates:

### B-1: Incremental Workspace Indexing

**Problem**: File changes trigger full workspace re-indexing, even when only one file changed.

**Current Behavior**:
- User edits `contract.rho`
- Language server re-parses ALL workspace files
- Symbol tables rebuilt from scratch
- Completion indices repopulated entirely
- **Time**: ~10-100ms for typical workspace

**Proposed Solution**:
- Track file modification timestamps
- Maintain dependency graph (which files reference which symbols)
- Only re-index changed files + dependent files
- Incremental update of completion indices
- Cache invalidation strategy for cross-file symbols

**Predicted Speedup**: **5-10x** for file change operations
- Current: Re-index 100 files (~50ms)
- With incremental: Re-index 1-5 files (~5-10ms)

**Implementation Complexity**: Medium-High
- File change tracking: ~1 day
- Dependency graph construction: ~2-3 days
- Incremental update logic: ~2-3 days
- Cache invalidation: ~1-2 days
- Testing + documentation: ~1-2 days
- **Total**: ~7-11 days (1.5-2 weeks)

**Risk Assessment**:
- **Medium**: Cache invalidation bugs could cause stale data
- **Mitigation**: Comprehensive integration tests, fallback to full re-index

**User Impact**: **HIGH**
- Primary user workflow: Edit code → Get completions/diagnostics
- Most impactful optimization for daily development experience

**Priority**: **HIGHEST** ⭐⭐⭐

---

### B-2: Document IR Caching

**Problem**: LSP operations repeatedly parse the same unchanged files.

**Current Behavior**:
- User triggers goto-definition on `file1.rho`
- Language server parses `file1.rho` with Tree-Sitter
- Converts to IR, builds symbol tables
- User triggers hover on same file 5 seconds later
- **Re-parses the exact same file again**
- **Time**: ~1-10ms per operation (cumulative waste)

**Proposed Solution**:
- Cache parsed IR + symbol tables per file
- Key: (file path, file hash/timestamp)
- Invalidate on file modification
- LRU eviction policy (e.g., keep 50 most recent files)

**Predicted Speedup**: **10-50x** for repeated operations on same file
- Current: Re-parse every operation (~5ms)
- With caching: Hash lookup (~100µs) + IR clone (~500µs)
- **Speedup**: 5ms → 600µs ≈ **8x faster**

**Implementation Complexity**: Medium
- Cache structure: ~1 day
- Hash/timestamp tracking: ~1 day
- LRU eviction policy: ~1 day
- Integration with LSP handlers: ~1-2 days
- Testing + documentation: ~1-2 days
- **Total**: ~5-7 days (1 week)

**Risk Assessment**:
- **Low**: Caching is well-understood, easy to test
- **Mitigation**: Hash-based invalidation is reliable

**User Impact**: **MEDIUM-HIGH**
- Benefits frequent operations on same files (hover, diagnostics, etc.)
- Less impactful than incremental indexing (one-time benefit per file)

**Priority**: **HIGH** ⭐⭐

---

### B-3: Lazy IR Construction

**Problem**: Workspace initialization parses ALL files eagerly, even if never accessed.

**Current Behavior**:
- Language server starts
- Discovers 500 `.rho` files in workspace
- Parses all 500 files immediately
- **Time**: ~500ms - 5 seconds (depending on workspace size)
- User waits before first LSP feature works

**Proposed Solution**:
- Parse files **only when needed** (goto-definition, hover, etc.)
- Background indexing: Parse files in priority order
  - Priority 1: Open documents
  - Priority 2: Recently accessed files
  - Priority 3: Files referenced by open documents
  - Priority 4: Remaining workspace files (background thread)
- Maintain "indexed" vs "unindexed" file sets

**Predicted Speedup**: **2-10x** for workspace initialization
- Current: Parse all 500 files upfront (~2 seconds)
- With lazy: Parse 5 open files immediately (~20ms), rest in background
- **User-perceived speedup**: 2000ms → 20ms = **100x faster startup**

**Implementation Complexity**: Medium-High
- File discovery without parsing: ~1 day
- Priority queue for background indexing: ~2 days
- Dependency tracking (which files to parse first): ~2-3 days
- Background thread management: ~1-2 days
- Testing + documentation: ~1-2 days
- **Total**: ~7-10 days (1.5-2 weeks)

**Risk Assessment**:
- **Medium**: Background threading + priority management adds complexity
- **Mitigation**: Use Tokio async tasks, well-tested priority queue

**User Impact**: **HIGH** (for large workspaces)
- Dramatically improves startup time for large projects
- Less impactful for small workspaces (<50 files)

**Priority**: **MEDIUM-HIGH** ⭐⭐

---

## Prioritization Matrix

| Optimization | Speedup | Complexity | User Impact | Time | Priority |
|--------------|---------|------------|-------------|------|----------|
| **B-1: Incremental Indexing** | 5-10x | Medium-High | **HIGHEST** | 1.5-2 weeks | ⭐⭐⭐ **TOP** |
| **B-2: Document IR Caching** | 10-50x | Medium | Medium-High | 1 week | ⭐⭐ |
| **B-3: Lazy IR Construction** | 2-10x* | Medium-High | High (large workspaces) | 1.5-2 weeks | ⭐⭐ |

*User-perceived speedup is 100x for startup, but average operation speedup is lower.

## Recommended Order

### Option A: User Experience First (Recommended)

**Priority**: Optimize for daily developer workflow first

1. **B-1: Incremental Indexing** (1.5-2 weeks)
   - Biggest impact on daily development (file edits)
   - Solid 5-10x speedup for most common operation

2. **B-2: Document IR Caching** (1 week)
   - Complements B-1 (cache benefits from smaller incremental updates)
   - Relatively quick implementation (lower risk)

3. **B-3: Lazy IR Construction** (1.5-2 weeks)
   - Benefits one-time startup, less frequent than file edits
   - Most complex, save for when team has experience with B-1 and B-2

**Total Time**: ~4-5 weeks for all three

### Option B: Low-Hanging Fruit First

**Priority**: Easiest wins first to build momentum

1. **B-2: Document IR Caching** (1 week)
   - Simplest implementation (well-understood caching)
   - Good speedup with low risk

2. **B-1: Incremental Indexing** (1.5-2 weeks)
   - More complex, but well-motivated by B-2 experience

3. **B-3: Lazy IR Construction** (1.5-2 weeks)
   - Most complex, saved for last

**Total Time**: ~4-5 weeks for all three

### Option C: MVP Approach

**Priority**: Implement minimum viable versions quickly

1. **B-2 MVP**: Basic LRU cache for IR (2-3 days)
   - No fancy eviction, just cache last 20 files

2. **B-1 MVP**: File timestamp tracking + simple invalidation (3-4 days)
   - No dependency graph, just re-index changed file

3. **B-3 MVP**: Parse on first access (2-3 days)
   - No background indexing, just lazy parsing

**Total Time**: ~1-2 weeks for all three MVPs

**Benefit**: Quick wins, then iterate to full versions

---

## Scientific Methodology for Phase B

Following the same rigorous approach as Phase A:

### 1. Baseline Measurement (Before Implementation)

For each optimization:
- Establish current performance metrics
- Document specific scenarios to benchmark
- Record hardware/environment specifications
- Generate flamegraphs if possible

**Example for B-1 (Incremental Indexing)**:
- **Scenario**: Edit single line in `contract.rho` (1KB file)
- **Current**: Full re-index 100 files (~50ms)
- **Measure**: Time from file save to diagnostics ready
- **Baseline**: Record average over 20 iterations

### 2. Hypothesis Formation

**Example for B-1**:
- **Claim**: Incremental indexing will reduce file change overhead by 5-10x
- **Rationale**: Only 1-2 files change per edit, not all 100
- **Predicted**: 50ms → 5-10ms

### 3. Implementation with Milestones

Break each optimization into phases:
- Phase 1: Core mechanism (file tracking, cache, priority queue)
- Phase 2: Integration with existing code
- Phase 3: Testing + edge cases
- Phase 4: Documentation

Track progress in optimization ledger at each milestone.

### 4. Validation Benchmarks

**Acceptance Criteria**:
- Speedup must meet or exceed prediction (within 20%)
- No regressions in other LSP operations
- All existing tests pass
- New integration tests cover edge cases

**Rejection Criteria**:
- Speedup <50% of prediction
- Introduces critical bugs
- Complexity exceeds benefit (maintainability)

### 5. Documentation

Each Phase B optimization gets:
- **Planning document** (this file)
- **Baseline measurements** (before implementation)
- **Implementation ledger** (during development)
- **Final results** (validation + lessons learned)

---

## Dependencies and Prerequisites

### Technical Prerequisites

**For B-1 (Incremental Indexing)**:
- File system watcher (already implemented via `notify` crate)
- Symbol table structure supports incremental updates
- Completion index supports partial updates

**For B-2 (Document IR Caching)**:
- File hashing (use `blake3` or similar)
- LRU cache data structure (use `lru` crate)
- IR clone operation (already implemented via `Arc`)

**For B-3 (Lazy IR Construction)**:
- Async task management (use `tokio`)
- File discovery without parsing (use `walkdir`)
- Priority queue (use `BinaryHeap`)

### Phase A Knowledge Transfer

Leverage lessons from Phase A:
- **A-1 (Lazy Subtrie)**: Lazy evaluation patterns
- **A-2 (LRU Cache rejected)**: When caching helps vs hurts
- **A-3 (Space Pooling)**: Object reuse patterns

### Risk Mitigation

**For All Phase B Optimizations**:
- Implement feature flags (enable/disable optimizations)
- Maintain fallback to Phase A behavior
- Comprehensive integration tests
- Performance regression test suite

---

## Success Metrics

### Quantitative Metrics

- **File Change Latency**: <10ms (currently ~50ms)
- **Startup Time**: <100ms for first LSP feature (currently ~2 seconds)
- **Memory Usage**: <10% increase (acceptable tradeoff for caching)
- **Cache Hit Rate**: >80% for IR cache

### Qualitative Metrics

- **User Feedback**: Perceived responsiveness improvement
- **Bug Reports**: No increase in stability issues
- **Maintainability**: Code remains understandable and testable

### Phase B Completion Criteria

- ✅ At least 2 of 3 optimizations implemented
- ✅ All quantitative metrics met
- ✅ No regressions in Phase A performance
- ✅ Comprehensive documentation in optimization ledger
- ✅ User testing validates improvements

---

## Timeline Estimate

### Conservative (Recommended)

**Phase B-1**: 2 weeks
**Phase B-2**: 1.5 weeks
**Phase B-3**: 2 weeks
**Total**: ~5.5 weeks

### Optimistic

**Phase B-1 MVP**: 1 week
**Phase B-2 MVP**: 1 week
**Phase B-3 MVP**: 1 week
**Total**: ~3 weeks (MVPs), then iterate

### Realistic (with buffer)

**Phase B-1**: 2-3 weeks (includes unexpected issues)
**Phase B-2**: 1-2 weeks
**Phase B-3**: 2-3 weeks
**Total**: ~5-8 weeks

Add 20% buffer for integration issues, edge cases, and documentation.

---

## Next Actions

### Immediate (This Week)

1. **Select first optimization** (B-1 recommended)
2. **Establish baseline measurements**
   - Run `benches/indexing_performance.rs` with current implementation
   - Document file change scenarios (1 file, 5 files, 20 files changed)
   - Measure time from file save to diagnostics ready
3. **Create Phase B-1 planning document**
   - Detailed architecture design
   - Data structures required (dependency graph, file change tracker)
   - Integration points with existing code

### Short-term (Next 2 Weeks)

1. **Implement Phase B-1** (Incremental Indexing)
2. **Validate with benchmarks**
3. **Document results in optimization ledger**

### Medium-term (Next 1-2 Months)

1. **Implement Phase B-2** (Document IR Caching)
2. **Implement Phase B-3** (Lazy IR Construction)
3. **User testing + feedback collection**
4. **Performance regression test suite**

---

## Open Questions

1. **Should we implement MVPs first or full versions?**
   - **Recommendation**: Start with MVP for B-2 (simplest), validate approach, then full implementations

2. **How large should the IR cache be?**
   - **Recommendation**: Start with 50 files, monitor memory usage, adjust

3. **What's the priority order for background indexing?**
   - **Recommendation**: Open files → Referenced files → Alphabetical

4. **Should Phase B optimizations be feature-flagged?**
   - **Recommendation**: YES - allows A/B testing and easy rollback

---

**Phase B Status**: 📋 **PLANNING COMPLETE**
**Next Step**: Select first optimization and establish baseline measurements
**Decision Point**: User to choose between Option A (UX first), Option B (low-hanging fruit), or Option C (MVPs)

**Hardware Specifications** (for future benchmarking):
- **CPU**: Intel Xeon E5-2699 v3 @ 2.30GHz (36 physical cores, 72 threads)
- **RAM**: 252 GB DDR4-2133 ECC (8× 32GB DIMMs)
- **Storage**: Samsung SSD 990 PRO 4TB (NVMe 2.0, PCIe)
- **OS**: Linux 6.17.7-arch1-1
- **Rust**: Edition 2024

See `.claude/CLAUDE.md` for complete hardware specifications.
