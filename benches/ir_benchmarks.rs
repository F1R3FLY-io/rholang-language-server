//! Comprehensive benchmarks for IR operations
//!
//! Measures performance of:
//! - Tree-Sitter CST to IR conversion
//! - Par node handling (flat vs deeply nested)
//! - Visitor pattern traversals
//! - Position calculations
//! - Metadata allocation
//! - Symbol table building for Rholang

use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use std::sync::Arc;
use std::time::Duration;
use ropey::Rope;

use rholang_language_server::tree_sitter;
use rholang_language_server::parsers::rholang::parse_to_ir;
use rholang_language_server::ir::rholang_node::RholangNode;
use rholang_language_server::ir::transforms::symbol_table_builder::SymbolTableBuilder;
use rholang_language_server::ir::visitor::Visitor;
use rholang_language_server::ir::symbol_table::SymbolTable;
use tower_lsp::lsp_types::Url;

// ============================================================================
// Sample Rholang code for benchmarking
// ============================================================================

/// Small: Simple contract definition (~5 lines)
const RHOLANG_SMALL: &str = r#"
contract @"myContract"(x, y) = {
  x!(y)
}
"#;

/// Medium: Multiple contracts with patterns (~50 lines)
const RHOLANG_MEDIUM: &str = r#"
new stdout(`rho:io:stdout`) in {
  contract processUser(@{name: userName, age: userAge}, ret) = {
    stdout!(["Processing user:", userName, "age:", userAge]) |
    ret!(userName)
  } |

  contract sumThree(@[first, second, third], ret) = {
    ret!(first + second + third)
  } |

  contract coordinate(@(x, y, z), ret) = {
    stdout!(["Coordinate:", x, y, z]) |
    ret!((x, y, z))
  } |

  contract processAddress(@{street: s, city: {name: cityName, zip: zipCode}}, ret) = {
    stdout!(["Address:", s, cityName, zipCode]) |
    ret!(s)
  } |

  @"processUser"!({"name": "Alice", "age": 30}, *stdout) |
  sumThree!([10, 20, 30], *stdout) |
  coordinate!((1, 2, 3), *stdout) |
  processAddress!({"street": "Main St", "city": {"name": "NYC", "zip": "10001"}}, *stdout)
}
"#;

/// Large: Complex contract with deep nesting (~120 lines)
const RHOLANG_LARGE: &str = r#"
new stdout(`rho:io:stdout`) in {
  contract processUser(@{name: userName, age: userAge}, ret) = {
    stdout!(["Processing user:", userName, "age:", userAge]) |
    ret!(userName)
  } |

  contract sumThree(@[first, second, third], ret) = {
    ret!(first + second + third)
  } |

  contract coordinate(@(x, y, z), ret) = {
    stdout!(["Coordinate:", x, y, z]) |
    ret!((x, y, z))
  } |

  contract processAddress(@{street: s, city: {name: cityName, zip: zipCode}}, ret) = {
    stdout!(["Address:", s, cityName, zipCode]) |
    ret!(s)
  } |

  contract processMatrix(@[[a, b], [c, d]], ret) = {
    stdout!(["Matrix:", a, b, c, d]) |
    ret!(a + b + c + d)
  } |

  contract processData(@{items: [item1, item2], total: count}, ret) = {
    stdout!(["Items:", item1, item2, "Total:", count]) |
    ret!(count)
  } |

  contract @{"action": "get_user"}(@{id: userId}, ret) = {
    stdout!(["Getting user with ID:", userId]) |
    ret!(userId)
  } |

  contract @["command", "execute"](@{name: cmdName, args: cmdArgs}, ret) = {
    stdout!(["Executing:", cmdName, "with args:", cmdArgs]) |
    ret!(cmdName)
  } |

  contract processDeep(@{
    outer: {
      middle: {
        inner: value
      }
    }
  }, ret) = {
    stdout!(["Deep value:", value]) |
    ret!(value)
  } |

  contract processComplex(@{
    user: {name: n, email: e},
    address: {street: st, city: ct},
    metadata: {created: cr, updated: up}
  }, ret) = {
    stdout!(["User:", n, e, "Address:", st, ct, "Meta:", cr, up]) |
    ret!((n, e, st, ct, cr, up))
  } |

  contract processPartial(@{required: req, optional: _, extra: ext}, ret) = {
    stdout!(["Required:", req, "Extra:", ext]) |
    ret!(req)
  } |

  contract processTupleMap(@(id, {name: itemName, price: itemPrice}), ret) = {
    stdout!(["Item:", id, itemName, itemPrice]) |
    ret!((id, itemName, itemPrice))
  } |

  contract processListTuple(@[(x1, y1), (x2, y2), (x3, y3)], ret) = {
    stdout!(["Points:", x1, y1, x2, y2, x3, y3]) |
    ret!(x1 + x2 + x3)
  } |

  @"processUser"!({"name": "Alice", "age": 30}, *stdout) |
  sumThree!([10, 20, 30], *stdout) |
  coordinate!((1, 2, 3), *stdout) |
  processAddress!({"street": "Main St", "city": {"name": "NYC", "zip": "10001"}}, *stdout) |
  processMatrix!([[1, 2], [3, 4]], *stdout) |
  processData!({"items": ["apple", "banana"], "total": 2}, *stdout) |
  @{"action": "get_user"}!({"id": "user123"}, *stdout) |
  @["command", "execute"]!({"name": "test", "args": []}, *stdout) |
  processDeep!({"outer": {"middle": {"inner": "secret"}}}, *stdout) |
  processComplex!({
    "user": {"name": "Bob", "email": "bob@example.com"},
    "address": {"street": "Oak Ave", "city": "LA"},
    "metadata": {"created": "2024-01-01", "updated": "2024-12-31"}
  }, *stdout) |
  processPartial!({"required": "yes", "optional": "ignored", "extra": "bonus"}, *stdout) |
  processTupleMap!((42, {"name": "Widget", "price": 19.99}), *stdout) |
  processListTuple!([(0, 0), (1, 1), (2, 2)], *stdout)
}
"#;

