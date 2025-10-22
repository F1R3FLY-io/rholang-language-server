use indoc::indoc;

use tower_lsp::lsp_types::{OneOf, Position, SymbolKind, WorkspaceSymbol};

use test_utils::with_lsp_client;
use test_utils::lsp::client::{CommType, LspClient};

with_lsp_client!(test_valid_syntax, CommType::Stdio, |client: &LspClient| {
    let doc = client.open_document("/path/to/valid.rho", "new x in { x!(\"Hello\") }").expect("Failed to open document");
    let diagnostic_params = client.await_diagnostics(&doc).unwrap();
    assert_eq!(diagnostic_params.diagnostics.len(), 0);  // No errors for valid syntax
});

with_lsp_client!(test_diagnostics_update, CommType::Stdio, |client: &LspClient| {
    // Open document with invalid code
    let doc = client.open_document("/path/to/test.rho", r#"new x in { x!("Hello") "#).unwrap();
    let diagnostics = client.await_diagnostics(&doc).unwrap();
    assert_eq!(diagnostics.diagnostics.len(), 1, "Expected one diagnostic initially");
    doc.move_cursor(1, 24);
    doc.insert_text("}".to_string()).expect("Failed to insert closing curly brace");
    println!("{}", doc.text().expect("Failed to get text"));
    let diagnostics = client.await_diagnostics(&doc).unwrap();
    println!("{:?}", diagnostics);
    assert_eq!(diagnostics.diagnostics.len(), 0, "Diagnostics should clear after fix");
});

with_lsp_client!(test_close_document, CommType::Stdio, |client: &LspClient| {
    let doc = client.open_document("/path/to/test.rho", "new x in { x!() }").unwrap();
    client.close_document(&doc).unwrap();
    // No diagnostics expected after close (server clears them)
    let diagnostics = client.await_diagnostics(&doc);
    assert!(diagnostics.is_err() || diagnostics.unwrap().diagnostics.is_empty(), "No diagnostics after close");
});

with_lsp_client!(test_rename, CommType::Stdio, |client: &LspClient| {
    // Define test file contents
    let contract_path = "/path/to/contract.rho";
    let contract_text = indoc! {r#"
        // contract.rho
        contract anotherContract(x) = { Nil }
        contract myContract(y) = { Nil }"#};

    let usage_path = "/path/to/usage.rho";
    let usage_text = indoc! {r#"
        // usage.rho
        new x in { myContract!("foo") }
        new y in { anotherContract!("bar") }
        new chan in {
            chan!() |
            new chan in {
                chan!()
            }
        }"#};

    // Open documents and wait for diagnostics
    let contract_doc = client
        .open_document(contract_path, contract_text)
        .expect("Failed to open contract.rho");
    client.await_diagnostics(&contract_doc)
        .expect("Failed to receive diagnostics for contract.rho");

    let usage_doc = client
        .open_document(usage_path, usage_text)
        .expect("Failed to open usage.rho");
    client.await_diagnostics(&usage_doc)
        .expect("Failed to receive diagnostics for usage.rho");

    // Test 1: Rename global contract 'myContract' to 'newContract'
    let contract_rename_pos = Position {
        line: 2,  // "contract myContract(y) = { Nil }"
        character: 9, // Start of 'myContract'
    };
    let new_contract_name = "newContract";
    client.rename(&contract_doc.uri(), contract_rename_pos, new_contract_name)
        .expect("Rename request for myContract failed");

    // Verify the changes
    let contract_text = contract_doc.text().expect("Failed to get contract_text");
    let contract_lines = contract_text.lines().collect::<Vec<_>>();
    assert_eq!(
        contract_lines[2].trim(),
        "contract newContract(y) = { Nil }",
        "Contract definition should be renamed"
    );
    assert_eq!(
        contract_lines[1].trim(),
        "contract anotherContract(x) = { Nil }",
        "Other contract should remain unchanged"
    );

    let usage_text = usage_doc.text().expect("Failed to get usage_text");
    let usage_lines = usage_text.lines().collect::<Vec<_>>();
    assert_eq!(
        usage_lines[1].trim(),
        "new x in { newContract!(\"foo\") }",
        "Contract usage should be renamed"
    );
    assert_eq!(
        usage_lines[2].trim(),
        "new y in { anotherContract!(\"bar\") }",
        "Other contract usage should remain unchanged"
    );

    // Test 2: Rename local variable 'chan' (outer scope) to 'outerChan'
    let chan_rename_pos = Position {
        line: 4,  // "new chan in {"
        character: 4, // Start of 'chan'
    };
    let new_chan_name = "outerChan";
    client.rename(&usage_doc.uri(), chan_rename_pos, new_chan_name)
        .expect("Rename request for chan failed");

    // Verify the changes
    let usage_text = usage_doc.text().expect("Failed to get usage_text");
    let usage_lines = usage_text.lines().collect::<Vec<_>>();
    assert_eq!(
        usage_lines[3].trim(),
        "new outerChan in {",
        "Outer chan declaration should be renamed"
    );
    assert_eq!(
        usage_lines[4].trim(),
        "outerChan!() |",
        "Outer chan usage should be renamed"
    );
    assert_eq!(
        usage_lines[5].trim(),
        "new chan in {",
        "Inner chan declaration should remain unchanged"
    );
    assert_eq!(
        usage_lines[6].trim(),
        "chan!()",
        "Inner chan usage should remain unchanged"
    );
});

with_lsp_client!(test_goto_declaration_same_file, CommType::Stdio, |client: &LspClient| {
    let code = indoc! {r#"
        contract myContract() = { Nil }
        new chan in { myContract!() }
    "#};
    let doc = client.open_document("/path/to/test.rho", code).unwrap();
    client.await_diagnostics(&doc).unwrap();

    let usage_pos = Position { line: 1, character: 14 }; // 'myContract' in 'myContract!()'
    let location = client.declaration(&doc.uri(), usage_pos).unwrap().unwrap();

    assert_eq!(location.uri.to_string(), doc.uri());
    assert_eq!(location.range.start.line, 0);
    assert_eq!(location.range.start.character, 9); // 'myContract' in declaration
});

with_lsp_client!(test_goto_definition_same_file, CommType::Stdio, |client: &LspClient| {
    let code = indoc! {r#"
        contract myContract() = { Nil }
        new chan in { myContract!() }
    "#};
    let doc = client.open_document("/path/to/test.rho", code).unwrap();
    client.await_diagnostics(&doc).unwrap();

    let usage_pos = Position { line: 1, character: 14 };
    let location = client.definition(&doc.uri(), usage_pos).unwrap().unwrap();

    assert_eq!(location.uri.to_string(), doc.uri());
    assert_eq!(location.range.start.line, 0);
    assert_eq!(location.range.start.character, 9);
});

with_lsp_client!(test_goto_declaration_cross_file, CommType::Stdio, |client: &LspClient| {
    let contract_code = indoc! {r#"
        contract myContract() = { Nil }
    "#};
    let usage_code = indoc! {r#"
        new chan in { myContract!() }
    "#};
    let contract_doc = client.open_document("/path/to/contract.rho", contract_code).unwrap();
    client.await_diagnostics(&contract_doc).unwrap();
    let usage_doc = client.open_document("/path/to/usage.rho", usage_code).unwrap();
    client.await_diagnostics(&usage_doc).unwrap();

    let usage_pos = Position { line: 0, character: 14 };
    let location = client.declaration(&usage_doc.uri(), usage_pos).unwrap().unwrap();

    assert_eq!(location.uri.to_string(), contract_doc.uri());
    assert_eq!(location.range.start.line, 0);
    assert_eq!(location.range.start.character, 9);
});

with_lsp_client!(test_goto_definition_cross_file, CommType::Stdio, |client: &LspClient| {
    let contract_code = indoc! {r#"
        contract myContract() = { Nil }
    "#};
    let usage_code = indoc! {r#"
        new chan in { myContract!() }
    "#};
    let contract_doc = client.open_document("/path/to/contract.rho", contract_code).unwrap();
    client.await_diagnostics(&contract_doc).unwrap();
    let usage_doc = client.open_document("/path/to/usage.rho", usage_code).unwrap();
    client.await_diagnostics(&usage_doc).unwrap();

    let usage_pos = Position { line: 0, character: 14 };
    let location = client.definition(&usage_doc.uri(), usage_pos).unwrap().unwrap();

    assert_eq!(location.uri.to_string(), contract_doc.uri());
    assert_eq!(location.range.start.line, 0);
    assert_eq!(location.range.start.character, 9);
});

with_lsp_client!(test_goto_definition_loop_param, CommType::Stdio, |client: &LspClient| {
    let loop_code = indoc! {r#"
        new input, output in {
            for (@message <- input) {
                output!(message)
            }
        }
    "#};
    let loop_doc = client.open_document("/path/to/loop.rho", loop_code).unwrap();
    client.await_diagnostics(&loop_doc).unwrap();

    let loop_pos = Position { line: 2, character: 16 };
    let location = client.definition(&loop_doc.uri(), loop_pos).unwrap().unwrap();

    assert_eq!(location.uri.to_string(), loop_doc.uri());
    assert_eq!(location.range.start.line, 1);
    assert_eq!(location.range.start.character, 9);  // Points to @ in @message (the bind pattern)
});

with_lsp_client!(test_references_local, CommType::Stdio, |client: &LspClient| {
    let code = indoc! {r#"
        new x in {
            x!() |
            x!()
        }
    "#};
    let doc = client.open_document("/path/to/test.rho", code).unwrap();
    client.await_diagnostics(&doc).unwrap();

    let position = Position { line: 1, character: 4 }; // First 'x'
    let references = client.references(&doc.uri(), position, true).unwrap();
    assert_eq!(references.len(), 3, "Should find declaration + two usages");

    let references = client.references(&doc.uri(), position, false).unwrap();
    assert_eq!(references.len(), 2, "Should find only two usages without declaration");
});

with_lsp_client!(test_references_global, CommType::Stdio, |client: &LspClient| {
    let contract_code = indoc! {r#"
        contract myContract() = { Nil }
    "#};
    let usage_code = indoc! {r#"
        new chan in { myContract!() }
    "#};
    let contract_doc = client.open_document("/path/to/contract.rho", contract_code).unwrap();
    client.await_diagnostics(&contract_doc).unwrap();
    let usage_doc = client.open_document("/path/to/usage.rho", usage_code).unwrap();
    client.await_diagnostics(&usage_doc).unwrap();

    let position = Position { line: 0, character: 9 }; // 'myContract' in declaration
    let references = client.references(&contract_doc.uri(), position, true).unwrap();
    assert_eq!(references.len(), 2, "Should find declaration + one usage");

    let usage_pos = Position { line: 0, character: 14 }; // 'myContract' in usage
    let references = client.references(&usage_doc.uri(), usage_pos, true).unwrap();
    assert_eq!(references.len(), 2, "Should find declaration + one usage from usage file");
});

with_lsp_client!(test_references_local_only, CommType::Stdio, |client: &LspClient| {
    let code1 = indoc! {r#"
        new x in { x!() }
    "#};
    let code2 = indoc! {r#"
        new x in { x!() }
    "#};
    let doc1 = client.open_document("/path/to/file1.rho", code1).unwrap();
    client.await_diagnostics(&doc1).unwrap();
    let doc2 = client.open_document("/path/to/file2.rho", code2).unwrap();
    client.await_diagnostics(&doc2).unwrap();

    let position = Position { line: 0, character: 4 }; // 'x' in file1
    let references = client.references(&doc1.uri(), position, true).unwrap();
    assert_eq!(references.len(), 2, "Should find declaration + one usage in file1 only");
});

with_lsp_client!(test_document_symbols, CommType::Stdio, |client: &LspClient| {
    let code = indoc! {r#"
        contract foo(x) = {
            new y in {
                y!()
            }
        }
        contract bar() = {
            let z = 42 in {
                z
            }
        }
    "#};
    let doc = client.open_document("/path/to/test.rho", code).unwrap();
    client.await_diagnostics(&doc).unwrap();

    let symbols = client.document_symbols(&doc.uri()).unwrap();
    assert_eq!(symbols.len(), 2, "Should find two top-level contracts");

    let foo_symbol = &symbols[0];
    assert_eq!(foo_symbol.name, "foo");
    assert_eq!(foo_symbol.kind, SymbolKind::FUNCTION);
    let foo_children = foo_symbol.children.as_ref().unwrap();
    assert_eq!(foo_children.len(), 2, "foo should have parameter x and new block");
    assert_eq!(foo_children[0].name, "x");
    assert_eq!(foo_children[0].kind, SymbolKind::VARIABLE);
    assert_eq!(foo_children[1].name, "new");
    assert_eq!(foo_children[1].kind, SymbolKind::NAMESPACE);
    let new_children = foo_children[1].children.as_ref().unwrap();
    assert_eq!(new_children.len(), 1, "new block should have variable y");
    assert_eq!(new_children[0].name, "y");
    assert_eq!(new_children[0].kind, SymbolKind::VARIABLE);

    let bar_symbol = &symbols[1];
    assert_eq!(bar_symbol.name, "bar");
    assert_eq!(bar_symbol.kind, SymbolKind::FUNCTION);
    let bar_children = bar_symbol.children.as_ref().unwrap();
    assert_eq!(bar_children.len(), 1, "bar should have let block");
    assert_eq!(bar_children[0].name, "let");
    assert_eq!(bar_children[0].kind, SymbolKind::NAMESPACE);
    let let_children = bar_children[0].children.as_ref().unwrap();
    assert_eq!(let_children.len(), 1, "let block should have variable z");
    assert_eq!(let_children[0].name, "z");
    assert_eq!(let_children[0].kind, SymbolKind::VARIABLE);
});

with_lsp_client!(test_workspace_symbols, CommType::Stdio, |client: &LspClient| {
    let code1 = indoc! {r#"
        contract foo() = { Nil }
    "#};
    let code2 = indoc! {r#"
        contract bar() = { Nil }
    "#};
    let doc1 = client.open_document("/path/to/file1.rho", code1).unwrap();
    client.await_diagnostics(&doc1).unwrap();
    let doc2 = client.open_document("/path/to/file2.rho", code2).unwrap();
    client.await_diagnostics(&doc2).unwrap();

    let symbols = client.workspace_symbols("foo").unwrap();
    assert_eq!(symbols.len(), 1, "Should find one symbol matching 'foo'");
    assert_eq!(symbols[0].name, "foo");
    assert_eq!(symbols[0].kind, SymbolKind::FUNCTION);
    assert_eq!(symbols[0].location.uri.to_string(), doc1.uri());

    let all_symbols = client.workspace_symbols("").unwrap();
    assert_eq!(all_symbols.len(), 2, "Should find all symbols with empty query");
});

with_lsp_client!(test_workspace_symbol_resolve, CommType::Stdio, |client: &LspClient| {
    let code = indoc! {r#"
        contract foo() = { Nil }
    "#};
    let doc = client.open_document("/path/to/test.rho", code).unwrap();
    client.await_diagnostics(&doc).unwrap();

    let symbols = client.workspace_symbols("foo").unwrap();
    assert_eq!(symbols.len(), 1, "Should find one symbol matching 'foo'");
    let symbol = symbols[0].clone();

    let workspace_symbol = WorkspaceSymbol {
        name: symbol.name.clone(),
        kind: symbol.kind,
        tags: symbol.tags,
        container_name: symbol.container_name,
        location: OneOf::Left(symbol.location.clone()),
        data: None,
    };
    let resolved_symbol = client.workspace_symbol_resolve(workspace_symbol).unwrap();
    assert_eq!(resolved_symbol.name, symbol.name, "Resolved symbol name should match");
    assert_eq!(resolved_symbol.kind, symbol.kind, "Resolved symbol kind should match");
    if let OneOf::Left(resolved_location) = resolved_symbol.location {
        assert_eq!(resolved_location.uri, symbol.location.uri, "Resolved symbol URI should match");
        assert_eq!(resolved_location.range, symbol.location.range, "Resolved symbol range should match");
    } else {
        panic!("expected OneOf::Left(Location)");
    }
});

with_lsp_client!(test_document_highlight_local, CommType::Stdio, |client: &LspClient| {
    let code = indoc! {r#"
        new x in {
            x!() |
            x!() |
            new x in {
                x!()
            }
        }
    "#};
    let doc = client.open_document("/path/to/test.rho", code).unwrap();
    client.await_diagnostics(&doc).unwrap();

    let position = Position { line: 1, character: 4 }; // First outer 'x' usage
    let highlights = client.document_highlight(&doc.uri(), position).expect("Failed to get document highlights");
    assert_eq!(highlights.len(), 3, "Should highlight declaration + two outer usages");

    let inner_position = Position { line: 4, character: 8 }; // Inner 'x' usage
    let inner_highlights = client.document_highlight(&doc.uri(), inner_position).expect("Failed to get inner document highlights");
    assert_eq!(inner_highlights.len(), 2, "Should highlight inner declaration + usage");
});

with_lsp_client!(test_document_highlight_contract, CommType::Stdio, |client: &LspClient| {
    let code = indoc! {r#"
        contract myContract() = { Nil }
        new chan in { myContract!() }
    "#};
    let doc = client.open_document("/path/to/test.rho", code).unwrap();
    client.await_diagnostics(&doc).unwrap();

    let decl_position = Position { line: 0, character: 9 }; // 'myContract' declaration
    let highlights = client.document_highlight(&doc.uri(), decl_position).expect("Failed to get document highlights");
    assert_eq!(highlights.len(), 2, "Should highlight declaration + usage");
});

with_lsp_client!(test_goto_definition_contract_on_name, CommType::Stdio, |client: &LspClient| {
    let code = indoc! {r#"
        new foo in {
            contract foo(@x) = {
                Nil
            } |
            foo!(42)
        }
    "#};
    let doc = client.open_document("/path/to/test.rho", code).unwrap();
    client.await_diagnostics(&doc).unwrap();

    let usage_pos = Position { line: 4, character: 4 }; // 'foo' in 'foo!(42)'
    let location = client.definition(&doc.uri(), usage_pos).unwrap().unwrap();

    assert_eq!(location.uri.to_string(), doc.uri());
    assert_eq!(location.range.start.line, 1);
    assert_eq!(location.range.start.character, 13); // 'foo' in 'contract foo'
});

with_lsp_client!(test_goto_declaration_contract_on_name, CommType::Stdio, |client: &LspClient| {
    let code = indoc! {r#"
        new foo in {
            contract foo(@x) = {
                Nil
            } |
            foo!(42)
        }
    "#};
    let doc = client.open_document("/path/to/test.rho", code).unwrap();
    client.await_diagnostics(&doc).unwrap();

    let usage_pos = Position { line: 4, character: 4 }; // 'foo' in 'foo!(42)'
    let location = client.declaration(&doc.uri(), usage_pos).unwrap().unwrap();

    assert_eq!(location.uri.to_string(), doc.uri());
    assert_eq!(location.range.start.line, 0);
    assert_eq!(location.range.start.character, 4); // 'foo' in 'new foo'
});

with_lsp_client!(test_references_contract_with_new, CommType::Stdio, |client: &LspClient| {
    let code = indoc! {r#"
        new foo in {
            contract foo(@x) = {
                Nil
            } |
            foo!(42)
        }
    "#};
    let doc = client.open_document("/path/to/test.rho", code).unwrap();
    client.await_diagnostics(&doc).unwrap();

    let position = Position { line: 4, character: 4 }; // 'foo' usage
    let references = client.references(&doc.uri(), position, true).unwrap();
    assert_eq!(references.len(), 3, "Should find new declaration, contract definition, and usage");

    let references_no_decl = client.references(&doc.uri(), position, false).unwrap();
    assert_eq!(references_no_decl.len(), 2, "Should find contract definition and usage without declaration");
});

with_lsp_client!(test_references_after_goto_definition_cross_file, CommType::Stdio, |client: &LspClient| {
    let contract_code = indoc! {r#"
        // contract.rho
        contract otherContract(x) = { x!("Hello World!") }
        contract myContract(y) = { Nil }
    "#};

    let usage_code = indoc! {r#"
        // usage.rho
        new x in { myContract!("foo") }
        new y in { otherContract!("bar") }
        new chan in {
            chan!() |
            new chan in {
                chan!()
            }
        }
    "#};

    let usage_doc = client
        .open_document("/path/to/usage.rho", usage_code)
        .expect("Failed to open usage.rho");

    let contract_doc = client
        .open_document("/path/to/contract.rho", contract_code)
        .expect("Failed to open contract.rho");

    // Open both documents at the same time to introduce a potential race
    // condition in how they are indexed.
    client.await_diagnostics(&usage_doc)
        .expect("Failed to receive diagnostics for usage.rho");
    client.await_diagnostics(&contract_doc)
        .expect("Failed to receive diagnostics for contract.rho");

    // Step 1: From usage.rho, goto definition of otherContract
    let usage_pos = Position { line: 2, character: 11 }; // 'otherContract' in usage
    let location = client.definition(&usage_doc.uri(), usage_pos).unwrap().unwrap();

    assert_eq!(location.uri.to_string(), contract_doc.uri());
    assert_eq!(location.range.start.line, 1);
    assert_eq!(location.range.start.character, 9); // 'otherContract' in declaration

    // Step 2: From contract.rho, find references of otherContract
    let decl_pos = Position { line: 1, character: 9 }; // 'otherContract' in declaration
    let references = client.references(&contract_doc.uri(), decl_pos, true).unwrap();

    assert_eq!(references.len(), 2, "Should find declaration + one usage");
    let usage_ref = references.iter().find(|r| r.uri.to_string() == usage_doc.uri()).expect("Usage reference not found");
    assert_eq!(usage_ref.range.start.line, 2);
    assert_eq!(usage_ref.range.start.character, 11);
});

// Add a new test to simulate race with reverse order
with_lsp_client!(test_references_after_goto_definition_reverse_order, CommType::Stdio, |client: &LspClient| {
    let contract_code = indoc! {r#"
        // contract.rho
        contract otherContract(x) = { x!("Hello World!") }
        contract myContract(y) = { Nil }
    "#};

    let usage_code = indoc! {r#"
        // usage.rho
        new x in { myContract!("foo") }
        new y in { otherContract!("bar") }
    "#};

    // Open contract first, then usage to simulate different order
    let contract_doc = client
        .open_document("/path/to/contract.rho", contract_code)
        .expect("Failed to open contract.rho");
    client.await_diagnostics(&contract_doc).unwrap();

    let usage_doc = client
        .open_document("/path/to/usage.rho", usage_code)
        .expect("Failed to open usage.rho");
    client.await_diagnostics(&usage_doc).unwrap();

    // Goto definition from usage
    let usage_pos = Position { line: 2, character: 11 };
    let location = client.definition(&usage_doc.uri(), usage_pos).unwrap().unwrap();
    assert_eq!(location.uri.to_string(), contract_doc.uri());
    assert_eq!(location.range.start.line, 1);
    assert_eq!(location.range.start.character, 9);

    // Find references from contract
    let decl_pos = Position { line: 1, character: 9 };
    let references = client.references(&contract_doc.uri(), decl_pos, true).unwrap();
    assert_eq!(references.len(), 2, "Should find declaration + usage in reverse order");
});
