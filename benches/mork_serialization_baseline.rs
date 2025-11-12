//! Baseline Benchmark for Phase A Quick Win #2: LRU Pattern Cache
//!
//! This benchmark measures the BASELINE performance of MORK serialization
//! WITHOUT caching. Results will be used to validate the hypothesis that
//! LRU caching provides 3-10x speedup for repeated patterns.
//!
//! Hypothesis (from MeTTaTron Phase 1):
//! - Current: 1-3µs per serialization
//! - With LRU cache: <100ns for cached patterns
//! - Expected speedup: 3-10x for repeated patterns
//!
//! Run with: cargo bench --bench mork_serialization_baseline

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::sync::Arc;
use std::time::Duration;

use rholang_language_server::ir::rholang_node::{NodeBase, RholangNode, Position as IrPosition};
use rholang_language_server::ir::rholang_pattern_index::RholangPatternIndex;
use mork::space::Space;
use mork_interning::SharedMapping;

/// Create test patterns of varying complexity
fn create_test_patterns() -> Vec<Arc<RholangNode>> {
    vec![
        // String literal: @"transport_object"
        Arc::new(RholangNode::StringLiteral {
            base: NodeBase::new_simple(
                IrPosition { row: 0, column: 0, byte: 0 },
                0, 0, 18
            ),
            value: "transport_object".to_string(),
            metadata: None,
        }),

        // Number literal: @42
        Arc::new(RholangNode::LongLiteral {
            base: NodeBase::new_simple(
                IrPosition { row: 0, column: 0, byte: 0 },
                0, 0, 2
            ),
            value: 42,
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

        // Boolean literal: @true
        Arc::new(RholangNode::BoolLiteral {
            base: NodeBase::new_simple(
                IrPosition { row: 0, column: 0, byte: 0 },
                0, 0, 4
            ),
            value: true,
            metadata: None,
        }),

        // Another string literal (different): @"initialize"
        Arc::new(RholangNode::StringLiteral {
            base: NodeBase::new_simple(
                IrPosition { row: 0, column: 0, byte: 0 },
                0, 0, 12
            ),
            value: "initialize".to_string(),
            metadata: None,
        }),
    ]
}

/// Benchmark: Baseline MORK serialization without caching
fn bench_mork_serialization_baseline(c: &mut Criterion) {
    let mut group = c.benchmark_group("mork_serialization_baseline");
    group.measurement_time(Duration::from_secs(10));

    let patterns = create_test_patterns();

    for (i, pattern) in patterns.iter().enumerate() {
        let pattern_name = match pattern.as_ref() {
            RholangNode::StringLiteral { value, .. } => format!("string_{}", value),
            RholangNode::LongLiteral { value, .. } => format!("int_{}", value),
            RholangNode::BoolLiteral { value, .. } => format!("bool_{}", value),
            RholangNode::Var { name, .. } => format!("var_{}", name),
            _ => format!("pattern_{}", i),
        };

        group.bench_with_input(
            BenchmarkId::from_parameter(&pattern_name),
            pattern,
            |b, pat| {
                b.iter(|| {
                    // Create Space for each iteration (no caching)
                    let shared_mapping = SharedMapping::new();
                    let space = Space {
                        btm: pathmap::PathMap::new(),
                        sm: shared_mapping,
                        mmaps: std::collections::HashMap::new(),
                    };

                    // Serialize pattern WITHOUT caching
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

/// Benchmark: Repeated serialization of same patterns (simulates contract indexing)
fn bench_repeated_pattern_serialization(c: &mut Criterion) {
    let mut group = c.benchmark_group("repeated_pattern_serialization");
    group.measurement_time(Duration::from_secs(10));

    // Simulate realistic scenario: 100 contracts, many use same patterns
    let repetition_counts = vec![10, 50, 100, 500];

    for repeat_count in repetition_counts {
        group.throughput(Throughput::Elements(repeat_count as u64));

        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}x_same_pattern", repeat_count)),
            &repeat_count,
            |b, &count| {
                let pattern = Arc::new(RholangNode::StringLiteral {
                    base: NodeBase::new_simple(
                        IrPosition { row: 0, column: 0, byte: 0 },
                        0, 0, 18
                    ),
                    value: "transport_object".to_string(),
                    metadata: None,
                });

                b.iter(|| {
                    let shared_mapping = SharedMapping::new();
                    let space = Space {
                        btm: pathmap::PathMap::new(),
                        sm: shared_mapping,
                        mmaps: std::collections::HashMap::new(),
                    };

                    // Serialize the SAME pattern multiple times (no cache benefit currently)
                    for _ in 0..count {
                        let _ = black_box(
                            RholangPatternIndex::pattern_to_mork_bytes(&pattern, &space)
                        );
                    }
                })
            },
        );
    }

    group.finish();
}

/// Benchmark: Mixed pattern serialization (simulates diverse workspace)
fn bench_mixed_pattern_serialization(c: &mut Criterion) {
    let mut group = c.benchmark_group("mixed_pattern_serialization");
    group.measurement_time(Duration::from_secs(10));

    let patterns = create_test_patterns();
    let contract_counts = vec![100, 500, 1000];

    for contract_count in contract_counts {
        group.throughput(Throughput::Elements(contract_count as u64));

        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_contracts", contract_count)),
            &contract_count,
            |b, &count| {
                b.iter(|| {
                    let shared_mapping = SharedMapping::new();
                    let space = Space {
                        btm: pathmap::PathMap::new(),
                        sm: shared_mapping,
                        mmaps: std::collections::HashMap::new(),
                    };

                    // Simulate indexing contracts with varied parameters
                    for i in 0..count {
                        let pattern_idx = i % patterns.len();
                        let _ = black_box(
                            RholangPatternIndex::pattern_to_mork_bytes(&patterns[pattern_idx], &space)
                        );
                    }
                })
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_mork_serialization_baseline,
    bench_repeated_pattern_serialization,
    bench_mixed_pattern_serialization,
);

criterion_main!(benches);
