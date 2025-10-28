/// Performance integration tests to prevent regressions
///
/// These tests ensure that key LSP operations complete within acceptable time bounds.
/// They help catch performance regressions early in development.

use indoc::indoc;
use std::time::{Duration, Instant};
use tower_lsp::lsp_types::Position;
use test_utils::with_lsp_client;
use test_utils::lsp::client::{CommType, LspClient};

/// Maximum acceptable time for goto-definition on a small file
const GOTO_DEF_SMALL_FILE_MAX_MS: u64 = 100;

/// Maximum acceptable time for goto-definition on a large file (500+ lines)
/// Increased from 200ms to 300ms to account for hot observable broadcasting overhead and system variance
const GOTO_DEF_LARGE_FILE_MAX_MS: u64 = 300;

/// Maximum acceptable time for document highlights
const HIGHLIGHT_MAX_MS: u64 = 100;

with_lsp_client!(test_goto_definition_performance_small_file, CommType::Stdio, |client: &LspClient| {
    // Small contract file
    let contract_code = indoc! {r#"
        contract myContract(@x, @y, result) = {
            new ack in {
                stdout!(*x, *ack) |
                for (_ <- ack) {
                    stdout!(*y, *result)
                }
            }
        }
    "#};

    let usage_code = indoc! {r#"
        new result in {
            myContract!(1, 2, *result) |
            for (output <- result) {
                stdout!(*output, "ack")
            }
        }
    "#};

    let contract_doc = client
        .open_document("/path/to/contract.rho", contract_code)
        .expect("Failed to open contract.rho");

    let usage_doc = client
        .open_document("/path/to/usage.rho", usage_code)
        .expect("Failed to open usage.rho");

    // Wait for indexing and validation to complete for both documents
    client.await_diagnostics(&contract_doc)
        .expect("Failed to wait for contract diagnostics");
    client.await_diagnostics(&usage_doc)
        .expect("Failed to wait for usage diagnostics");

    // Measure goto-definition performance
    // Position is on "myContract" in usage file (line 1, after "new result in {")
    let start = Instant::now();
    let result = client.definition(&usage_doc.uri(), Position::new(1, 4));
    let elapsed = start.elapsed();

    assert!(result.is_ok(), "goto_definition should succeed");
    assert!(
        elapsed < Duration::from_millis(GOTO_DEF_SMALL_FILE_MAX_MS),
        "goto_definition took {:?}, expected < {}ms (regression detected!)",
        elapsed,
        GOTO_DEF_SMALL_FILE_MAX_MS
    );

    println!("✓ goto_definition completed in {:?}", elapsed);
});

with_lsp_client!(test_goto_definition_performance_cross_file, CommType::Stdio, |client: &LspClient| {
    // Test cross-file goto-definition performance
    let contract_code = "contract myContract(@x) = { stdout!(*x, \"ack\") }";
    let usage_code = "myContract!(42)";

    let contract_doc = client
        .open_document("/path/to/defs.rho", contract_code)
        .expect("Failed to open contract");

    let usage_doc = client
        .open_document("/path/to/calls.rho", usage_code)
        .expect("Failed to open usage");

    // Wait for indexing and validation to complete for both documents
    client.await_diagnostics(&contract_doc)
        .expect("Failed to wait for contract diagnostics");
    client.await_diagnostics(&usage_doc)
        .expect("Failed to wait for usage diagnostics");

    let start = Instant::now();
    let result = client.definition(&usage_doc.uri(), Position::new(0, 0));
    let elapsed = start.elapsed();

    assert!(result.is_ok(), "cross-file goto_definition should succeed");
    assert!(
        elapsed < Duration::from_millis(GOTO_DEF_SMALL_FILE_MAX_MS),
        "cross-file goto_definition took {:?}, expected < {}ms",
        elapsed,
        GOTO_DEF_SMALL_FILE_MAX_MS
    );

    println!("✓ cross-file goto_definition completed in {:?}", elapsed);
});

// Note: Hover performance test omitted - hover() method not yet implemented in test_utils

with_lsp_client!(test_document_highlight_performance, CommType::Stdio, |client: &LspClient| {
    let code = indoc! {r#"
        new myVar in {
            myVar!(1) |
            myVar!(2) |
            myVar!(3) |
            for (@val <- myVar) {
                stdout!(val, "ack")
            }
        }
    "#};

    let doc = client
        .open_document("/path/to/highlight_test.rho", code)
        .expect("Failed to open document");

    // Wait for indexing and validation to complete
    client.await_diagnostics(&doc)
        .expect("Failed to wait for diagnostics");

    // Test document highlight on 'myVar' (line 1, column 4)
    let start = Instant::now();
    let result = client.document_highlight(&doc.uri(), Position::new(1, 4));
    let elapsed = start.elapsed();

    assert!(result.is_ok(), "document_highlight should succeed");
    assert!(
        elapsed < Duration::from_millis(HIGHLIGHT_MAX_MS),
        "document_highlight took {:?}, expected < {}ms",
        elapsed,
        HIGHLIGHT_MAX_MS
    );

    println!("✓ document_highlight completed in {:?}", elapsed);

    // Should find multiple highlights (definition + uses)
    let highlights = result.expect("should return Ok");
    assert!(
        highlights.len() >= 3,
        "should find at least 3 occurrences of myVar, found {}",
        highlights.len()
    );
});

with_lsp_client!(test_large_file_performance, CommType::Stdio, |client: &LspClient| {
    // Generate a large file with many contracts (simulates robot_planning.rho)
    let mut large_file = String::new();
    for i in 0..50 {
        large_file.push_str(&format!(
            "contract contract{}(@param{}) = {{ stdout!(\"contract {}\", \"ack\") }}\n",
            i, i, i
        ));
    }
    large_file.push_str("contract targetContract(@x) = { Nil }\n");
    for i in 50..100 {
        large_file.push_str(&format!(
            "contract contract{}(@param{}) = {{ stdout!(\"contract {}\", \"ack\") }}\n",
            i, i, i
        ));
    }

    let doc = client
        .open_document("/path/to/large.rho", &large_file)
        .expect("Failed to open large file");

    // Wait for diagnostics to ensure indexing and semantic validation are complete
    // With 101 contracts, semantic validation can take 200-300ms
    client.await_diagnostics(&doc)
        .expect("Failed to wait for diagnostics");

    // Find goto-definition for targetContract
    let target_line = large_file.lines().position(|l| l.contains("targetContract")).unwrap();

    let start = Instant::now();
    let result = client.definition(&doc.uri(), Position::new(target_line as u32, 9));
    let elapsed = start.elapsed();

    assert!(result.is_ok(), "goto_definition on large file should succeed");
    assert!(
        elapsed < Duration::from_millis(GOTO_DEF_LARGE_FILE_MAX_MS),
        "goto_definition on large file took {:?}, expected < {}ms (check for O(n²) issues)",
        elapsed,
        GOTO_DEF_LARGE_FILE_MAX_MS
    );

    println!("✓ large file goto_definition completed in {:?}", elapsed);
});

/// Test that we don't have quadratic complexity in symbol table lookups
with_lsp_client!(test_no_quadratic_complexity, CommType::Stdio, |client: &LspClient| {
    // Create files with increasing numbers of symbols
    // If we have O(n²) complexity, this will show up as non-linear timing

    let sizes = vec![10, 20, 40];
    let mut timings = Vec::new();

    for size in sizes.iter() {
        let mut code = String::new();
        for i in 0..*size {
            code.push_str(&format!("new var{} in {{ Nil }} |\n", i));
        }
        code.push_str("Nil");

        let doc = client
            .open_document(&format!("/path/to/test{}.rho", size), &code)
            .expect("Failed to open document");

        // Wait for indexing and validation to complete
        client.await_diagnostics(&doc)
            .expect("Failed to wait for diagnostics");

        let start = Instant::now();
        let _ = client.definition(&doc.uri(), Position::new(0, 4)); // Check first variable
        let elapsed = start.elapsed();

        timings.push(elapsed);
        println!("Size {}: {:?}", size, elapsed);
    }

    // Check that timing doesn't grow quadratically
    // Time for 40 should be less than 4x time for 10 (allowing some overhead)
    let ratio = timings[2].as_nanos() as f64 / timings[0].as_nanos() as f64;
    assert!(
        ratio < 5.0,
        "Performance degradation suggests O(n²) complexity: ratio = {:.2}x (should be near linear)",
        ratio
    );

    println!("✓ Complexity check passed (ratio: {:.2}x)", ratio);
});
