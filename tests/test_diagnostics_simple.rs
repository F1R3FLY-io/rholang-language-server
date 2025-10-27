/// Simplified test for diagnostics update
/// This test isolates the didChange + await_diagnostics functionality

use test_utils::with_lsp_client;
use test_utils::lsp::client::{CommType, LspClient};

#[test]
fn test_diagnostics_update_simple() {
    with_lsp_client!(test_diagnostics_update_simple_inner, CommType::Stdio, |client: &LspClient| {
        // Open document with invalid code
        let doc = client.open_document("/tmp/test.rho", r#"new x in { x!("Hello") "#).unwrap();
        println!("Opened document, awaiting initial diagnostics...");

        let diagnostics = client.await_diagnostics(&doc).unwrap();
        println!("Got {} diagnostics initially", diagnostics.diagnostics.len());
        assert_eq!(diagnostics.diagnostics.len(), 1, "Expected one diagnostic initially");

        // Fix the code by adding the closing brace
        doc.move_cursor(1, 24);
        doc.insert_text("}".to_string()).expect("Failed to insert closing curly brace");
        println!("Inserted closing brace, text is now: {}", doc.text().expect("Failed to get text"));

        println!("Awaiting diagnostics after fix...");
        let diagnostics = client.await_diagnostics(&doc).unwrap();
        println!("Got {} diagnostics after fix: {:?}", diagnostics.diagnostics.len(), diagnostics);

        assert_eq!(diagnostics.diagnostics.len(), 0, "Diagnostics should clear after fix");
    });
}

#[test]
fn test_diagnostics_basic_valid() {
    with_lsp_client!(test_diagnostics_basic_valid_inner, CommType::Stdio, |client: &LspClient| {
        let doc = client.open_document("/tmp/valid.rho", "new x in { x!(\"Hello\") }").expect("Failed to open document");
        let diagnostic_params = client.await_diagnostics(&doc).unwrap();
        assert_eq!(diagnostic_params.diagnostics.len(), 0);  // No errors for valid syntax
    });
}
