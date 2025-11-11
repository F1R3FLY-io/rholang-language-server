//! Benchmarks for workspace indexing performance
//!
//! This benchmark suite measures the performance of workspace indexing operations
//! to establish baselines for Phase 11 (Incremental Indexing) optimization.
//!
//! Benchmarks:
//! - Full workspace indexing (10, 100, 500, 1000 files)
//! - Symbol linking (current O(n) approach)
//! - Single file re-indexing (measures current full rebuild cost)
//! - Completion index population
//!
//! Run with: cargo bench --bench indexing_performance

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::sync::Arc;
use std::time::Duration;

// Helper to create test Rholang code
fn generate_test_rholang_code(file_id: usize, contract_count: usize) -> String {
    let mut code = String::new();

    // Generate contracts
    for i in 0..contract_count {
        code.push_str(&format!(
            r#"
contract testContract{}_{} (@arg1, @arg2, ret) = {{
  new x, y, z in {{
    x!(arg1) |
    y!(arg2) |
    for (@result <- z) {{
      ret!(result)
    }}
  }}
}}

"#,
            file_id, i
        ));
    }

    // Generate some calls
    for i in 0..contract_count / 2 {
        code.push_str(&format!(
            r#"testContract{}_{}!(42, "hello", *result{}) |
"#,
            file_id, i, i
        ));
    }

    code
}

// Benchmark: Parse and index a single file
fn bench_index_single_file(c: &mut Criterion) {
    use rholang_language_server::tree_sitter::parse_to_document_ir;
    use rholang_language_server::tree_sitter::parse_code;
    use ropey::Rope;

    let mut group = c.benchmark_group("index_single_file");
    group.sample_size(50);
    group.measurement_time(Duration::from_secs(10));

    for contract_count in [10, 50, 100] {
        let code = generate_test_rholang_code(0, contract_count);
        let rope = Rope::from_str(&code);

        group.bench_with_input(
            BenchmarkId::new("contracts", contract_count),
            &contract_count,
            |b, _| {
                b.iter(|| {
                    let tree = Arc::new(parse_code(&code));
                    let document_ir = parse_to_document_ir(&tree, &rope);
                    black_box(document_ir)
                });
            },
        );
    }

    group.finish();
}

// Benchmark: Symbol table building for a single file
fn bench_symbol_table_building(c: &mut Criterion) {
    use rholang_language_server::tree_sitter::{parse_to_document_ir, parse_code};
    use rholang_language_server::ir::symbol_table::SymbolTable;
    use rholang_language_server::ir::transforms::symbol_table_builder::SymbolTableBuilder;
    use rholang_language_server::ir::pipeline::{Pipeline, Transform, TransformKind};
    use ropey::Rope;
    use tower_lsp::lsp_types::Url;

    let mut group = c.benchmark_group("symbol_table_building");
    group.sample_size(50);

    for contract_count in [10, 50, 100] {
        let code = generate_test_rholang_code(0, contract_count);
        let rope = Rope::from_str(&code);
        let tree = Arc::new(parse_code(&code));
        let document_ir = parse_to_document_ir(&tree, &rope);
        let uri = Url::parse("file:///test.rho").unwrap();
        let global_table = Arc::new(SymbolTable::new(None));

        group.bench_with_input(
            BenchmarkId::new("contracts", contract_count),
            &contract_count,
            |b, _| {
                b.iter(|| {
                    let mut pipeline = Pipeline::new();
                    let builder = Arc::new(SymbolTableBuilder::new(
                        document_ir.root.clone(),
                        uri.clone(),
                        global_table.clone(),
                        None,
                    ));
                    pipeline.add_transform(Transform {
                        id: "symbol_table_builder".to_string(),
                        dependencies: vec![],
                        kind: TransformKind::Specific(builder.clone()),
                    });
                    let transformed_ir = pipeline.apply(&document_ir.root);
                    black_box(transformed_ir)
                });
            },
        );
    }

    group.finish();
}

// Benchmark: Simulated symbol linking across N files
fn bench_symbol_linking_simulation(c: &mut Criterion) {
    use std::collections::HashMap;
    use tower_lsp::lsp_types::Url;

    let mut group = c.benchmark_group("symbol_linking_simulation");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(15));

    // Simulate symbol linking by iterating over files and symbols
    // This approximates the O(n × m) behavior of link_symbols()

    for file_count in [10, 50, 100, 500] {
        let symbols_per_file = 100;

        // Pre-generate "symbol tables" (simulated as HashMaps)
        let mut documents: HashMap<Url, Vec<String>> = HashMap::new();
        for i in 0..file_count {
            let uri = Url::parse(&format!("file:///test{}.rho", i)).unwrap();
            let symbols: Vec<String> = (0..symbols_per_file)
                .map(|j| format!("symbol_{}_{}", i, j))
                .collect();
            documents.insert(uri, symbols);
        }

        group.bench_with_input(
            BenchmarkId::new("files", file_count),
            &file_count,
            |b, _| {
                b.iter(|| {
                    // Simulate O(n × m) symbol linking
                    let mut cross_refs: HashMap<String, Vec<Url>> = HashMap::new();

                    for (uri, symbols) in &documents {
                        for symbol in symbols {
                            cross_refs
                                .entry(symbol.clone())
                                .or_insert_with(Vec::new)
                                .push(uri.clone());
                        }
                    }

                    black_box(cross_refs)
                });
            },
        );
    }

    group.finish();
}

