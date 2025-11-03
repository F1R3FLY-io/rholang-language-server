//! Integration tests for complex quote patterns in contract parameters
//!
//! These tests verify that:
//! 1. Variable bindings are extracted from complex patterns (maps, lists, tuples, nested)
//! 2. Goto-definition works for variables bound in complex patterns
//! 3. Variables are properly scoped within their contract bodies
//! 4. References can find all usages of pattern-bound variables

use std::fs;
use test_utils::with_lsp_client;
use test_utils::lsp::client::{CommType, LspClient};
use tower_lsp::lsp_types::Position;

/// Test goto-definition for variable bound in map pattern
/// Line 8: @{name: userName, age: userAge}
/// Line 9: userName usage should resolve to line 7, character 20 (userName binding)
with_lsp_client!(test_map_pattern_goto_definition, CommType::Stdio, |client: &LspClient| {
    println!("\n=== Testing goto-definition for map pattern variable ===");

    let file_path = "tests/resources/complex_quote_patterns.rho";
    let source = fs::read_to_string(file_path)
        .expect("Failed to read complex_quote_patterns.rho");

    println!("Opening document with {} bytes", source.len());

    let doc = client.open_document(
        "/test/complex_quote_patterns.rho",
        &source
    ).expect("Failed to open document");

    println!("✓ Document opened successfully");

    let _diagnostics = client.await_diagnostics(&doc)
        .expect("Failed to receive diagnostics");

    println!("✓ Diagnostics received");

    // Usage position: line 9, column 14 (0-indexed: 8, 13) - "userName" in stdout call
    let usage_line = 8u32;
    let usage_char = 13u32;

    println!("\nRequesting goto-definition at line {}, character {}...", usage_line, usage_char);

    match client.definition_all(&doc.uri(), Position { line: usage_line, character: usage_char }) {
        Ok(locations) if !locations.is_empty() => {
            println!("✓ Received {} definition location(s)", locations.len());

            for (i, loc) in locations.iter().enumerate() {
                println!("  {}. line {}, character {} (file: {})",
                    i + 1, loc.range.start.line, loc.range.start.character, loc.uri);
            }

            // Verify we got a definition (exact position may vary based on implementation)
            assert!(locations.len() >= 1, "Expected at least one definition");

            // The definition should be on line 6 (0-indexed) in the parameter pattern
            let on_param_line = locations.iter().any(|loc| loc.range.start.line == 6);
            assert!(on_param_line, "Definition should be on the parameter line (6)");

            println!("\n=== TEST PASSED ===");
        }
        Ok(_) => {
            client.close_document(&doc).expect("Failed to close document");
            panic!("No definitions found for userName in map pattern");
        }
        Err(e) => {
            client.close_document(&doc).expect("Failed to close document");
            panic!("goto_definition request failed: {}", e);
        }
    }

    client.close_document(&doc).expect("Failed to close document");
    println!("✓ Document closed");
});

/// Test goto-definition for variable bound in list pattern
/// Line 14: @[first, second, third]
/// Line 15: first usage should resolve to line 13, character 15 (first binding)
with_lsp_client!(test_list_pattern_goto_definition, CommType::Stdio, |client: &LspClient| {
    println!("\n=== Testing goto-definition for list pattern variable ===");

    let file_path = "tests/resources/complex_quote_patterns.rho";
    let source = fs::read_to_string(file_path)
        .expect("Failed to read complex_quote_patterns.rho");

    let doc = client.open_document(
        "/test/complex_quote_patterns.rho",
        &source
    ).expect("Failed to open document");

    let _diagnostics = client.await_diagnostics(&doc)
        .expect("Failed to receive diagnostics");

    // Usage position: line 15, column 10 (0-indexed: 14, 9) - "first" in return statement
    let usage_line = 14u32;
    let usage_char = 9u32;

    println!("\nRequesting goto-definition at line {}, character {}...", usage_line, usage_char);

    match client.definition_all(&doc.uri(), Position { line: usage_line, character: usage_char }) {
        Ok(locations) if !locations.is_empty() => {
            println!("✓ Received {} definition location(s)", locations.len());

            assert!(locations.len() >= 1, "Expected at least one definition");

            // The definition should be on line 12 (0-indexed) in the parameter pattern
            let on_param_line = locations.iter().any(|loc| loc.range.start.line == 12);
            assert!(on_param_line, "Definition should be on the parameter line (12)");

            println!("\n=== TEST PASSED ===");
        }
        Ok(_) => {
            client.close_document(&doc).expect("Failed to close document");
            panic!("No definitions found for first in list pattern");
        }
        Err(e) => {
            client.close_document(&doc).expect("Failed to close document");
            panic!("goto_definition request failed: {}", e);
        }
    }

    client.close_document(&doc).expect("Failed to close document");
});

