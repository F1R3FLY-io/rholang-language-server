//! Tests for comment parsing and comment channel functionality
//!
//! These tests verify that:
//! - Comments are collected from Tree-Sitter parse trees
//! - Comment nodes are correctly created with position tracking
//! - DocumentIR properly separates semantic tree from comments
//! - Directive parsing works on collected comments
//! - Doc comment detection works correctly

use rholang_language_server::parsers::rholang::{parse_code, parse_to_document_ir};
use ropey::Rope;

#[test]
fn test_collect_line_comments() {
    let source = r#"
// This is a line comment
Nil | Nil
// Another comment
"#;
    let tree = parse_code(source);
    let rope = Rope::from_str(source);
    let doc_ir = parse_to_document_ir(&tree, &rope);

    assert_eq!(doc_ir.comments.len(), 2, "Should collect 2 line comments");

    // Verify first comment
    assert_eq!(doc_ir.comments[0].text, "// This is a line comment");

    // Verify second comment
    assert_eq!(doc_ir.comments[1].text, "// Another comment");
}

#[test]
fn test_collect_block_comments() {
    let source = r#"
/* Block comment */
Nil | Nil
/* Another
   multiline
   block comment */
"#;
    let tree = parse_code(source);
    let rope = Rope::from_str(source);
    let doc_ir = parse_to_document_ir(&tree, &rope);

    assert_eq!(doc_ir.comments.len(), 2, "Should collect 2 block comments");

    // Verify comments are block type
    use rholang_language_server::ir::rholang_node::node_types::CommentKind;
    assert!(matches!(doc_ir.comments[0].kind, CommentKind::Block));
    assert!(matches!(doc_ir.comments[1].kind, CommentKind::Block));
}

#[test]
fn test_mixed_comments() {
    let source = r#"
// Line comment
/* Block comment */
Nil | Nil
/// Doc comment
"#;
    let tree = parse_code(source);
    let rope = Rope::from_str(source);
    let doc_ir = parse_to_document_ir(&tree, &rope);

    assert_eq!(doc_ir.comments.len(), 3, "Should collect 3 comments of mixed types");
}

#[test]
fn test_doc_comment_detection() {
    let source = r#"
/// This is a doc comment
contract foo() = { Nil }

// Regular comment
contract bar() = { Nil }

/** Block doc comment */
contract baz() = { Nil }
"#;
    let tree = parse_code(source);
    let rope = Rope::from_str(source);
    let doc_ir = parse_to_document_ir(&tree, &rope);

    assert_eq!(doc_ir.comments.len(), 3, "Should collect 3 comments");

    // Check doc comment flags
    assert!(doc_ir.comments[0].is_doc_comment, "First comment should be a doc comment (///)");
    assert!(!doc_ir.comments[1].is_doc_comment, "Second comment should not be a doc comment");
    assert!(doc_ir.comments[2].is_doc_comment, "Third comment should be a doc comment (/**)");
}

#[test]
fn test_doc_comment_text_extraction() {
    let source = r#"
/// This is documentation
contract foo() = { Nil }
"#;
    let tree = parse_code(source);
    let rope = Rope::from_str(source);
    let doc_ir = parse_to_document_ir(&tree, &rope);

    assert_eq!(doc_ir.comments.len(), 1);

    let doc_text = doc_ir.comments[0].doc_text();
    assert!(doc_text.is_some(), "Doc comment should have extractable text");
    assert_eq!(doc_text.unwrap(), "This is documentation");
}

#[test]
fn test_directive_comment_parsing() {
    let source = r#"
// @metta
new metta in {
    @"some code"!(metta)
}

/* @language: python */
new py in {
    @"print('hello')"!(py)
}
"#;
    let tree = parse_code(source);
    let rope = Rope::from_str(source);
    let doc_ir = parse_to_document_ir(&tree, &rope);

    // Get directive comments
    let directives = doc_ir.directive_comments();

    assert_eq!(directives.len(), 2, "Should find 2 directive comments");

    // Check directive languages
    assert_eq!(directives[0].1, "metta", "First directive should be 'metta'");
    assert_eq!(directives[1].1, "python", "Second directive should be 'python'");
}