// Benchmark: Completion index population
fn bench_completion_index_population(c: &mut Criterion) {
    use rholang_language_server::lsp::features::completion::{
        WorkspaceCompletionIndex, SymbolMetadata,
    };
    use tower_lsp::lsp_types::CompletionItemKind;

    let mut group = c.benchmark_group("completion_index_population");
    group.sample_size(30);

    for symbol_count in [100, 500, 1000, 5000] {
        let symbols: Vec<(String, SymbolMetadata)> = (0..symbol_count)
            .map(|i| {
                (
                    format!("testSymbol{}", i),
                    SymbolMetadata {
                        name: format!("testSymbol{}", i),
                        kind: CompletionItemKind::VARIABLE,
                        documentation: None,
                        signature: None,
                        reference_count: i % 10,
                    },
                )
            })
            .collect();

        group.bench_with_input(
            BenchmarkId::new("symbols", symbol_count),
            &symbol_count,
            |b, _| {
                b.iter(|| {
                    let index = WorkspaceCompletionIndex::new();
                    for (name, metadata) in &symbols {
                        index.insert(name.clone(), metadata.clone());
                    }
                    black_box(index)
                });
            },
        );
    }

    group.finish();
}

// Benchmark: Completion index update (full rebuild - current approach)
fn bench_completion_index_update(c: &mut Criterion) {
    use rholang_language_server::lsp::features::completion::{
        WorkspaceCompletionIndex, SymbolMetadata,
    };
    use tower_lsp::lsp_types::CompletionItemKind;

    let mut group = c.benchmark_group("completion_index_update");
    group.sample_size(30);

    // Simulate full rebuild (current approach) vs incremental update (Phase 11)
    for symbol_count in [100, 500, 1000, 5000] {
        let symbols: Vec<(String, SymbolMetadata)> = (0..symbol_count)
            .map(|i| {
                (
                    format!("symbol{}", i),
                    SymbolMetadata {
                        name: format!("symbol{}", i),
                        kind: CompletionItemKind::VARIABLE,
                        documentation: None,
                        signature: None,
                        reference_count: 0,
                    },
                )
            })
            .collect();

        group.bench_with_input(
            BenchmarkId::new("full_rebuild", symbol_count),
            &symbol_count,
            |b, _| {
                b.iter(|| {
                    // Current approach: full rebuild on every file change
                    let index = WorkspaceCompletionIndex::new();
                    for (name, metadata) in &symbols {
                        index.insert(name.clone(), metadata.clone());
                    }
                    black_box(index)
                });
            },
        );
    }

    group.finish();
}

// Benchmark: File change pipeline (parse + index + link)
fn bench_file_change_overhead(c: &mut Criterion) {
    use rholang_language_server::tree_sitter::{parse_to_document_ir, parse_code};
    use rholang_language_server::ir::symbol_table::SymbolTable;
    use rholang_language_server::ir::transforms::symbol_table_builder::SymbolTableBuilder;
    use rholang_language_server::ir::pipeline::{Pipeline, Transform, TransformKind};
    use ropey::Rope;
    use tower_lsp::lsp_types::Url;

    let mut group = c.benchmark_group("file_change_overhead");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(10));

    let code = generate_test_rholang_code(0, 50);
    let uri = Url::parse("file:///test.rho").unwrap();
    let global_table = Arc::new(SymbolTable::new(None));

    group.bench_function("parse_and_index", |b| {
        b.iter(|| {
            let rope = Rope::from_str(&code);
            let tree = Arc::new(parse_code(&code));
            let document_ir = parse_to_document_ir(&tree, &rope);

            let mut pipeline = Pipeline::new();
            let builder = Arc::new(SymbolTableBuilder::new(
                document_ir.root.clone(),
                uri.clone(),
                global_table.clone(),
                None,
            ));
            pipeline.add_transform(Transform {
                id: "symbol_table_builder".to_string(),
                dependencies: vec![],
                kind: TransformKind::Specific(builder),
            });

            let transformed_ir = pipeline.apply(&document_ir.root);
            black_box(transformed_ir)
        });
    });

    group.finish();
}

// Criterion configuration
criterion_group!(
    name = benches;
    config = Criterion::default()
        .sample_size(50)
        .measurement_time(Duration::from_secs(10))
        .warm_up_time(Duration::from_secs(3));
    targets =
        bench_index_single_file,
        bench_symbol_table_building,
        bench_symbol_linking_simulation,
        bench_completion_index_population,
        bench_completion_index_update,
        bench_file_change_overhead
);

criterion_main!(benches);
