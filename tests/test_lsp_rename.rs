//! Integration tests for LSP rename functionality
//!
//! This test suite validates rename operations for all symbol types,
//! particularly focusing on LinearBind, RepeatedBind, PeekBind nodes
//! and quoted identifiers which were previously unsupported.
//!
//! ## Bug Fixed
//!
//! These tests verify the fix for missing symbol extraction in rename/references
//! features where LinearBind/RepeatedBind/PeekBind and quoted string literals
//! were not handled in extract_symbol_name() methods.

use test_utils::with_lsp_client;
use test_utils::lsp::client::{CommType, LspClient};
use tower_lsp::lsp_types::Position;

/// Test renaming a variable bound in LinearBind (for (@x <- ch))
///
/// Issue: Rename should work for variables in linear receive patterns
/// Example: for (@fromRoom <- getAll("all")) { fromRoom!() }
with_lsp_client!(test_rename_linear_bind, CommType::Stdio, |client: &LspClient| {
    println!("\n=== Test: Rename LinearBind variable ===");

    let source = r#"
new getAll in {
  for (@fromRoom <- getAll("all")) {
    new x in {
      fromRoom!(x)
    }
  }
}
"#;

    let doc = client.open_document("/test/linear_bind_test.rho", source)
        .expect("Failed to open document");

    let _diagnostics = client.await_diagnostics(&doc)
        .expect("Failed to receive diagnostics");

    // Position of @fromRoom in the bind (line 2, after @)
    // Line 0: empty
    // Line 1: new getAll in {
    // Line 2:   for (@fromRoom <- getAll("all")) {
    //               ^column 8  (@ is at 7, fromRoom starts at 8)
    let bind_position = Position {
        line: 2,
        character: 8,
    };

    println!("Renaming @fromRoom to @sourceRoom at position {:?}", bind_position);

    match client.rename(&doc.uri(), bind_position, "sourceRoom") {
        Ok(workspace_edit) => {
            println!("Rename successful! Changes: {:?}", workspace_edit.changes);

            // Verify the edit contains changes to the document
            assert!(workspace_edit.changes.is_some() || workspace_edit.document_changes.is_some(),
                "Expected workspace edit to contain changes");

            // Get the document text after applying edits
            if let Some(changes) = workspace_edit.changes {
                let doc_uri = doc.uri().parse().expect("Valid URI");
                if let Some(text_edits) = changes.get(&doc_uri) {
                    println!("Found {} edits for document", text_edits.len());

                    // Should have at least 2 edits: the bind and the usage
                    assert!(text_edits.len() >= 2,
                        "Expected at least 2 edits (bind + usage), got {}", text_edits.len());

                    // Verify edits contain the new name
                    for edit in text_edits {
                        assert!(edit.new_text.contains("sourceRoom"),
                            "Edit should contain new name 'sourceRoom', got: {}", edit.new_text);
                    }

                    println!("✓ Verified {} edits contain 'sourceRoom'", text_edits.len());
                }
            }
        }
        Ok(()) => {
            panic!("✗ BUG: Rename returned empty workspace edit");
        }
        Err(e) => {
            panic!("✗ Rename failed: {}", e);
        }
    }

    client.close_document(&doc).expect("Failed to close document");
    println!("✓ Test completed");
});

/// Test renaming a variable bound in RepeatedBind (for (@x <= ch))
with_lsp_client!(test_rename_repeated_bind, CommType::Stdio, |client: &LspClient| {
    println!("\n=== Test: Rename RepeatedBind variable ===");

    let source = r#"
new stream in {
  for (@item <= stream) {
    new x in {
      item!(x)
    }
  }
}
"#;

    let doc = client.open_document("/test/repeated_bind_test.rho", source)
        .expect("Failed to open document");

    let _diagnostics = client.await_diagnostics(&doc)
        .expect("Failed to receive diagnostics");

    // Position of @item in the repeated bind
    let bind_position = Position {
        line: 2,
        character: 8,  // @item - start at 'i'
    };

    println!("Renaming @item to @element");

    match client.rename(&doc.uri(), bind_position, "element") {
        Ok(workspace_edit) => {
            if let Some(changes) = workspace_edit.changes {
                let doc_uri = doc.uri().parse().expect("Valid URI");
                if let Some(text_edits) = changes.get(&doc_uri) {
                    assert!(text_edits.len() >= 2,
                        "Expected at least 2 edits for repeated bind + usage");
                    println!("✓ RepeatedBind rename successful with {} edits", text_edits.len());
                }
            }
        }
        Err(e) => {
            panic!("✗ RepeatedBind rename failed: {}", e);
        }
    }

    client.close_document(&doc).expect("Failed to close document");
    println!("✓ Test completed");
});

/// Test renaming a variable bound in PeekBind (for (@x <<- ch))
with_lsp_client!(test_rename_peek_bind, CommType::Stdio, |client: &LspClient| {
    println!("\n=== Test: Rename PeekBind variable ===");

    let source = r#"
new channel in {
  for (@peeked <<- channel) {
    new x in {
      peeked!(x)
    }
  }
}
"#;

    let doc = client.open_document("/test/peek_bind_test.rho", source)
        .expect("Failed to open document");

    let _diagnostics = client.await_diagnostics(&doc)
        .expect("Failed to receive diagnostics");

    // Position of @peeked in the peek bind
    let bind_position = Position {
        line: 2,
        character: 8,  // @peeked - start at 'p'
    };

    println!("Renaming @peeked to @observed");

    match client.rename(&doc.uri(), bind_position, "observed") {
        Ok(workspace_edit) => {
            if let Some(changes) = workspace_edit.changes {
                let doc_uri = doc.uri().parse().expect("Valid URI");
                if let Some(text_edits) = changes.get(&doc_uri) {
                    assert!(text_edits.len() >= 2,
                        "Expected at least 2 edits for peek bind + usage");
                    println!("✓ PeekBind rename successful with {} edits", text_edits.len());
                }
            }
        }
        Err(e) => {
            panic!("✗ PeekBind rename failed: {}", e);
        }
    }

    client.close_document(&doc).expect("Failed to close document");
    println!("✓ Test completed");
});

