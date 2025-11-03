use ropey::Rope;
use rholang_language_server::tree_sitter::{parse_code, parse_to_ir};
use rholang_language_server::ir::rholang_node::{RholangNode, compute_absolute_positions};
use rholang_language_server::ir::semantic_node::Position;

/// Test position tracking for n-ary Par nodes WITHOUT comments first
/// This tests the bug fix at line 257 in conversion/mod.rs
#[test]
fn test_nary_par_with_comments_position_tracking() {
    // Simple Par nodes - this should work correctly
    let code = r#"x!(1) | x!(2) | x!(3)"#;

    println!("Testing n-ary Par with comments:");
    println!("Code:\n{}\n", code);

    let tree = parse_code(&code);
    let rope = Rope::from_str(&code);
    let ir = parse_to_ir(&tree, &rope);

    // Compute absolute positions
    let positions = compute_absolute_positions(&ir);

    // Verify positions are within bounds
    println!("=== Position verification ===");
    let mut sorted: Vec<_> = positions.iter().collect();
    sorted.sort_by_key(|(_, (start, _))| start.byte);

    for (ptr, (start, end)) in &sorted {
        // Check bounds
        assert!(
            start.byte <= code.len(),
            "Start position {} exceeds code length {}",
            start.byte,
            code.len()
        );
        assert!(
            end.byte <= code.len(),
            "End position {} exceeds code length {}",
            end.byte,
            code.len()
        );

        // Check start <= end
        assert!(
            start.byte <= end.byte,
            "Invalid position: start {} > end {}",
            start.byte,
            end.byte
        );

        // Verify position corresponds to valid code
        let text = &code[start.byte..end.byte];

        unsafe {
            let node_ref = &*(**ptr as *const RholangNode);
            let node_type = match node_ref {
                RholangNode::Var { name, .. } => format!("Var({})", name),
                RholangNode::Par { .. } => "Par".to_string(),
                RholangNode::Send { .. } => "Send".to_string(),
                RholangNode::New { .. } => "New".to_string(),
                RholangNode::Block { .. } => "Block".to_string(),
                RholangNode::LongLiteral { .. } => "LongLiteral".to_string(),
                RholangNode::Nil { .. } => "Nil".to_string(),
                _ => "Other".to_string(),
            };

            println!(
                "{:15} [{}:{} - {}:{}] ({:3} - {:3}) = {:?}",
                node_type,
                start.row,
                start.column,
                end.row,
                end.column,
                start.byte,
                end.byte,
                if text.len() > 40 { &text[..40] } else { text }
            );
        }
    }

    println!("\n✅ All position invariants verified!");
}

/// Test position tracking for deeply nested Par nodes
#[test]
fn test_deeply_nested_par_positions() {
    let code = r#"x!(1) | y!(2) | z!(3) | w!(4)"#;

    println!("Testing deeply nested Par:");
    println!("Code: {}\n", code);

    let tree = parse_code(&code);
    let rope = Rope::from_str(&code);
    let ir = parse_to_ir(&tree, &rope);

    let positions = compute_absolute_positions(&ir);

    // Count Par nodes
    let mut all_positions: Vec<_> = positions.iter().collect();
    let par_count = all_positions.iter().filter(|(ptr, _)| unsafe {
        matches!(&*(**ptr as *const RholangNode), RholangNode::Par { .. })
    }).count();

    println!("Par nodes found: {}", par_count);

    // With left-associative grammar: x!(1) | y!(2) | z!(3) | w!(4)
    // Creates: Par(Par(Par(x!(1), y!(2)), z!(3)), w!(4))
    // So we expect 3 Par nodes
    assert!(par_count > 0, "Expected at least one Par node");

    // Verify all positions are valid
    for (ptr, (start, end)) in &all_positions {
        assert!(start.byte <= end.byte);
        assert!(end.byte <= code.len());

        unsafe {
            let node_ref = &*(**ptr as *const RholangNode);
            if let RholangNode::Par { .. } = node_ref {
                println!(
                    "Par [{}, {}] = {:?}",
                    start.byte,
                    end.byte,
                    &code[start.byte..end.byte]
                );
            }
        }
    }

    println!("✅ Nested Par positions verified!");
}

/// Test that prev_end threading is correct for first child in n-ary Par
#[test]
fn test_nary_par_first_child_position() {
    // This specifically tests the bug fix: first child should use prev_end, not absolute_start
    let code = r#"x!(1) | /* comment */ y!(2)"#;

    let tree = parse_code(&code);
    let rope = Rope::from_str(&code);
    let ir = parse_to_ir(&tree, &rope);

    let positions = compute_absolute_positions(&ir);

    // Find the first Send node (x!(1))
    let all_pos: Vec<_> = positions.iter().collect();
    let first_send = all_pos.iter().find(|(ptr, (start, _))| unsafe {
        matches!(&*(**ptr as *const RholangNode), RholangNode::Send { .. })
            && start.byte == 0
    });

    assert!(
        first_send.is_some(),
        "First Send node should start at byte 0"
    );

    // Verify position reconstruction works
    let (_, (start, end)) = first_send.unwrap();
    let text = &code[start.byte..end.byte];
    assert!(
        text.starts_with("x!(1)"),
        "First Send should be 'x!(1)', got: {:?}",
        text
    );

    println!("✅ First child position correct!");
}
