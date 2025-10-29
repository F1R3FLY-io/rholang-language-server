//! Test go-to-definition for MeTTa function calls
use tower_lsp::lsp_types::Url;
use rholang_language_server::parsers::MettaParser;
use rholang_language_server::ir::transforms::metta_symbol_table_builder::MettaSymbolTableBuilder;

#[test]
fn test_function_call_goto_definition() {
    // MeTTa code with a function definition and a call
    let metta_code = r#"
(= (is_connected $from $to)
   (match & self (connected $from $to) true))

(= (find_path_1hop $from $to)
   (if (is_connected $from $to)
       (path $from $to)
       ()))
"#;

    let mut parser = MettaParser::new().expect("Failed to create parser");
    let nodes = parser.parse_to_ir(metta_code).expect("Failed to parse");

    let uri = Url::parse("file:///test.metta").unwrap();
    let builder = MettaSymbolTableBuilder::new_simple(uri.clone());
    let table = builder.build(&nodes);

    println!("\n=== Symbol Table Stats ===");
    println!("Total symbols: {}", table.all_occurrences.len());
    println!("Pattern matcher has {} definitions",
        table.pattern_matcher.get_definitions_by_name("is_connected").len());

    // Find the symbol "is_connected" in the function call on line 5
    // The call is: (if (is_connected $from $to) ...)
    // Looking for the "is_connected" at approximately line 5, character 18

    // First, let's list all "is_connected" symbols
    let is_connected_symbols: Vec<_> = table.all_occurrences.iter()
        .filter(|occ| occ.name == "is_connected")
        .collect();

    println!("\n=== All 'is_connected' symbols ===");
    for sym in &is_connected_symbols {
        println!("  - L{}:C{}-{} (is_def={}, scope={})",
            sym.range.start.line,
            sym.range.start.character,
            sym.range.end.character,
            sym.is_definition,
            sym.scope_id);
    }

    // The call should be at line 5 (the one inside the if statement)
    let call_symbol = is_connected_symbols.iter()
        .find(|occ| occ.range.start.line >= 5 && occ.range.start.line <= 6 && !occ.is_definition)
        .expect("Should find is_connected call site");

    println!("\n=== Testing goto-definition for call site ===");
    println!("Call at L{}:C{}", call_symbol.range.start.line, call_symbol.range.start.character);

    // Use the symbol table's pattern matcher to find definitions
    // We need to find the call node that contains this position
    use rholang_language_server::ir::semantic_node::Position as IrPosition;
    let ir_pos = IrPosition {
        row: call_symbol.range.start.line as usize,
        column: call_symbol.range.start.character as usize,
        byte: 0,
    };

    // Search for the containing SExpr
    use rholang_language_server::ir::metta_node::{MettaNode, compute_positions_with_prev_end};
    use std::collections::HashMap;

    fn find_call_at_position(
        nodes: &[std::sync::Arc<MettaNode>],
        position: &IrPosition,
    ) -> Option<MettaNode> {
        let mut prev_end = IrPosition {
            row: 0,
            column: 0,
            byte: 0,
        };

        for node in nodes.iter() {
            let (positions, new_prev_end) = compute_positions_with_prev_end(node, prev_end);
            prev_end = new_prev_end;

            if let Some(call) = find_call_in_node(node, position, &positions) {
                return Some(call);
            }
        }
        None
    }

    fn find_call_in_node(
        node: &std::sync::Arc<MettaNode>,
        position: &IrPosition,
        positions: &HashMap<usize, (IrPosition, IrPosition)>,
    ) -> Option<MettaNode> {
        let node_ptr = &**node as *const MettaNode as usize;
        let (start, end) = positions.get(&node_ptr)?;

        // Check if position is within the node's range
        let position_in_node = if position.row < start.row || position.row > end.row {
            false
        } else if position.row == start.row && position.column < start.column {
            false
        } else if position.row == end.row && position.column > end.column {
            false
        } else {
            true
        };

        if !position_in_node {
            return None;
        }

        match &**node {
            MettaNode::SExpr { elements, .. } if elements.len() > 0 => {
                // Check children first
                for elem in elements {
                    if let Some(call) = find_call_in_node(elem, position, positions) {
                        return Some(call);
                    }
                }

                // If no child matched, check if this is a call
                if matches!(&*elements[0], MettaNode::Atom { .. }) {
                    return Some((**node).clone());
                }

                None
            }
            MettaNode::Definition { pattern, body, .. } => {
                find_call_in_node(pattern, position, positions)
                    .or_else(|| find_call_in_node(body, position, positions))
            }
            MettaNode::If { condition, consequence, alternative, .. } => {
                find_call_in_node(condition, position, positions)
                    .or_else(|| find_call_in_node(consequence, position, positions))
                    .or_else(|| {
                        if let Some(alt) = alternative {
                            find_call_in_node(alt, position, positions)
                        } else {
                            None
                        }
                    })
            }
            _ => None,
        }
    }

    let call_node = find_call_at_position(&table.ir_nodes, &ir_pos)
        .expect("Should find call node");

    println!("Found call node");

    // Now use the pattern matcher
    let matching_defs = table.find_function_definitions(&call_node);

    println!("\n=== Pattern matching results ===");
    println!("Found {} matching definitions", matching_defs.len());
    for def in &matching_defs {
        println!("  - at L{}:C{}", def.range.start.line, def.range.start.character);
    }

    assert_eq!(matching_defs.len(), 1, "Should find exactly one matching definition");
    assert_eq!(matching_defs[0].range.start.line, 1, "Definition should be on line 1");

    println!("\n✓ Pattern matching go-to-definition works!");
}
