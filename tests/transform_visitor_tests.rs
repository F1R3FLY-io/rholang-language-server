//! Tests for TransformVisitor trait and immutable transformations
//!
//! This test suite verifies that TransformVisitor can perform immutable
//! transformations on IR trees using the index-based traversal system.

use rholang_language_server::ir::rholang_node::{BinOperator, RholangNode};
use rholang_language_server::ir::semantic_node::{NodeBase, Position, RelativePosition, SemanticNode, TransformVisitor};
use rholang_language_server::tree_sitter::{parse_code, parse_to_ir};
use ropey::Rope;
use std::sync::Arc;

/// Identity transformation - returns all nodes unchanged
struct IdentityTransform;

impl TransformVisitor for IdentityTransform {
    // Use default implementation (returns None, triggering recursive transformation)
}

/// Transforms all integer literals by negating them
struct NegateIntegers;

impl TransformVisitor for NegateIntegers {
    fn transform_node(&mut self, node: &dyn SemanticNode) -> Option<Arc<dyn SemanticNode>> {
        if let Some(rho) = node.as_any().downcast_ref::<RholangNode>() {
            if let RholangNode::LongLiteral { value, base, metadata } = rho {
                // Create negated literal
                let negated = RholangNode::LongLiteral {
                    base: base.clone(),
                    value: -value,
                    metadata: metadata.clone(),
                };
                return Some(Arc::new(negated) as Arc<dyn SemanticNode>);
            }
        }
        None  // Not an integer literal, use default transformation
    }
}

/// Replaces all boolean true literals with false
struct FlipBooleans;

impl TransformVisitor for FlipBooleans {
    fn transform_node(&mut self, node: &dyn SemanticNode) -> Option<Arc<dyn SemanticNode>> {
        if let Some(rho) = node.as_any().downcast_ref::<RholangNode>() {
            if let RholangNode::BoolLiteral { value, base, metadata } = rho {
                if *value {
                    // Flip true to false
                    let flipped = RholangNode::BoolLiteral {
                        base: base.clone(),
                        value: false,
                        metadata: metadata.clone(),
                    };
                    return Some(Arc::new(flipped) as Arc<dyn SemanticNode>);
                }
            }
        }
        None
    }
}

/// Counts how many nodes were transformed
struct CountingTransform {
    transform_count: usize,
}

impl CountingTransform {
    fn new() -> Self {
        Self { transform_count: 0 }
    }
}

impl TransformVisitor for CountingTransform {
    fn transform_node(&mut self, node: &dyn SemanticNode) -> Option<Arc<dyn SemanticNode>> {
        // Transform all integer literals to 0
        if let Some(rho) = node.as_any().downcast_ref::<RholangNode>() {
            if let RholangNode::LongLiteral { base, metadata, .. } = rho {
                self.transform_count += 1;
                let zero = RholangNode::LongLiteral {
                    base: base.clone(),
                    value: 0,
                    metadata: metadata.clone(),
                };
                return Some(Arc::new(zero) as Arc<dyn SemanticNode>);
            }
        }
        None
    }
}

#[test]
fn test_identity_transformation() {
    let _ = rholang_language_server::logging::init_logger(false, Some("warn"), false);

    let rho_code = r#"new x in { x!(42) }"#;
    let tree = parse_code(rho_code);
    let rope = Rope::from_str(rho_code);
    let ir = parse_to_ir(&tree, &rope);

    let mut transform = IdentityTransform;
    let transformed = transform.transform_with_children(&*ir);

    // Identity transform should return structurally same tree
    // (though Arc pointers may differ due to cloning)
    assert!(transformed.as_any().is::<RholangNode>());

    println!("Identity transformation complete");
}

