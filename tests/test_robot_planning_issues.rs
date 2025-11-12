//! Integration tests for reported issues with robot_planning.rho
//!
//! This test suite reproduces and validates fixes for:
//! 1. Document highlight range bug (state vs run on line 211)
//! 2. RobotAPI goto-definition pattern matching (line 409 -> should go to 279, not 298)
//! 3. MeTTa goto-definition in virtual documents
//! 4. MeTTa hover in virtual documents

use std::fs;
use test_utils::with_lsp_client;
use test_utils::lsp::client::{CommType, LspClient};
use tower_lsp::lsp_types::Position;

/// Test 1: Document highlight for 'state' variable on line 211
///
/// Issue: Cursor over `state` on line 211 highlights `run(` instead of `state`
/// Expected: Should highlight all occurrences of the `state` variable in scope
with_lsp_client!(test_document_highlight_state_variable, CommType::Stdio, |client: &LspClient| {
    println!("\n=== Test 1: Document highlight for 'state' variable ===");

    let file_path = "tests/resources/robot_planning.rho";
    let source = fs::read_to_string(file_path)
        .expect("Failed to read robot_planning.rho");

    let doc = client.open_document("/test/robot_planning.rho", &source)
        .expect("Failed to open document");

    let _diagnostics = client.await_diagnostics(&doc)
        .expect("Failed to receive diagnostics");

    // Line 211 (1-indexed) = line 210 (0-indexed)
    // Column where 'state' appears: approximately column 54
    // `.run(state).run(compiledQuery)` - 'state' is at column 54-59
    let position = Position {
        line: 210,  // Line 211 in 1-indexed
        character: 54,  // On the 's' of 'state'
    };

    println!("Requesting document highlight at line 211, character 54 (on 'state')");

    match client.document_highlight(&doc.uri(), position) {
        Ok(highlights) => {
            println!("Got {} highlights", highlights.len());

            if highlights.is_empty() {
                panic!("No highlights returned for 'state' variable");
            }

            // Print all highlighted ranges for debugging
            for (i, highlight) in highlights.iter().enumerate() {
                let range = highlight.range;
                println!("  Highlight {}: L{}:C{}-L{}:C{}",
                    i + 1,
                    range.start.line + 1, range.start.character,
                    range.end.line + 1, range.end.character
                );
            }

            // Verify that at least one highlight includes our cursor position
            let contains_cursor = highlights.iter().any(|h| {
                let r = h.range;
                r.start.line == position.line &&
                r.start.character <= position.character &&
                r.end.character > position.character
            });

            assert!(contains_cursor,
                "Expected highlights to include cursor position at L211:C54, but they don't. \
                This is the bug: highlighting 'run(' instead of 'state'");

            println!("✓ Document highlight correctly includes cursor position");
        }
        Err(e) => {
            panic!("Document highlight failed: {}", e);
        }
    }

    client.close_document(&doc).expect("Failed to close document");
    println!("✓ Test completed");
});

/// Test 2: RobotAPI goto-definition pattern matching
///
/// Issue: goto-definition of robotAPI on line 409 goes to line 298 instead of line 279
/// Line 409: robotAPI!("transport_object", "ball1", "room_a", *result4c)
/// Line 279: contract robotAPI(@"transport_object", @objectName, @destRoom, ret) = { ... }
/// Line 298: contract robotAPI(@"validate_plan", @objectName, @destRoom, ret) = { ... }
///
/// Expected: Should go to line 279 (transport_object), not 298 (validate_plan)
with_lsp_client!(test_robotapi_pattern_matching, CommType::Stdio, |client: &LspClient| {
    println!("\n=== Test 2: RobotAPI goto-definition pattern matching ===");

    let file_path = "tests/resources/robot_planning.rho";
    let source = fs::read_to_string(file_path)
        .expect("Failed to read robot_planning.rho");

    let doc = client.open_document("/test/robot_planning.rho", &source)
        .expect("Failed to open document");

    let _diagnostics = client.await_diagnostics(&doc)
        .expect("Failed to receive diagnostics");

    // Line 409 (1-indexed) = line 408 (0-indexed)
    // robotAPI!("transport_object", "ball1", "room_a", *result4c)
    // Position on 'robotAPI' (starts at column 35 in 1-indexed = 34 in 0-indexed)
    let position = Position {
        line: 408,
        character: 34,
    };

    println!("Requesting goto-definition at line 409, character 35 (on 'robotAPI')");

    match client.definition(&doc.uri(), position) {
        Ok(Some(location)) => {
            let def_line = location.range.start.line;
            println!("Found definition at line {} (1-indexed: {})", def_line, def_line + 1);

            // Expected: line 278 (0-indexed) = line 279 (1-indexed)
            // contract robotAPI(@"transport_object", @objectName, @destRoom, ret)
            let expected_line = 278u32;

            assert_eq!(def_line, expected_line,
                "Expected goto-definition to go to line 279 (transport_object contract), \
                but got line {}. This is the pattern matching bug.", def_line + 1);

            println!("✓ Goto-definition correctly goes to line 279 (transport_object)");
        }
        Ok(None) => {
            panic!("No definition found for robotAPI");
        }
        Err(e) => {
            panic!("Goto-definition failed: {}", e);
        }
    }

    client.close_document(&doc).expect("Failed to close document");
    println!("✓ Test completed");
});

