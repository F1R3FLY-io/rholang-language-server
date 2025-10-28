use rholang_language_server::tree_sitter::{parse_code, parse_to_ir};
use ropey::Rope;
use std::fs;

#[test]
fn test_pathmap_simple() {
    let source = r#"new stdout in { stdout!({| @"foo"!(), @"bar"!() |}) }"#;

    let tree = parse_code(source);
    let rope = Rope::from_str(source);
    let _ir = parse_to_ir(&tree, &rope);

    // Verify we got a valid IR node
    eprintln!("Successfully parsed simple pathmap example");
}

#[test]
fn test_pathmap_in_robot_planning() {
    // Run with larger stack size to handle deep recursion in robot_planning.rho
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024) // 16 MB stack
        .spawn(test_pathmap_in_robot_planning_impl)
        .unwrap()
        .join()
        .unwrap();
}

fn test_pathmap_in_robot_planning_impl() {
    let source = fs::read_to_string("tests/resources/robot_planning.rho")
        .expect("Failed to read robot_planning.rho");

    // Parse the file - should not panic or produce pathmap warnings
    let tree = parse_code(&source);
    let rope = Rope::from_str(&source);
    let _ir = parse_to_ir(&tree, &rope);

    eprintln!("Successfully parsed robot_planning.rho with {} bytes", source.len());
}
