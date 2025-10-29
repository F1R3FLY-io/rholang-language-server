//! Debug test to see what Span values MeTTaTron provides
use tower_lsp::lsp_types::Url;

use rholang_language_server::parsers::MettaParser;
use rholang_language_server::ir::transforms::metta_symbol_table_builder::MettaSymbolTableBuilder;
use std::fs;

#[test]
fn test_metta_parser_span_output() {
    // Use a simpler test case without // comments
    let metta_code = r#"
; First rule
(= (find_any_path $from $to)
   (find_path_1hop $from $to))

; Second rule
(= (find_any_path $from $to)
   (find_path_2hop $from $to))

; Third rule spanning many lines
(= (is_connected $from $to)
   (match & self (connected $from $to) true))

; Fourth rule
(= (get_location $obj)
   (match & self (object_at $obj $loc) $loc))
"#;

    println!("\n=== MeTTa Content ===");
    println!("Total lines: {}", metta_code.lines().count());
    println!("Total bytes: {}", metta_code.len());

    let mut parser = MettaParser::new().expect("Failed to create parser");
    let nodes = parser.parse_to_ir(&metta_code).expect("Failed to parse");

    println!("\n=== Parsed IR ===");
    println!("Top-level nodes: {}", nodes.len());

    // Build symbol table
    let builder = MettaSymbolTableBuilder::new_simple(Url::parse("file:///test.metta").unwrap());
    let table = builder.build(&nodes);

    println!("\n=== Symbol Table ===");
    println!("Total symbols: {}", table.all_occurrences.len());
    println!("Sample symbols:");
    for (i, sym) in table.all_occurrences.iter().take(20).enumerate() {
        println!("  [{}] '{}' at L{}:C{}-{} (scope {}, def={})",
            i, sym.name,
            sym.range.start.line, sym.range.start.character, sym.range.end.character,
            sym.scope_id, sym.is_definition);
    }

    println!("\n=== Test completed successfully ===");
}
