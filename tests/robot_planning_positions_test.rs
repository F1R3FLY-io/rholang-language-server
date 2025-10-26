//! Test MeTTa symbol positions using actual robot_planning.rho content

use rholang_language_server::parsers::MettaParser;
use rholang_language_server::ir::transforms::metta_symbol_table_builder::MettaSymbolTableBuilder;
use std::fs;
use tower_lsp::lsp_types::Url;

#[test]
fn test_robot_planning_from_symbol_positions() {
    // Read the actual file
    let full_content = fs::read_to_string("tests/resources/robot_planning.rho")
        .expect("Failed to read robot_planning.rho");

    let lines: Vec<&str> = full_content.lines().collect();

    // Extract the MeTTa content (simplified - doesn't handle partial last line)
    // Content lines are 24-187 (1-indexed), indices 23-186
    let extracted_lines: Vec<&str> = lines[23..187].iter().copied().collect();
    let metta_code = format!("\n{}", extracted_lines.join("\n"));

    println!("\n=== MeTTa Content Info ===");
    println!("Total lines (with leading newline): {}", metta_code.lines().count());
    println!("Total bytes: {}", metta_code.len());

    // Find the is_connected definition
    let is_connected_line_1indexed = lines.iter().position(|l| l.contains("(= (is_connected $from $to)")).unwrap() + 1;
    println!("\n=== Source File Info ===");
    println!("is_connected definition at file line {} (1-indexed)", is_connected_line_1indexed);
    println!("Line content: {}", lines[is_connected_line_1indexed - 1]);
    println!("Next line content: {}", lines[is_connected_line_1indexed]);

    // In the virtual document:
    // Virtual line 0 = blank (the leading \n)
    // Virtual line 1 = file line 24
    // Virtual line N = file line (23 + N)
    // So for file line F: N = F - 23
    let virtual_line_0indexed = is_connected_line_1indexed - 23;
    println!("\n=== Virtual Document Mapping ===");
    println!("File line {} (1-indexed) maps to virtual line {} (0-indexed)", is_connected_line_1indexed, virtual_line_0indexed);

    // Parse the MeTTa content
    let mut parser = MettaParser::new().expect("Failed to create parser");
    let nodes = parser.parse_to_ir(&metta_code).expect("Failed to parse");

    println!("\n=== Parsed IR ===");
    println!("Top-level nodes: {}", nodes.len());

    // Build symbol table
    let test_uri = Url::parse("file:///test/robot_planning.rho").unwrap();
    let builder = MettaSymbolTableBuilder::new(test_uri);
    let table = builder.build(&nodes);

    println!("\n=== Symbol Table ===");
    println!("Total symbols: {}", table.all_occurrences.len());
    println!("Total scopes: {}", table.scopes.len());

    // Find all 'from' symbols
    let from_symbols: Vec<_> = table.all_occurrences.iter()
        .filter(|occ| occ.name == "from")
        .collect();

    println!("\n=== All 'from' symbols ===");
    for (i, sym) in from_symbols.iter().enumerate() {
        println!("[{}] '{}' at L{}:C{}-{} (scope {}, def={})",
            i, sym.name,
            sym.range.start.line, sym.range.start.character, sym.range.end.character,
            sym.scope_id, sym.is_definition);
    }

    // Find the is_connected scope
    let is_connected_symbols: Vec<_> = from_symbols.iter()
        .filter(|occ| occ.range.start.line == virtual_line_0indexed as u32 ||
                       occ.range.start.line == (virtual_line_0indexed + 1) as u32)
        .collect();

    println!("\n=== 'from' symbols in is_connected (virtual lines {}-{}) ===",
        virtual_line_0indexed, virtual_line_0indexed + 1);
    for sym in &is_connected_symbols {
        println!("  '{}' at L{}:C{}-{} (scope {}, def={})",
            sym.name,
            sym.range.start.line, sym.range.start.character, sym.range.end.character,
            sym.scope_id, sym.is_definition);

        // Map to parent document (LSP 0-indexed)
        let parent_line = 22 + sym.range.start.line;  // parent_start.line = 22
        let parent_start_char = sym.range.start.character;
        let parent_end_char = sym.range.end.character;

        println!("    -> Parent L{}:C{}-{}", parent_line, parent_start_char, parent_end_char);

        // Verify against actual content
        let actual_line = lines[parent_line as usize];
        let token = &actual_line[parent_start_char as usize..parent_end_char as usize];
        println!("    -> Actual text: {:?}", token);

        // The position should include the $ prefix, so the text should be "$from"
        // But the symbol name (stored in sym.name) is "from" without the $
        assert_eq!(token, "$from",
            "Expected '$from' at L{}:C{}-{}, but got {:?}",
            parent_line, parent_start_char, parent_end_char, token);
    }

    assert_eq!(is_connected_symbols.len(), 2,
        "Should find exactly 2 'from' symbols in is_connected definition");

    // Verify they're in the same scope
    let first_scope = is_connected_symbols[0].scope_id;
    let second_scope = is_connected_symbols[1].scope_id;
    assert_eq!(first_scope, second_scope,
        "Both 'from' symbols should be in the same scope, but got {} and {}",
        first_scope, second_scope);

    // Verify the first is a definition and the second is a reference
    assert!(is_connected_symbols[0].is_definition,
        "First 'from' should be a definition");
    assert!(!is_connected_symbols[1].is_definition,
        "Second 'from' should be a reference, not a definition");
}