/// Test 3: MeTTa goto-definition in virtual documents
///
/// Issue: goto-definition doesn't work for MeTTa symbols in embedded code
/// Example: goto-definition on 'get_neighbors' should go to its definition
with_lsp_client!(test_metta_goto_definition, CommType::Stdio, |client: &LspClient| {
    println!("\n=== Test 3: MeTTa goto-definition in virtual documents ===");

    let file_path = "tests/resources/robot_planning.rho";
    let source = fs::read_to_string(file_path)
        .expect("Failed to read robot_planning.rho");

    let doc = client.open_document("/test/robot_planning.rho", &source)
        .expect("Failed to open document");

    let _diagnostics = client.await_diagnostics(&doc)
        .expect("Failed to receive diagnostics");

    // The MeTTa code is embedded in a string literal starting around line 22
    // Looking for a usage of 'get_neighbors' to test goto-definition
    // Line 208 in robot_planning.rho has: queryCode!("!(get_neighbors " ++ fromRoom ++ ")")
    // But that's in a Rholang string, not MeTTa source

    // The actual MeTTa definition is inside the codeFile string literal
    // According to the MeTTa code: (= (get_neighbors $room) ...)
    // This should be around line 32 of the virtual document (line 54 in parent)

    // Let's test goto-definition ON a MeTTa symbol inside the virtual document
    // We'll use a reference to 'get_neighbors' in the MeTTa code itself
    // Looking at find_path_2hop definition which uses get_neighbors

    // Line 71 in virtual doc (approximately line 93 in parent): (let $mid (get_neighbors $from)
    // This is harder to calculate precisely, so let's search for it

    // For now, let's just verify the infrastructure works by checking any MeTTa position
    // Line 128 in robot_planning.rho is inside MeTTa code: (= (path_hop_count (path $a $b $c)) 2)

    let position = Position {
        line: 127,  // Line 128 in 1-indexed
        character: 20,  // On 'path_hop_count'
    };

    println!("Requesting goto-definition at line 128, character 20 (on 'path_hop_count' in MeTTa code)");

    match client.definition(&doc.uri(), position) {
        Ok(Some(location)) => {
            println!("Found definition at line {} (1-indexed: {})",
                location.range.start.line, location.range.start.line + 1);

            // The definition should be around line 127-129 (one of the path_hop_count definitions)
            // We expect it to go to one of the definition lines
            let def_line = location.range.start.line;

            // path_hop_count definitions are at lines 127, 128, 129 (1-indexed)
            // In 0-indexed: 126, 127, 128
            assert!((126..=128).contains(&def_line),
                "Expected goto-definition to go to path_hop_count definition (lines 127-129), \
                but got line {}. This is the MeTTa goto-definition bug.", def_line + 1);

            println!("✓ MeTTa goto-definition works for path_hop_count");
        }
        Ok(None) => {
            panic!("No definition found for MeTTa symbol 'path_hop_count'. \
                This indicates the MeTTa goto-definition bug.");
        }
        Err(e) => {
            panic!("MeTTa goto-definition failed: {}", e);
        }
    }

    client.close_document(&doc).expect("Failed to close document");
    println!("✓ Test completed");
});

/// Test 4: MeTTa hover in virtual documents
///
/// Issue: hover doesn't work for MeTTa symbols in embedded code
/// Expected: Hovering over a MeTTa symbol should show information about it
with_lsp_client!(test_metta_hover, CommType::Stdio, |client: &LspClient| {
    println!("\n=== Test 4: MeTTa hover in virtual documents ===");

    let file_path = "tests/resources/robot_planning.rho";
    let source = fs::read_to_string(file_path)
        .expect("Failed to read robot_planning.rho");

    let doc = client.open_document("/test/robot_planning.rho", &source)
        .expect("Failed to open document");

    let _diagnostics = client.await_diagnostics(&doc)
        .expect("Failed to receive diagnostics");

    // Test hover on a MeTTa symbol inside the virtual document
    // Line 128 in robot_planning.rho is inside MeTTa code: (= (path_hop_count (path $a $b $c)) 2)
    let position = Position {
        line: 127,  // Line 128 in 1-indexed
        character: 20,  // On 'path_hop_count'
    };

    println!("Requesting hover at line 128, character 20 (on 'path_hop_count' in MeTTa code)");

    match client.hover(&doc.uri(), position) {
        Ok(Some(hover)) => {
            println!("Got hover response: {:?}", hover);
            println!("✓ MeTTa hover works for path_hop_count");
        }
        Ok(None) => {
            panic!("No hover information returned for MeTTa symbol 'path_hop_count'. \
                This indicates the MeTTa hover bug.");
        }
        Err(e) => {
            panic!("MeTTa hover failed: {}", e);
        }
    }

    client.close_document(&doc).expect("Failed to close document");
    println!("✓ Test completed");
});

