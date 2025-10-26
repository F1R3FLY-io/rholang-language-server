//! Tests for MeTTa symbol position indexing
//!
//! Verify that symbols are indexed at the correct positions

use tower_lsp::lsp_types::Url;
use rholang_language_server::parsers::MettaParser;
use rholang_language_server::ir::transforms::metta_symbol_table_builder::MettaSymbolTableBuilder;
use tower_lsp::lsp_types::Position as LspPosition;

#[test]
fn test_simple_definition_positions() {
    let metta_code = "(= (find_any_path $from $to)\n   (find_path_1hop $from $to))";

    let mut parser = MettaParser::new().expect("Failed to create parser");
    let nodes = parser.parse_to_ir(metta_code).expect("Failed to parse");

    let builder = MettaSymbolTableBuilder::new(Url::parse("file:///test.metta").unwrap());
    let table = builder.build(&nodes);

    println!("Total scopes: {}", table.scopes.len());
    println!("Total occurrences: {}", table.all_occurrences.len());

    // Print all symbol occurrences with their positions
    for (i, occ) in table.all_occurrences.iter().enumerate() {
        println!("  [{}] '{}' at {}:{}-{}:{} scope={} is_def={}",
            i, occ.name,
            occ.range.start.line, occ.range.start.character,
            occ.range.end.line, occ.range.end.character,
            occ.scope_id, occ.is_definition);
    }

    // Find 'from' symbols
    let from_syms: Vec<_> = table.all_occurrences.iter()
        .filter(|occ| occ.name == "from")
        .collect();

    assert!(!from_syms.is_empty(), "Should find 'from' symbols");

    // Verify we can look up symbols at their positions
    for sym in &from_syms {
        let found = table.find_symbol_at_position(&sym.range.start);
        assert!(found.is_some(),
            "Should find symbol '{}' at position {}:{}",
            sym.name, sym.range.start.line, sym.range.start.character);

        let found_sym = found.unwrap();
        assert_eq!(found_sym.name, sym.name);
        println!("✓ Found '{}' at {}:{}",
            found_sym.name,
            found_sym.range.start.line,
            found_sym.range.start.character);
    }
}

#[test]
fn test_position_lookup_in_pattern() {
    // Test looking up symbols at specific character positions
    let metta_code = "(= (find_any_path $from $to) (find_path_1hop $from $to))";
    //                                   ^18    ^24                          ^42    ^48
    // The $from in the pattern should be around column 18

    let mut parser = MettaParser::new().expect("Failed to create parser");
    let nodes = parser.parse_to_ir(metta_code).expect("Failed to parse");

    let builder = MettaSymbolTableBuilder::new(Url::parse("file:///test.metta").unwrap());
    let table = builder.build(&nodes);

    println!("\nAll symbol occurrences:");
    for occ in &table.all_occurrences {
        println!("  '{}' at 0:{}-{} (scope {}, def={})",
            occ.name,
            occ.range.start.character,
            occ.range.end.character,
            occ.scope_id,
            occ.is_definition);
    }

    // Try to find a symbol in the pattern
    // The pattern is "(find_any_path $from $to)"
    // $from should start around column 19 (after the $ character)

    // Let's try a range of positions to see what we can find
    for col in 15..30 {
        let pos = LspPosition { line: 0, character: col as u32 };
        if let Some(sym) = table.find_symbol_at_position(&pos) {
            println!("At column {}: found '{}' ({}:{}-{}:{})",
                col, sym.name,
                sym.range.start.line, sym.range.start.character,
                sym.range.end.line, sym.range.end.character);
        }
    }
}
