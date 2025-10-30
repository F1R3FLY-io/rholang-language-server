//! Tests for GenericVisitor trait and index-based traversal
//!
//! This test suite verifies that the new index-based traversal (`children_count()` and
//! `child_at()`) works correctly for all SemanticNode implementations, enabling
//! language-agnostic tree traversal via GenericVisitor.

use rholang_language_server::ir::rholang_node::RholangNode;
use rholang_language_server::ir::semantic_node::{GenericVisitor, SemanticCategory, SemanticNode};
use rholang_language_server::tree_sitter::{parse_code, parse_to_ir};
use ropey::Rope;
use std::collections::HashMap;

/// Simple visitor that counts nodes and tracks semantic categories
struct NodeCounter {
    /// Total number of nodes visited
    pub count: usize,
    /// Count of nodes by semantic category
    pub by_category: HashMap<SemanticCategory, usize>,
    /// Count of nodes by type name
    pub by_type: HashMap<String, usize>,
}

impl NodeCounter {
    fn new() -> Self {
        Self {
            count: 0,
            by_category: HashMap::new(),
            by_type: HashMap::new(),
        }
    }
}

impl GenericVisitor for NodeCounter {
    fn visit_node(&mut self, node: &dyn SemanticNode) {
        // Count total nodes
        self.count += 1;

        // Count by semantic category
        *self.by_category
            .entry(node.semantic_category())
            .or_insert(0) += 1;

        // Count by type name
        *self.by_type
            .entry(node.type_name().to_string())
            .or_insert(0) += 1;

        // Recurse to children using index-based traversal
        self.visit_children(node);
    }
}

/// Visitor that collects all literal values
struct LiteralCollector {
    pub literals: Vec<String>,
}

impl LiteralCollector {
    fn new() -> Self {
        Self {
            literals: Vec::new(),
        }
    }
}

impl GenericVisitor for LiteralCollector {
    fn visit_node(&mut self, node: &dyn SemanticNode) {
        // Check if this is a literal
        if node.semantic_category() == SemanticCategory::Literal {
            // Downcast to RholangNode to extract literal value
            if let Some(rho) = node.as_any().downcast_ref::<RholangNode>() {
                match rho {
                    RholangNode::BoolLiteral { value, .. } => {
                        self.literals.push(format!("Bool({})", value));
                    }
                    RholangNode::LongLiteral { value, .. } => {
                        self.literals.push(format!("Long({})", value));
                    }
                    RholangNode::StringLiteral { value, .. } => {
                        self.literals.push(format!("String(\"{}\")", value));
                    }
                    RholangNode::UriLiteral { value, .. } => {
                        self.literals.push(format!("Uri({})", value));
                    }
                    _ => {}
                }
            }
        }

        // Continue traversal to find more literals
        self.visit_children(node);
    }
}

/// Visitor that verifies tree structure integrity
struct TreeIntegrityChecker {
    /// Number of children visited per node
    pub children_counts: Vec<usize>,
    /// Detected any errors
    pub errors: Vec<String>,
}

impl TreeIntegrityChecker {
    fn new() -> Self {
        Self {
            children_counts: Vec::new(),
            errors: Vec::new(),
        }
    }
}

impl GenericVisitor for TreeIntegrityChecker {
    fn visit_node(&mut self, node: &dyn SemanticNode) {
        let count = node.children_count();
        self.children_counts.push(count);

        // Verify all indexed children are accessible
        for i in 0..count {
            match node.child_at(i) {
                Some(_) => {}
                None => {
                    self.errors.push(format!(
                        "Node {} claims {} children but child_at({}) returned None",
                        node.type_name(),
                        count,
                        i
                    ));
                }
            }
        }

        // Verify out-of-bounds returns None
        if node.child_at(count).is_some() {
            self.errors.push(format!(
                "Node {} claims {} children but child_at({}) returned Some (should be None)",
                node.type_name(),
                count,
                count
            ));
        }

        // Recurse
        self.visit_children(node);
    }
}