/// Test 5: Goto-definition on LinearBind pattern (definition) should not jump to source
///
/// Issue: Clicking on `@result` (pattern/definition) in `for (@result <- queryResult)`
/// at line 288, column 21 incorrectly jumps to `queryResult` definition at line 283
/// instead of recognizing this is a definition itself.
///
/// Expected: Either no result or returns the same location (the pattern is the definition)
/// Should NOT jump to queryResult
with_lsp_client!(test_goto_definition_on_linearbind_pattern, CommType::Stdio, |client: &LspClient| {
    println!("\n=== Test 5: Goto-definition on LinearBind pattern (definition) ===");

    let file_path = "tests/resources/robot_planning.rho";
    let source = fs::read_to_string(file_path)
        .expect("Failed to read robot_planning.rho");

    let doc = client.open_document("/test/robot_planning.rho", &source)
        .expect("Failed to open document");

    let _diagnostics = client.await_diagnostics(&doc)
        .expect("Failed to receive diagnostics");

    // Line 288 (1-indexed) = line 287 (0-indexed)
    // Column 21 (1-indexed) = column 20 (0-indexed)
    // Code: `for (@result <- queryResult) {`
    // Position is on `@result` (the pattern/definition on left side)
    let pattern_position = Position {
        line: 287,       // Line 288 in 1-indexed
        character: 20,   // Column 21 in 1-indexed - on the '@' of '@result'
    };

    println!("Testing goto-definition on @result pattern at line 288, column 21");

    match client.definition(&doc.uri(), pattern_position) {
        Ok(Some(location)) => {
            let def_line = location.range.start.line;
            let def_char = location.range.start.character;

            // Check if we stayed at the same location (definition itself)
            if def_line == 287 && def_char >= 20 && def_char <= 27 {
                println!("✓ Goto-definition correctly stayed at pattern definition (line {}, char {})",
                         def_line + 1, def_char + 1);
            } else if def_line == 282 && def_char == 23 {
                // This is the bug: jumped to queryResult at line 283, col 24 (0-indexed: 282, 23)
                panic!("✗ BUG: Goto-definition incorrectly jumped to queryResult at line {}, char {} \
                        instead of staying at @result pattern definition at line 288, col 21",
                       def_line + 1, def_char + 1);
            } else {
                panic!("✗ Goto-definition jumped to unexpected location: line {}, char {}",
                       def_line + 1, def_char + 1);
            }
        }
        Ok(None) => {
            // This is acceptable - no result for clicking on a definition
            println!("✓ Goto-definition returned no result (acceptable for definitions)");
        }
        Err(e) => {
            panic!("✗ Goto-definition failed with error: {}", e);
        }
    }

    // Also test that goto-definition on the source (queryResult) still works correctly
    let source_position = Position {
        line: 287,       // Line 288 in 1-indexed
        character: 31,   // Column 32 in 1-indexed - on 'queryResult' (right side)
    };

    println!("Testing goto-definition on queryResult source at line 288, column 32");

    match client.definition(&doc.uri(), source_position) {
        Ok(Some(location)) => {
            let def_line = location.range.start.line;
            let def_char = location.range.start.character;

            // Should jump to queryResult definition at line 283, col 24 (0-indexed: 282, 23)
            if def_line == 282 && def_char == 23 {
                println!("✓ Goto-definition correctly jumped to queryResult definition at line {}, char {}",
                         def_line + 1, def_char + 1);
            } else {
                panic!("✗ Goto-definition on queryResult source went to wrong location: line {}, char {} \
                        (expected line 283, col 24)",
                       def_line + 1, def_char + 1);
            }
        }
        Ok(None) => {
            panic!("✗ Goto-definition on queryResult source returned no result (should find definition at line 283)");
        }
        Err(e) => {
            panic!("✗ Goto-definition on queryResult source failed: {}", e);
        }
    }

    client.close_document(&doc).expect("Failed to close document");
    println!("✓ Test completed - LinearBind position-aware goto-definition working correctly");
});
