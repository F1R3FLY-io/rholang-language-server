/// Performance tests for MORK serialization and PathMap pattern matching
///
/// These tests measure the performance characteristics of:
/// 1. MORK serialization of RholangNode arguments
/// 2. PathMap insertion and lookup operations
/// 3. Pattern index building with many contracts
/// 4. Workspace indexing with pattern matching enabled

use std::sync::Arc;
use std::time::Instant;
use rholang_language_server::ir::rholang_node::RholangNode;
use rholang_language_server::ir::semantic_node::{NodeBase, Position};
use rholang_language_server::ir::mork_canonical::{MorkForm, LiteralValue};
use rholang_language_server::ir::rholang_pattern_index::RholangPatternIndex;
use rholang_language_server::ir::global_index::{GlobalSymbolIndex, SymbolLocation as GlobalSymbolLocation, SymbolKind};
use tower_lsp::lsp_types::{Range, Url};

fn test_base() -> NodeBase {
    NodeBase::new_simple(Position { row: 0, column: 0, byte: 0 }, 0, 0, 0)
}

fn create_test_location(line: u32) -> GlobalSymbolLocation {
    GlobalSymbolLocation {
        uri: Url::parse("file:///test.rho").unwrap(),
        range: Range {
            start: tower_lsp::lsp_types::Position { line, character: 0 },
            end: tower_lsp::lsp_types::Position { line, character: 10 },
        },
        kind: SymbolKind::Contract,
        documentation: None,
        signature: None,
    }
}

fn create_pattern_index_location(line: u32) -> rholang_language_server::ir::rholang_pattern_index::SymbolLocation {
    rholang_language_server::ir::rholang_pattern_index::SymbolLocation {
        uri: "file:///test.rho".to_string(),
        start: Position { row: line as usize, column: 0, byte: 0 },
        end: Position { row: line as usize, column: 10, byte: line as usize * 100 + 10 },
    }
}

/// Test 1: MORK Serialization Performance
///
/// Measures the time to serialize different types of MorkForm to MORK bytes.
#[test]
fn test_mork_serialization_performance() {
    println!("\n=== MORK Serialization Performance ===");

    let space = mork::space::Space::new();

    // Test 1: Simple string literals
    let string_literal = MorkForm::Literal(LiteralValue::String("hello".to_string()));

    let start = Instant::now();
    for _ in 0..1000 {
        let _ = string_literal.to_mork_bytes(&space);
    }
    let duration = start.elapsed();
    println!("String literal (1000x): {:?} ({:.2}µs each)", duration, duration.as_micros() as f64 / 1000.0);

    // Test 2: Numbers
    let number_literal = MorkForm::Literal(LiteralValue::Int(42));

    let start = Instant::now();
    for _ in 0..1000 {
        let _ = number_literal.to_mork_bytes(&space);
    }
    let duration = start.elapsed();
    println!("Number literal (1000x): {:?} ({:.2}µs each)", duration, duration.as_micros() as f64 / 1000.0);

    // Test 3: Variable patterns
    let var_pattern = MorkForm::VarPattern("x".to_string());

    let start = Instant::now();
    for _ in 0..1000 {
        let _ = var_pattern.to_mork_bytes(&space);
    }
    let duration = start.elapsed();
    println!("Var pattern (1000x): {:?} ({:.2}µs each)", duration, duration.as_micros() as f64 / 1000.0);

    assert!(duration.as_millis() < 100, "MORK serialization should be fast (< 100ms for 1000 operations)");
}

/// Test 2: PathMap Insertion Performance
///
/// Measures the time to insert many patterns into the PathMap index.
#[test]
fn test_pathmap_insertion_performance() {
    println!("\n=== PathMap Insertion Performance ===");

    let mut index = RholangPatternIndex::new();

    // Insert 100 different contract patterns
    let start = Instant::now();
    for i in 0..100 {
        let contract_node = Arc::new(RholangNode::Contract {
            base: test_base(),
            name: Arc::new(RholangNode::Var {
                base: test_base(),
                name: format!("contract{}", i),
                metadata: None,
            }),
            formals: rpds::Vector::new_with_ptr_kind()
                .push_back(Arc::new(RholangNode::LongLiteral {
                    base: test_base(),
                    value: i as i64,
                    metadata: None,
                })),
            formals_remainder: None,
            proc: Arc::new(RholangNode::Nil {
                base: test_base(),
                metadata: None,
            }),
            metadata: None,
        });

        let location = create_pattern_index_location(i as u32);
        index.index_contract(&contract_node, location).unwrap();
    }
    let duration = start.elapsed();
    println!("Insert 100 contracts: {:?} ({:.2}µs each)", duration, duration.as_micros() as f64 / 100.0);

    assert!(duration.as_millis() < 500, "PathMap insertion should be fast (< 500ms for 100 contracts)");
}

