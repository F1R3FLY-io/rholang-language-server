//! Benchmark suite for code completion performance optimizations
//!
//! This benchmark measures:
//! - AST traversal performance (find_node_at_position)
//! - Fuzzy matching with various dictionary sizes
//! - Prefix matching performance
//! - Full completion pipeline
//!
//! Used to establish baselines and measure improvements from:
//! - Phase 6: Position-indexed AST (O(log n) vs O(n) traversal)
//! - Phase 7: Parallel fuzzy matching with Rayon
//! - Phase 8: DoubleArrayTrie for static symbols

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use liblevenshtein::prelude::Algorithm;
use rholang_language_server::lsp::features::completion::{
    WorkspaceCompletionIndex, SymbolMetadata,
};
use rholang_language_server::lsp::features::node_finder::find_node_at_position;
use rholang_language_server::ir::rholang_node::{RholangNode, Position};
use rholang_language_server::parsers::rholang::{parse_code, parse_to_ir};
use ropey::Rope;
use tower_lsp::lsp_types::CompletionItemKind;
use std::sync::Arc;

/// Generate a Rholang AST with nested scopes
fn generate_nested_rholang(depth: usize, contracts_per_level: usize) -> String {
    let mut code = String::new();

    for level in 0..depth {
        code.push_str(&"  ".repeat(level));
        code.push_str("new ");
        for i in 0..contracts_per_level {
            if i > 0 {
                code.push_str(", ");
            }
            code.push_str(&format!("chan{}{}", level, i));
        }
        code.push_str(" in {\n");

        for i in 0..contracts_per_level {
            code.push_str(&"  ".repeat(level + 1));
            code.push_str(&format!(
                "contract proc{}{}(@arg) = {{ Nil }} |\n",
                level, i
            ));
        }
    }

    // Add target at deepest level
    code.push_str(&"  ".repeat(depth));
    code.push_str("Nil  // Target here\n");

    // Close all scopes
    for level in (0..depth).rev() {
        code.push_str(&"  ".repeat(level));
        code.push_str("}\n");
    }

    code
}

/// Benchmark AST traversal (find_node_at_position)
fn bench_ast_traversal(c: &mut Criterion) {
    let mut group = c.benchmark_group("ast_traversal");

    // Test with different AST sizes
    for (depth, contracts) in &[(5, 5), (10, 5), (15, 5), (20, 5)] {
        let node_count = depth * contracts * 2; // Approximate node count
        let code = generate_nested_rholang(*depth, *contracts);
        let tree = parse_code(&code);
        let rope = Rope::from_str(&code);
        let ir = parse_to_ir(&tree, &rope);

        // Target position at deepest level
        let target_pos = Position {
            row: depth + depth * contracts,
            column: 2,
            byte: 0,
        };

        group.throughput(Throughput::Elements(*depth as u64));
        group.bench_with_input(
            BenchmarkId::new("linear_search", node_count),
            &(ir, target_pos),
            |b: &mut criterion::Bencher, (ir, pos): &(Arc<RholangNode>, Position)| {
                b.iter(|| {
                    find_node_at_position(ir.as_ref(), black_box(pos))
                });
            },
        );
    }

    group.finish();
}

/// Benchmark fuzzy matching with different dictionary sizes
fn bench_fuzzy_matching(c: &mut Criterion) {
    let mut group = c.benchmark_group("fuzzy_matching");

    // Dictionary sizes: small, medium, large, very large
    for dict_size in &[100, 500, 1000, 5000, 10000] {
        let index = WorkspaceCompletionIndex::new();

        // Populate with generated symbols
        for i in 0..*dict_size {
            let name = format!("symbol_{:04}_contract", i);
            index.insert(
                name.clone(),
                SymbolMetadata {
                    name,
                    kind: CompletionItemKind::FUNCTION,
                    documentation: None,
                    signature: Some("contract".to_string()),
                    reference_count: 0,
                },
            );
        }

        group.throughput(Throughput::Elements(*dict_size as u64));

        // Benchmark fuzzy query with 1 edit distance
        group.bench_with_input(
            BenchmarkId::new("fuzzy_distance_1", dict_size),
            &index,
            |b, idx| {
                b.iter(|| {
                    idx.query_fuzzy(
                        black_box("symbol_042_contract"),  // Intentional typo: 042 instead of 0042
                        1,
                        Algorithm::Transposition,
                    )
                });
            },
        );

        // Benchmark prefix query (baseline comparison)
        group.bench_with_input(
            BenchmarkId::new("prefix_match", dict_size),
            &index,
            |b, idx| {
                b.iter(|| {
                    idx.query_prefix(black_box("symbol_004"))
                });
            },
        );
    }

    group.finish();
}

