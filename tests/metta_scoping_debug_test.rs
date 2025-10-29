//! Debug test to trace exactly what find_symbol_references returns
use tower_lsp::lsp_types::Url;

use rholang_language_server::parsers::MettaParser;
use rholang_language_server::ir::transforms::metta_symbol_table_builder::MettaSymbolTableBuilder;

#[test]
fn test_multiple_definitions_reference_lookup() {
    let metta_code = r#"(= (find_any_path $from $to)
   (find_path_1hop $from $to))

(= (find_any_path $from $to)
   (find_path_2hop $from $to))"#;

    let mut parser = MettaParser::new().expect("Failed to create parser");
    let nodes = parser.parse_to_ir(metta_code).expect("Failed to parse");

    let builder = MettaSymbolTableBuilder::new_simple(Url::parse("file:///test.metta").unwrap());
    let table = builder.build(&nodes);

    println!("\n=== DETAILED SCOPE ANALYSIS ===");
    println!("Total scopes: {}", table.scopes.len());

    for scope in &table.scopes {
        println!("\nScope {}: parent={:?}", scope.id, scope.parent_id);
        for (name, occs) in &scope.symbols {
            println!("  Symbol '{}': {} occurrences", name, occs.len());
            for occ in occs {
                println!("    - L{}:C{}-{} (def={}, kind={:?})",
                    occ.range.start.line,
                    occ.range.start.character,
                    occ.range.end.character,
                    occ.is_definition,
                    occ.kind);
            }
        }
    }

    // Get all 'from' occurrences
    let from_occs: Vec<_> = table.all_occurrences.iter()
        .filter(|occ| occ.name == "from")
        .collect();

    println!("\n=== ALL 'from' OCCURRENCES ===");
    for (i, occ) in from_occs.iter().enumerate() {
        println!("[{}] Scope={} L{}:C{}-{} def={}",
            i, occ.scope_id,
            occ.range.start.line,
            occ.range.start.character,
            occ.range.end.character,
            occ.is_definition);
    }

    // Test: Click on the FIRST 'from' (definition in pattern of first rule)
    // This should be at line 0, around column 19
    let first_from_def = from_occs.iter()
        .find(|occ| occ.range.start.line == 0 && occ.is_definition)
        .expect("Should find first 'from' definition");

    println!("\n=== TESTING REFERENCES FOR FIRST 'from' DEFINITION ===");
    println!("Clicked symbol: scope={} L{}:C{}-{}",
        first_from_def.scope_id,
        first_from_def.range.start.line,
        first_from_def.range.start.character,
        first_from_def.range.end.character);

    let refs = table.find_symbol_references(first_from_def);
    println!("\nfind_symbol_references returned {} occurrences:", refs.len());
    for r in &refs {
        println!("  - Scope={} L{}:C{}-{} def={}",
            r.scope_id,
            r.range.start.line,
            r.range.start.character,
            r.range.end.character,
            r.is_definition);
    }

    // Verify: Should only return 2 occurrences from the SAME scope
    assert_eq!(refs.len(), 2, "Should only return 2 references (def + ref in same scope)");

    let scope_id = refs[0].scope_id;
    for r in &refs {
        assert_eq!(r.scope_id, scope_id,
            "All references should be in the same scope ({})", scope_id);
    }

    // Verify none are from line 2 (the second definition)
    let has_line_2 = refs.iter().any(|r| r.range.start.line == 2);
    assert!(!has_line_2, "Should NOT include references from the second definition (line 2)");

    println!("\n✓ PASS: Scoping works correctly for definition parameters");
}
