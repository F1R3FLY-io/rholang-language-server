//! Integration test for pattern matching with the real robot_planning.rho file
use tower_lsp::lsp_types::Url;
use std::fs;
use std::sync::Arc;

use rholang_language_server::parsers::MettaParser;
use rholang_language_server::ir::transforms::metta_symbol_table_builder::MettaSymbolTableBuilder;
use rholang_language_server::ir::metta_node::MettaNode;

#[test]
fn test_robot_planning_pattern_indexing() {
    // Read the robot_planning.rho file from test resources
    let full_content = fs::read_to_string(
        "tests/resources/robot_planning.rho"
    ).expect("Failed to read robot_planning.rho");

    let lines: Vec<&str> = full_content.lines().collect();

    // Extract the MeTTa content (lines 24-187, indices 23-186)
    let extracted_lines: Vec<&str> = lines[23..187].iter().copied().collect();
    let metta_code = format!("\n{}", extracted_lines.join("\n"));

    println!("\n=== MeTTa Content Info ===");
    println!("Total lines: {}", metta_code.lines().count());
    println!("Total bytes: {}", metta_code.len());

    // Parse the MeTTa content
    let mut parser = MettaParser::new().expect("Failed to create parser");
    let nodes = parser.parse_to_ir(&metta_code).expect("Failed to parse MeTTa");

    println!("\n=== Parsed {} top-level nodes ===", nodes.len());

    // Build symbol table with pattern indexing
    let uri = Url::parse("file:///test/robot_planning.rho").unwrap();
    let builder = MettaSymbolTableBuilder::new(uri.clone());
    let table = builder.build(&nodes);

    println!("\n=== Symbol Table Stats ===");
    println!("Total symbols: {}", table.all_occurrences.len());
    println!("Scopes: {}", table.scopes.len());

    // Verify pattern matching index has definitions
    // We expect at least:
    // - is_connected (line 75)
    // - find_any_path (lines 108, 111, 114 - 3 definitions!)
    // - get_neighbors, locate, etc.

    let is_connected_defs = table.pattern_matcher.get_definitions_by_name("is_connected");
    println!("\n=== 'is_connected' definitions ===");
    println!("Found {} definitions", is_connected_defs.len());
    for def in &is_connected_defs {
        println!("  - Arity {}, at L{}:C{}",
            def.arity,
            def.location.range.start.line,
            def.location.range.start.character);
    }
    assert!(is_connected_defs.len() >= 1, "Should find is_connected definition");
    assert_eq!(is_connected_defs[0].arity, 2, "is_connected should have arity 2");

    let find_any_path_defs = table.pattern_matcher.get_definitions_by_name("find_any_path");
    println!("\n=== 'find_any_path' definitions ===");
    println!("Found {} definitions", find_any_path_defs.len());
    for def in &find_any_path_defs {
        println!("  - Arity {}, at L{}:C{}",
            def.arity,
            def.location.range.start.line,
            def.location.range.start.character);
    }
    assert_eq!(find_any_path_defs.len(), 3, "Should find 3 definitions of find_any_path");
    // All should have arity 2
    for def in &find_any_path_defs {
        assert_eq!(def.arity, 2, "All find_any_path definitions should have arity 2");
    }

    let get_neighbors_defs = table.pattern_matcher.get_definitions_by_name("get_neighbors");
    println!("\n=== 'get_neighbors' definitions ===");
    println!("Found {} definitions", get_neighbors_defs.len());
    assert!(get_neighbors_defs.len() >= 1, "Should find get_neighbors definition");
    assert_eq!(get_neighbors_defs[0].arity, 1, "get_neighbors should have arity 1");

    println!("\n✓ Pattern indexing works correctly on robot_planning.rho");
}

