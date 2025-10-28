use rholang_language_server::ir::rholang_node::RholangNode;
use std::mem::size_of;

#[test]
fn measure_type_sizes() {
    eprintln!("\n=== Type Sizes ===");
    eprintln!("RholangNode: {} bytes", size_of::<RholangNode>());
    eprintln!("Arc<RholangNode>: {} bytes", size_of::<std::sync::Arc<RholangNode>>());
    eprintln!("Vector (rpds): {} bytes", size_of::<rpds::Vector<std::sync::Arc<RholangNode>, archery::ArcK>>());
    eprintln!("tree_sitter::Node: {} bytes", size_of::<tree_sitter::Node>());
    eprintln!("ropey::Rope: {} bytes", size_of::<ropey::Rope>());
    
    eprintln!("\n=== Actual Stack Frame Estimate ===");
    eprintln!("With 80-deep recursion:");
    let rholang_node_stack = 80 * size_of::<RholangNode>();
    let total_estimate = rholang_node_stack + (80 * 500); // 500 bytes overhead per frame
    eprintln!("  RholangNode copies: {} KB", rholang_node_stack / 1024);
    eprintln!("  Total with overhead: {} KB", total_estimate / 1024);
}