/// Test 3: PathMap Lookup Performance
///
/// Measures the time to look up patterns in the PathMap index.
#[test]
fn test_pathmap_lookup_performance() {
    println!("\n=== PathMap Lookup Performance ===");

    let mut index = RholangPatternIndex::new();

    // Pre-populate with 100 contracts
    for i in 0..100 {
        let contract_node = Arc::new(RholangNode::Contract {
            base: test_base(),
            name: Arc::new(RholangNode::Var {
                base: test_base(),
                name: "process".to_string(),
                metadata: None,
            }),
            formals: rpds::Vector::new_with_ptr_kind()
                .push_back(Arc::new(RholangNode::LongLiteral {
                    base: test_base(),
                    value: i as i64,
                    metadata: None,
                })),
            formals_remainder: None,
            proc: Arc::new(RholangNode::Nil {
                base: test_base(),
                metadata: None,
            }),
            metadata: None,
        });

        let location = create_pattern_index_location(i as u32);
        index.index_contract(&contract_node, location).unwrap();
    }

    // Perform 1000 lookups
    let query_args = vec![
        RholangNode::LongLiteral {
            base: test_base(),
            value: 42,
            metadata: None,
        }
    ];
    let query_refs: Vec<&RholangNode> = query_args.iter().map(|n| n).collect();

    let start = Instant::now();
    for _ in 0..1000 {
        let _ = index.query_call_site("process", &query_refs);
    }
    let duration = start.elapsed();
    println!("Lookup 1000 times: {:?} ({:.2}µs each)", duration, duration.as_micros() as f64 / 1000.0);

    assert!(duration.as_millis() < 100, "PathMap lookup should be fast (< 100ms for 1000 lookups)");
}

/// Test 4: Global Index with Pattern Matching
///
/// Measures the overhead of pattern-based indexing in the global symbol index.
#[test]
fn test_global_index_pattern_overhead() {
    println!("\n=== Global Index Pattern Matching Overhead ===");

    let mut global_index = GlobalSymbolIndex::new();

    // Measure time to add 50 contracts WITH pattern indexing
    let start = Instant::now();
    for i in 0..50 {
        let contract_node = Arc::new(RholangNode::Contract {
            base: test_base(),
            name: Arc::new(RholangNode::Var {
                base: test_base(),
                name: format!("contract{}", i),
                metadata: None,
            }),
            formals: rpds::Vector::new_with_ptr_kind()
                .push_back(Arc::new(RholangNode::StringLiteral {
                    base: test_base(),
                    value: format!("arg{}", i),
                    metadata: None,
                })),
            formals_remainder: None,
            proc: Arc::new(RholangNode::Nil {
                base: test_base(),
                metadata: None,
            }),
            metadata: None,
        });

        let location = create_test_location(i as u32);
        let _ = global_index.add_contract_with_pattern_index(&contract_node, location);
    }
    let duration = start.elapsed();
    println!("Add 50 contracts with pattern index: {:?} ({:.2}ms each)",
             duration, duration.as_millis() as f64 / 50.0);

    assert!(duration.as_millis() < 1000, "Pattern indexing should add minimal overhead (< 1s for 50 contracts)");
}

/// Test 5: Multi-argument Pattern Performance
///
/// Measures performance with contracts that have multiple arguments.
#[test]
fn test_multi_argument_pattern_performance() {
    println!("\n=== Multi-Argument Pattern Performance ===");

    let mut index = RholangPatternIndex::new();

    // Add 50 contracts with 3 arguments each
    let start = Instant::now();
    for i in 0..50 {
        let contract_node = Arc::new(RholangNode::Contract {
            base: test_base(),
            name: Arc::new(RholangNode::Var {
                base: test_base(),
                name: "process".to_string(),
                metadata: None,
            }),
            formals: rpds::Vector::new_with_ptr_kind()
                .push_back(Arc::new(RholangNode::LongLiteral {
                    base: test_base(),
                    value: i as i64,
                    metadata: None,
                }))
                .push_back(Arc::new(RholangNode::StringLiteral {
                    base: test_base(),
                    value: format!("arg{}", i),
                    metadata: None,
                }))
                .push_back(Arc::new(RholangNode::BoolLiteral {
                    base: test_base(),
                    value: i % 2 == 0,
                    metadata: None,
                })),
            formals_remainder: None,
            proc: Arc::new(RholangNode::Nil {
                base: test_base(),
                metadata: None,
            }),
            metadata: None,
        });

        let location = create_pattern_index_location(i as u32);
        index.index_contract(&contract_node, location).unwrap();
    }
    let insert_duration = start.elapsed();
    println!("Insert 50 3-arg contracts: {:?} ({:.2}ms each)",
             insert_duration, insert_duration.as_millis() as f64 / 50.0);

    // Query with 3 arguments
    let query_args = vec![
        RholangNode::LongLiteral {
            base: test_base(),
            value: 25,
            metadata: None,
        },
        RholangNode::StringLiteral {
            base: test_base(),
            value: "arg25".to_string(),
            metadata: None,
        },
        RholangNode::BoolLiteral {
            base: test_base(),
            value: false,
            metadata: None,
        },
    ];
    let query_refs: Vec<&RholangNode> = query_args.iter().map(|n| n).collect();

    let start = Instant::now();
    for _ in 0..100 {
        let _ = index.query_call_site("process", &query_refs);
    }
    let lookup_duration = start.elapsed();
    println!("Lookup 100 3-arg queries: {:?} ({:.2}µs each)",
             lookup_duration, lookup_duration.as_micros() as f64 / 100.0);

    assert!(insert_duration.as_millis() < 500, "Multi-arg insertion should be fast");
    assert!(lookup_duration.as_millis() < 100, "Multi-arg lookup should be fast");
}

/// Test 6: Performance Summary
///
/// Prints a summary of expected performance characteristics.
#[test]
fn test_performance_summary() {
    println!("\n=== Pattern Matching Performance Summary ===");
    println!("Expected performance characteristics:");
    println!("- MORK serialization: < 100µs per argument");
    println!("- PathMap insertion: < 5ms per contract");
    println!("- PathMap lookup: < 100µs per query");
    println!("- Multi-argument patterns: Similar to single-argument");
    println!("- Global index overhead: < 20ms per contract");
    println!("\nThese are acceptable for LSP operations (< 200ms target)");
}
