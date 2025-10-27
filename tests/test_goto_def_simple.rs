/// Simplified test for goto_definition on contract definitions
/// This test isolates the goto_definition functionality without semantic validation overhead

use indoc::indoc;
use tower_lsp::lsp_types::Position;
use test_utils::with_lsp_client;
use test_utils::lsp::client::{CommType, LspClient};

#[test]
fn test_goto_def_simple_contract() {
    with_lsp_client!(test_goto_def_simple_contract_inner, CommType::Stdio, |client: &LspClient| {
        let code = indoc! {r#"
            contract targetContract(@x) = { Nil }
        "#};

        let doc = client.open_document("/tmp/simple.rho", code).unwrap();
        client.await_diagnostics(&doc).unwrap();

        // Click on "targetContract" in the DEFINITION (line 0, column 9)
        // The 't' in "targetContract" starts at column 9
        let result = client.definition(&doc.uri(), Position::new(0, 9));

        println!("Result: {:?}", result);
        assert!(result.is_ok(), "goto_definition should succeed");
        assert!(result.unwrap().is_some(), "Should find definition location");
    });
}

#[test]
fn test_goto_def_with_few_contracts() {
    with_lsp_client!(test_goto_def_with_few_contracts_inner, CommType::Stdio, |client: &LspClient| {
        let code = indoc! {r#"
            contract contract1(@x) = { Nil }
            contract contract2(@x) = { Nil }
            contract targetContract(@x) = { Nil }
            contract contract3(@x) = { Nil }
        "#};

        let doc = client.open_document("/tmp/few.rho", code).unwrap();
        client.await_diagnostics(&doc).unwrap();

        // Find the line containing targetContract
        let target_line = code.lines().position(|l| l.contains("targetContract")).unwrap();

        // Click on "targetContract" (column 9 is the 't' in "targetContract")
        let result = client.definition(&doc.uri(), Position::new(target_line as u32, 9));

        println!("Result for line {}: {:?}", target_line, result);
        assert!(result.is_ok(), "goto_definition should succeed");
        assert!(result.unwrap().is_some(), "Should find definition location");
    });
}

#[test]
fn test_goto_def_with_ten_contracts() {
    with_lsp_client!(test_goto_def_with_ten_contracts_inner, CommType::Stdio, |client: &LspClient| {
        let mut code = String::new();
        for i in 0..5 {
            code.push_str(&format!("contract contract{}(@x) = {{ Nil }}\n", i));
        }
        code.push_str("contract targetContract(@x) = { Nil }\n");
        for i in 5..10 {
            code.push_str(&format!("contract contract{}(@x) = {{ Nil }}\n", i));
        }

        let doc = client.open_document("/tmp/ten.rho", &code).unwrap();
        client.await_diagnostics(&doc).unwrap();

        // Find the line containing targetContract
        let target_line = code.lines().position(|l| l.contains("targetContract")).unwrap();

        // Click on "targetContract" (column 9 is the 't' in "targetContract")
        let result = client.definition(&doc.uri(), Position::new(target_line as u32, 9));

        println!("Result for line {}: {:?}", target_line, result);
        assert!(result.is_ok(), "goto_definition should succeed");
        assert!(result.unwrap().is_some(), "Should find definition location");
    });
}