#[test]
fn test_negate_integers() {
    let _ = rholang_language_server::logging::init_logger(false, Some("warn"), false);

    let rho_code = r#"new x in { x!(5) | x!(10) }"#;
    let tree = parse_code(rho_code);
    let rope = Rope::from_str(rho_code);
    let ir = parse_to_ir(&tree, &rope);

    let mut transform = NegateIntegers;
    let transformed = transform.transform_with_children(&*ir);

    // Count integers in transformed tree
    let mut count_positives = 0;
    let mut count_negatives = 0;

    fn count_integers(node: &dyn SemanticNode, pos: &mut usize, neg: &mut usize) {
        if let Some(rho) = node.as_any().downcast_ref::<RholangNode>() {
            if let RholangNode::LongLiteral { value, .. } = rho {
                if *value > 0 {
                    *pos += 1;
                } else if *value < 0 {
                    *neg += 1;
                }
            }
        }

        // Recurse to children
        for i in 0..node.children_count() {
            if let Some(child) = node.child_at(i) {
                count_integers(child, pos, neg);
            }
        }
    }

    count_integers(&*transformed, &mut count_positives, &mut count_negatives);

    println!("After negation: {} positive, {} negative", count_positives, count_negatives);

    // Original had 2 positive integers (5, 10)
    // After negation, should have 2 negative integers (-5, -10)
    assert_eq!(count_positives, 0, "Should have no positive integers after negation");
    assert_eq!(count_negatives, 2, "Should have 2 negative integers after negation");
}

#[test]
fn test_flip_booleans() {
    let _ = rholang_language_server::logging::init_logger(false, Some("warn"), false);

    let rho_code = r#"new x in { x!(true, false, true) }"#;
    let tree = parse_code(rho_code);
    let rope = Rope::from_str(rho_code);
    let ir = parse_to_ir(&tree, &rope);

    let mut transform = FlipBooleans;
    let transformed = transform.transform_with_children(&*ir);

    // Count booleans in transformed tree
    let mut count_true = 0;
    let mut count_false = 0;

    fn count_booleans(node: &dyn SemanticNode, t: &mut usize, f: &mut usize) {
        if let Some(rho) = node.as_any().downcast_ref::<RholangNode>() {
            if let RholangNode::BoolLiteral { value, .. } = rho {
                if *value {
                    *t += 1;
                } else {
                    *f += 1;
                }
            }
        }

        // Recurse
        for i in 0..node.children_count() {
            if let Some(child) = node.child_at(i) {
                count_booleans(child, t, f);
            }
        }
    }

    count_booleans(&*transformed, &mut count_true, &mut count_false);

    println!("After flipping: {} true, {} false", count_true, count_false);

    // Original: 2 true, 1 false
    // After flipping true→false: 0 true, 3 false
    assert_eq!(count_true, 0, "All true values should be flipped to false");
    assert_eq!(count_false, 3, "Should have 3 false values total");
}

#[test]
fn test_counting_transform() {
    let _ = rholang_language_server::logging::init_logger(false, Some("warn"), false);

    let rho_code = r#"
        new x, y in {
            x!(1, 2, 3) |
            y!(10, 20)
        }
    "#;
    let tree = parse_code(rho_code);
    let rope = Rope::from_str(rho_code);
    let ir = parse_to_ir(&tree, &rope);

    let mut transform = CountingTransform::new();
    let transformed = transform.transform_with_children(&*ir);

    println!("Transformed {} integer literals", transform.transform_count);

    // Should have transformed 5 integers (1, 2, 3, 10, 20)
    assert_eq!(transform.transform_count, 5, "Should transform exactly 5 integers");

    // Verify all are now 0
    let mut zero_count = 0;

    fn count_zeros(node: &dyn SemanticNode, count: &mut usize) {
        if let Some(rho) = node.as_any().downcast_ref::<RholangNode>() {
            if let RholangNode::LongLiteral { value: 0, .. } = rho {
                *count += 1;
            }
        }

        for i in 0..node.children_count() {
            if let Some(child) = node.child_at(i) {
                count_zeros(child, count);
            }
        }
    }

    count_zeros(&*transformed, &mut zero_count);
    assert_eq!(zero_count, 5, "All integers should be transformed to 0");
}