/// Generate code with deeply nested Par nodes
fn generate_deeply_nested_par(depth: usize) -> String {
    let mut code = String::from("new stdout(`rho:io:stdout`) in {\n");
    for i in 0..depth {
        if i > 0 {
            code.push_str(" |\n");
        }
        code.push_str(&format!("  stdout!(\"Process {}\")", i));
    }
    code.push_str("\n}");
    code
}

/// Generate code with many parallel processes (tests n-ary Par performance)
fn generate_many_parallel_processes(count: usize) -> String {
    let mut code = String::from("new x, y, z in {\n");
    for i in 0..count {
        if i > 0 {
            code.push_str(" |\n");
        }
        code.push_str(&format!("  x!({}) | y!({}) | z!({})", i, i + 1, i + 2));
    }
    code.push_str("\n}");
    code
}

// ============================================================================
// Benchmark: Tree-Sitter to IR Conversion
// ============================================================================

fn bench_tree_sitter_to_ir_conversion(c: &mut Criterion) {
    let mut group = c.benchmark_group("tree_sitter_to_ir");

    group.bench_function("small", |b| {
        b.iter(|| {
            let tree = tree_sitter::parse_code(RHOLANG_SMALL);
            let rope = Rope::from_str(RHOLANG_SMALL);
            black_box(parse_to_ir(&tree, &rope))
        })
    });

    group.bench_function("medium", |b| {
        b.iter(|| {
            let tree = tree_sitter::parse_code(RHOLANG_MEDIUM);
            let rope = Rope::from_str(RHOLANG_MEDIUM);
            black_box(parse_to_ir(&tree, &rope))
        })
    });

    group.bench_function("large", |b| {
        b.iter(|| {
            let tree = tree_sitter::parse_code(RHOLANG_LARGE);
            let rope = Rope::from_str(RHOLANG_LARGE);
            black_box(parse_to_ir(&tree, &rope))
        })
    });

    group.finish();
}

// ============================================================================
// Benchmark: Par Node Handling (Nested vs Flat)
// ============================================================================

