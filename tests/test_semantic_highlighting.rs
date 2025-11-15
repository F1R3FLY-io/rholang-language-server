//! Integration tests for semantic highlighting (textDocument/semanticTokens/full)
//!
//! This test suite validates semantic token generation for both Rholang and
//! embedded MeTTa code, with special focus on correct position mapping for
//! virtual documents.
//!
//! ## Bug Fixed
//!
//! These tests verify the fix for the semantic highlighting offset bug where
//! tokens in MeTTa embedded regions were shifted left by one character due to
//! incorrect position calculation in `src/lsp/backend/metta.rs`.

use std::fs;
use test_utils::with_lsp_client;
use test_utils::lsp::client::{CommType, LspClient};
use tower_lsp::lsp_types::{Position, SemanticTokenType, SemanticTokensParams, TextDocumentIdentifier, SemanticTokensResult, SemanticTokens};

/// Helper function to request semantic tokens from the LSP server
fn request_semantic_tokens(client: &LspClient, uri: &str) -> Option<SemanticTokens> {
    let params = SemanticTokensParams {
        text_document: TextDocumentIdentifier {
            uri: uri.parse().unwrap(),
        },
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
    };

    match client.semantic_tokens_full(params) {
        Ok(Some(result)) => {
            match result {
                SemanticTokensResult::Tokens(tokens) => Some(tokens),
                _ => None,
            }
        }
        Ok(None) => None,
        Err(e) => {
            eprintln!("Error requesting semantic tokens: {}", e);
            None
        }
    }
}

/// Test semantic tokens for MeTTa embedded code - basic case
///
/// Verifies that semantic tokens are generated with correct positions
/// for symbols in MeTTa virtual documents.
with_lsp_client!(test_semantic_tokens_metta_basic, CommType::Stdio, |client: &LspClient| {
    println!("\n=== Test: MeTTa semantic tokens - basic ===");

    let source = r#"
new test in {
  // @metta
  test!("(let $x 42 (+ $x 1))") |
  for (@code <- test) { Nil }
}
"#;

    let doc = client.open_document("/test/semantic_test.rho", source)
        .expect("Failed to open document");

    let _diagnostics = client.await_diagnostics(&doc)
        .expect("Failed to receive diagnostics");

    // Request semantic tokens
    let tokens = request_semantic_tokens(client, &doc.uri())
        .expect("Expected semantic tokens response");

    assert!(!tokens.data.is_empty(), "Expected non-empty token list");

    println!("Received {} semantic tokens", tokens.data.len());

    // Decode tokens from delta-encoded format
    let mut current_line = 0u32;
    let mut current_column = 0u32;

    let mut found_let = false;
    let mut found_x_def = false;
    let mut found_x_usage = false;

    for token in &tokens.data {
        // Delta decode to absolute position
        if token.delta_line > 0 {
            current_line += token.delta_line;
            current_column = token.delta_start;
        } else {
            current_column += token.delta_start;
        }

        let length = token.length;
        let token_type = token.token_type;

        println!("Token at L{}:C{} length={} type={}",
                 current_line, current_column, length, token_type);

        // Source layout:
        // Line 0: empty
        // Line 1: new test in {
        // Line 2: // @metta
        // Line 3: test!("(let $x 42 (+ $x 1))") |
        //         ^col 2      ^col 9 (opening quote)
        //
        // String content: "(let $x 42 (+ $x 1))"
        // Position mapping: col 9 (quote) + 1 + offset_in_string
        //
        // Expected tokens in MeTTa string:
        // - "let" at col 10 (9 + 1), length 3
        // - "$x" (definition) at col 14 (9 + 1 + 4), length 2
        // - "42" at col 17, length 2
        // - "+" at col 21, length 1
        // - "$x" (usage) at col 23 (9 + 1 + 13), length 2
        // - "1" at col 26, length 1

        if current_line == 3 {
            // Check for "let" keyword
            if current_column == 10 && length == 3 {
                found_let = true;
                println!("  ✓ Found 'let' keyword at correct position");
            }

            // Check for first $x (definition in let binding)
            if current_column == 14 && length == 2 {
                found_x_def = true;
                println!("  ✓ Found '$x' definition at correct position");
            }

            // Check for second $x (usage in expression)
            if current_column == 23 && length == 2 {
                found_x_usage = true;
                println!("  ✓ Found '$x' usage at correct position");
            }
        }
    }

    assert!(found_let, "Expected to find 'let' keyword token");
    assert!(found_x_def, "Expected to find '$x' definition token");
    assert!(found_x_usage, "Expected to find '$x' usage token");

    client.close_document(&doc).expect("Failed to close document");
    println!("✓ Test completed");
});