#[test]
fn test_comment_position_tracking() {
    let source = r#"Nil // comment at position 4
Nil
// comment at new line"#;

    let tree = parse_code(source);
    let rope = Rope::from_str(source);
    let doc_ir = parse_to_document_ir(&tree, &rope);

    assert_eq!(doc_ir.comments.len(), 2, "Should collect 2 comments");

    // Verify position tracking works
    use rholang_language_server::ir::semantic_node::Position;
    let prev_end = Position { row: 0, column: 0, byte: 0 };

    let first_comment_pos = doc_ir.comments[0].absolute_position(prev_end);
    assert!(first_comment_pos.byte >= 4, "First comment should start at or after byte 4");

    // Second comment should be after first
    let first_comment_end = doc_ir.comments[0].absolute_end(first_comment_pos);
    let second_comment_pos = doc_ir.comments[1].absolute_position(first_comment_end);
    assert!(second_comment_pos.byte > first_comment_end.byte, "Second comment should start after first ends");
}

#[test]
fn test_semantic_tree_excludes_comments() {
    let source = r#"
// This comment should NOT be in the semantic tree
Nil | Nil
"#;
    let tree = parse_code(source);
    let rope = Rope::from_str(source);
    let doc_ir = parse_to_document_ir(&tree, &rope);

    // Comments should be in comment channel
    assert_eq!(doc_ir.comments.len(), 1);

    // Semantic tree should be clean (no comment nodes)
    // The root should be a Par node with two Nil children
    use rholang_language_server::ir::rholang_node::node_types::RholangNode;
    use rholang_language_server::ir::semantic_node::SemanticNode;

    match doc_ir.root.as_ref() {
        RholangNode::Par { processes: Some(procs), .. } => {
            // Should have 2 processes (both Nil), no comment nodes
            assert_eq!(procs.len(), 2, "Par should have 2 processes");
        }
        RholangNode::Par { left: Some(_), right: Some(_), .. } => {
            // Binary form also acceptable
            assert_eq!(doc_ir.root.children_count(), 2, "Par should have 2 children");
        }
        _ => panic!("Root should be a Par node"),
    }
}

#[test]
fn test_empty_source_no_comments() {
    let source = "Nil";
    let tree = parse_code(source);
    let rope = Rope::from_str(source);
    let doc_ir = parse_to_document_ir(&tree, &rope);

    assert_eq!(doc_ir.comments.len(), 0, "Source with no comments should have empty comment channel");
    assert!(!doc_ir.has_comments(), "has_comments() should return false");
}

#[test]
fn test_document_ir_helper_methods() {
    let source = r#"
/// Doc comment
Nil
// Regular comment
Nil
"#;
    let tree = parse_code(source);
    let rope = Rope::from_str(source);
    let doc_ir = parse_to_document_ir(&tree, &rope);

    // Test helper methods
    assert!(doc_ir.has_comments(), "Should have comments");
    assert_eq!(doc_ir.comment_count(), 2);
    assert!(doc_ir.has_doc_comments(), "Should have doc comments");

    let doc_comments: Vec<_> = doc_ir.doc_comments().collect();
    assert_eq!(doc_comments.len(), 1, "Should find 1 doc comment");
}

#[test]
fn test_comment_at_position_query() {
    let source = r#"
Nil // comment
Nil
"#;
    let tree = parse_code(source);
    let rope = Rope::from_str(source);
    let doc_ir = parse_to_document_ir(&tree, &rope);

    // Find the comment's position
    use rholang_language_server::ir::semantic_node::Position;
    let prev_end = Position { row: 0, column: 0, byte: 0 };
    let comment_start = doc_ir.comments[0].absolute_position(prev_end);
    let comment_end = doc_ir.comments[0].absolute_end(comment_start);

    // Query a position within the comment
    let mid_byte = (comment_start.byte + comment_end.byte) / 2;
    let query_pos = Position {
        row: comment_start.row,
        column: comment_start.column + 2,
        byte: mid_byte,
    };

    let found = doc_ir.comment_at_position(&query_pos);
    assert!(found.is_some(), "Should find comment at position within comment");
    assert_eq!(found.unwrap().text, doc_ir.comments[0].text);
}