fn bench_par_node_handling(c: &mut Criterion) {
    let mut group = c.benchmark_group("par_node_handling");

    // Benchmark with varying depths of nested Par nodes
    for depth in [10, 50, 100, 500].iter() {
        let code = generate_deeply_nested_par(*depth);
        group.bench_with_input(
            BenchmarkId::new("nested_par", depth),
            &code,
            |b, code| {
                b.iter(|| {
                    let tree = tree_sitter::parse_code(code);
                    let rope = Rope::from_str(code);
                    black_box(parse_to_ir(&tree, &rope))
                })
            },
        );
    }

    // Benchmark with many parallel processes
    for count in [10, 50, 100].iter() {
        let code = generate_many_parallel_processes(*count);
        group.bench_with_input(
            BenchmarkId::new("parallel_processes", count),
            &code,
            |b, code| {
                b.iter(|| {
                    let tree = tree_sitter::parse_code(code);
                    let rope = Rope::from_str(code);
                    black_box(parse_to_ir(&tree, &rope))
                })
            },
        );
    }

    group.finish();
}

// ============================================================================
// Benchmark: Symbol Table Building
// ============================================================================

fn bench_symbol_table_building(c: &mut Criterion) {
    let mut group = c.benchmark_group("symbol_table_building");

    // Pre-parse for benchmarking
    let small_tree = tree_sitter::parse_code(RHOLANG_SMALL);
    let small_rope = Rope::from_str(RHOLANG_SMALL);
    let small_ir = parse_to_ir(&small_tree, &small_rope);

    let medium_tree = tree_sitter::parse_code(RHOLANG_MEDIUM);
    let medium_rope = Rope::from_str(RHOLANG_MEDIUM);
    let medium_ir = parse_to_ir(&medium_tree, &medium_rope);

    let large_tree = tree_sitter::parse_code(RHOLANG_LARGE);
    let large_rope = Rope::from_str(RHOLANG_LARGE);
    let large_ir = parse_to_ir(&large_tree, &large_rope);

    let uri = Url::parse("file:///test.rho").unwrap();
    let global_table = Arc::new(SymbolTable::new(None));

    group.bench_function("small", |b| {
        b.iter(|| {
            let builder = SymbolTableBuilder::new(small_ir.clone(), uri.clone(), global_table.clone(), None);
            black_box(builder.visit_node(&small_ir))
        })
    });

    group.bench_function("medium", |b| {
        b.iter(|| {
            let builder = SymbolTableBuilder::new(medium_ir.clone(), uri.clone(), global_table.clone(), None);
            black_box(builder.visit_node(&medium_ir))
        })
    });

    group.bench_function("large", |b| {
        b.iter(|| {
            let builder = SymbolTableBuilder::new(large_ir.clone(), uri.clone(), global_table.clone(), None);
            black_box(builder.visit_node(&large_ir))
        })
    });

    group.finish();
}

// ============================================================================
// Benchmark: Visitor Pattern Traversal
// ============================================================================