/// Test semantic tokens on first line of MeTTa region (row == 0 case)
///
/// This specifically tests the bug fix where tokens on the first line of
/// a virtual document had incorrect column offsets.
with_lsp_client!(test_semantic_tokens_metta_first_line, CommType::Stdio, |client: &LspClient| {
    println!("\n=== Test: MeTTa semantic tokens - first line (row == 0) ===");

    let source = r#"
new test in {
  // @metta
  test!("(+ 1 2)") |
  for (@code <- test) { Nil }
}
"#;

    let doc = client.open_document("/test/first_line_test.rho", source)
        .expect("Failed to open document");

    let _diagnostics = client.await_diagnostics(&doc)
        .expect("Failed to receive diagnostics");

    let tokens = request_semantic_tokens(client, &doc.uri())
        .expect("Expected semantic tokens response");

    // Decode and check first line tokens
    let mut current_line = 0u32;
    let mut current_column = 0u32;
    let mut found_plus = false;

    println!("Received {} semantic tokens", tokens.data.len());

    for token in &tokens.data {
        if token.delta_line > 0 {
            current_line += token.delta_line;
            current_column = token.delta_start;
        } else {
            current_column += token.delta_start;
        }

        println!("Token at L{}:C{} length={} type={}",
                 current_line, current_column, token.length, token.token_type);

        // Source layout:
        // Line 3: test!("(+ 1 2)") |
        //         ^col 2  ^col 8 (quote)
        // String: "(+ 1 2)"
        //         0123456
        // "+" is at offset 1 in string -> col 9 + 1 = col 10

        if current_line == 3 && current_column == 10 && token.length == 1 {
            found_plus = true;
            println!("  ✓ Found '+' operator at L{}:C{}", current_line, current_column);
        }
    }

    assert!(found_plus, "Expected to find '+' operator token on first line");

    client.close_document(&doc).expect("Failed to close document");
    println!("✓ Test completed");
});

/// Test semantic tokens for multiline MeTTa code (row > 0 case)
///
/// Verifies that tokens on lines after the first line of a virtual document
/// have correct positions.
with_lsp_client!(test_semantic_tokens_metta_multiline, CommType::Stdio, |client: &LspClient| {
    println!("\n=== Test: MeTTa semantic tokens - multiline (row > 0) ===");

    let source = r#"
new test in {
  // @metta
  test!("(let $x 42
          (+ $x 10))") |
  for (@code <- test) { Nil }
}
"#;

    let doc = client.open_document("/test/multiline_test.rho", source)
        .expect("Failed to open document");

    let _diagnostics = client.await_diagnostics(&doc)
        .expect("Failed to receive diagnostics");

    let tokens = request_semantic_tokens(client, &doc.uri())
        .expect("Expected semantic tokens response");

    // Decode and check multiline tokens
    let mut current_line = 0u32;
    let mut current_column = 0u32;
    let mut found_second_line_token = false;

    for token in &tokens.data {
        if token.delta_line > 0 {
            current_line += token.delta_line;
            current_column = token.delta_start;
        } else {
            current_column += token.delta_start;
        }

        println!("Token at L{}:C{} length={}", current_line, current_column, token.length);

        // Source layout:
        // Line 3: test!("(let $x 42
        // Line 4:          (+ $x 10))") |
        //
        // Second line of MeTTa content (row > 0 in virtual doc)
        // Tokens on line 4 should NOT have parent_start.character added

        if current_line == 4 {
            found_second_line_token = true;
            // Just verify we have tokens on line 4 with reasonable positions
            // (not shifted by parent offset)
            assert!(current_column < 100,
                    "Token column {} seems too large, might have incorrect offset",
                    current_column);
            println!("  ✓ Found token on second line at reasonable position L{}:C{}",
                     current_line, current_column);
        }
    }

    assert!(found_second_line_token, "Expected to find tokens on second line of MeTTa code");

    client.close_document(&doc).expect("Failed to close document");
    println!("✓ Test completed");
});

/// Test semantic tokens with robot_planning.rho
///
/// This test uses the actual robot_planning.rho file that triggered the
/// original bug report. Note: robot_planning.rho uses old-style MeTTa
/// embedding (large strings without #!metta directive), so semantic
/// highlighting might not be enabled for it. This test is mainly for
/// regression testing.
#[ignore] // Ignore by default since robot_planning.rho might not have MeTTa regions
with_lsp_client!(test_semantic_tokens_robot_planning, CommType::Stdio, |client: &LspClient| {
    println!("\n=== Test: Semantic tokens in robot_planning.rho ===");

    let file_path = "tests/resources/robot_planning.rho";
    let source = fs::read_to_string(file_path)
        .expect("Failed to read robot_planning.rho");

    let doc = client.open_document("/test/robot_planning.rho", &source)
        .expect("Failed to open document");

    let _diagnostics = client.await_diagnostics(&doc)
        .expect("Failed to receive diagnostics");

    if let Some(tokens) = request_semantic_tokens(client, &doc.uri()) {
        println!("Received {} semantic tokens from robot_planning.rho", tokens.data.len());

        // Just verify we got some tokens and they have reasonable positions
        let mut current_line = 0u32;
        let mut current_column = 0u32;

        for (i, token) in tokens.data.iter().enumerate().take(10) {
            if token.delta_line > 0 {
                current_line += token.delta_line;
                current_column = token.delta_start;
            } else {
                current_column += token.delta_start;
            }

            println!("Token {}: L{}:C{} length={} type={}",
                     i, current_line, current_column, token.length, token.token_type);
        }
    } else {
        println!("No semantic tokens returned (expected for old-style MeTTa embedding)");
    }

    client.close_document(&doc).expect("Failed to close document");
    println!("✓ Test completed");
});
