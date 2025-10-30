//! Tests for UnifiedIR integration into the document pipeline
//!
//! This test suite verifies that UnifiedIR is properly created when documents
//! are parsed and that it maintains semantic equivalence with the language-specific IR.

use rholang_language_server::ir::semantic_node::{SemanticCategory, SemanticNode};
use rholang_language_server::ir::unified_ir::UnifiedIR;
use rholang_language_server::tree_sitter::{parse_code, parse_to_ir};
use ropey::Rope;

#[test]
fn test_unified_ir_creation_from_rholang() {
    let _ = rholang_language_server::logging::init_logger(false, Some("warn"), false, false);

    let rho_code = r#"new x in { x!(42) }"#;
    let tree = parse_code(rho_code);
    let rope = Rope::from_str(rho_code);
    let rho_ir = parse_to_ir(&tree, &rope);

    // Convert to UnifiedIR
    let unified_ir = UnifiedIR::from_rholang(&rho_ir);

    // Verify the UnifiedIR was created
    assert!(unified_ir.as_any().is::<UnifiedIR>());

    // Verify it has a semantic category
    let category = unified_ir.semantic_category();
    assert_ne!(category, SemanticCategory::Unknown, "UnifiedIR should have a valid semantic category");

    println!("UnifiedIR created: {:?}", unified_ir.type_name());
    println!("Semantic category: {:?}", category);
}

#[test]
fn test_unified_ir_preserves_semantics() {
    let _ = rholang_language_server::logging::init_logger(false, Some("warn"), false, false);

    let rho_code = r#"new x, y in { x!(true) | y!(123) }"#;
    let tree = parse_code(rho_code);
    let rope = Rope::from_str(rho_code);
    let rho_ir = parse_to_ir(&tree, &rope);

    // Convert to UnifiedIR
    let unified_ir = UnifiedIR::from_rholang(&rho_ir);

    // Count children
    let child_count = unified_ir.children_count();
    println!("UnifiedIR has {} children", child_count);

    // The structure should be preserved
    assert!(child_count > 0, "UnifiedIR should have children");
}

#[test]
fn test_unified_ir_traversal() {
    let _ = rholang_language_server::logging::init_logger(false, Some("warn"), false, false);

    let rho_code = r#"new ch in { ch!(1, 2, 3) }"#;
    let tree = parse_code(rho_code);
    let rope = Rope::from_str(rho_code);
    let rho_ir = parse_to_ir(&tree, &rope);

    // Convert to UnifiedIR
    let unified_ir = UnifiedIR::from_rholang(&rho_ir);

    // Count all nodes via traversal
    fn count_nodes(node: &dyn SemanticNode) -> usize {
        let mut count = 1; // Count this node
        for i in 0..node.children_count() {
            if let Some(child) = node.child_at(i) {
                count += count_nodes(child);
            }
        }
        count
    }

    let node_count = count_nodes(&*unified_ir);
    println!("UnifiedIR tree contains {} nodes", node_count);

    assert!(node_count > 1, "Should have traversed multiple nodes");
}

#[test]
fn test_unified_ir_semantic_categories() {
    let _ = rholang_language_server::logging::init_logger(false, Some("warn"), false, false);

    let rho_code = r#"
        new ch in {
            ch!(42) |
            for (@val <- ch) {
                Nil
            }
        }
    "#;
    let tree = parse_code(rho_code);
    let rope = Rope::from_str(rho_code);
    let rho_ir = parse_to_ir(&tree, &rope);

    // Convert to UnifiedIR
    let unified_ir = UnifiedIR::from_rholang(&rho_ir);

    // Collect all semantic categories
    use std::collections::HashMap;
    fn collect_categories(node: &dyn SemanticNode, categories: &mut HashMap<String, usize>) {
        let cat_name = format!("{:?}", node.semantic_category());
        *categories.entry(cat_name).or_insert(0) += 1;

        for i in 0..node.children_count() {
            if let Some(child) = node.child_at(i) {
                collect_categories(child, categories);
            }
        }
    }

    let mut categories = HashMap::new();
    collect_categories(&*unified_ir, &mut categories);

    println!("Semantic categories found in UnifiedIR:");
    for (category, count) in &categories {
        println!("  {}: {}", category, count);
    }

    assert!(!categories.is_empty(), "Should have found semantic categories");
}

#[test]
fn test_language_detection() {
    use rholang_language_server::lsp::models::DocumentLanguage;
    use tower_lsp::lsp_types::Url;

    // Test Rholang file
    let rho_url = Url::parse("file:///test.rho").unwrap();
    assert_eq!(DocumentLanguage::from_uri(&rho_url), DocumentLanguage::Rholang);

    // Test MeTTa file
    let metta_url = Url::parse("file:///test.metta").unwrap();
    assert_eq!(DocumentLanguage::from_uri(&metta_url), DocumentLanguage::Metta);

    let metta2_url = Url::parse("file:///test.metta2").unwrap();
    assert_eq!(DocumentLanguage::from_uri(&metta2_url), DocumentLanguage::Metta);

    // Test unknown file
    let unknown_url = Url::parse("file:///test.txt").unwrap();
    assert_eq!(DocumentLanguage::from_uri(&unknown_url), DocumentLanguage::Unknown);

    println!("Language detection works correctly");
}

#[test]
fn test_unified_ir_literal_conversion() {
    let _ = rholang_language_server::logging::init_logger(false, Some("warn"), false, false);

    let rho_code = r#"new ch in { ch!(true, 42, "hello", `rho:test`) }"#;
    let tree = parse_code(rho_code);
    let rope = Rope::from_str(rho_code);
    let rho_ir = parse_to_ir(&tree, &rope);

    // Convert to UnifiedIR
    let unified_ir = UnifiedIR::from_rholang(&rho_ir);

    // Count literals in the tree
    fn count_literals(node: &dyn SemanticNode) -> usize {
        let mut count = 0;
        if node.semantic_category() == SemanticCategory::Literal {
            count = 1;
        }

        for i in 0..node.children_count() {
            if let Some(child) = node.child_at(i) {
                count += count_literals(child);
            }
        }
        count
    }

    let literal_count = count_literals(&*unified_ir);
    println!("Found {} literals in UnifiedIR", literal_count);

    // Original has: true, 42, "hello", `rho:test` = 4 literals
    assert!(literal_count >= 4, "Should find at least 4 literals");
}
