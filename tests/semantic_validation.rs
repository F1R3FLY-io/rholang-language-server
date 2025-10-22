//! Integration tests for semantic validation
//!
//! These tests verify that the semantic validator correctly integrates with the LSP server
//! and provides appropriate diagnostics for semantic errors in Rholang code.

use indoc::indoc;
use test_utils::with_lsp_client;
use test_utils::lsp::client::{CommType, LspClient};

// Test that valid code produces no semantic diagnostics
#[cfg(feature = "interpreter")]
with_lsp_client!(test_semantic_valid_code, CommType::Stdio, |client: &LspClient| {
    let source = indoc! {r#"
        new x in {
            x!(42)
        }
    "#};

    let doc = client.open_document("/path/to/valid.rho", source)
        .expect("Failed to open document");
    let diagnostics = client.await_diagnostics(&doc).unwrap();

    assert_eq!(
        diagnostics.diagnostics.len(),
        0,
        "Valid code should produce no diagnostics"
    );
});

// Test that unbound variables are detected
#[cfg(feature = "interpreter")]
with_lsp_client!(test_semantic_unbound_variable, CommType::Stdio, |client: &LspClient| {
    let source = indoc! {r#"
        new x in {
            y!(42)
        }
    "#};

    let doc = client.open_document("/path/to/unbound.rho", source)
        .expect("Failed to open document");
    let diagnostics = client.await_diagnostics(&doc).unwrap();

    assert!(
        !diagnostics.diagnostics.is_empty(),
        "Should detect unbound variable 'y'"
    );

    assert!(
        diagnostics.diagnostics.iter().any(|d|
            d.message.contains("Unbound") || d.message.contains("unbound") || d.message.contains("free variable")
        ),
        "Diagnostic should mention unbound variable, got: {:?}",
        diagnostics.diagnostics
    );
});

// Test that top-level free variables are detected
#[cfg(feature = "interpreter")]
with_lsp_client!(test_semantic_top_level_free_var, CommType::Stdio, |client: &LspClient| {
    let source = "x!(42)";  // Top-level free variable

    let doc = client.open_document("/path/to/toplevel.rho", source)
        .expect("Failed to open document");
    let diagnostics = client.await_diagnostics(&doc).unwrap();

    assert!(
        !diagnostics.diagnostics.is_empty(),
        "Should detect top-level free variable"
    );
});

// Test that contracts are validated correctly
#[cfg(feature = "interpreter")]
with_lsp_client!(test_semantic_valid_contract, CommType::Stdio, |client: &LspClient| {
    let source = indoc! {r#"
        new myContract in {
            contract myContract(input, output) = {
                output!(*input)
            }
        }
    "#};

    let doc = client.open_document("/path/to/contract.rho", source)
        .expect("Failed to open document");
    let diagnostics = client.await_diagnostics(&doc).unwrap();

    assert_eq!(
        diagnostics.diagnostics.len(),
        0,
        "Valid contract should produce no diagnostics"
    );
});

// Test that semantic validation only runs after syntax validation passes
#[cfg(feature = "interpreter")]
with_lsp_client!(test_semantic_after_syntax, CommType::Stdio, |client: &LspClient| {
    let source = indoc! {r#"
        new x in {
            x!(42
        }
    "#};  // Syntax error: missing closing paren

    let doc = client.open_document("/path/to/syntax_error.rho", source)
        .expect("Failed to open document");
    let diagnostics = client.await_diagnostics(&doc).unwrap();

    assert!(
        !diagnostics.diagnostics.is_empty(),
        "Should report syntax error"
    );

    // Should report syntax error, not semantic error
    assert!(
        diagnostics.diagnostics.iter().any(|d|
            d.source.as_ref().map_or(false, |s| s.contains("parser"))
        ),
        "Should report parser error before semantic validation"
    );
});

// Test nested scopes
#[cfg(feature = "interpreter")]
with_lsp_client!(test_semantic_nested_scopes, CommType::Stdio, |client: &LspClient| {
    let source = indoc! {r#"
        new outer in {
            outer!(42) |
            new inner in {
                inner!(100) |
                outer!(200)
            }
        }
    "#};

    let doc = client.open_document("/path/to/nested.rho", source)
        .expect("Failed to open document");
    let diagnostics = client.await_diagnostics(&doc).unwrap();

    assert_eq!(
        diagnostics.diagnostics.len(),
        0,
        "Valid nested scopes should produce no diagnostics"
    );
});

// Test receive patterns
#[cfg(feature = "interpreter")]
with_lsp_client!(test_semantic_receive_pattern, CommType::Stdio, |client: &LspClient| {
    let source = indoc! {r#"
        new ch in {
            for (x <- ch) {
                x!(42)
            }
        }
    "#};

    let doc = client.open_document("/path/to/receive.rho", source)
        .expect("Failed to open document");
    let diagnostics = client.await_diagnostics(&doc).unwrap();

    assert_eq!(
        diagnostics.diagnostics.len(),
        0,
        "Valid receive pattern should produce no diagnostics"
    );
});

// Test complex program with multiple constructs
#[cfg(feature = "interpreter")]
with_lsp_client!(test_semantic_complex_program, CommType::Stdio, |client: &LspClient| {
    let source = indoc! {r#"
        new stdout(`rho:io:stdout`), helloWorld in {
            contract helloWorld(input) = {
                for (msg <- input) {
                    stdout!(*msg)
                }
            } |
            new ch in {
                helloWorld!(*ch) |
                ch!("Hello, World!")
            }
        }
    "#};

    let doc = client.open_document("/path/to/complex.rho", source)
        .expect("Failed to open document");
    let diagnostics = client.await_diagnostics(&doc).unwrap();

    assert_eq!(
        diagnostics.diagnostics.len(),
        0,
        "Complex valid program should produce no diagnostics"
    );
});

// Test that updates trigger re-validation
#[cfg(feature = "interpreter")]
with_lsp_client!(test_semantic_revalidation_on_change, CommType::Stdio, |client: &LspClient| {
    // Start with invalid code
    let invalid_source = "x!(42)";  // Unbound variable

    let doc = client.open_document("/path/to/update.rho", invalid_source)
        .expect("Failed to open document");
    let diagnostics = client.await_diagnostics(&doc).unwrap();

    assert!(
        !diagnostics.diagnostics.is_empty(),
        "Should initially report error"
    );

    // Close and reopen with fixed code
    doc.close().expect("Failed to close document");
    let fixed_source = "new x in { x!(42) }";
    let doc2 = client.open_document("/path/to/update.rho", fixed_source)
        .expect("Failed to open document");

    let diagnostics = client.await_diagnostics(&doc2).unwrap();

    assert_eq!(
        diagnostics.diagnostics.len(),
        0,
        "Should clear diagnostics after fix"
    );
});

// Test multiple top-level processes with errors
#[cfg(feature = "interpreter")]
with_lsp_client!(test_semantic_multiple_processes, CommType::Stdio, |client: &LspClient| {
    let source = indoc! {r#"
        new x in { myContract!("foo") }
        new y in { otherContract!("bar") }
        new chan in {
            chan!() |
            new chan in {
                chan!() |
                han!()
            }
        }
    "#};

    let doc = client.open_document("/path/to/multiproc.rho", source)
        .expect("Failed to open document");
    let diagnostics = client.await_diagnostics(&doc).unwrap();

    // Should detect unbound variables: myContract, otherContract, han
    assert!(
        !diagnostics.diagnostics.is_empty(),
        "Should detect unbound variables in multiple processes"
    );

    // Check that we found at least the unbound variables
    let error_messages: Vec<String> = diagnostics.diagnostics
        .iter()
        .map(|d| d.message.clone())
        .collect();

    println!("Found {} diagnostics:", diagnostics.diagnostics.len());
    for msg in &error_messages {
        println!("  - {}", msg);
    }

    assert!(
        diagnostics.diagnostics.len() >= 3,
        "Should find at least 3 unbound variables (myContract, otherContract, han), found: {}",
        diagnostics.diagnostics.len()
    );
});

// Test without interpreter feature (stub behavior)
#[cfg(not(feature = "interpreter"))]
with_lsp_client!(test_no_semantic_validation_without_feature, CommType::Stdio, |client: &LspClient| {
    let source = "x!(42)";  // Would be invalid with semantic validation

    let doc = client.open_document("/path/to/no_feature.rho", source)
        .expect("Failed to open document");
    let diagnostics = client.await_diagnostics(&doc).unwrap();

    // Without the interpreter feature, only syntax errors are detected
    // This should not produce diagnostics (it's syntactically valid)
    assert!(
        diagnostics.diagnostics.is_empty() ||
        diagnostics.diagnostics.iter().all(|d|
            d.source.as_ref().map_or(true, |s| s.contains("parser"))
        ),
        "Without interpreter feature, should only see parser diagnostics"
    );
});