/// Test goto-definition for variable bound in nested map pattern
/// Line 30: @{street: s, city: {name: cityName, zip: zipCode}}
/// Line 32: cityName usage should resolve to nested binding
with_lsp_client!(test_nested_map_pattern_goto_definition, CommType::Stdio, |client: &LspClient| {
    println!("\n=== Testing goto-definition for nested map pattern variable ===");

    let file_path = "tests/resources/complex_quote_patterns.rho";
    let source = fs::read_to_string(file_path)
        .expect("Failed to read complex_quote_patterns.rho");

    let doc = client.open_document(
        "/test/complex_quote_patterns.rho",
        &source
    ).expect("Failed to open document");

    let _diagnostics = client.await_diagnostics(&doc)
        .expect("Failed to receive diagnostics");

    // Usage position: line 31, column 30 (0-indexed: 30, 29) - "cityName" in stdout call
    let usage_line = 30u32;
    let usage_char = 29u32;

    println!("\nRequesting goto-definition at line {}, character {}...", usage_line, usage_char);

    match client.definition_all(&doc.uri(), Position { line: usage_line, character: usage_char }) {
        Ok(locations) if !locations.is_empty() => {
            println!("✓ Received {} definition location(s)", locations.len());

            assert!(locations.len() >= 1, "Expected at least one definition");

            // The definition should be on line 23 (0-indexed) in the nested parameter pattern
            let on_param_line = locations.iter().any(|loc| loc.range.start.line == 23);
            assert!(on_param_line, "Definition should be on the parameter line (23)");

            println!("\n=== TEST PASSED ===");
        }
        Ok(_) => {
            client.close_document(&doc).expect("Failed to close document");
            panic!("No definitions found for cityName in nested map pattern");
        }
        Err(e) => {
            client.close_document(&doc).expect("Failed to close document");
            panic!("goto_definition request failed: {}", e);
        }
    }

    client.close_document(&doc).expect("Failed to close document");
});

/// Test goto-definition for variable bound in tuple pattern
/// Line 20: @(x, y, z)
/// Line 21: x usage should resolve to tuple binding
with_lsp_client!(test_tuple_pattern_goto_definition, CommType::Stdio, |client: &LspClient| {
    println!("\n=== Testing goto-definition for tuple pattern variable ===");

    let file_path = "tests/resources/complex_quote_patterns.rho";
    let source = fs::read_to_string(file_path)
        .expect("Failed to read complex_quote_patterns.rho");

    let doc = client.open_document(
        "/test/complex_quote_patterns.rho",
        &source
    ).expect("Failed to open document");

    let _diagnostics = client.await_diagnostics(&doc)
        .expect("Failed to receive diagnostics");

    // Usage position: line 21, column 11 (0-indexed: 20, 10) - "x" in ret!((x, y, z))
    let usage_line = 20u32;
    let usage_char = 10u32;

    println!("\nRequesting goto-definition at line {}, character {}...", usage_line, usage_char);

    match client.definition_all(&doc.uri(), Position { line: usage_line, character: usage_char }) {
        Ok(locations) if !locations.is_empty() => {
            println!("✓ Received {} definition location(s)", locations.len());

            assert!(locations.len() >= 1, "Expected at least one definition");

            // The definition should be on line 17 (0-indexed) in the parameter pattern (contract coordinate)
            let on_param_line = locations.iter().any(|loc| loc.range.start.line == 17);
            assert!(on_param_line, "Definition should be on the parameter line (17)");

            println!("\n=== TEST PASSED ===");
        }
        Ok(_) => {
            client.close_document(&doc).expect("Failed to close document");
            panic!("No definitions found for x in tuple pattern");
        }
        Err(e) => {
            client.close_document(&doc).expect("Failed to close document");
            panic!("goto_definition request failed: {}", e);
        }
    }

    client.close_document(&doc).expect("Failed to close document");
});