#[test]
fn test_transform_preserves_structure() {
    let _ = rholang_language_server::logging::init_logger(false, Some("warn"), false);

    let rho_code = r#"new x in { for (@val <- x) { Nil } }"#;
    let tree = parse_code(rho_code);
    let rope = Rope::from_str(rho_code);
    let ir = parse_to_ir(&tree, &rope);

    // Transform with identity (no changes)
    let mut transform = IdentityTransform;
    let transformed = transform.transform_with_children(&*ir);

    // Verify structure is preserved
    fn verify_structure(node: &dyn SemanticNode) {
        // Should still have valid base and children
        let _ = node.base();
        let child_count = node.children_count();

        for i in 0..child_count {
            if let Some(child) = node.child_at(i) {
                verify_structure(child);
            }
        }
    }

    verify_structure(&*transformed);
    println!("Structure preserved after transformation");
}

#[test]
fn test_transform_nested_structures() {
    let _ = rholang_language_server::logging::init_logger(false, Some("warn"), false);

    let rho_code = r#"[1, [2, [3, [4]]]]"#;
    let tree = parse_code(rho_code);
    let rope = Rope::from_str(rho_code);
    let ir = parse_to_ir(&tree, &rope);

    let mut transform = NegateIntegers;
    let transformed = transform.transform_with_children(&*ir);

    // Count negatives in deeply nested structure
    let mut count = 0;

    fn count_negatives(node: &dyn SemanticNode, count: &mut usize) {
        if let Some(rho) = node.as_any().downcast_ref::<RholangNode>() {
            if let RholangNode::LongLiteral { value, .. } = rho {
                if *value < 0 {
                    *count += 1;
                }
            }
        }

        for i in 0..node.children_count() {
            if let Some(child) = node.child_at(i) {
                count_negatives(child, count);
            }
        }
    }

    count_negatives(&*transformed, &mut count);

    println!("Found {} negative integers in nested structure", count);
    assert_eq!(count, 4, "Should negate all 4 integers in nested lists");
}

#[test]
fn test_transform_empty_code() {
    let _ = rholang_language_server::logging::init_logger(false, Some("warn"), false);

    let rho_code = r#"Nil"#;
    let tree = parse_code(rho_code);
    let rope = Rope::from_str(rho_code);
    let ir = parse_to_ir(&tree, &rope);

    let mut transform = NegateIntegers;
    let transformed = transform.transform_with_children(&*ir);

    // Should handle Nil without issues
    assert!(transformed.as_any().is::<RholangNode>());
    println!("Successfully transformed Nil");
}

#[test]
fn test_transform_mixed_types() {
    let _ = rholang_language_server::logging::init_logger(false, Some("warn"), false);

    let rho_code = r#"
        new ch in {
            ch!(42, true, "hello", false, 100, `rho:test`)
        }
    "#;
    let tree = parse_code(rho_code);
    let rope = Rope::from_str(rho_code);
    let ir = parse_to_ir(&tree, &rope);

    // Apply both transformations sequentially
    let mut negate = NegateIntegers;
    let step1 = negate.transform_with_children(&*ir);

    let mut flip = FlipBooleans;
    let step2 = flip.transform_with_children(&*step1);

    // Count final results
    let mut neg_count = 0;
    let mut false_count = 0;

    fn count_transformed(node: &dyn SemanticNode, neg: &mut usize, f: &mut usize) {
        if let Some(rho) = node.as_any().downcast_ref::<RholangNode>() {
            match rho {
                RholangNode::LongLiteral { value, .. } if *value < 0 => *neg += 1,
                RholangNode::BoolLiteral { value: false, .. } => *f += 1,
                _ => {}
            }
        }

        for i in 0..node.children_count() {
            if let Some(child) = node.child_at(i) {
                count_transformed(child, neg, f);
            }
        }
    }

    count_transformed(&*step2, &mut neg_count, &mut false_count);

    println!("After both transforms: {} negatives, {} false", neg_count, false_count);

    // Original: 2 integers (42, 100), 2 booleans (true, false)
    // After negations: 2 negative integers
    // After flips: 2 false booleans (true→false, false unchanged)
    assert_eq!(neg_count, 2, "Should have 2 negative integers");
    assert_eq!(false_count, 2, "Should have 2 false booleans");
}