/// Test renaming a quoted string literal (@"string")
with_lsp_client!(test_rename_quoted_string, CommType::Stdio, |client: &LspClient| {
    println!("\n=== Test: Rename quoted string literal ===");

    let source = r#"
contract process(@"init", ret) = {
  ret!(@"init")
}
"#;

    let doc = client.open_document("/test/quoted_string_test.rho", source)
        .expect("Failed to open document");

    let _diagnostics = client.await_diagnostics(&doc)
        .expect("Failed to receive diagnostics");

    // Position of @"init" in contract parameter
    let param_position = Position {
        line: 1,
        character: 18,  // Inside "init" string
    };

    println!("Renaming @\"init\" to @\"start\"");

    match client.rename(&doc.uri(), param_position, "start") {
        Ok(workspace_edit) => {
            if let Some(changes) = workspace_edit.changes {
                let doc_uri = doc.uri().parse().expect("Valid URI");
                if let Some(text_edits) = changes.get(&doc_uri) {
                    // Should rename both the parameter and the usage
                    assert!(text_edits.len() >= 2,
                        "Expected at least 2 edits for quoted string parameter + usage");
                    println!("✓ Quoted string rename successful with {} edits", text_edits.len());
                }
            }
        }
        Err(e) => {
            panic!("✗ Quoted string rename failed: {}", e);
        }
    }

    client.close_document(&doc).expect("Failed to close document");
    println!("✓ Test completed");
});

/// Test renaming LinearBind variable used in multiple locations
with_lsp_client!(test_rename_linear_bind_multiple_usages, CommType::Stdio, |client: &LspClient| {
    println!("\n=== Test: Rename LinearBind with multiple usages ===");

    let source = r#"
new getData in {
  for (@value <- getData) {
    new x, y in {
      value!(x) |
      value!(y) |
      for (@result <- value) {
        result!(Nil)
      }
    }
  }
}
"#;

    let doc = client.open_document("/test/multiple_usages_test.rho", source)
        .expect("Failed to open document");

    let _diagnostics = client.await_diagnostics(&doc)
        .expect("Failed to receive diagnostics");

    // Rename from the bind position
    let bind_position = Position {
        line: 2,
        character: 8,  // @value
    };

    println!("Renaming @value to @data (should update 4 locations)");

    match client.rename(&doc.uri(), bind_position, "data") {
        Ok(workspace_edit) => {
            if let Some(changes) = workspace_edit.changes {
                let doc_uri = doc.uri().parse().expect("Valid URI");
                if let Some(text_edits) = changes.get(&doc_uri) {
                    // Should have: 1 bind + 3 usages (value!x, value!y, value in for)
                    assert!(text_edits.len() >= 4,
                        "Expected at least 4 edits (bind + 3 usages), got {}", text_edits.len());

                    // Verify all edits contain "data"
                    for edit in text_edits {
                        assert!(edit.new_text.contains("data"),
                            "All edits should contain 'data'");
                    }

                    println!("✓ Multiple usages renamed successfully ({} edits)", text_edits.len());
                }
            }
        }
        Err(e) => {
            panic!("✗ Multiple usages rename failed: {}", e);
        }
    }

    client.close_document(&doc).expect("Failed to close document");
    println!("✓ Test completed");
});

/// Test renaming quoted contract parameter
with_lsp_client!(test_rename_quoted_contract_param, CommType::Stdio, |client: &LspClient| {
    println!("\n=== Test: Rename quoted contract parameter ===");

    let source = r#"
contract execute(@"action", @data, ret) = {
  new x in {
    x!(@"action") |
    for (@result <- x) {
      match @"action" {
        "run" => ret!(true)
        _ => ret!(false)
      }
    }
  }
}
"#;

    let doc = client.open_document("/test/contract_param_test.rho", source)
        .expect("Failed to open document");

    let _diagnostics = client.await_diagnostics(&doc)
        .expect("Failed to receive diagnostics");

    // Position in the contract parameter @"action"
    let param_position = Position {
        line: 1,
        character: 18,  // Inside "action"
    };

    println!("Renaming @\"action\" to @\"command\"");

    match client.rename(&doc.uri(), param_position, "command") {
        Ok(workspace_edit) => {
            if let Some(changes) = workspace_edit.changes {
                let doc_uri = doc.uri().parse().expect("Valid URI");
                if let Some(text_edits) = changes.get(&doc_uri) {
                    // Should rename: parameter + 2 usages in body
                    assert!(text_edits.len() >= 3,
                        "Expected at least 3 edits (param + 2 usages), got {}", text_edits.len());
                    println!("✓ Contract parameter renamed successfully ({} edits)", text_edits.len());
                }
            }
        }
        Err(e) => {
            panic!("✗ Contract parameter rename failed: {}", e);
        }
    }

    client.close_document(&doc).expect("Failed to close document");
    println!("✓ Test completed");
});
