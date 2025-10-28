//! Test goto_definition for robotAPI identifier across all character positions
//!
//! This test verifies that goto_definition works consistently when clicking on any
//! character of the "robotAPI" identifier, including the word boundary at the end.
//!
//! Line 409 (1-indexed) contains: robotAPI!("transport_object", "ball1", "room_a", *result4c)
//! - Column 35-42 (1-indexed) covers "robotAPI"
//! - Expected definition: line 20, column 12 (1-indexed) where robotAPI is declared in `new`

use std::fs;
use test_utils::with_lsp_client;
use test_utils::lsp::client::{CommType, LspClient};
use tower_lsp::lsp_types::Position;

/// Test goto_definition for each character position in "robotAPI" identifier
with_lsp_client!(test_goto_definition_robotapi_all_positions, CommType::Stdio, |client: &LspClient| {
    println!("\n=== Testing goto_definition for robotAPI identifier ===");

    // Read the robot_planning.rho file
    let file_path = "tests/resources/robot_planning.rho";
    let source = fs::read_to_string(file_path)
        .expect("Failed to read robot_planning.rho");

    println!("Opening document with {} bytes", source.len());

    // Open the document
    let doc = client.open_document(
        "/test/robot_planning.rho",
        &source
    ).expect("Failed to open robot_planning.rho");

    println!("✓ Document opened successfully");

    // Wait for diagnostics to ensure parsing is complete
    let _diagnostics = client.await_diagnostics(&doc)
        .expect("Failed to receive diagnostics");

    println!("✓ Diagnostics received");

    // Expected definition location (1-indexed: line 20, column 12)
    // In LSP (0-indexed): line 19, character 11
    let expected_line = 19u32;  // Line 20 in 1-indexed
    let expected_char = 11u32;  // Column 12 in 1-indexed

    println!("\n=== Testing goto_definition across 'robotAPI' identifier ===");
    println!("Expected definition: line {} (0-indexed), character {} (0-indexed)", expected_line, expected_char);
    println!("                    (line 20, column 12 in 1-indexed)\n");

    // Line 409 (1-indexed) = line 408 (0-indexed)
    // Columns 35-43 (1-indexed) = characters 34-42 (0-indexed)
    let test_line = 408u32;

    // Test each character position in "robotAPI" (columns 35-42 in 1-indexed, 34-41 in 0-indexed)
    // Including the word boundary at column 43 (character 42 in 0-indexed)
    let start_char = 34u32;  // Column 35 in 1-indexed (start of "robotAPI")
    let end_char = 42u32;    // Column 43 in 1-indexed (word boundary after "robotAPI")

    let mut all_passed = true;
    let mut results = Vec::new();

    for char_pos in start_char..=end_char {
        let position = Position {
            line: test_line,
            character: char_pos,
        };

        print!("Testing position line {}, character {} (1-indexed: {}, {}): ",
               test_line, char_pos, test_line + 1, char_pos + 1);

        match client.definition(&doc.uri(), position) {
            Ok(Some(location)) => {
                let def_line = location.range.start.line;
                let def_char = location.range.start.character;

                let passed = def_line == expected_line && def_char == expected_char;

                if passed {
                    println!("✓ PASS - Found definition at line {}, character {}",
                           def_line, def_char);
                } else {
                    println!("✗ FAIL - Found definition at line {}, character {} (expected {}, {})",
                           def_line, def_char, expected_line, expected_char);
                    all_passed = false;
                }

                results.push((char_pos, Some((def_line, def_char)), passed));
            }
            Ok(None) => {
                println!("✗ FAIL - No definition found");
                all_passed = false;
                results.push((char_pos, None, false));
            }
            Err(e) => {
                println!("✗ FAIL - Error: {}", e);
                all_passed = false;
                results.push((char_pos, None, false));
            }
        }
    }

    // Print summary
    println!("\n=== Summary ===");
    println!("Total positions tested: {}", results.len());

    let passed_count = results.iter().filter(|(_, _, passed)| *passed).count();
    let failed_count = results.len() - passed_count;

    println!("Passed: {}", passed_count);
    println!("Failed: {}", failed_count);

    if !all_passed {
        println!("\n=== Failed Positions ===");
        for (char_pos, result, passed) in &results {
            if !passed {
                match result {
                    Some((line, char)) => {
                        println!("Character {} (1-indexed: {}): Found ({}, {}) instead of ({}, {})",
                               char_pos, char_pos + 1, line, char, expected_line, expected_char);
                    }
                    None => {
                        println!("Character {} (1-indexed: {}): No definition found",
                               char_pos, char_pos + 1);
                    }
                }
            }
        }
    }

    // Close the document
    client.close_document(&doc)
        .expect("Failed to close document");

    println!("\n✓ Document closed");

    // Assert that all positions passed
    assert!(all_passed, "goto_definition failed for some positions in 'robotAPI' identifier");

    println!("\n=== All goto_definition tests PASSED ===");
});