#[test]
fn test_generic_visitor_counts_rholang() {
    let _ = rholang_language_server::logging::init_logger(false, Some("warn"), false, false);

    let rho_code = r#"new x in { x!(1) | for (@val <- x) { Nil } }"#;
    let tree = parse_code(rho_code);
    let rope = Rope::from_str(rho_code);
    let ir = parse_to_ir(&tree, &rope);

    let mut counter = NodeCounter::new();
    counter.visit_node(&*ir);

    // Verify we counted nodes
    assert!(
        counter.count > 0,
        "Should have visited at least one node, got {}",
        counter.count
    );

    println!("Total nodes: {}", counter.count);
    println!("By category: {:?}", counter.by_category);
    println!("By type: {:?}", counter.by_type);

    // Verify we found expected semantic categories
    assert!(
        counter.by_category.contains_key(&SemanticCategory::Binding),
        "Should have found Binding (new declaration)"
    );
    // Note: Send is categorized as LanguageSpecific in Rholang (correct!)
    assert!(
        counter.by_category.contains_key(&SemanticCategory::LanguageSpecific),
        "Should have found LanguageSpecific (send, for/receive)"
    );
    assert!(
        counter.by_category.get(&SemanticCategory::LanguageSpecific).unwrap() >= &2,
        "Should have multiple LanguageSpecific nodes"
    );
}

#[test]
fn test_generic_visitor_finds_literals() {
    let _ = rholang_language_server::logging::init_logger(false, Some("warn"), false, false);

    let rho_code = r#"new ch in { ch!(true, 42, "hello", `rho:registry:lookup`) }"#;
    let tree = parse_code(rho_code);
    let rope = Rope::from_str(rho_code);
    let ir = parse_to_ir(&tree, &rope);

    let mut collector = LiteralCollector::new();
    collector.visit_node(&*ir);

    println!("Found literals: {:?}", collector.literals);

    // Verify we found expected literals
    assert!(
        collector.literals.iter().any(|l| l.contains("Bool(true)")),
        "Should have found boolean literal"
    );
    assert!(
        collector.literals.iter().any(|l| l.contains("Long(42)")),
        "Should have found integer literal"
    );
    assert!(
        collector.literals.iter().any(|l| l.contains("String")),
        "Should have found string literal"
    );
    assert!(
        collector.literals.iter().any(|l| l.contains("Uri")),
        "Should have found URI literal"
    );
}

#[test]
fn test_index_based_traversal_integrity() {
    let _ = rholang_language_server::logging::init_logger(false, Some("warn"), false, false);

    let rho_code = r#"
        new x, y, z in {
            x!(1) |
            y!(2) |
            for (@a <- x; @b <- y) {
                z!(a + b)
            } |
            match 42 {
                val => Nil
            }
        }
    "#;
    let tree = parse_code(rho_code);
    let rope = Rope::from_str(rho_code);
    let ir = parse_to_ir(&tree, &rope);

    let mut checker = TreeIntegrityChecker::new();
    checker.visit_node(&*ir);

    // Verify no errors detected
    for error in &checker.errors {
        eprintln!("Integrity error: {}", error);
    }
    assert!(
        checker.errors.is_empty(),
        "Found {} integrity errors",
        checker.errors.len()
    );

    // Verify we traversed multiple levels
    assert!(
        checker.children_counts.len() > 10,
        "Should have visited multiple nodes"
    );

    // Verify some nodes have children
    let nodes_with_children = checker.children_counts.iter().filter(|&&c| c > 0).count();
    assert!(
        nodes_with_children > 0,
        "Some nodes should have children"
    );

    println!(
        "Visited {} nodes, {} with children",
        checker.children_counts.len(),
        nodes_with_children
    );
}

#[test]
fn test_generic_visitor_nested_structures() {
    let _ = rholang_language_server::logging::init_logger(false, Some("warn"), false, false);

    // Test deeply nested structure
    let rho_code = r#"
        new outer in {
            new inner in {
                new deepest in {
                    deepest!([1, 2, [3, 4, [5]]])
                }
            }
        }
    "#;
    let tree = parse_code(rho_code);
    let rope = Rope::from_str(rho_code);
    let ir = parse_to_ir(&tree, &rope);

    let mut counter = NodeCounter::new();
    counter.visit_node(&*ir);

    // Verify we can traverse deeply nested structures
    assert!(counter.count > 5, "Should traverse nested structure");

    // Verify we found collections (lists)
    assert!(
        counter
            .by_category
            .contains_key(&SemanticCategory::Collection),
        "Should have found Collection nodes (lists)"
    );
}