#[test]
fn test_robot_planning_call_site_matching() {
    // Read robot_planning.rho from test resources
    let full_content = fs::read_to_string(
        "tests/resources/robot_planning.rho"
    ).expect("Failed to read robot_planning.rho");

    let lines: Vec<&str> = full_content.lines().collect();
    let extracted_lines: Vec<&str> = lines[23..187].iter().copied().collect();
    let metta_code = format!("\n{}", extracted_lines.join("\n"));

    let mut parser = MettaParser::new().expect("Failed to create parser");
    let nodes = parser.parse_to_ir(&metta_code).expect("Failed to parse MeTTa");

    let uri = Url::parse("file:///test/robot_planning.rho").unwrap();
    let builder = MettaSymbolTableBuilder::new(uri.clone());
    let table = builder.build(&nodes);

    // Find a call to is_connected in the IR
    // We know there's (is_connected $from $to) in the if statement on line 88
    // Let's search the IR for SExpr nodes that are function calls to is_connected

    fn find_is_connected_calls(node: &Arc<MettaNode>) -> Vec<Arc<MettaNode>> {
        let mut calls = Vec::new();

        match &**node {
            MettaNode::SExpr { elements, .. } if elements.len() > 0 => {
                // Check if this is a call to is_connected
                if let Some(name) = elements[0].name() {
                    if name == "is_connected" {
                        calls.push(node.clone());
                    }
                }

                // Recurse into children
                for elem in elements {
                    calls.extend(find_is_connected_calls(elem));
                }
            }
            MettaNode::Definition { pattern, body, .. } => {
                calls.extend(find_is_connected_calls(pattern));
                calls.extend(find_is_connected_calls(body));
            }
            // Recurse into other node types that might contain SExprs
            _ => {}
        }

        calls
    }

    let mut all_calls = Vec::new();
    for node in &nodes {
        all_calls.extend(find_is_connected_calls(node));
    }

    println!("\n=== Found {} calls to 'is_connected' ===", all_calls.len());

    // We should find multiple calls (in the if conditions)
    assert!(all_calls.len() > 0, "Should find at least one call to is_connected");

    // Test pattern matching for the first call
    let first_call = &all_calls[0];
    let matching_defs = table.find_function_definitions(&**first_call);

    println!("\n=== Pattern matching results for first is_connected call ===");
    println!("Found {} matching definitions", matching_defs.len());
    for def in &matching_defs {
        println!("  - at L{}:C{}",
            def.range.start.line,
            def.range.start.character);
    }

    assert!(matching_defs.len() >= 1, "Should find at least one matching definition");

    println!("\n✓ Call site pattern matching works correctly");
}

#[test]
fn test_multiple_find_any_path_definitions() {
    // Verify that we correctly distinguish multiple definitions of the same function
    let full_content = fs::read_to_string(
        "tests/resources/robot_planning.rho"
    ).expect("Failed to read robot_planning.rho");

    let lines: Vec<&str> = full_content.lines().collect();
    let extracted_lines: Vec<&str> = lines[23..187].iter().copied().collect();
    let metta_code = format!("\n{}", extracted_lines.join("\n"));

    let mut parser = MettaParser::new().expect("Failed to create parser");
    let nodes = parser.parse_to_ir(&metta_code).expect("Failed to parse MeTTa");

    let uri = Url::parse("file:///test/robot_planning.rho").unwrap();
    let builder = MettaSymbolTableBuilder::new(uri.clone());
    let table = builder.build(&nodes);

    // Get all find_any_path definitions
    let defs = table.pattern_matcher.get_definitions_by_name("find_any_path");

    println!("\n=== All 'find_any_path' definitions ===");
    assert_eq!(defs.len(), 3, "Should have exactly 3 definitions");

    // Verify they're at different locations
    let mut line_numbers: Vec<u32> = defs.iter()
        .map(|d| d.location.range.start.line)
        .collect();
    line_numbers.sort();

    println!("Definition line numbers: {:?}", line_numbers);

    // All should be unique (different lines)
    for i in 1..line_numbers.len() {
        assert!(line_numbers[i] > line_numbers[i-1],
            "Definitions should be at different line numbers");
    }

    println!("\n✓ Multiple definitions correctly indexed at separate locations");
}
