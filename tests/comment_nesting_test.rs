use ropey::Rope;
use rholang_language_server::tree_sitter::{parse_code, parse_to_ir};
use rholang_language_server::ir::rholang_node::RholangNode;

/// Counts the maximum depth of nested Par nodes in the IR
fn count_max_par_depth(node: &RholangNode) -> usize {
    match node {
        RholangNode::Par { left: Some(left), right: Some(right), .. } => {
            let left_depth = count_max_par_depth(left);
            let right_depth = count_max_par_depth(right);
            1 + left_depth.max(right_depth)
        }
        RholangNode::New { proc, .. } => count_max_par_depth(proc),
        RholangNode::Block { proc, .. } => count_max_par_depth(proc),
        RholangNode::Input { proc, .. } => count_max_par_depth(proc),
        RholangNode::Match { cases, .. } => {
            cases.iter().map(|(_, body)| count_max_par_depth(body)).max().unwrap_or(0)
        }
        _ => 0
    }
}

#[test]
fn test_comments_dont_increase_nesting() {
    // Code with 3 processes and 3 comments between them
    let code = r#"new x, y in {
  // Comment 1
  x!(1) |
  // Comment 2
  y!(2) |
  // Comment 3
  for (@a <- x) { Nil }
}
"#;

    let tree = parse_code(&code);
    let rope = Rope::from_str(&code);
    let ir = parse_to_ir(&tree, &rope);

    let max_depth = count_max_par_depth(&ir);

    // With 3 processes, we expect max 2 Par nodes deep (binary tree: Par(P1, Par(P2, P3)))
    // Comments should NOT add extra Par nodes
    println!("Max Par depth: {}", max_depth);
    assert!(max_depth <= 2, "Expected max Par depth of 2 for 3 processes, but got {}", max_depth);
}

#[test]
fn test_no_comment_nodes_in_ir() {
    let code = r#"new x in {
  // This is a comment
  x!(1) |
  /* Block comment */
  x!(2)
}
"#;

    let tree = parse_code(&code);
    let rope = Rope::from_str(&code);
    let ir = parse_to_ir(&tree, &rope);

    // Verify no Comment nodes exist in the IR
    fn has_comment_nodes(node: &RholangNode) -> bool {
        match node {
            RholangNode::Comment { .. } => true,
            RholangNode::Par { left: Some(left), right: Some(right), .. } => {
                has_comment_nodes(left) || has_comment_nodes(right)
            }
            RholangNode::New { proc, .. } => has_comment_nodes(proc),
            RholangNode::Block { proc, .. } => has_comment_nodes(proc),
            RholangNode::Send { channel, inputs, .. } => {
                has_comment_nodes(channel) || inputs.iter().any(|i| has_comment_nodes(i))
            }
            _ => false
        }
    }

    assert!(!has_comment_nodes(&ir), "IR should not contain any Comment nodes");
}
