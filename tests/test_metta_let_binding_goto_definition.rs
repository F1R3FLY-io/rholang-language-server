//! Test goto-definition for MeTTa let-bound variables
//!
//! This test verifies that goto-definition works correctly for variables
//! bound in MeTTa let expressions.

use std::fs;
use test_utils::with_lsp_client;
use test_utils::lsp::client::{CommType, LspClient};
use tower_lsp::lsp_types::Position;

/// Test goto-definition for a simple let-bound variable
///
/// Issue: Goto-definition on a let-bound variable usage should jump to the let binding
/// Example: (let $x 42 (+ $x 1)) - clicking on second $x should go to first $x
with_lsp_client!(test_simple_let_binding, CommType::Stdio, |client: &LspClient| {
    println!("\n=== Test: Simple let-binding goto-definition ===");

    let source = r#"
new test in {
  // @metta
  test!("(let $x 42 (+ $x 1))") |
  for (@code <- test) { Nil }
}
"#;

    let doc = client.open_document("/test/let_test.rho", source)
        .expect("Failed to open document");

    let _diagnostics = client.await_diagnostics(&doc)
        .expect("Failed to receive diagnostics");

    // Position of the SECOND $x (in the usage: (+ $x 1))
    // Line 3 (1-indexed) = line 2 (0-indexed)
    // test!("(let $x 42 (+ $x 1))") |
    //  ^column 2                      ^column 31
    // The string content starts after the opening quote at column 9
    // "(let $x 42 (+ $x 1))"
    //  0123456789012345678
    //            ^-- $x usage at position 14 inside the string
    // String starts at column 9, so absolute position is 9 + 14 = 23
    let usage_position = Position {
        line: 2,
        character: 23,
    };

    println!("Testing goto-definition on $x usage at line 3, character 24");

    match client.definition(&doc.uri(), usage_position) {
        Ok(Some(location)) => {
            println!("Found definition at line {}, character {}",
                location.range.start.line + 1, location.range.start.character + 1);

            // Expected: Should go to the FIRST $x (the let binding)
            // "(let $x 42 (+ $x 1))"
            //  0123456
            //       ^-- $x definition at position 6 inside the string
            // String starts at column 9, so absolute position is 9 + 6 = 15
            let expected_line = 2u32;
            let expected_char_min = 14u32; // Approximate range for $x binding
            let expected_char_max = 16u32;

            let def_line = location.range.start.line;
            let def_char = location.range.start.character;

            assert_eq!(def_line, expected_line,
                "Expected definition on line {}, got line {}",
                expected_line + 1, def_line + 1);

            assert!(def_char >= expected_char_min && def_char <= expected_char_max,
                "Expected definition around character {}-{}, got character {}",
                expected_char_min + 1, expected_char_max + 1, def_char + 1);

            println!("✓ Goto-definition correctly jumps to let-binding");
        }
        Ok(None) => {
            panic!("✗ BUG: No definition found for let-bound variable $x");
        }
        Err(e) => {
            panic!("✗ Goto-definition failed: {}", e);
        }
    }

    client.close_document(&doc).expect("Failed to close document");
    println!("✓ Test completed");
});

