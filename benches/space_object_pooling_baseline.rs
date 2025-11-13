//! Baseline Benchmark for Phase A Quick Win #3: Space Object Pooling
//!
//! This benchmark measures the BASELINE cost of creating MORK `Space` objects
//! to determine if object pooling would provide sufficient performance benefit.
//!
//! ## Hypothesis
//!
//! **Primary**: Space object pooling will provide 10x speedup by eliminating allocation overhead
//!
//! **Secondary (from Phase A-2 evidence)**: Space::new() cost is negligible,
//! pooling will provide <2x benefit and should be REJECTED
//!
//! ## Measurement Strategy
//!
//! 1. **Group 1: Space Creation Baseline**
//!    - Measure raw cost of `Space::new()` at different scales
//!    - Acceptance threshold: >1µs per Space creation
//!
//! 2. **Group 2: Space + MORK Serialization**
//!    - Measure combined cost (Space creation + serialization)
//!    - Compare to Phase A-2 baseline (3.0µs per pattern)
//!
//! 3. **Group 3: Workspace Simulation**
//!    - Realistic scenario: 1000 contracts with unique patterns
//!    - Measure total indexing time with Space::new() per contract
//!
//! ## Phase A-2 Evidence
//!
//! Phase A-2 benchmarks created new Space objects on every iteration:
//! ```rust
//! let space = Space { btm: PathMap::new(), sm: shared_mapping, mmaps: HashMap::new() };
//! ```
//!
//! Yet still achieved 8-10x speedup for repeated patterns (0.37µs vs 3µs baseline).
//! This suggests Space::new() cost is NOT a significant bottleneck.
//!
//! ## Expected Outcome
//!
//! **Prediction**: Space::new() costs <0.5µs, pooling provides <1.5x speedup → REJECT

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use mork::space::Space;
use mork_interning::SharedMapping;
use pathmap::PathMap;
use rholang_language_server::ir::rholang_node::{NodeBase, RholangNode, Position as IrPosition};
use rholang_language_server::ir::rholang_pattern_index::RholangPatternIndex;
use std::collections::HashMap;
use std::sync::Arc;

/// Create test patterns for MORK serialization
fn create_test_patterns() -> Vec<Arc<RholangNode>> {
    vec![
        // String literal: @"transport_object"
        Arc::new(RholangNode::StringLiteral {
            value: "transport_object".to_string(),
            base: NodeBase::new_simple(
                IrPosition { row: 0, column: 0, byte: 0 },
                0, 0, 21
            ),
            metadata: None,
        }),
        // Number literal: @42
        Arc::new(RholangNode::LongLiteral {
            value: 42,
            base: NodeBase::new_simple(
                IrPosition { row: 0, column: 0, byte: 0 },
                0, 0, 2
            ),
            metadata: None,
        }),
        // Variable pattern: @x
        Arc::new(RholangNode::Var {
            name: "x".to_string(),
            base: NodeBase::new_simple(
                IrPosition { row: 0, column: 0, byte: 0 },
                0, 0, 1
            ),
            metadata: None,
        }),
    ]
}

/// Benchmark Group 1: Space Creation Baseline
///
/// Measures the RAW cost of creating Space objects at different scales.
/// This isolates the allocation overhead that pooling aims to eliminate.
fn bench_space_creation_baseline(c: &mut Criterion) {
    let mut group = c.benchmark_group("space_creation_baseline");

    let counts = vec![1, 10, 100, 1000];

    for count in counts {
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(
            BenchmarkId::new("create_spaces", count),
            &count,
            |b, &count| {
                b.iter(|| {
                    for _ in 0..count {
                        // Create new Space object (what pooling aims to avoid)
                        let space = black_box(Space::new());

                        // Ensure space is not optimized away
                        black_box(space);
                    }
                })
            },
        );
    }

    group.finish();
}

/// Benchmark Group 2: Space + MORK Serialization
///
/// Measures combined cost of Space creation + MORK serialization.
/// Compares to Phase A-2 baseline (3.0µs per pattern).
fn bench_space_with_mork_serialization(c: &mut Criterion) {
    let mut group = c.benchmark_group("space_with_mork_serialization");

    let patterns = create_test_patterns();
    let pattern_names = vec!["string", "integer", "variable"];

    for (i, pattern) in patterns.iter().enumerate() {
        group.bench_with_input(
            BenchmarkId::new("serialize_with_new_space", pattern_names[i]),
            pattern,
            |b, pat| {
                b.iter(|| {
                    // Create new Space (pooling would reuse this)
                    let space = Space::new();

                    // Serialize pattern to MORK bytes
                    let result = black_box(
                        RholangPatternIndex::pattern_to_mork_bytes(pat, &space)
                    );
                    result
                })
            },
        );
    }

    group.finish();
}

/// Benchmark Group 3: Workspace Simulation
///
/// Realistic scenario: Index 1000 contracts, each requiring Space for MORK serialization.
/// This measures the cumulative impact of Space::new() overhead at workspace scale.
fn bench_workspace_simulation(c: &mut Criterion) {
    let mut group = c.benchmark_group("workspace_simulation");

    let contract_counts = vec![100, 500, 1000];

    for count in contract_counts {
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(
            BenchmarkId::new("index_contracts", count),
            &count,
            |b, &count| {
                // Create unique patterns (cycling through 5 base patterns)
                let base_patterns = create_test_patterns();

                b.iter(|| {
                    for i in 0..count {
                        // Select pattern (cycle through base patterns)
                        let pattern = &base_patterns[i % base_patterns.len()];

                        // Create new Space for each contract (pooling would reuse)
                        let space = Space::new();

                        // Serialize to MORK
                        let result = black_box(
                            RholangPatternIndex::pattern_to_mork_bytes(pattern, &space)
                        );
                        black_box(result);
                    }
                })
            },
        );
    }

    group.finish();
}

/// Benchmark Group 4: Space Creation vs Reuse
///
/// Direct comparison: Creating new Space vs reusing existing Space (simulated pooling).
/// This measures the ACTUAL benefit pooling would provide.
fn bench_space_creation_vs_reuse(c: &mut Criterion) {
    let mut group = c.benchmark_group("space_creation_vs_reuse");

    let patterns = create_test_patterns();

    // Benchmark: Create new Space each time
    group.bench_function("create_new_space", |b| {
        b.iter(|| {
            for pattern in &patterns {
                let space = Space::new();

                let result = black_box(
                    RholangPatternIndex::pattern_to_mork_bytes(pattern, &space)
                );
                black_box(result);
            }
        })
    });

    // Benchmark: Reuse same Space (simulated pooling)
    group.bench_function("reuse_space", |b| {
        b.iter(|| {
            // Reuse same Space instance (what pooling provides)
            let mut space = Space::new();

            for pattern in &patterns {
                let result = black_box(
                    RholangPatternIndex::pattern_to_mork_bytes(pattern, &space)
                );
                black_box(result);

                // Reset Space state (pooling would do this on release)
                space.btm = PathMap::new();
                space.mmaps.clear();
            }
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_space_creation_baseline,
    bench_space_with_mork_serialization,
    bench_workspace_simulation,
    bench_space_creation_vs_reuse,
);
criterion_main!(benches);
