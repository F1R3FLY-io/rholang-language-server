//! Tests for MeTTa symbol scoping
//!
//! These tests verify that variable references are correctly scoped
//! to their definition context.

use tower_lsp::lsp_types::Url;
use rholang_language_server::parsers::MettaParser;
use rholang_language_server::ir::transforms::metta_symbol_table_builder::MettaSymbolTableBuilder;

#[test]
fn test_multiple_definitions_separate_scopes() {
    // Test that $from and $to in different definitions have separate scopes
    let metta_code = r#"
(= (find_any_path $from $to)
   (find_path_1hop $from $to))

(= (find_any_path $from $to)
   (find_path_2hop $from $to))
"#;

    let mut parser = MettaParser::new().expect("Failed to create parser");
    let nodes = parser.parse_to_ir(metta_code).expect("Failed to parse");

    let builder = MettaSymbolTableBuilder::new(Url::parse("file:///test.metta").unwrap());
    let table = builder.build(&nodes);

    // Should have global scope + 2 definition scopes = 3 scopes
    assert_eq!(table.scopes.len(), 3, "Expected 3 scopes: global + 2 definitions");

    // Find all occurrences of "from"
    let from_refs: Vec<_> = table.all_occurrences.iter()
        .filter(|occ| occ.name == "from")
        .collect();

    // Should have 4 occurrences: 2 definitions + 2 references
    assert_eq!(from_refs.len(), 4, "Expected 4 occurrences of 'from': 2 defs + 2 refs");

    // Get the two different scopes (should be scopes 1 and 2)
    let scope_ids: std::collections::HashSet<_> = from_refs.iter()
        .map(|occ| occ.scope_id)
        .collect();
    assert_eq!(scope_ids.len(), 2, "Expected 'from' to appear in 2 different scopes");

    // For each scope, verify that references only include symbols from that scope
    for scope_id in scope_ids {
        let symbols_in_scope: Vec<_> = from_refs.iter()
            .filter(|occ| occ.scope_id == scope_id)
            .collect();

        // Each scope should have exactly 2 occurrences: 1 def + 1 ref
        assert_eq!(symbols_in_scope.len(), 2,
            "Each definition scope should have 2 'from' occurrences (1 def + 1 ref)");

        // Verify one is a definition and one is a reference
        let defs = symbols_in_scope.iter().filter(|s| s.is_definition).count();
        let refs = symbols_in_scope.iter().filter(|s| !s.is_definition).count();
        assert_eq!(defs, 1, "Should have 1 definition");
        assert_eq!(refs, 1, "Should have 1 reference");
    }
}

#[test]
fn test_find_references_respects_scope_boundaries() {
    let metta_code = r#"
(= (find_any_path $from $to)
   (find_path_1hop $from $to))

(= (find_any_path $from $to)
   (find_path_2hop $from $to))
"#;

    let mut parser = MettaParser::new().expect("Failed to create parser");
    let nodes = parser.parse_to_ir(metta_code).expect("Failed to parse");

    let builder = MettaSymbolTableBuilder::new(Url::parse("file:///test.metta").unwrap());
    let table = builder.build(&nodes);

    // Get all 'from' occurrences
    let from_occs: Vec<_> = table.all_occurrences.iter()
        .filter(|occ| occ.name == "from")
        .collect();

    // Take the first occurrence (should be in first definition)
    let first_from = from_occs[0];

    // Find references for this symbol
    let refs = table.find_symbol_references(first_from);

    // Should only find 2 references: the def and ref in the SAME scope
    assert_eq!(refs.len(), 2,
        "find_symbol_references should only return occurrences from the same scope");

    // All references should be in the same scope
    let scope_id = refs[0].scope_id;
    for r in &refs {
        assert_eq!(r.scope_id, scope_id,
            "All references should be in the same scope");
    }
}
