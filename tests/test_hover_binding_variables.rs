//! Test hover support for binding variables in for comprehensions
//!
//! This test verifies that hovering over variables bound in LinearBind,
//! RepeatedBind, and PeekBind nodes shows the correct hover information.
//!
//! Regression test for: Hover not extracting symbol names from Quote nodes
//! in binding patterns like @result, @code, @queryResult.

use test_utils::with_lsp_client;
use test_utils::lsp::client::{CommType, LspClient};
use tower_lsp::lsp_types::Position;

/// Test hover on binding variables in for comprehensions
///
/// Verifies that the hover extraction correctly handles Quote { quotable: Var }
/// patterns in LinearBind nodes (for (@result <- channel)).
with_lsp_client!(test_hover_binding_variables, CommType::Stdio, |client: &LspClient| {
    println!("\n=== Testing hover for binding variables ===");

    let source = r#"
new channel in {
  // LinearBind: for (@result <- channel)
  for (@result <- channel) {
    result!("processed")
  } |

  // RepeatedBind: for (@code <= channel)
  for (@code <= channel) {
    code!("executed")
  } |

  // PeekBind: for (@queryResult <<- channel)
  for (@queryResult <<- channel) {
    queryResult!("responded")
  }
}
"#;

    println!("Opening document with binding variable test code");

    let doc = client.open_document("/test/binding_variables.rho", source)
        .expect("Failed to open document");

    let _diagnostics = client.await_diagnostics(&doc)
        .expect("Failed to receive diagnostics");

    println!("✓ Document opened and parsed");

    // Test cases: (line, character, variable_name, description)
    let test_cases = vec![
        (3u32, 11u32, "result", "LinearBind @result in for comprehension"),
        (8u32, 11u32, "code", "RepeatedBind @code in for comprehension"),
        (13u32, 11u32, "queryResult", "PeekBind @queryResult in for comprehension"),
        // Also test usage sites (not just binding sites)
        (4u32, 4u32, "result", "Usage of result in send"),
        (9u32, 4u32, "code", "Usage of code in send"),
        (14u32, 4u32, "queryResult", "Usage of queryResult in send"),
    ];

    let mut all_passed = true;

    for (line, char, expected_symbol, desc) in test_cases {
        print!("Testing {}: line {}, char {} ... ", desc, line, char);

        match client.hover(&doc.uri(), Position { line, character: char }) {
            Ok(Some(hover)) => {
                // Check that hover content is not empty
                match &hover.contents {
                    tower_lsp::lsp_types::HoverContents::Scalar(content) => {
                        // Check that the hover contains the expected symbol name
                        let content_str = match content {
                            tower_lsp::lsp_types::MarkedString::String(s) => s.as_str(),
                            tower_lsp::lsp_types::MarkedString::LanguageString(ls) => ls.value.as_str(),
                        };
                        if content_str.contains(expected_symbol) {
                            println!("✓ PASS - Hover contains '{}'", expected_symbol);
                        } else {
                            println!("✗ FAIL - Hover doesn't contain '{}': {:?}", expected_symbol, content_str);
                            all_passed = false;
                        }
                    }
                    tower_lsp::lsp_types::HoverContents::Array(contents) => {
                        let found = contents.iter().any(|c| {
                            match c {
                                tower_lsp::lsp_types::MarkedString::String(s) => s.contains(expected_symbol),
                                tower_lsp::lsp_types::MarkedString::LanguageString(ls) => ls.value.contains(expected_symbol),
                            }
                        });
                        if found {
                            println!("✓ PASS - Hover contains '{}'", expected_symbol);
                        } else {
                            println!("✗ FAIL - Hover doesn't contain '{}': {:?}", expected_symbol, contents);
                            all_passed = false;
                        }
                    }
                    tower_lsp::lsp_types::HoverContents::Markup(content) => {
                        if content.value.contains(expected_symbol) {
                            println!("✓ PASS - Hover contains '{}'", expected_symbol);
                        } else {
                            println!("✗ FAIL - Hover doesn't contain '{}': {:?}", expected_symbol, content.value);
                            all_passed = false;
                        }
                    }
                }
            }
            Ok(None) => {
                println!("✗ FAIL - No hover information found");
                all_passed = false;
            }
            Err(e) => {
                println!("✗ FAIL - Error: {}", e);
                all_passed = false;
            }
        }
    }

    assert!(all_passed, "Some test cases failed");
    println!("\n=== All binding variable hover tests passed ===");
});
