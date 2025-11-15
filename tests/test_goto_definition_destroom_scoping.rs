//! Test goto_definition for destRoom parameter scoping issue
//!
//! This test verifies that goto_definition correctly resolves to the enclosing contract's
//! parameter, not to parameters from other contracts with the same name.
//!
//! Issue: Line 303, column 68 (1-indexed) has `destRoom` inside the validate_plan contract
//! - Expected definition: line 298, column 53 (1-indexed) - validate_plan contract's @destRoom parameter
//! - Should NOT resolve to: line 279, column 59 (1-indexed) - transport_object contract's @destRoom parameter
//!
//! This tests proper lexical scoping where contract parameters should only be visible
//! within their own contract body, not in other parallel contract definitions.

use std::fs;
use test_utils::with_lsp_client;
use test_utils::lsp::client::{CommType, LspClient};
use tower_lsp::lsp_types::Position;

/// Test that destRoom at line 303 resolves to the correct contract parameter
with_lsp_client!(test_goto_definition_destroom_validate_plan, CommType::Stdio, |client: &LspClient| {
    println!("\n=== Testing goto_definition for destRoom scoping issue ===");

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

    // Test position: line 303, column 68 (1-indexed)
    // In LSP (0-indexed): line 302, character 67
    let usage_line = 302u32;
    let usage_char = 67u32;

    // Expected definition: line 298, column 53 (1-indexed) - validate_plan contract's @destRoom
    // In LSP (0-indexed): line 297, character 52 (the identifier, start of Var node)
    let expected_line = 297u32;
    let expected_char = 52u32;

    // Wrong definition to check against: line 279, column 56 (1-indexed) - transport_object contract's @destRoom
    // In LSP (0-indexed): line 278, character 55 (the identifier)
    let wrong_line = 278u32;
    let wrong_char = 55u32;

    println!("\n=== Test Details ===");
    println!("Usage position: line {} (0-indexed), character {} (0-indexed)", usage_line, usage_char);
    println!("                (line 303, column 68 in 1-indexed)");
    println!("                Context: destRoom inside validate_plan contract body");
    println!();
    println!("Expected definition: line {} (0-indexed), character {} (0-indexed)", expected_line, expected_char);
    println!("                     (line 298, column 50 in 1-indexed - @ symbol)");
    println!("                     Context: @destRoom parameter of validate_plan contract");
    println!();
    println!("Wrong definition: line {} (0-indexed), character {} (0-indexed)", wrong_line, wrong_char);
    println!("                  (line 279, column 53 in 1-indexed - @ symbol)");
    println!("                  Context: @destRoom parameter of transport_object contract");
    println!();

    let position = Position {
        line: usage_line,
        character: usage_char,
    };

    println!("Requesting goto_definition...");
    match client.definition_all(&doc.uri(), position) {
        Ok(locations) if !locations.is_empty() => {
            println!("✓ Received goto_definition response");

            println!("\nFound {} definition location(s):", locations.len());
            for (i, location) in locations.iter().enumerate() {
                let def_line = location.range.start.line;
                let def_char = location.range.start.character;
                println!("  {}. line {}, character {} (1-indexed: {}, {})",
                       i + 1, def_line, def_char, def_line + 1, def_char + 1);
            }

            // Check if we got the expected definition
            let has_expected = locations.iter().any(|loc| {
                loc.range.start.line == expected_line && loc.range.start.character == expected_char
            });

            // Check if we got the wrong definition (from different contract)
            let has_wrong = locations.iter().any(|loc| {
                loc.range.start.line == wrong_line && loc.range.start.character == wrong_char
            });

            println!();
            if has_expected {
                println!("✓ Found expected definition (validate_plan contract's @destRoom at line {}, char {})",
                       expected_line, expected_char);
            } else {
                println!("✗ Missing expected definition (validate_plan contract's @destRoom at line {}, char {})",
                       expected_line, expected_char);
            }

            if has_wrong {
                println!("✗ Found wrong definition (transport_object contract's @destRoom at line {}, char {})",
                       wrong_line, wrong_char);
                println!("  ERROR: Symbol resolution is leaking symbols from different contract scope!");
            } else {
                println!("✓ Correctly excluded wrong definition (transport_object contract's @destRoom)");
            }

            println!();

            // Test assertions
            if locations.len() == 1 && has_expected && !has_wrong {
                println!("=== TEST PASSED ===");
                println!("goto_definition correctly resolved to the enclosing contract's parameter");
            } else {
                println!("=== TEST FAILED ===");
                if locations.len() > 1 {
                    println!("ISSUE: Multiple definitions returned (expected exactly 1)");
                }
                if !has_expected {
                    println!("ISSUE: Expected definition not found");
                }
                if has_wrong {
                    println!("ISSUE: Wrong definition from different contract scope included");
                    println!("       This indicates a lexical scoping bug in symbol resolution");
                }

                // Close before failing
                client.close_document(&doc).expect("Failed to close document");

                panic!("goto_definition returned incorrect results - see issues above");
            }
        }
        Ok(_) => {
            println!("✗ FAIL - No definitions found");
            client.close_document(&doc).expect("Failed to close document");
            panic!("goto_definition returned empty array, expected definition at line {}, character {}",
                   expected_line, expected_char);
        }
        Err(e) => {
            println!("✗ FAIL - Error: {}", e);
            client.close_document(&doc).expect("Failed to close document");
            panic!("goto_definition request failed: {}", e);
        }
    }

    // Close the document
    client.close_document(&doc)
        .expect("Failed to close document");

    println!("\n✓ Document closed");
    println!("\n=== goto_definition scoping test PASSED ===");
});

