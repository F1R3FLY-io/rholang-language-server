/// Test to investigate pattern matching impact on goto_definition
/// Tests goto_definition at different scope levels with pattern matching disabled/enabled

use indoc::indoc;
use tower_lsp::lsp_types::Position;

// Add this to tests/ directory to run with: cargo test --test test_pattern_matching_impact

use test_utils::with_lsp_client;
use test_utils::lsp::client::{CommType, LspClient};

// Test 1: Global scope - single-defined contract
#[test]
fn test_goto_global_scope_single() {
    with_lsp_client!(test_goto_global_scope_single_inner, CommType::Stdio, |client: &LspClient| {
        let code = indoc! {r#"
            contract globalContract(@x) = {
              stdout!(x, "ack")
            }

            new chan in {
              globalContract!("hello")
            }
        "#};

        let doc = client.open_document("/tmp/test_global.rho", code).unwrap();
        client.await_diagnostics(&doc).unwrap();

        // Click on "globalContract" in the call (line 5, column 3)
        let result = client.definition(&doc.uri(), Position::new(5, 3));

        println!("Test 1 (Global scope, single definition):");
        println!("  Result: {:?}", result);
        assert!(result.is_ok(), "goto_definition should succeed for global contract");
        assert!(result.unwrap().is_some(), "Should find definition");
    });
}

// Test 2: Nested scope - contract defined in parent scope
#[test]
fn test_goto_nested_scope() {
    with_lsp_client!(test_goto_nested_scope_inner, CommType::Stdio, |client: &LspClient| {
        let code = indoc! {r#"
            new outer in {
              contract nestedContract(@x) = {
                stdout!(x, "ack")
              }

              new inner in {
                nestedContract!("hello")
              }
            }
        "#};

        let doc = client.open_document("/tmp/test_nested.rho", code).unwrap();
        client.await_diagnostics(&doc).unwrap();

        // Click on "nestedContract" in the call (line 6, column 5)
        let result = client.definition(&doc.uri(), Position::new(6, 5));

        println!("Test 2 (Nested scope):");
        println!("  Result: {:?}", result);
        assert!(result.is_ok(), "goto_definition should succeed for nested contract");
        assert!(result.unwrap().is_some(), "Should find definition in parent scope");
    });
}

// Test 3: Click on contract DEFINITION (not call)
#[test]
fn test_goto_definition_click() {
    with_lsp_client!(test_goto_definition_click_inner, CommType::Stdio, |client: &LspClient| {
        let code = indoc! {r#"
            contract myContract(@x) = {
              stdout!(x, "ack")
            }
        "#};

        let doc = client.open_document("/tmp/test_defn_click.rho", code).unwrap();
        client.await_diagnostics(&doc).unwrap();

        // Click on "myContract" in the DEFINITION (line 0, column 9)
        let result = client.definition(&doc.uri(), Position::new(0, 9));

        println!("Test 3 (Click on definition):");
        println!("  Result: {:?}", result);
        assert!(result.is_ok(), "goto_definition should succeed when clicking on definition");
        // Should return the same location (goto self)
        assert!(result.unwrap().is_some(), "Should return definition location");
    });
}

// Test 4: Multiply-defined contract (pattern matching scenario)
#[test]
fn test_goto_multiply_defined() {
    with_lsp_client!(test_goto_multiply_defined_inner, CommType::Stdio, |client: &LspClient| {
        let code = indoc! {r#"
            contract multiContract(@"start") = {
              stdout!("Starting", "ack")
            }

            contract multiContract(@"stop") = {
              stdout!("Stopping", "ack")
            }

            new chan in {
              multiContract!("start")
            }
        "#};

        let doc = client.open_document("/tmp/test_multi.rho", code).unwrap();
        client.await_diagnostics(&doc).unwrap();

        // Click on "multiContract" in the call (line 9, column 3)
        let result = client.definition(&doc.uri(), Position::new(9, 3));

        println!("Test 4 (Multiply-defined contract):");
        println!("  Result: {:?}", result);
        assert!(result.is_ok(), "goto_definition should succeed for multi-defined contract");

        // With pattern matching enabled, should find the first definition (line 0)
        // The actual behavior depends on match_contract implementation
        let locations = result.unwrap();
        assert!(locations.is_some(), "Should find at least one definition");
    });
}

// Test 5: Symbol table test - verify symbols are indexed correctly
#[test]
fn test_symbol_table_indexing() {
    with_lsp_client!(test_symbol_table_indexing_inner, CommType::Stdio, |client: &LspClient| {
        let code = indoc! {r#"
            contract testContract(@x) = {
              new y in {
                stdout!(y, "ack")
              }
            }
        "#};

        let doc = client.open_document("/tmp/test_symbols.rho", code).unwrap();
        client.await_diagnostics(&doc).unwrap();

        // Get document symbols to verify symbol table construction
        let symbols = client.document_symbols(&doc.uri()).unwrap();

        println!("Test 5 (Symbol table):");
        println!("  Symbols: {:?}", symbols);
        assert!(!symbols.is_empty(), "Should have at least one symbol (testContract)");

        // Verify contract is indexed
        let has_contract = symbols.iter().any(|s| {
            if let tower_lsp::lsp_types::DocumentSymbol { name, .. } = s {
                name.contains("testContract")
            } else {
                false
            }
        });
        assert!(has_contract, "Should have testContract in symbols");
    });
}
