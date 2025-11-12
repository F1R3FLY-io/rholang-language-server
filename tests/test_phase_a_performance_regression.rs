//! Phase A Performance Regression Tests
//!
//! This test suite validates that the Phase A optimizations maintain their
//! performance characteristics and don't regress over time.
//!
//! Tests cover:
//! - Lazy subtrie extraction (Phase A-1)
//! - Full PathMap traversal for query_all_contracts() (Phase A+)
//!
//! Run with: cargo test --test test_phase_a_performance_regression

use rholang_language_server::ir::rholang_node::{NodeBase, Position as IrPosition, RholangNode};
use rholang_language_server::ir::global_index::{GlobalSymbolIndex, SymbolLocation};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tower_lsp::lsp_types::Url;

/// Create a test contract node with specified name and parameter count
/// (Copied from benches/lazy_subtrie_benchmark.rs - known working implementation)
fn create_test_contract(name: &str, param_count: usize) -> RholangNode {
    use rholang_language_server::ir::rholang_node::RholangNodeVector;

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

    let name_node = Arc::new(RholangNode::Var {
        name: name.to_string(),
        base: NodeBase::new_simple(
            IrPosition { row: 0, column: 0, byte: 0 },
            0, 0, name.len()
        ),
        metadata: None,
    });

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

/// Create a test SymbolLocation
/// (Copied from benches/lazy_subtrie_benchmark.rs - known working implementation)
fn create_test_location(uri_str: &str, line: u32) -> SymbolLocation {
    use rholang_language_server::ir::global_index::SymbolKind;
    use tower_lsp::lsp_types::{Position, Range};

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

/// Phase A-1 Regression: Lazy subtrie extraction should be O(1)
///
/// Validates that extracting the contract subtrie is constant time,
/// not O(total_workspace_symbols).
#[test]
fn test_phase_a1_lazy_subtrie_extraction_performance() {
    let mut index = GlobalSymbolIndex::new();

    // Add 1000 non-contract symbols (simulating large workspace)
    for i in 0..1000 {
        let uri = Url::parse(&format!("file:///test{}.rho", i)).unwrap();
        let location = create_test_location(uri.as_str(), 10);
        index.add_channel_definition(&format!("variable{}", i), location)
            .expect("Failed to add channel");
    }

    // Add 10 contract symbols
    for i in 0..10 {
        let uri = Url::parse(&format!("file:///contract{}.rho", i)).unwrap();
        let contract_node = create_test_contract(&format!("Contract{}", i), 2);
        let location = create_test_location(uri.as_str(), 5);

        index.add_contract_with_pattern_index(&contract_node, location)
            .expect("Failed to add contract");
    }

    // Measure first query (initializes subtrie)
    let start = Instant::now();
    let contracts = index.query_all_contracts()
        .expect("Failed to query contracts");
    let first_duration = start.elapsed();

    assert_eq!(contracts.len(), 10, "Should find all 10 contracts");

    // Measure second query (uses cached subtrie)
    let start = Instant::now();
    let contracts2 = index.query_all_contracts()
        .expect("Failed to query contracts");
    let second_duration = start.elapsed();

    assert_eq!(contracts2.len(), 10, "Should find all 10 contracts on second query");

    // Phase A-1 regression: Both queries should be fast (<1ms)
    // The key insight is that query time should NOT scale with total_workspace_symbols
    assert!(
        first_duration < Duration::from_millis(1),
        "First query (subtrie initialization) took {:?}, should be <1ms", first_duration
    );

    assert!(
        second_duration < Duration::from_micros(500),
        "Second query (cached subtrie) took {:?}, should be <500µs", second_duration
    );

    println!("Phase A-1 regression test passed:");
    println!("  First query (with initialization): {:?}", first_duration);
    println!("  Second query (cached): {:?}", second_duration);
    println!("  Workspace size: 1010 symbols (1000 non-contract + 10 contract)");
}

/// Phase A+ Regression: Full PathMap traversal should find all contracts
///
/// Validates that the depth-first traversal implementation correctly
/// collects all contracts from the subtrie, not just those at the root.
#[test]
fn test_phase_a_plus_full_traversal_collects_all_contracts() {
    let mut index = GlobalSymbolIndex::new();
    let uri = Url::parse("file:///test.rho").unwrap();

    // Add multiple contracts with different arities (creates different trie paths)
    let test_cases = vec![
        ("ContractA", 0, 10usize),  // 0-arity contract
        ("ContractB", 1, 20usize),  // 1-arity contract
        ("ContractC", 2, 30usize),  // 2-arity contract
        ("ContractD", 3, 40usize),  // 3-arity contract
        ("ContractE", 1, 50usize),  // Another 1-arity (different path from ContractB)
    ];

    for (name, arity, line) in &test_cases {
        let contract_node = create_test_contract(name, *arity);
        let location = create_test_location(uri.as_str(), *line as u32);

        index.add_contract_with_pattern_index(&contract_node, location)
            .expect(&format!("Failed to add contract {}", name));
    }

    // Query all contracts
    let contracts = index.query_all_contracts()
        .expect("Failed to query contracts");

    // Phase A+ regression: Should find ALL contracts, not just those at root
    assert_eq!(
        contracts.len(),
        test_cases.len(),
        "Full traversal should find all {} contracts, found {}",
        test_cases.len(),
        contracts.len()
    );

    // Verify each contract is present
    let mut found_names: Vec<String> = contracts.iter()
        .filter_map(|loc| loc.signature.as_ref())
        .map(|sig| {
            // Extract contract name from signature "contract ContractA(...)"
            sig.split_whitespace()
                .nth(1)
                .unwrap_or("")
                .split('(')
                .next()
                .unwrap_or("")
                .to_string()
        })
        .collect();

    found_names.sort();

    let mut expected_names: Vec<String> = test_cases.iter()
        .map(|(name, _, _)| name.to_string())
        .collect();
    expected_names.sort();

    assert_eq!(
        found_names,
        expected_names,
        "Should find all contract names via full traversal"
    );

    println!("Phase A+ regression test passed:");
    println!("  Found all {} contracts via depth-first traversal", contracts.len());
    println!("  Contract names: {:?}", found_names);
}

/// Phase A+ Regression: Full traversal performance should be O(n)
///
/// Validates that traversing N contracts takes O(N) time, not O(N^2) or worse.
#[test]
fn test_phase_a_plus_traversal_performance_scaling() {
    // Test with increasing contract counts to verify O(n) scaling
    let test_sizes = vec![10, 50, 100, 500];
    let mut measurements = Vec::new();

    for size in test_sizes {
        let mut index = GlobalSymbolIndex::new();
        let uri = Url::parse("file:///test.rho").unwrap();

        // Add N contracts
        for i in 0..size {
            let contract_node = create_test_contract(
                &format!("Contract{}", i),
                (i % 4) as usize  // Vary arity to create different paths
            );
            let location = create_test_location(uri.as_str(), (i * 10) as u32);

            index.add_contract_with_pattern_index(&contract_node, location)
                .expect("Failed to add contract");
        }

        // Measure query time
        let start = Instant::now();
        let contracts = index.query_all_contracts()
            .expect("Failed to query contracts");
        let duration = start.elapsed();

        assert_eq!(contracts.len(), size, "Should find all {} contracts", size);

        measurements.push((size, duration));

        println!("  {} contracts: {:?}", size, duration);
    }

    // Phase A+ regression: Time should scale linearly with contract count
    // Calculate time per contract for each measurement
    let times_per_contract: Vec<Duration> = measurements.iter()
        .map(|(size, duration)| *duration / (*size as u32))
        .collect();

    // Verify all measurements are roughly the same (within 5x factor)
    // This proves O(n) scaling, not O(n^2) or worse
    let min_time = times_per_contract.iter().min().unwrap();
    let max_time = times_per_contract.iter().max().unwrap();

    let ratio = max_time.as_nanos() as f64 / min_time.as_nanos() as f64;

    assert!(
        ratio < 5.0,
        "Time per contract should be consistent (O(n) scaling), but varied by {:.2}x (min: {:?}/contract, max: {:?}/contract)",
        ratio, min_time, max_time
    );

    println!("Phase A+ scaling regression test passed:");
    println!("  Time per contract range: {:?} to {:?}", min_time, max_time);
    println!("  Variation factor: {:.2}x (target: <5x)", ratio);
    println!("  Conclusion: O(n) scaling confirmed");
}

/// Phase A Integration: Combined lazy subtrie + full traversal
///
/// Validates that the two optimizations work correctly together.
#[test]
fn test_phase_a_integration_lazy_subtrie_with_full_traversal() {
    let mut index = GlobalSymbolIndex::new();

    // Realistic scenario: Large workspace with contracts and other symbols
    let _uri = Url::parse("file:///workspace.rho").unwrap();

    // Add 500 contracts across multiple files
    for i in 0..500 {
        let file_uri = Url::parse(&format!("file:///file{}.rho", i / 10)).unwrap();
        let contract_node = create_test_contract(
            &format!("Contract{}", i),
            ((i % 5) + 1) as usize  // 1-5 parameters
        );
        let location = create_test_location(file_uri.as_str(), (i % 100) as u32);

        index.add_contract_with_pattern_index(&contract_node, location)
            .expect("Failed to add contract");
    }

    // Add 2000 non-contract symbols
    for i in 0..2000 {
        let sym_uri = Url::parse(&format!("file:///vars{}.rho", i / 100)).unwrap();
        let location = create_test_location(sym_uri.as_str(), (i % 100) as u32);
        index.add_channel_definition(&format!("var{}", i), location)
            .expect("Failed to add channel");
    }

    // Query all contracts multiple times (tests caching)
    let start = Instant::now();

    for _ in 0..10 {
        let contracts = index.query_all_contracts()
            .expect("Failed to query contracts");

        assert_eq!(
            contracts.len(),
            500,
            "Should consistently find all 500 contracts"
        );
    }

    let total_duration = start.elapsed();
    let avg_duration = total_duration / 10;

    // Phase A integration regression: 10 queries of 500 contracts should complete in <100ms total
    // Note: Realistic threshold based on actual O(n) traversal performance (~5ms per query)
    assert!(
        total_duration < Duration::from_millis(100),
        "10 queries took {:?}, should be <100ms (avg: {:?}/query)",
        total_duration, avg_duration
    );

    println!("Phase A integration test passed:");
    println!("  Workspace: 500 contracts + 2000 other symbols");
    println!("  10 queries total time: {:?}", total_duration);
    println!("  Average query time: {:?}", avg_duration);
    println!("  Lazy subtrie caching + full traversal working correctly");
}