/// Test that destRoom at line 284 resolves to the transport_object contract parameter
with_lsp_client!(test_goto_definition_destroom_transport_object, CommType::Stdio, |client: &LspClient| {
    println!("\n=== Testing goto_definition for destRoom in transport_object contract ===");

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

    // Test position: line 284, column 68 (1-indexed) - destRoom in transport_object body
    // In LSP (0-indexed): line 283, character 67
    let usage_line = 283u32;
    let usage_char = 67u32;

    // Expected definition: line 279, column 56 (1-indexed) - transport_object contract's @destRoom
    // In LSP (0-indexed): line 278, character 55 (the identifier, start of Var node)
    let expected_line = 278u32;
    let expected_char = 55u32;

    // Wrong definition to check against: line 298, column 53 (1-indexed) - validate_plan contract's @destRoom
    // In LSP (0-indexed): line 297, character 52 (the identifier)
    let wrong_line = 297u32;
    let wrong_char = 52u32;

    println!("\n=== Test Details ===");
    println!("Usage position: line {} (0-indexed), character {} (0-indexed)", usage_line, usage_char);
    println!("                (line 284, column 68 in 1-indexed)");
    println!("                Context: destRoom inside transport_object contract body");
    println!();
    println!("Expected definition: line {} (0-indexed), character {} (0-indexed)", expected_line, expected_char);
    println!("                     (line 279, column 53 in 1-indexed - @ symbol)");
    println!("                     Context: @destRoom parameter of transport_object contract");
    println!();
    println!("Wrong definition: line {} (0-indexed), character {} (0-indexed)", wrong_line, wrong_char);
    println!("                  (line 298, column 50 in 1-indexed - @ symbol)");
    println!("                  Context: @destRoom parameter of validate_plan contract");
    println!();

    let position = Position {
        line: usage_line,
        character: usage_char,
    };

    println!("Requesting goto_definition...");
    match client.definition_all(&doc.uri(), position) {
        Ok(locations) if !locations.is_empty() => {
            println!("✓ Received goto_definition response");

            println!("\nFound {} definition location(s):", locations.len());
            for (i, location) in locations.iter().enumerate() {
                let def_line = location.range.start.line;
                let def_char = location.range.start.character;
                println!("  {}. line {}, character {} (1-indexed: {}, {})",
                       i + 1, def_line, def_char, def_line + 1, def_char + 1);
            }

            // Check if we got the expected definition
            let has_expected = locations.iter().any(|loc| {
                loc.range.start.line == expected_line && loc.range.start.character == expected_char
            });

            // Check if we got the wrong definition (from different contract)
            let has_wrong = locations.iter().any(|loc| {
                loc.range.start.line == wrong_line && loc.range.start.character == wrong_char
            });

            println!();
            if has_expected {
                println!("✓ Found expected definition (transport_object contract's @destRoom at line {}, char {})",
                       expected_line, expected_char);
            } else {
                println!("✗ Missing expected definition (transport_object contract's @destRoom at line {}, char {})",
                       expected_line, expected_char);
            }

            if has_wrong {
                println!("✗ Found wrong definition (validate_plan contract's @destRoom at line {}, char {})",
                       wrong_line, wrong_char);
                println!("  ERROR: Symbol resolution is leaking symbols from different contract scope!");
            } else {
                println!("✓ Correctly excluded wrong definition (validate_plan contract's @destRoom)");
            }

            println!();

            // Test assertions
            if locations.len() == 1 && has_expected && !has_wrong {
                println!("=== TEST PASSED ===");
                println!("goto_definition correctly resolved to the enclosing contract's parameter");
            } else {
                println!("=== TEST FAILED ===");
                if locations.len() > 1 {
                    println!("ISSUE: Multiple definitions returned (expected exactly 1)");
                }
                if !has_expected {
                    println!("ISSUE: Expected definition not found");
                }
                if has_wrong {
                    println!("ISSUE: Wrong definition from different contract scope included");
                    println!("       This indicates a lexical scoping bug in symbol resolution");
                }

                // Close before failing
                client.close_document(&doc).expect("Failed to close document");

                panic!("goto_definition returned incorrect results - see issues above");
            }
        }
        Ok(_) => {
            println!("✗ FAIL - No definitions found");
            client.close_document(&doc).expect("Failed to close document");
            panic!("goto_definition returned empty array, expected definition at line {}, character {}",
                   expected_line, expected_char);
        }
        Err(e) => {
            println!("✗ FAIL - Error: {}", e);
            client.close_document(&doc).expect("Failed to close document");
            panic!("goto_definition request failed: {}", e);
        }
    }

    // Close the document
    client.close_document(&doc)
        .expect("Failed to close document");

    println!("\n✓ Document closed");
    println!("\n=== goto_definition scoping test PASSED ===");
});
