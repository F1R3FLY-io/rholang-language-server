use std::sync::Arc;
use ropey::Rope;
use rholang_language_server::tree_sitter::{parse_code, parse_to_ir};
use rholang_language_server::ir::rholang_node::{RholangNode, compute_absolute_positions};
use tree_sitter::Node as TSNode;

fn print_ts_structure(node: TSNode, code: &str, depth: usize, max_depth: usize) {
    if depth > max_depth {
        return;
    }
    let indent = "  ".repeat(depth);
    let text_preview = if node.start_byte() < code.len() && node.end_byte() <= code.len() {
        let text = &code[node.start_byte()..node.end_byte()];
        if text.len() > 30 {
            format!("{:?}...", &text[..30])
        } else {
            format!("{:?}", text)
        }
    } else {
        "OUT OF BOUNDS".to_string()
    };

    println!("{}{} [{}, {}] named_count={} {}",
             indent, node.kind(), node.start_byte(), node.end_byte(),
             node.named_child_count(), text_preview);

    for child in node.children(&mut node.walk()) {
        print_ts_structure(child, code, depth + 1, max_depth);
    }
}

#[test]
fn test_simple_positions() {
    let code = r#"new x, y in {
  x!(1) |
  for (@a <- x) { Nil }
}"#;

    println!("Code:\n{}\n", code);
    println!("Code bytes:");
    for (i, ch) in code.chars().enumerate() {
        println!("  {}: {:?}", i, ch);
    }

    let tree = parse_code(&code);

    // Print tree-sitter structure
    println!("\n=== Tree-sitter structure ===");
    print_ts_structure(tree.root_node(), &code, 0, 10);

    // Find and print receipts node in detail
    fn find_receipts(node: TSNode) -> Option<TSNode> {
        if node.kind() == "receipts" {
            return Some(node);
        }
        for child in node.children(&mut node.walk()) {
            if let Some(found) = find_receipts(child) {
                return Some(found);
            }
        }
        None
    }

    if let Some(receipts) = find_receipts(tree.root_node()) {
        println!("\n=== Detailed receipts structure ===");
        print_ts_structure(receipts, &code, 0, 10);
    }

    let rope = Rope::from_str(&code);
    let ir = parse_to_ir(&tree, &rope);

    let positions = compute_absolute_positions(&ir);

    println!("\n=== All positions ===");
    let mut sorted: Vec<_> = positions.iter().collect();
    sorted.sort_by_key(|(_, (start, _))| start.byte);

    for (ptr, (start, end)) in sorted.iter() {
        let node_type = unsafe {
            let node_ref = &*(**ptr as *const RholangNode);
            match node_ref {
                RholangNode::Var { name, .. } => format!("Var({})", name),
                RholangNode::Par { .. } => "Par".to_string(),
                RholangNode::Send { .. } => "Send".to_string(),
                RholangNode::SendSync { .. } => "SendSync".to_string(),
                RholangNode::New { .. } => "New".to_string(),
                RholangNode::Block { .. } => "Block".to_string(),
                RholangNode::Input { .. } => "Input".to_string(),
                RholangNode::Nil { .. } => "Nil".to_string(),
                _ => "Other".to_string(),
            }
        };

        if start.byte < code.len() && end.byte <= code.len() {
            let text = &code[start.byte..end.byte];
            println!("{:15} [{:3}, {:3}] = {:?}", node_type, start.byte, end.byte,
                     if text.len() > 40 { &text[..40] } else { text });
        } else {
            println!("{:15} [{:3}, {:3}] = OUT OF BOUNDS (code len = {})",
                     node_type, start.byte, end.byte, code.len());
        }
    }

    // Check specific nodes
    println!("\n=== Checking Var nodes ===");
    for (ptr, (start, end)) in sorted.iter() {
        unsafe {
            let node_ref = &*(**ptr as *const RholangNode);
            if let RholangNode::Var { name, .. } = node_ref {
                let text = if start.byte < code.len() && end.byte <= code.len() {
                    &code[start.byte..end.byte]
                } else {
                    "OUT OF BOUNDS"
                };
                println!("  Var '{}': [{}, {}] = {:?}", name, start.byte, end.byte, text);

                // Check if position matches the variable name
                if text != *name {
                    println!("    ERROR: Position text doesn't match variable name!");
                }
            }
        }
    }
}
