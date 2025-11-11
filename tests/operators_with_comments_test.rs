//! Tests for operators with inline comments
//!
//! These tests verify that the IR conversion correctly handles comments
//! appearing between operands in binary and unary operators.
//!
//! Background: Comments are marked as "extras" in the Tree-Sitter grammar,
//! meaning they can appear anywhere in the AST. The conversion code must
//! filter them out when building the semantic IR, while preserving them
//! in the comment channel for directive parsing and documentation.

use rholang_language_server::parsers::rholang::{parse_code, parse_to_document_ir};
use rholang_language_server::ir::rholang_node::RholangNode;
use ropey::Rope;

#[test]
fn test_binary_op_with_line_comment_between_operands() {
    let source = r#"
new result in {
  result!(x // inline comment
  + y)
}
"#;
    let tree = parse_code(source);
    let rope = Rope::from_str(source);
    let doc_ir = parse_to_document_ir(&tree, &rope);

    // Verify comment is collected in comment channel
    assert_eq!(doc_ir.comments.len(), 1, "Should collect 1 comment");
    assert!(doc_ir.comments[0].text.contains("inline comment"));

    // Verify IR conversion succeeds (no panic)
    // The semantic tree should not contain the comment
    assert!(matches!(&*doc_ir.root, RholangNode::New { .. }));
}

#[test]
fn test_binary_op_with_block_comment_between_operands() {
    let source = r#"
new result in {
  result!(x /* block comment */ + y)
}
"#;
    let tree = parse_code(source);
    let rope = Rope::from_str(source);
    let doc_ir = parse_to_document_ir(&tree, &rope);

    // Verify comment is collected in comment channel
    assert_eq!(doc_ir.comments.len(), 1, "Should collect 1 comment");
    assert!(doc_ir.comments[0].text.contains("block comment"));

    // Verify IR conversion succeeds (no panic)
    assert!(matches!(&*doc_ir.root, RholangNode::New { .. }));
}

#[test]
fn test_unary_op_with_comment_between_operator_and_operand() {
    let source = r#"
new result in {
  result!(- /* comment */ x)
}
"#;
    let tree = parse_code(source);
    let rope = Rope::from_str(source);
    let doc_ir = parse_to_document_ir(&tree, &rope);

    // Verify comment is collected in comment channel
    assert_eq!(doc_ir.comments.len(), 1, "Should collect 1 comment");
    assert!(doc_ir.comments[0].text.contains("comment"));

    // Verify IR conversion succeeds (no panic)
    assert!(matches!(&*doc_ir.root, RholangNode::New { .. }));
}

#[test]
fn test_multiple_comments_in_complex_expression() {
    let source = r#"
new result in {
  result!(
    x /* first */ + y // second
    * z /* third */
  )
}
"#;
    let tree = parse_code(source);
    let rope = Rope::from_str(source);
    let doc_ir = parse_to_document_ir(&tree, &rope);

    // Verify all comments are collected in comment channel
    assert_eq!(doc_ir.comments.len(), 3, "Should collect 3 comments");
    assert!(doc_ir.comments[0].text.contains("first"));
    assert!(doc_ir.comments[1].text.contains("second"));
    assert!(doc_ir.comments[2].text.contains("third"));

    // Verify IR conversion succeeds (no panic)
    assert!(matches!(&*doc_ir.root, RholangNode::New { .. }));
}

#[test]
fn test_all_binary_operators_with_comments() {
    // Test various binary operators to ensure comment filtering works consistently
    let operators = vec![
        ("x /* c */ + y", "addition"),
        ("x /* c */ - y", "subtraction"),
        ("x /* c */ * y", "multiplication"),
        ("x /* c */ / y", "division"),
        ("x /* c */ % y", "modulo"),
        ("x /* c */ ++ y", "concatenation"),
        ("x /* c */ == y", "equality"),
        ("x /* c */ != y", "inequality"),
        ("x /* c */ < y", "less than"),
        ("x /* c */ > y", "greater than"),
        ("x /* c */ <= y", "less or equal"),
        ("x /* c */ >= y", "greater or equal"),
    ];

    for (expr, op_name) in operators {
        let source = format!("new result in {{ result!({}) }}", expr);
        let tree = parse_code(&source);
        let rope = Rope::from_str(&source);
        let doc_ir = parse_to_document_ir(&tree, &rope);

        // Verify comment is collected
        assert_eq!(doc_ir.comments.len(), 1,
            "Should collect comment for {} operator", op_name);

        // Verify IR conversion succeeds (no panic)
        assert!(matches!(&*doc_ir.root, RholangNode::New { .. }),
            "IR conversion should succeed for {} operator", op_name);
    }
}

#[test]
fn test_nested_operators_with_multiple_comments() {
    let source = r#"
new result in {
  result!(
    (a /* c1 */ + b) /* c2 */ * (c /* c3 */ - d)
  )
}
"#;
    let tree = parse_code(source);
    let rope = Rope::from_str(source);
    let doc_ir = parse_to_document_ir(&tree, &rope);

    // Verify all comments are collected
    assert_eq!(doc_ir.comments.len(), 3, "Should collect 3 comments from nested expression");

    // Verify IR conversion succeeds (no panic)
    assert!(matches!(&*doc_ir.root, RholangNode::New { .. }));
}

#[test]
fn test_comment_with_directive_preserved_in_channel() {
    // This test verifies that comments with directives remain accessible
    // via the comment channel after IR conversion
    let source = r#"
new result in {
  result!(x // #!directive
  + y)
}
"#;
    let tree = parse_code(source);
    let rope = Rope::from_str(source);
    let doc_ir = parse_to_document_ir(&tree, &rope);

    // Verify directive comment is in comment channel
    assert_eq!(doc_ir.comments.len(), 1);
    assert!(doc_ir.comments[0].text.contains("#!directive"),
        "Directive should be preserved in comment channel");

    // Verify semantic IR doesn't contain comment
    assert!(matches!(&*doc_ir.root, RholangNode::New { .. }));
}