/// Benchmark prefix matching performance
fn bench_prefix_matching(c: &mut Criterion) {
    let mut group = c.benchmark_group("prefix_matching");

    // Test different prefix lengths
    for prefix_len in &[0, 1, 2, 3, 5, 8] {
        let index = WorkspaceCompletionIndex::new();

        // Populate with 1000 symbols
        for i in 0..1000 {
            let name = format!("contract_{:04}", i);
            index.insert(
                name.clone(),
                SymbolMetadata {
                    name,
                    kind: CompletionItemKind::FUNCTION,
                    documentation: None,
                    signature: Some("contract".to_string()),
                    reference_count: 0,
                },
            );
        }

        let prefix = &"contract_0001"[..*prefix_len];
        let prefix_string = prefix.to_string();

        group.bench_with_input(
            BenchmarkId::new("prefix_length", prefix_len),
            &prefix_string,
            |b: &mut criterion::Bencher, p: &String| {
                b.iter(|| {
                    index.query_prefix(black_box(p))
                });
            },
        );
    }

    group.finish();
}

/// Benchmark sequential vs parallel fuzzy matching (Phase 7 preparation)
fn bench_parallel_fuzzy(c: &mut Criterion) {
    let mut group = c.benchmark_group("parallel_vs_sequential");

    for dict_size in &[500, 1000, 2000, 5000, 10000] {
        let index = WorkspaceCompletionIndex::new();

        // Populate dictionary
        for i in 0..*dict_size {
            let name = format!("function_{:05}_test", i);
            index.insert(
                name.clone(),
                SymbolMetadata {
                    name,
                    kind: CompletionItemKind::FUNCTION,
                    documentation: None,
                    signature: None,
                    reference_count: 0,
                },
            );
        }

        group.throughput(Throughput::Elements(*dict_size as u64));

        // Sequential fuzzy matching (current implementation)
        group.bench_with_input(
            BenchmarkId::new("sequential", dict_size),
            &index,
            |b, idx| {
                b.iter(|| {
                    idx.query_fuzzy(
                        black_box("function_1234_test"),  // Typo: 1234 instead of 01234
                        1,
                        Algorithm::Transposition,
                    )
                });
            },
        );

        // TODO: Add parallel implementation benchmark after Phase 7
    }

    group.finish();
}

/// Benchmark context detection (determine_context)
fn bench_context_detection(c: &mut Criterion) {
    let mut group = c.benchmark_group("context_detection");

    // Test code with various contexts
    let code = r#"
        new result, x in {
            contract process(@arg1, @arg2) = {
                for (@val <- x) {
                    match val {
                        42 => {
                            // Target position here
                            Nil
                        }
                    }
                }
            } |
            process!(1, 2)
        }
    "#;

    let tree = parse_code(code);
    let rope = Rope::from_str(code);
    let ir = parse_to_ir(&tree, &rope);

    // Different target positions
    let positions = vec![
        ("contract_body", Position { row: 7, column: 16, byte: 0 }),
        ("for_body", Position { row: 5, column: 12, byte: 0 }),
        ("pattern", Position { row: 6, column: 16, byte: 0 }),
    ];

    for (name, pos) in positions {
        group.bench_with_input(
            BenchmarkId::new("find_and_classify", name),
            &(ir.clone(), pos),
            |b: &mut criterion::Bencher, (ir, pos): &(Arc<RholangNode>, Position)| {
                b.iter(|| {
                    use rholang_language_server::lsp::features::completion::determine_context;
                    determine_context(
                        black_box(ir),
                        black_box(&tower_lsp::lsp_types::Position {
                            line: pos.row as u32,
                            character: pos.column as u32,
                        }),
                    )
                });
            },
        );
    }

    group.finish();
}

/// Benchmark dictionary insertion and removal (incremental updates)
fn bench_incremental_updates(c: &mut Criterion) {
    let mut group = c.benchmark_group("incremental_updates");

    for update_size in &[10, 50, 100, 500] {
        let index = WorkspaceCompletionIndex::new();

        // Pre-populate with 1000 symbols
        for i in 0..1000 {
            let name = format!("symbol_{:04}", i);
            index.insert(
                name.clone(),
                SymbolMetadata {
                    name,
                    kind: CompletionItemKind::VARIABLE,
                    documentation: None,
                    signature: None,
                    reference_count: 0,
                },
            );
        }

        group.throughput(Throughput::Elements(*update_size as u64));

        // Benchmark removal + re-insertion (simulating file change)
        let size = *update_size;
        group.bench_with_input(
            BenchmarkId::new("update_cycle", update_size),
            &size,
            |b: &mut criterion::Bencher, size: &usize| {
                b.iter(|| {
                    // Remove symbols
                    for i in 0..*size {
                        index.remove(&format!("symbol_{:04}", i));
                    }

                    // Re-insert with new values
                    for i in 0..*size {
                        let name = format!("newsymbol_{:04}", i);
                        index.insert(
                            name.clone(),
                            SymbolMetadata {
                                name,
                                kind: CompletionItemKind::VARIABLE,
                                documentation: None,
                                signature: None,
                                reference_count: 0,
                            },
                        );
                    }
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_ast_traversal,
    bench_fuzzy_matching,
    bench_prefix_matching,
    bench_parallel_fuzzy,
    bench_context_detection,
    bench_incremental_updates,
);

criterion_main!(benches);
