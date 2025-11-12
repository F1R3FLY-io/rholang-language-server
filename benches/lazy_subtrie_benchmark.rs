//! Benchmarks for Phase A Quick Win #1: Lazy Subtrie Extraction
//!
//! This benchmark suite scientifically validates the lazy subtrie extraction optimization
//! by comparing workspace symbol queries with different approaches:
//!
//! 1. **Baseline (HashMap iteration)**: O(n) where n = total symbols
//! 2. **Lazy Subtrie (PathMap.restrict())**: O(k+m) where k = prefix length, m = contracts
//!
//! Expected Results (from MeTTaTron Phase 1):
//! - 100 contracts in 10K workspace: ~100x speedup
//! - 1000 contracts in 10K workspace: ~10x speedup
//! - 100 contracts in 100K workspace: ~1000x speedup (551x measured in MeTTaTron)
//!
//! Hypothesis: Speedup = total_symbols / contracts (asymptotically)
//!
//! Run with: cargo bench --bench lazy_subtrie_benchmark
//! Generate flamegraph: cargo flamegraph --bench lazy_subtrie_benchmark -- --bench

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::sync::Arc;
use std::time::Duration;
use tower_lsp::lsp_types::{Position, Range, Url};

use rholang_language_server::ir::global_index::{GlobalSymbolIndex, SymbolKind, SymbolLocation};
use rholang_language_server::ir::rholang_node::{NodeBase, RholangNode, RholangNodeVector, Position as IrPosition};

/// Generate a test contract node with given name
fn create_test_contract(name: &str, param_count: usize) -> RholangNode {
    // Create contract formals (parameters) using RholangNodeVector
    let mut formals: RholangNodeVector = RholangNodeVector::new_with_ptr_kind();
    for i in 0..param_count {
        let param = Arc::new(RholangNode::Var {
            name: format!("param{}", i),
            base: NodeBase::new_simple(
                IrPosition { row: 0, column: 0, byte: 0 },
                0, 0, 10
            ),
            metadata: None,
        });
        formals = formals.push_back(param);
    }

    // Create contract name node
    let name_node = Arc::new(RholangNode::Var {
        name: name.to_string(),
        base: NodeBase::new_simple(
            IrPosition { row: 0, column: 0, byte: 0 },
            0, 0, name.len()
        ),
        metadata: None,
    });

    // Create contract proc (Nil for simplicity)
    let proc = Arc::new(RholangNode::Nil {
        base: NodeBase::new_simple(
            IrPosition { row: 0, column: 0, byte: 0 },
            0, 0, 3
        ),
        metadata: None,
    });

    RholangNode::Contract {
        base: NodeBase::new_simple(
            IrPosition { row: 0, column: 0, byte: 0 },
            0, 0, 100
        ),
        name: name_node,
        formals,
        formals_remainder: None,
        proc,
        metadata: None,
    }
}

/// Create a test symbol location
fn create_test_location(uri_str: &str, line: u32) -> SymbolLocation {
    SymbolLocation {
        uri: Url::parse(uri_str).unwrap(),
        range: Range {
            start: Position { line, character: 0 },
            end: Position { line, character: 100 },
        },
        kind: SymbolKind::Contract,
        documentation: None,
        signature: Some(format!("contract test{}", line)),
    }
}

/// Populate GlobalSymbolIndex with test contracts
fn populate_index_with_contracts(
    index: &mut GlobalSymbolIndex,
    contract_count: usize,
    param_count: usize,
) -> Result<(), String> {
    for i in 0..contract_count {
        let name = format!("contract{}", i);
        let contract_node = create_test_contract(&name, param_count);
        let location = create_test_location("file:///test.rho", i as u32);

        index.add_contract_with_pattern_index(&contract_node, location)?;
    }
    Ok(())
}

/// Populate GlobalSymbolIndex with test channels (non-contracts)
fn populate_index_with_channels(
    index: &mut GlobalSymbolIndex,
    channel_count: usize,
) -> Result<(), String> {
    for i in 0..channel_count {
        let name = format!("channel{}", i);
        let location = create_test_location("file:///channels.rho", i as u32);
        index.add_channel_definition(&name, location)?;
    }
    Ok(())
}

