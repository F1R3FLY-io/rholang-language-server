//! Simple test to verify all contracts are parsed
use std::fs;
use std::sync::Arc;
use ropey::Rope;
use rholang_language_server::tree_sitter::{parse_code, parse_to_ir};
use rholang_language_server::ir::rholang_node::RholangNode;

#[test]
fn test_all_seven_contracts_parsed() {
    // Run with larger stack size to handle deep recursion
    std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024) // 8 MB stack
        .spawn(|| {
            let full_content = fs::read_to_string("tests/resources/robot_planning.rho")
                .expect("Failed to read robot_planning.rho");

            println!("File size: {} bytes", full_content.len());

            let tree = parse_code(&full_content);
            println!("Tree-sitter parsing complete");

            let rope = Rope::from_str(&full_content);
            println!("Rope created, starting IR conversion...");

            let ir = parse_to_ir(&tree, &rope);
            println!("IR conversion complete");

            // Count contracts in IR
            let contract_count = count_contracts(&ir);

            println!("Found {} contract nodes in IR", contract_count);

            // The file has 7 contracts total (init + 6 query contracts)
            assert!(contract_count >= 7, "Expected at least 7 contracts, found {}", contract_count);
        })
        .unwrap()
        .join()
        .unwrap();
}

fn count_contracts(node: &Arc<RholangNode>) -> usize {
    // Use iterative approach with explicit stack to avoid stack overflow
    let mut count = 0;
    let mut stack = vec![node.clone()];

    while let Some(current) = stack.pop() {
        if matches!(&*current, RholangNode::Contract { .. }) {
            count += 1;
        }

        // Push children onto stack
        match &*current {
            RholangNode::Par { left: Some(left), right: Some(right), processes: None, .. } => {
                // Binary Par node
                stack.push(right.clone());
                stack.push(left.clone());
            }
            RholangNode::Par { processes: Some(procs), left: None, right: None, .. } => {
                // N-ary Par node
                for proc in procs.iter().rev() {
                    stack.push(proc.clone());
                }
            }
            RholangNode::Block { proc, .. } => {
                stack.push(proc.clone());
            }
            RholangNode::New { proc, .. } => {
                stack.push(proc.clone());
            }
            RholangNode::Contract { proc, .. } => {
                stack.push(proc.clone());
            }
            RholangNode::Input { proc, .. } => {
                stack.push(proc.clone());
            }
            _ => {}
        }
    }

    count
}