/// Test goto-definition for let-bound variable in robot_planning.rho
///
/// Issue: Goto-definition on $mid in (path $from $mid $to) at line 95, col 38
/// should jump to its let binding at line 94, col 20
with_lsp_client!(test_robot_planning_let_binding, CommType::Stdio, |client: &LspClient| {
    println!("\n=== Test: Robot planning let-binding goto-definition ===");

    let file_path = "tests/resources/robot_planning.rho";
    let source = fs::read_to_string(file_path)
        .expect("Failed to read robot_planning.rho");

    let doc = client.open_document("/test/robot_planning.rho", &source)
        .expect("Failed to open document");

    let _diagnostics = client.await_diagnostics(&doc)
        .expect("Failed to receive diagnostics");

    // Position of $mid usage in (path $from $mid $to) on line 95
    // Line 95 (1-indexed) = line 94 (0-indexed)
    // Column 38 (1-indexed) = column 37 (0-indexed)
    let usage_position = Position {
        line: 94,
        character: 37,
    };

    println!("Testing goto-definition on $mid at line 95, character 38");

    match client.definition(&doc.uri(), usage_position) {
        Ok(Some(location)) => {
            println!("Found definition at line {}, character {}",
                location.range.start.line + 1, location.range.start.character + 1);

            // Expected: Should go to $mid in (let $mid ...) at line 94, col 20
            // Line 94 (1-indexed) = line 93 (0-indexed)
            // Column 20 (1-indexed) = column 19 (0-indexed)
            let expected_line = 93u32;
            let expected_char_min = 18u32;
            let expected_char_max = 22u32;

            let def_line = location.range.start.line;
            let def_char = location.range.start.character;

            assert_eq!(def_line, expected_line,
                "Expected definition on line {}, got line {}",
                expected_line + 1, def_line + 1);

            assert!(def_char >= expected_char_min && def_char <= expected_char_max,
                "Expected definition around character {}-{}, got character {}",
                expected_char_min + 1, expected_char_max + 1, def_char + 1);

            println!("✓ Goto-definition correctly jumps to let-binding");
        }
        Ok(None) => {
            panic!("✗ BUG: No definition found for let-bound variable $mid");
        }
        Err(e) => {
            panic!("✗ Goto-definition failed: {}", e);
        }
    }

    client.close_document(&doc).expect("Failed to close document");
    println!("✓ Test completed");
});
/// Test goto-definition for match pattern variables (grounded query)
///
/// Issue: Goto-definition on $item in the return position should jump to $item in the pattern
/// Example: (match & self (robot_carrying $item) $item) - second $item should go to first $item
with_lsp_client!(test_match_pattern_variable, CommType::Stdio, |client: &LspClient| {
    println!("\n=== Test: Match pattern variable goto-definition ===");

    let file_path = "tests/resources/robot_planning.rho";
    let source = fs::read_to_string(file_path)
        .expect("Failed to read robot_planning.rho");

    let doc = client.open_document("/test/robot_planning.rho", &source)
        .expect("Failed to open document");

    let _diagnostics = client.await_diagnostics(&doc)
        .expect("Failed to receive diagnostics");

    // Position of $item in return position at line 72, col 52
    // Line 72 (1-indexed) = line 71 (0-indexed)
    // (match & self (robot_carrying $item) $item)
    //                              ^col 44  ^col 52
    let usage_position = Position {
        line: 71,
        character: 52,
    };

    println!("Testing goto-definition on $item at line 72, character 52 (return position)");

    match client.definition(&doc.uri(), usage_position) {
        Ok(Some(location)) => {
            println!("Found definition at line {}, character {}",
                location.range.start.line + 1, location.range.start.character + 1);

            // Expected: Should go to $item in the pattern at col 44
            let expected_line = 71u32;
            let expected_char_min = 43u32;
            let expected_char_max = 49u32;

            let def_line = location.range.start.line;
            let def_char = location.range.start.character;

            assert_eq!(def_line, expected_line,
                "Expected definition on line {}, got line {}",
                expected_line + 1, def_line + 1);

            assert!(def_char >= expected_char_min && def_char <= expected_char_max,
                "Expected definition around character {}-{}, got character {}",
                expected_char_min + 1, expected_char_max + 1, def_char + 1);

            println!("✓ Goto-definition correctly jumps to pattern variable");
        }
        Ok(None) => {
            panic!("✗ BUG: No definition found for match pattern variable $item");
        }
        Err(e) => {
            panic!("✗ Goto-definition failed: {}", e);
        }
    }

    client.close_document(&doc).expect("Failed to close document");
    println!("✓ Test completed");
});