/// Benchmark: Lazy subtrie extraction with different contract counts
fn bench_lazy_subtrie_query(c: &mut Criterion) {
    let mut group = c.benchmark_group("lazy_subtrie_query");
    group.measurement_time(Duration::from_secs(10));

    for contract_count in [100, 500, 1000, 5000].iter() {
        group.throughput(Throughput::Elements(*contract_count as u64));

        group.bench_with_input(
            BenchmarkId::from_parameter(contract_count),
            contract_count,
            |b, &count| {
                // Setup: Create index with contracts
                let mut index = GlobalSymbolIndex::new();
                populate_index_with_contracts(&mut index, count, 2)
                    .expect("Failed to populate index");

                b.iter(|| {
                    // Query all contracts using lazy subtrie
                    let results = black_box(index.query_all_contracts());
                    results.expect("Query failed")
                });
            },
        );
    }

    group.finish();
}

/// Benchmark: Lazy subtrie with varying workspace sizes (different ratios)
///
/// This tests the hypothesis: speedup = total_symbols / contracts
fn bench_lazy_subtrie_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("lazy_subtrie_scaling");
    group.measurement_time(Duration::from_secs(15));

    // Test cases: (contracts, channels, expected_ratio)
    let test_cases = vec![
        (100, 900, "10%_contracts"),     // 100 contracts in 1K workspace → 10% ratio
        (100, 9900, "1%_contracts"),     // 100 contracts in 10K workspace → 1% ratio
        (1000, 9000, "10%_contracts"),   // 1000 contracts in 10K workspace → 10% ratio
        (100, 99900, "0.1%_contracts"),  // 100 contracts in 100K workspace → 0.1% ratio
    ];

    for (contracts, channels, label) in test_cases {
        group.throughput(Throughput::Elements(contracts as u64));

        group.bench_with_input(
            BenchmarkId::from_parameter(label),
            &(contracts, channels),
            |b, &(c_count, ch_count)| {
                // Setup: Create index with contracts and channels
                let mut index = GlobalSymbolIndex::new();
                populate_index_with_contracts(&mut index, c_count, 2)
                    .expect("Failed to populate contracts");
                populate_index_with_channels(&mut index, ch_count)
                    .expect("Failed to populate channels");

                b.iter(|| {
                    // Query all contracts using lazy subtrie
                    let results = black_box(index.query_all_contracts());
                    results.expect("Query failed")
                });
            },
        );
    }

    group.finish();
}

/// Benchmark: Baseline comparison using HashMap iteration
///
/// This measures the cost of iterating through all definitions to find contracts
fn bench_baseline_hashmap_iteration(c: &mut Criterion) {
    let mut group = c.benchmark_group("baseline_hashmap");
    group.measurement_time(Duration::from_secs(10));

    for total_symbols in [1000, 10000, 100000].iter() {
        group.throughput(Throughput::Elements(*total_symbols as u64));

        group.bench_with_input(
            BenchmarkId::from_parameter(total_symbols),
            total_symbols,
            |b, &count| {
                // Setup: Create index with mix of contracts and channels
                let mut index = GlobalSymbolIndex::new();
                let contracts = count / 10; // 10% contracts
                let channels = count - contracts;

                populate_index_with_contracts(&mut index, contracts, 2)
                    .expect("Failed to populate contracts");
                populate_index_with_channels(&mut index, channels)
                    .expect("Failed to populate channels");

                b.iter(|| {
                    // Baseline: Iterate through all definitions
                    let mut contract_count = 0;
                    for (_symbol_id, location) in &index.definitions {
                        if location.kind == SymbolKind::Contract {
                            contract_count += 1;
                        }
                    }
                    black_box(contract_count)
                });
            },
        );
    }

    group.finish();
}

/// Benchmark: Cache effectiveness (first query vs subsequent queries)
fn bench_cache_effectiveness(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache_effectiveness");
    group.measurement_time(Duration::from_secs(10));

    let contract_count = 1000;
    let channel_count = 9000;

    // Setup once
    let mut index = GlobalSymbolIndex::new();
    populate_index_with_contracts(&mut index, contract_count, 2)
        .expect("Failed to populate contracts");
    populate_index_with_channels(&mut index, channel_count)
        .expect("Failed to populate channels");

    group.bench_function("first_query_cold_cache", |b| {
        b.iter(|| {
            // Invalidate cache before each iteration
            index.invalidate_contract_index();
            let results = black_box(index.query_all_contracts());
            results.expect("Query failed")
        });
    });

    group.bench_function("subsequent_query_warm_cache", |b| {
        // Prime the cache once
        let _ = index.query_all_contracts();

        b.iter(|| {
            // Query with warm cache
            let results = black_box(index.query_all_contracts());
            results.expect("Query failed")
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_lazy_subtrie_query,
    bench_lazy_subtrie_scaling,
    bench_baseline_hashmap_iteration,
    bench_cache_effectiveness,
);

criterion_main!(benches);