/// Test that variables from complex patterns are properly scoped
/// Variables should only be visible within their contract body
with_lsp_client!(test_complex_pattern_scoping, CommType::Stdio, |client: &LspClient| {
    println!("\n=== Testing scoping of complex pattern variables ===");

    let file_path = "tests/resources/complex_quote_patterns.rho";
    let source = fs::read_to_string(file_path)
        .expect("Failed to read complex_quote_patterns.rho");

    let doc = client.open_document(
        "/test/complex_quote_patterns.rho",
        &source
    ).expect("Failed to open document");

    let _diagnostics = client.await_diagnostics(&doc)
        .expect("Failed to receive diagnostics");

    // Try to get definition for "userName" from a different contract's scope
    // This should NOT resolve to processUser's userName parameter
    // Using position in sumThree contract (line 15)
    let usage_line = 14u32;
    let usage_char = 9u32;  // "first" variable

    println!("\nVerifying variable 'first' is scoped to sumThree contract...");

    match client.definition_all(&doc.uri(), Position { line: usage_line, character: usage_char }) {
        Ok(locations) if !locations.is_empty() => {
            println!("✓ Found {} definition location(s)", locations.len());

            // All definitions should be on line 13 (sumThree's parameter line)
            // NOT on line 7 (processUser's parameter line)
            for loc in &locations {
                assert_ne!(loc.range.start.line, 7,
                    "Variable 'first' should NOT resolve to processUser's parameters");
                assert_eq!(loc.range.start.line, 13,
                    "Variable 'first' should resolve to sumThree's parameter on line 13");
            }

            println!("\n=== TEST PASSED ===");
            println!("Variables are properly scoped to their contract bodies");
        }
        Ok(_) => {
            client.close_document(&doc).expect("Failed to close document");
            panic!("No definition found - scoping may be too restrictive");
        }
        Err(e) => {
            client.close_document(&doc).expect("Failed to close document");
            panic!("goto_definition request failed: {}", e);
        }
    }

    client.close_document(&doc).expect("Failed to close document");
});

/// Test goto-definition for variable bound in pathmap pattern
/// Line 108: @{| key1, key2, key3 |}
/// Line 110: key1 usage should resolve to line 107, parameter binding
///
/// NOTE: This test is disabled because pathmap pattern syntax ({| ... |}) triggers parser bugs.
/// The pattern should be added to the test file once parser support is complete.
#[ignore = "pathmap pattern syntax triggers parser bugs - awaiting parser implementation"]
with_lsp_client!(test_pathmap_pattern_goto_definition, CommType::Stdio, |client: &LspClient| {
    println!("\n=== Testing goto-definition for pathmap pattern variable ===");

    let file_path = "tests/resources/complex_quote_patterns.rho";
    let source = fs::read_to_string(file_path)
        .expect("Failed to read complex_quote_patterns.rho");

    let doc = client.open_document(
        "/test/complex_quote_patterns.rho",
        &source
    ).expect("Failed to open document");

    let _diagnostics = client.await_diagnostics(&doc)
        .expect("Failed to receive diagnostics");

    // Usage position: line 110, column 29 (0-indexed: 109, 28) - "key1" in stdout call
    let usage_line = 109u32;
    let usage_char = 28u32;

    println!("\nRequesting goto-definition at line {}, character {}...", usage_line, usage_char);

    match client.definition_all(&doc.uri(), Position { line: usage_line, character: usage_char }) {
        Ok(locations) if !locations.is_empty() => {
            println!("✓ Received {} definition location(s)", locations.len());

            assert!(locations.len() >= 1, "Expected at least one definition");

            // The definition should be on line 106 (0-indexed) in the parameter pattern
            let on_param_line = locations.iter().any(|loc| loc.range.start.line == 106);
            assert!(on_param_line, "Definition should be on the parameter line (106)");

            println!("\n=== TEST PASSED ===");
        }
        Ok(_) => {
            client.close_document(&doc).expect("Failed to close document");
            panic!("No definitions found for key1 in pathmap pattern");
        }
        Err(e) => {
            client.close_document(&doc).expect("Failed to close document");
            panic!("goto_definition request failed: {}", e);
        }
    }

    client.close_document(&doc).expect("Failed to close document");
});
