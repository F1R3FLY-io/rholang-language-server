//! Integration test for MeTTa pattern matching and go-to-definition

use rholang_language_server::parsers::MettaParser;
use rholang_language_server::ir::transforms::metta_symbol_table_builder::MettaSymbolTableBuilder;
use rholang_language_server::ir::metta_node::MettaNode;
use std::sync::Arc;
use tower_lsp::lsp_types::Url;

#[test]
fn test_pattern_matching_simple_function() {
    // Test code with a simple function definition and call
    let code = r#"
(= (add $x $y)
   (+ $x $y))

(= (result)
   (add 1 2))
"#;

    let mut parser = MettaParser::new().expect("Failed to create parser");
    let nodes = parser.parse_to_ir(code).expect("Failed to parse");

    let test_uri = Url::parse("file:///test.metta").unwrap();
    let builder = MettaSymbolTableBuilder::new_simple(test_uri);
    let table = builder.build(&nodes);

    println!("\n=== Symbol Table ===");
    println!("Total scopes: {}", table.scopes.len());
    println!("Total symbols: {}", table.all_occurrences.len());

    // Check that "add" was indexed as a definition
    let add_defs = table.pattern_matcher.get_definitions_by_name("add");
    assert_eq!(add_defs.len(), 1, "Should have exactly 1 'add' definition");
    assert_eq!(add_defs[0].name, "add");
    assert_eq!(add_defs[0].arity, 2, "'add' should have arity 2");

    println!("\n=== Add Definition ===");
    println!("Name: {}", add_defs[0].name);
    println!("Arity: {}", add_defs[0].arity);
    println!("Location: {:?}", add_defs[0].location);
}

#[test]
fn test_pattern_matching_multiple_definitions() {
    // Test code with multiple definitions of the same function (different arities)
    let code = r#"
(= (foo $x)
   $x)

(= (foo $x $y)
   (+ $x $y))

(= (foo $x $y $z)
   (+ $x (+ $y $z)))

(= (result1)
   (foo 1))

(= (result2)
   (foo 1 2))

(= (result3)
   (foo 1 2 3))
"#;

    let mut parser = MettaParser::new().expect("Failed to create parser");
    let nodes = parser.parse_to_ir(code).expect("Failed to parse");

    let test_uri = Url::parse("file:///test.metta").unwrap();
    let builder = MettaSymbolTableBuilder::new_simple(test_uri);
    let table = builder.build(&nodes);

    println!("\n=== Symbol Table ===");
    println!("Total scopes: {}", table.scopes.len());
    println!("Total symbols: {}", table.all_occurrences.len());

    // Check that all "foo" definitions were indexed
    let foo_defs = table.pattern_matcher.get_definitions_by_name("foo");
    assert_eq!(foo_defs.len(), 3, "Should have exactly 3 'foo' definitions");

    // Check arities
    let arities: Vec<usize> = foo_defs.iter().map(|d| d.arity).collect();
    assert!(arities.contains(&1), "Should have 'foo' with arity 1");
    assert!(arities.contains(&2), "Should have 'foo' with arity 2");
    assert!(arities.contains(&3), "Should have 'foo' with arity 3");

    println!("\n=== Foo Definitions ===");
    for def in &foo_defs {
        println!("Name: {}, Arity: {}, Location: {:?}", def.name, def.arity, def.location);
    }
}

#[test]
fn test_pattern_matching_find_call() {
    // Test finding a function call pattern
    let code = r#"
(= (is_connected $from $to)
   (match & self (connected $from $to) true))

(= (test)
   (is_connected room_a room_b))
"#;

    let mut parser = MettaParser::new().expect("Failed to create parser");
    let nodes = parser.parse_to_ir(code).expect("Failed to parse");

    let test_uri = Url::parse("file:///test.metta").unwrap();
    let builder = MettaSymbolTableBuilder::new_simple(test_uri);
    let table = builder.build(&nodes);

    // Check that "is_connected" was indexed
    let defs = table.pattern_matcher.get_definitions_by_name("is_connected");
    assert_eq!(defs.len(), 1, "Should have exactly 1 'is_connected' definition");
    assert_eq!(defs[0].arity, 2, "'is_connected' should have arity 2");

    println!("\n=== is_connected Definition ===");
    println!("Name: {}", defs[0].name);
    println!("Arity: {}", defs[0].arity);
    println!("Location: {:?}", defs[0].location);

    // Create a call pattern to match against
    // This simulates what happens when clicking on "is_connected" in "(is_connected room_a room_b)"
    use rholang_language_server::ir::semantic_node::NodeBase;
    use rholang_language_server::ir::rholang_node::RelativePosition;

    fn test_base() -> NodeBase {
        NodeBase::new_simple(
            RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            0,
            0,
            0,
        )
    }

    let call_pattern = MettaNode::SExpr {
        base: test_base(),
        elements: vec![
            Arc::new(MettaNode::Atom {
                base: test_base(),
                name: "is_connected".to_string(),
                metadata: None,
            }),
            Arc::new(MettaNode::Atom {
                base: test_base(),
                name: "room_a".to_string(),
                metadata: None,
            }),
            Arc::new(MettaNode::Atom {
                base: test_base(),
                name: "room_b".to_string(),
                metadata: None,
            }),
        ],
        metadata: None,
    };

    // Find matching definitions
    let matches = table.find_function_definitions(&call_pattern);
    assert_eq!(matches.len(), 1, "Should find 1 matching definition for 'is_connected' call");

    println!("\n=== Matching Definitions ===");
    for loc in &matches {
        println!("Location: {:?}", loc);
    }
}

#[test]
fn test_arity_filtering() {
    // Test that arity filtering works correctly
    let code = r#"
(= (func $x)
   $x)

(= (func $x $y)
   (+ $x $y))

(= (result)
   (func 1 2))
"#;

    let mut parser = MettaParser::new().expect("Failed to create parser");
    let nodes = parser.parse_to_ir(code).expect("Failed to parse");

    let test_uri = Url::parse("file:///test.metta").unwrap();
    let builder = MettaSymbolTableBuilder::new_simple(test_uri);
    let table = builder.build(&nodes);

    // Create call pattern with arity 2
    use rholang_language_server::ir::semantic_node::NodeBase;
    use rholang_language_server::ir::rholang_node::RelativePosition;

    fn test_base() -> NodeBase {
        NodeBase::new_simple(
            RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            0,
            0,
            0,
        )
    }

    let call_pattern = MettaNode::SExpr {
        base: test_base(),
        elements: vec![
            Arc::new(MettaNode::Atom {
                base: test_base(),
                name: "func".to_string(),
                metadata: None,
            }),
            Arc::new(MettaNode::Atom {
                base: test_base(),
                name: "1".to_string(),
                metadata: None,
            }),
            Arc::new(MettaNode::Atom {
                base: test_base(),
                name: "2".to_string(),
                metadata: None,
            }),
        ],
        metadata: None,
    };

    // Find matching definitions - should only match the arity-2 version
    let matches = table.find_function_definitions(&call_pattern);
    assert_eq!(matches.len(), 1, "Should find exactly 1 matching definition (arity 2)");

    // Verify it's the arity-2 definition
    let func_defs = table.pattern_matcher.get_definitions_by_name("func");
    let arity_2_def = func_defs.iter().find(|d| d.arity == 2).unwrap();
    assert_eq!(matches[0].range, arity_2_def.location.range, "Should match the arity-2 definition");
}
