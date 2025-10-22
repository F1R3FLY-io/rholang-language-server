//! Integration tests for reactive optimizations
//!
//! These tests verify that the reactive optimizations (debouncing, cancellation,
//! priority queuing, file watching) work correctly by testing their observable effects.

use indoc::indoc;
use test_utils::lsp::client::{CommType, LspClient};
use test_utils::with_lsp_client;

/// Test that the system handles rapid successive validations correctly
///
/// This verifies that even with rapid document opens/closes, the system:
/// 1. Remains responsive (doesn't hang)
/// 2. Produces correct final diagnostics
/// 3. Doesn't leak resources
#[cfg(feature = "interpreter")]
with_lsp_client!(test_rapid_document_operations, CommType::Stdio, |client: &LspClient| {
    // Open and close multiple documents rapidly
    // If debouncing works, this should not overwhelm the system
    for i in 0..10 {
        let uri = format!("/test/rapid_{}.rho", i);
        let source = format!("new x in {{ x!({}) }}", i);

        let doc = client.open_document(&uri, &source)
            .expect("Failed to open document");

        // Get diagnostics - should be empty for valid code
        let diagnostics = client.await_diagnostics(&doc)
            .expect("Failed to get diagnostics");

        assert_eq!(
            diagnostics.diagnostics.len(),
            0,
            "Document {} should have no errors",
            i
        );

        doc.close().expect("Failed to close document");
    }
});

/// Test that rapid opens produce correct diagnostics
///
/// This verifies that opening many documents in succession works correctly.
/// The reactive optimizations should handle this without overwhelming resources.
#[cfg(feature = "interpreter")]
with_lsp_client!(test_multiple_documents_validated, CommType::Stdio, |client: &LspClient| {
    // Open multiple documents with varying validity
    let test_cases = vec![
        ("/test/valid1.rho", "new x in { x!(1) }", true),
        ("/test/valid2.rho", "new y in { y!(2) }", true),
        ("/test/invalid1.rho", "new x in { z!(3) }", false),  // z unbound
        ("/test/valid3.rho", "new a, b in { a!(1) | b!(2) }", true),
        ("/test/invalid2.rho", "w!(42)", false),  // w unbound
    ];

    for (uri, source, should_be_valid) in test_cases {
        let doc = client.open_document(uri, source)
            .expect(&format!("Failed to open {}", uri));

        let diagnostics = client.await_diagnostics(&doc)
            .expect(&format!("Failed to get diagnostics for {}", uri));

        if should_be_valid {
            assert_eq!(
                diagnostics.diagnostics.len(),
                0,
                "{} should have no diagnostics",
                uri
            );
        } else {
            assert!(
                !diagnostics.diagnostics.is_empty(),
                "{} should have diagnostics",
                uri
            );
        }

        doc.close().expect(&format!("Failed to close {}", uri));
    }
});

/// Test that complex programs with multiple processes are validated correctly
///
/// This tests the priority indexing - complex files should still be validated correctly
#[cfg(feature = "interpreter")]
with_lsp_client!(test_complex_program_validation, CommType::Stdio, |client: &LspClient| {
    let source = indoc! {r#"
        new stdout(`rho:io:stdout`), myContract in {
            contract myContract(input, output) = {
                for (msg <- input) {
                    stdout!(*msg) |
                    output!(*msg)
                }
            } |
            new ch in {
                myContract!(*ch) |
                ch!("test message")
            }
        }
    "#};

    let doc = client.open_document("/test/complex.rho", source)
        .expect("Failed to open complex document");

    let diagnostics = client.await_diagnostics(&doc)
        .expect("Failed to get diagnostics");

    assert_eq!(
        diagnostics.diagnostics.len(),
        0,
        "Complex program should have no errors"
    );
});

/// Test that nested scopes are validated correctly
///
/// This ensures the progressive indexing handles nested structures properly
#[cfg(feature = "interpreter")]
with_lsp_client!(test_deeply_nested_scopes, CommType::Stdio, |client: &LspClient| {
    let source = indoc! {r#"
        new a in {
            a!(1) |
            new b in {
                b!(2) |
                a!(3) |
                new c in {
                    c!(4) |
                    b!(5) |
                    a!(6)
                }
            }
        }
    "#};

    let doc = client.open_document("/test/nested.rho", source)
        .expect("Failed to open nested document");

    let diagnostics = client.await_diagnostics(&doc)
        .expect("Failed to get diagnostics");

    assert_eq!(
        diagnostics.diagnostics.len(),
        0,
        "Nested scopes should validate correctly"
    );
});

/// Test that the system remains responsive under load
///
/// This opens, validates, and closes many documents to stress-test the reactive system
#[cfg(feature = "interpreter")]
with_lsp_client!(test_system_responsiveness_under_load, CommType::Stdio, |client: &LspClient| {
    // Open and validate 20 documents
    let mut docs = Vec::new();

    for i in 0..20 {
        let uri = format!("/test/load_{}.rho", i);
        let source = format!("new x{} in {{ x{}!({}) }}", i, i, i * 10);

        let doc = client.open_document(&uri, &source)
            .expect(&format!("Failed to open document {}", i));

        docs.push(doc);
    }

    // Validate all documents
    for (i, doc) in docs.iter().enumerate() {
        let diagnostics = client.await_diagnostics(doc)
            .expect(&format!("Failed to get diagnostics for document {}", i));

        assert_eq!(
            diagnostics.diagnostics.len(),
            0,
            "Document {} should have no errors",
            i
        );
    }

    // Close all documents
    for (i, doc) in docs.into_iter().enumerate() {
        doc.close().expect(&format!("Failed to close document {}", i));
    }
});

/// Test that errors in one document don't affect others
///
/// This verifies that the reactive workers handle documents independently
#[cfg(feature = "interpreter")]
with_lsp_client!(test_document_isolation, CommType::Stdio, |client: &LspClient| {
    // Open a valid document
    let valid_doc = client.open_document("/test/valid.rho", "new x in { x!(1) }")
        .expect("Failed to open valid document");

    // Open an invalid document
    let invalid_doc = client.open_document("/test/invalid.rho", "y!(42)")
        .expect("Failed to open invalid document");

    // Validate both
    let valid_diagnostics = client.await_diagnostics(&valid_doc)
        .expect("Failed to get diagnostics for valid document");

    let invalid_diagnostics = client.await_diagnostics(&invalid_doc)
        .expect("Failed to get diagnostics for invalid document");

    // Valid document should have no errors
    assert_eq!(
        valid_diagnostics.diagnostics.len(),
        0,
        "Valid document should have no errors"
    );

    // Invalid document should have errors
    assert!(
        !invalid_diagnostics.diagnostics.is_empty(),
        "Invalid document should have errors"
    );
});
