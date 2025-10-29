//! Compare scoping behavior between let expressions and rule definitions
use tower_lsp::lsp_types::Url;

use rholang_language_server::parsers::MettaParser;
use rholang_language_server::ir::transforms::metta_symbol_table_builder::MettaSymbolTableBuilder;

#[test]
fn test_let_expression_scoping() {
    // Test that $x in different let expressions have separate scopes
    let metta_code = r#"
(= (test1)
   (let $x 1 $x))

(= (test2)
   (let $x 2 $x))
"#;

    let mut parser = MettaParser::new().expect("Failed to create parser");
    let nodes = parser.parse_to_ir(metta_code).expect("Failed to parse");

    let builder = MettaSymbolTableBuilder::new_simple(Url::parse("file:///test.metta").unwrap());
    let table = builder.build(&nodes);

    println!("\n=== LET EXPRESSION SCOPING ===");
    println!("Total scopes: {}", table.scopes.len());

    // Print scope hierarchy
    for scope in &table.scopes {
        println!("Scope {}: parent={:?}", scope.id, scope.parent_id);
        for (name, occs) in &scope.symbols {
            println!("  '{}': {} occurrences", name, occs.len());
            for occ in occs {
                println!("    - at {}:{} (def={})",
                    occ.range.start.line, occ.range.start.character, occ.is_definition);
            }
        }
    }

    // Find all occurrences of "x"
    let x_refs: Vec<_> = table.all_occurrences.iter()
        .filter(|occ| occ.name == "x")
        .collect();

    println!("\nAll 'x' occurrences:");
    for (i, occ) in x_refs.iter().enumerate() {
        println!("  [{}] scope={} line={} char={} def={}",
            i, occ.scope_id, occ.range.start.line,
            occ.range.start.character, occ.is_definition);
    }

    // Get the scope IDs
    let scope_ids: std::collections::HashSet<_> = x_refs.iter()
        .map(|occ| occ.scope_id)
        .collect();
    println!("\n'x' appears in {} different scopes: {:?}", scope_ids.len(), scope_ids);

    // For the first 'x', get all references
    if let Some(first_x) = x_refs.first() {
        let refs = table.find_symbol_references(first_x);
        println!("\nReferences for first 'x' (scope {}): {} found", first_x.scope_id, refs.len());
        for r in &refs {
            println!("  - scope={} line={} char={}",
                r.scope_id, r.range.start.line, r.range.start.character);
        }
    }
}

#[test]
fn test_definition_parameter_scoping() {
    // Test that $from in different definitions have separate scopes
    let metta_code = r#"
(= (find_any_path $from $to)
   (find_path_1hop $from $to))

(= (find_any_path $from $to)
   (find_path_2hop $from $to))
"#;

    let mut parser = MettaParser::new().expect("Failed to create parser");
    let nodes = parser.parse_to_ir(metta_code).expect("Failed to parse");

    let builder = MettaSymbolTableBuilder::new_simple(Url::parse("file:///test.metta").unwrap());
    let table = builder.build(&nodes);

    println!("\n=== DEFINITION PARAMETER SCOPING ===");
    println!("Total scopes: {}", table.scopes.len());

    // Print scope hierarchy
    for scope in &table.scopes {
        println!("Scope {}: parent={:?}", scope.id, scope.parent_id);
        for (name, occs) in &scope.symbols {
            println!("  '{}': {} occurrences", name, occs.len());
            for occ in occs {
                println!("    - at {}:{} (def={})",
                    occ.range.start.line, occ.range.start.character, occ.is_definition);
            }
        }
    }

    // Find all occurrences of "from"
    let from_refs: Vec<_> = table.all_occurrences.iter()
        .filter(|occ| occ.name == "from")
        .collect();

    println!("\nAll 'from' occurrences:");
    for (i, occ) in from_refs.iter().enumerate() {
        println!("  [{}] scope={} line={} char={} def={}",
            i, occ.scope_id, occ.range.start.line,
            occ.range.start.character, occ.is_definition);
    }

    // Get the scope IDs
    let scope_ids: std::collections::HashSet<_> = from_refs.iter()
        .map(|occ| occ.scope_id)
        .collect();
    println!("\n'from' appears in {} different scopes: {:?}", scope_ids.len(), scope_ids);

    // For the first 'from', get all references
    if let Some(first_from) = from_refs.first() {
        let refs = table.find_symbol_references(first_from);
        println!("\nReferences for first 'from' (scope {}): {} found", first_from.scope_id, refs.len());
        for r in &refs {
            println!("  - scope={} line={} char={}",
                r.scope_id, r.range.start.line, r.range.start.character);
        }

        // THIS IS THE KEY TEST: Should only return 2 references (def + ref in same scope)
        assert_eq!(refs.len(), 2,
            "Should only return references from the same definition scope");
    }
}
