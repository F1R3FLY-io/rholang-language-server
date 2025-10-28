use rholang_language_server::tree_sitter::parse_code;
use std::fs;

#[test]
fn measure_robot_planning_ast_depth() {
    let source = fs::read_to_string("tests/resources/robot_planning.rho")
        .expect("Failed to read robot_planning.rho");

    eprintln!("File size: {} bytes", source.len());
    eprintln!("File lines: {}", source.lines().count());

    let tree = parse_code(&source);
    let root = tree.root_node();

    fn count_and_depth(node: tree_sitter::Node, depth: usize) -> (usize, usize) {
        let mut count = 1;
        let mut max_depth = depth;

        for child in node.children(&mut node.walk()) {
            let (child_count, child_depth) = count_and_depth(child, depth + 1);
            count += child_count;
            max_depth = max_depth.max(child_depth);
        }

        (count, max_depth)
    }

    let (total_nodes, max_depth) = count_and_depth(root, 0);

    eprintln!("\nTree-Sitter AST:");
    eprintln!("  Total nodes: {}", total_nodes);
    eprintln!("  Max depth: {}", max_depth);
    eprintln!("\nThis means convert_ts_node_to_ir recurses {} times", total_nodes);
    eprintln!("And the call stack is {} frames deep", max_depth);
    
    // Estimate stack usage
    // Each frame needs space for:
    // - Function arguments (~100 bytes)
    // - Local variables (~200 bytes)
    // - Return address and saved registers (~50 bytes)
    // Total per frame: ~350 bytes
    let estimated_stack = max_depth * 350;
    eprintln!("\nEstimated stack usage: {} KB", estimated_stack / 1024);
    eprintln!("With 16MB stack: {} KB available", 16 * 1024);
    eprintln!("Safety margin: {}x", (16 * 1024) / (estimated_stack / 1024));
}