/// Simple visitor that just counts nodes (no modifications)
struct NodeCounterVisitor {
    count: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl Visitor for NodeCounterVisitor {
    fn visit_node(&self, node: &Arc<RholangNode>) -> Arc<RholangNode> {
        self.count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // Use default traversal by pattern matching and visiting children
        // For now, just clone the node (simple counting visitor)
        Arc::clone(node)
    }
}

fn bench_visitor_traversal(c: &mut Criterion) {
    let mut group = c.benchmark_group("visitor_traversal");

    // Pre-parse
    let small_tree = tree_sitter::parse_code(RHOLANG_SMALL);
    let small_rope = Rope::from_str(RHOLANG_SMALL);
    let small_ir = parse_to_ir(&small_tree, &small_rope);

    let medium_tree = tree_sitter::parse_code(RHOLANG_MEDIUM);
    let medium_rope = Rope::from_str(RHOLANG_MEDIUM);
    let medium_ir = parse_to_ir(&medium_tree, &medium_rope);

    let large_tree = tree_sitter::parse_code(RHOLANG_LARGE);
    let large_rope = Rope::from_str(RHOLANG_LARGE);
    let large_ir = parse_to_ir(&large_tree, &large_rope);

    group.bench_function("small", |b| {
        b.iter(|| {
            let counter = NodeCounterVisitor {
                count: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            };
            black_box(counter.visit_node(&small_ir))
        })
    });

    group.bench_function("medium", |b| {
        b.iter(|| {
            let counter = NodeCounterVisitor {
                count: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            };
            black_box(counter.visit_node(&medium_ir))
        })
    });

    group.bench_function("large", |b| {
        b.iter(|| {
            let counter = NodeCounterVisitor {
                count: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            };
            black_box(counter.visit_node(&large_ir))
        })
    });

    group.finish();
}

// ============================================================================
// Benchmark: Position Calculations
// ============================================================================

fn bench_position_calculations(c: &mut Criterion) {
    let mut group = c.benchmark_group("position_calculations");

    // Parse a medium-sized file
    let tree = tree_sitter::parse_code(RHOLANG_MEDIUM);
    let rope = Rope::from_str(RHOLANG_MEDIUM);
    let ir = parse_to_ir(&tree, &rope);

    group.bench_function("compute_start_position", |b| {
        b.iter(|| {
            // This benchmarks the compute_absolute_positions function used internally
            black_box(ir.absolute_start(&ir))
        })
    });

    group.bench_function("compute_end_position", |b| {
        b.iter(|| {
            black_box(ir.absolute_end(&ir))
        })
    });

    group.finish();
}

// ============================================================================
// Benchmark: Metadata Allocation
// ============================================================================

fn bench_metadata_allocation(c: &mut Criterion) {
    let mut group = c.benchmark_group("metadata_allocation");

    group.bench_function("parse_small_with_metadata", |b| {
        b.iter(|| {
            let tree = tree_sitter::parse_code(RHOLANG_SMALL);
            let rope = Rope::from_str(RHOLANG_SMALL);
            // Each node allocation includes metadata HashMap creation
            black_box(parse_to_ir(&tree, &rope))
        })
    });

    group.bench_function("parse_medium_with_metadata", |b| {
        b.iter(|| {
            let tree = tree_sitter::parse_code(RHOLANG_MEDIUM);
            let rope = Rope::from_str(RHOLANG_MEDIUM);
            black_box(parse_to_ir(&tree, &rope))
        })
    });

    group.finish();
}

// ============================================================================
// Benchmark: End-to-End Pipeline
// ============================================================================

fn bench_end_to_end_pipeline(c: &mut Criterion) {
    let mut group = c.benchmark_group("end_to_end");

    let uri = Url::parse("file:///test.rho").unwrap();
    let global_table = Arc::new(SymbolTable::new(None));

    group.bench_function("small", |b| {
        b.iter(|| {
            // Parse
            let tree = tree_sitter::parse_code(RHOLANG_SMALL);
            let rope = Rope::from_str(RHOLANG_SMALL);
            let ir = parse_to_ir(&tree, &rope);

            // Build symbol table
            let builder = SymbolTableBuilder::new(ir.clone(), uri.clone(), global_table.clone(), None);
            let result = builder.visit_node(&ir);

            black_box(result)
        })
    });

    group.bench_function("medium", |b| {
        b.iter(|| {
            let tree = tree_sitter::parse_code(RHOLANG_MEDIUM);
            let rope = Rope::from_str(RHOLANG_MEDIUM);
            let ir = parse_to_ir(&tree, &rope);

            let builder = SymbolTableBuilder::new(ir.clone(), uri.clone(), global_table.clone(), None);
            let result = builder.visit_node(&ir);

            black_box(result)
        })
    });

    group.bench_function("large", |b| {
        b.iter(|| {
            let tree = tree_sitter::parse_code(RHOLANG_LARGE);
            let rope = Rope::from_str(RHOLANG_LARGE);
            let ir = parse_to_ir(&tree, &rope);

            let builder = SymbolTableBuilder::new(ir.clone(), uri.clone(), global_table.clone(), None);
            let result = builder.visit_node(&ir);

            black_box(result)
        })
    });

    group.finish();
}

// ============================================================================
// Criterion Configuration
// ============================================================================

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(100)
        .measurement_time(Duration::from_secs(10))
        .warm_up_time(Duration::from_secs(3));
    targets =
        bench_tree_sitter_to_ir_conversion,
        bench_par_node_handling,
        bench_symbol_table_building,
        bench_visitor_traversal,
        bench_position_calculations,
        bench_metadata_allocation,
        bench_end_to_end_pipeline
}

criterion_main!(benches);
