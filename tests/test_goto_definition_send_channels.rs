//! Test goto_definition for channel references in Send expressions
//!
//! This test verifies that goto-definition works when clicking on channel names
//! inside Send expressions (contract invocations).
//!
//! Covers:
//! - Regular send: `channel!(data)`
//! - Peek send: `channel!?(data)`
//! - Persistent send: `channel!!(data)`
//! - Send inside LinearBind: `for (@x <- channel!?(y)) { ... }`
//!
//! Issue: Previously, extract_symbol_name() did not handle RholangNode::Send,
//! causing goto-definition to fail when clicking on channel names in send expressions.
//!
//! Fix: Added Send and SendSync cases to extract_symbol_name() to recursively
//! extract from the channel field.

use test_utils::with_lsp_client;
use test_utils::lsp::client::{CommType, LspClient};
use tower_lsp::lsp_types::Position;

/// Test goto_definition for various Send types using a simple test case
///
/// Verifies that the Send and SendSync node handling in extract_symbol_name()
/// correctly extracts channel names from Send expressions for goto-definition.
with_lsp_client!(test_goto_definition_send_types, CommType::Stdio, |client: &LspClient| {
    println!("\n=== Testing goto_definition for different Send types ===");

    let source = r#"
new myChannel in {
  // Regular send
  myChannel!(42) |

  // Peek send
  for (@result <- myChannel!?("query")) {
    Nil
  } |

  // Persistent send
  myChannel!!("data")
}
"#;

    println!("Opening document with test code");

    let doc = client.open_document("/test/send_types.rho", source)
        .expect("Failed to open document");

    let _diagnostics = client.await_diagnostics(&doc)
        .expect("Failed to receive diagnostics");

    println!("✓ Document opened and parsed");

    // Expected definition: line 1, character 4 (0-indexed) - "new myChannel in {"
    let expected_line = 1u32;
    let expected_char = 4u32;

    // Test cases: (line, character, description)
    let test_cases = vec![
        (3u32, 2u32, "Regular send (!)"),
        (6u32, 25u32, "Peek send (!?) in LinearBind"),
        (11u32, 2u32, "Persistent send (!!)"),
    ];

    let mut all_passed = true;

    for (line, char, desc) in test_cases {
        print!("Testing {}: line {}, char {} ... ", desc, line, char);

        match client.definition(&doc.uri(), Position { line, character: char }) {
            Ok(Some(location)) => {
                let def_line = location.range.start.line;
                let def_char = location.range.start.character;

                if def_line == expected_line && def_char == expected_char {
                    println!("✓ PASS");
                } else {
                    println!("✗ FAIL - Expected ({}, {}), got ({}, {})",
                           expected_line, expected_char, def_line, def_char);
                    all_passed = false;
                }
            }
            Ok(None) => {
                println!("✗ FAIL - No definition found");
                all_passed = false;
            }
            Err(e) => {
                println!("✗ FAIL - Error: {}", e);
                all_passed = false;
            }
        }
    }

    assert!(all_passed, "Some test cases failed");
    println!("\n=== All Send type tests passed ===");
});
