//! Test that symbol positions in virtual documents match lookup coordinates
use tower_lsp::lsp_types::Url;

use rholang_language_server::parsers::MettaParser;
use rholang_language_server::ir::transforms::metta_symbol_table_builder::MettaSymbolTableBuilder;

#[test]
fn test_symbol_positions_match_virtual_coordinates() {
    // This simulates MeTTa content that starts at line 0 of the virtual doc
    // (even if it's embedded at line 22 in the parent)
    let metta_code = r#"(= (find_any_path $from $to)
   (find_path_1hop $from $to))"#;

    let mut parser = MettaParser::new().expect("Failed to create parser");
    let nodes = parser.parse_to_ir(metta_code).expect("Failed to parse");

    let builder = MettaSymbolTableBuilder::new(Url::parse("file:///test.metta").unwrap());
    let table = builder.build(&nodes);

    println!("\n=== Symbol Positions ===");
    for occ in &table.all_occurrences {
        println!("'{}' at L{}:C{}-{} (scope {})",
            occ.name,
            occ.range.start.line,
            occ.range.start.character,
            occ.range.end.character,
            occ.scope_id);
    }

    // Find 'from' symbol
    let from_sym = table.all_occurrences.iter()
        .find(|occ| occ.name == "from" && occ.is_definition)
        .expect("Should find 'from' definition");

    println!("\nFirst 'from' is at L{}:C{}", from_sym.range.start.line, from_sym.range.start.character);

    // The symbol should be at line 0 (virtual doc coordinates)
    assert_eq!(from_sym.range.start.line, 0,
        "Symbol should be at line 0 in virtual document coordinates");

    // Now simulate what happens when a virtual doc at parent line 22 does lookup:
    // User clicks at parent L75:C40
    // Mapping: virtual_line = 75 - 22 = 53
    // But symbols are at line 0!

    println!("\nIf parent_start is line 22:");
    println!("  User clicks parent L75:C40");
    println!("  Maps to virtual L53:C? (75 - 22 = 53)");
    println!("  But symbols are at L0!");
    println!("  Result: MISMATCH - won't find symbol");
}
