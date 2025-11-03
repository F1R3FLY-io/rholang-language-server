use ropey::Rope;
use rholang_language_server::tree_sitter::{parse_code, parse_to_ir};
use rholang_language_server::ir::rholang_node::RholangNode;
use std::sync::Arc;

/// Helper to count Par node depth
fn count_par_depth(node: &Arc<RholangNode>) -> usize {
    match &**node {
        RholangNode::Par { left, right, processes, .. } => {
            if let Some(procs) = processes {
                // N-ary Par - depth is 1 (flat)
                1 + procs.iter().map(|p| count_par_depth(p)).max().unwrap_or(0)
            } else if let (Some(l), Some(r)) = (left, right) {
                // Binary Par - depth is 1 + max depth of children
                1 + count_par_depth(l).max(count_par_depth(r))
            } else {
                0
            }
        }
        _ => 0,
    }
}

/// Helper to count total Par nodes
fn count_par_nodes(node: &Arc<RholangNode>) -> usize {
    let mut count = 0;
    match &**node {
        RholangNode::Par { left, right, processes, .. } => {
            count = 1;
            if let Some(procs) = processes {
                count += procs.iter().map(|p| count_par_nodes(p)).sum::<usize>();
            } else if let (Some(l), Some(r)) = (left, right) {
                count += count_par_nodes(l) + count_par_nodes(r);
            }
        }
        RholangNode::Send { channel, inputs, .. } => {
            count += count_par_nodes(channel);
            count += inputs.iter().map(|i| count_par_nodes(i)).sum::<usize>();
        }
        RholangNode::New { decls, proc, .. } => {
            count += decls.iter().map(|d| count_par_nodes(d)).sum::<usize>();
            count += count_par_nodes(proc);
        }
        RholangNode::Block { proc, .. } => {
            count += count_par_nodes(proc);
        }
        _ => {}
    }
    count
}

/// Helper to get process count from a Par node
fn get_par_process_count(node: &Arc<RholangNode>) -> Option<usize> {
    match &**node {
        RholangNode::Par { left, right, processes, .. } => {
            if let Some(procs) = processes {
                Some(procs.len())
            } else if left.is_some() && right.is_some() {
                Some(2)
            } else {
                None
            }
        }
        _ => None,
    }
}

#[test]
fn test_par_flattening_basic() {
    // This creates: Par(Par(a, b), Par(c, d))
    // Which should flatten to: Par([a, b, c, d])
    let code = "a!(1) | b!(2) | c!(3) | d!(4)";

    println!("Testing Par flattening:");
    println!("Code: {}\n", code);

    let tree = parse_code(&code);
    let rope = Rope::from_str(&code);
    let ir = parse_to_ir(&tree, &rope);

    let depth = count_par_depth(&ir);
    let par_count = count_par_nodes(&ir);

    println!("Par depth: {}", depth);
    println!("Total Par nodes: {}", par_count);

    // With flattening, we should have depth 1 (all processes in one flat Par)
    // The exact count depends on how Tree-Sitter parses left-associative operators
    assert!(depth <= 2, "Expected Par depth <= 2 after flattening, got {}", depth);

    // We should have significantly fewer Par nodes than without flattening
    // Without flattening: 3 Par nodes for "a | b | c | d"
    // With flattening: 1 Par node
    assert!(par_count <= 2, "Expected <= 2 Par nodes after flattening, got {}", par_count);
}

#[test]
fn test_par_flattening_deeply_nested() {
    // Create a deeply nested Par expression
    let code = "x!(1) | x!(2) | x!(3) | x!(4) | x!(5) | x!(6) | x!(7) | x!(8)";

    println!("Testing deeply nested Par:");
    println!("Code: {}\n", code);

    let tree = parse_code(&code);
    let rope = Rope::from_str(&code);
    let ir = parse_to_ir(&tree, &rope);

    let depth = count_par_depth(&ir);
    let par_count = count_par_nodes(&ir);

    println!("Par depth: {}", depth);
    println!("Total Par nodes: {}", par_count);

    // Without flattening, 8 processes would create depth of 7
    // With flattening, depth should be O(1), ideally 1 or 2
    assert!(depth <= 3, "Expected Par depth <= 3 after flattening, got {}", depth);

    // Without flattening: 7 Par nodes
    // With flattening: Should be much fewer
    assert!(par_count <= 3, "Expected <= 3 Par nodes after flattening, got {}", par_count);
}

#[test]
fn test_par_flattening_process_count() {
    // Test that a flattened Par has the correct number of processes
    let code = "a!(1) | b!(2) | c!(3)";

    let tree = parse_code(&code);
    let rope = Rope::from_str(&code);
    let ir = parse_to_ir(&tree, &rope);

    // The root should be a Par node
    if let Some(count) = get_par_process_count(&ir) {
        println!("Par has {} processes", count);
        // Should have 3 processes after flattening
        assert_eq!(count, 3, "Expected 3 processes in flattened Par");
    } else {
        panic!("Expected root to be a Par node");
    }
}

#[test]
fn test_par_flattening_mixed() {
    // Test Par flattening with mixed expressions
    let code = "new x in { x!(1) | x!(2) } | y!(3) | z!(4)";

    println!("Testing mixed Par flattening:");
    println!("Code: {}\n", code);

    let tree = parse_code(&code);
    let rope = Rope::from_str(&code);
    let ir = parse_to_ir(&tree, &rope);

    let depth = count_par_depth(&ir);
    let par_count = count_par_nodes(&ir);

    println!("Par depth: {}", depth);
    println!("Total Par nodes: {}", par_count);

    // The outer Par should be flattened, but there's an inner Par inside the block
    // So we expect 2 levels max
    assert!(depth <= 3, "Expected Par depth <= 3 with mixed expressions, got {}", depth);
}

#[test]
fn test_par_no_flattening_needed() {
    // Test that simple 2-process Par doesn't create unnecessary overhead
    let code = "x!(1) | y!(2)";

    let tree = parse_code(&code);
    let rope = Rope::from_str(&code);
    let ir = parse_to_ir(&tree, &rope);

    // Should have exactly 1 Par node with 2 processes
    let par_count = count_par_nodes(&ir);
    assert_eq!(par_count, 1, "Expected exactly 1 Par node for simple binary Par");

    if let Some(count) = get_par_process_count(&ir) {
        assert_eq!(count, 2, "Expected 2 processes in binary Par");
    }
}