#[test]
fn test_generic_visitor_empty_structures() {
    let _ = rholang_language_server::logging::init_logger(false, Some("warn"), false, false);

    let rho_code = r#"Nil"#;
    let tree = parse_code(rho_code);
    let rope = Rope::from_str(rho_code);
    let ir = parse_to_ir(&tree, &rope);

    let mut counter = NodeCounter::new();
    counter.visit_node(&*ir);

    // Even Nil should be counted
    assert_eq!(counter.count, 1, "Should visit Nil node");
    assert!(
        counter.by_type.contains_key("Rholang::Nil"),
        "Should identify Nil node (actual type name: {:?})",
        counter.by_type.keys()
    );
}

#[test]
fn test_generic_visitor_complex_pattern_matching() {
    let _ = rholang_language_server::logging::init_logger(false, Some("warn"), false, false);

    let rho_code = r#"
        match [1, 2, 3] {
            [a, b, ...rest] => a + b
            [] => 0
        }
    "#;
    let tree = parse_code(rho_code);
    let rope = Rope::from_str(rho_code);
    let ir = parse_to_ir(&tree, &rope);

    let mut counter = NodeCounter::new();
    counter.visit_node(&*ir);

    // Verify match structure is traversed
    assert!(
        counter.by_category.contains_key(&SemanticCategory::Match),
        "Should have found Match node"
    );
    assert!(
        counter.by_category.contains_key(&SemanticCategory::Collection),
        "Should have found Collection (list)"
    );

    println!("Match structure nodes: {}", counter.count);
}

#[test]
fn test_semantic_category_distribution() {
    let _ = rholang_language_server::logging::init_logger(false, Some("warn"), false, false);

    let rho_code = r#"
        new x, y in {
            contract @"add"(@a, @b, return) = {
                return!(a + b)
            } |
            @"add"!(1, 2, *x) |
            for (@result <- x) {
                y!(result)
            }
        }
    "#;
    let tree = parse_code(rho_code);
    let rope = Rope::from_str(rho_code);
    let ir = parse_to_ir(&tree, &rope);

    let mut counter = NodeCounter::new();
    counter.visit_node(&*ir);

    println!("\n=== Semantic Category Distribution ===");
    let mut categories: Vec<_> = counter.by_category.iter().collect();
    categories.sort_by_key(|(_, count)| std::cmp::Reverse(**count));

    for (category, count) in categories {
        println!("  {:?}: {}", category, count);
    }

    // Verify we have a diverse set of categories
    assert!(
        counter.by_category.len() >= 4,
        "Should have at least 4 different semantic categories"
    );

    // Verify specific categories exist
    assert!(counter.by_category.contains_key(&SemanticCategory::Binding));
    assert!(counter.by_category.contains_key(&SemanticCategory::Invocation));
    assert!(counter.by_category.contains_key(&SemanticCategory::Variable));
    assert!(counter.by_category.contains_key(&SemanticCategory::Literal));
}

#[test]
fn test_visitor_doesnt_loop_infinitely() {
    let _ = rholang_language_server::logging::init_logger(false, Some("warn"), false, false);

    // Test with potentially problematic circular-looking structure
    let rho_code = r#"
        new loop in {
            contract loop(@n) = {
                if (n > 0) {
                    loop!(n - 1)
                } else {
                    Nil
                }
            } |
            loop!(10)
        }
    "#;
    let tree = parse_code(rho_code);
    let rope = Rope::from_str(rho_code);
    let ir = parse_to_ir(&tree, &rope);

    let mut counter = NodeCounter::new();
    counter.visit_node(&*ir);

    // If we get here without hanging, traversal terminated correctly
    assert!(counter.count > 0, "Should have visited nodes");
    assert!(
        counter.count < 1000,
        "Should not have visited excessive nodes (possible infinite loop)"
    );

    println!("Visited {} nodes (no infinite loop)", counter.count);
}
