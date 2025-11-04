//! Documentation Attacher Transform
//!
//! This transform attaches documentation comments to their associated declarations
//! by leveraging the comment channel in DocumentIR.
//!
//! **Phase 3**: Implements documentation extraction and attachment to IR nodes.
//!
//! # Architecture
//!
//! - Uses `DocumentIR::doc_comment_before()` to find doc comments
//! - Attaches doc text as metadata to declaration nodes
//! - Works with contracts, new bindings, and let bindings
//!
//! # Metadata Key
//!
//! Documentation is stored in node metadata with the key `"documentation"`.
//! Access it with:
//! ```rust,ignore
//! if let Some(metadata) = node.metadata() {
//!     if let Some(doc_any) = metadata.get("documentation") {
//!         if let Some(doc_text) = doc_any.downcast_ref::<String>() {
//!             println!("Documentation: {}", doc_text);
//!         }
//!     }
//! }
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::any::Any;
use tracing::trace;

use crate::ir::rholang_node::{RholangNode, RholangNodeVector, NodeBase, Metadata};
use crate::ir::rholang_node::position_tracking::compute_absolute_positions;
use crate::ir::{DocumentIR, semantic_node::{SemanticNode, Position}};
use crate::ir::visitor::Visitor;
use crate::ir::structured_documentation::StructuredDocumentation;

/// Metadata key for attached documentation
pub const DOC_METADATA_KEY: &str = "documentation";

/// Attaches documentation comments to declaration nodes
///
/// This visitor traverses the IR tree and for each declaration node,
/// checks if there's a documentation comment immediately before it.
/// If found, the doc text is attached as metadata.
///
/// # Example
///
/// ```rust,ignore
/// let attacher = DocumentationAttacher::new(document_ir.clone());
/// let documented_ir = attacher.visit_node(&document_ir.root);
/// ```
pub struct DocumentationAttacher {
    /// Reference to DocumentIR for accessing comment channel
    document_ir: Arc<DocumentIR>,
    /// Precomputed absolute positions for all nodes (node pointer -> (start, end))
    positions: HashMap<usize, (Position, Position)>,
}

impl DocumentationAttacher {
    /// Create a new DocumentationAttacher with access to the comment channel
    ///
    /// # Arguments
    /// * `document_ir` - The DocumentIR containing both IR tree and comments
    pub fn new(document_ir: Arc<DocumentIR>) -> Self {
        // Precompute positions for all nodes
        let positions = compute_absolute_positions(&document_ir.root);

        Self {
            document_ir,
            positions,
        }
    }

    /// Phase 7: Extract and parse structured documentation at a position
    ///
    /// Gets all consecutive doc comments before the position and parses them
    /// into a StructuredDocumentation object with support for @param, @return, etc.
    ///
    /// # Arguments
    /// * `node_pos` - The position to extract documentation for
    ///
    /// # Returns
    /// Parsed StructuredDocumentation if doc comments exist, None otherwise
    fn extract_structured_documentation(&self, node_pos: &Position) -> Option<StructuredDocumentation> {
        // Phase 7: Get ALL consecutive doc comments (not just the last one)
        let doc_comments = self.document_ir.doc_comments_before(node_pos);

        if doc_comments.is_empty() {
            return None;
        }

        // Extract cleaned text from each comment (need to collect Strings first)
        let doc_text_strings: Vec<String> = doc_comments
            .iter()
            .filter_map(|comment| comment.doc_text())
            .collect();

        if doc_text_strings.is_empty() {
            return None;
        }

        // Convert to &str for parsing
        let doc_texts: Vec<&str> = doc_text_strings.iter().map(|s| s.as_str()).collect();

        // Parse into structured documentation
        let structured = StructuredDocumentation::parse(doc_texts.into_iter());

        trace!(
            "Extracted structured documentation at {:?}: summary length = {}, params = {}, examples = {}",
            node_pos,
            structured.summary.len(),
            structured.params.len(),
            structured.examples.len()
        );

        Some(structured)
    }

}

impl Visitor for DocumentationAttacher {
    /// Override visit_contract to attach documentation to contracts (Phase 7: with structured docs)
    fn visit_contract(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        name: &Arc<RholangNode>,
        formals: &RholangNodeVector,
        formals_remainder: &Option<Arc<RholangNode>>,
        proc: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        // Phase 7: Extract structured documentation using new method
        let structured_doc = {
            let node_ptr = Arc::as_ptr(node) as usize;
            if let Some((node_pos, _node_end)) = self.positions.get(&node_ptr) {
                self.extract_structured_documentation(node_pos)
            } else {
                None
            }
        };

        // Visit children
        let new_name = self.visit_node(name);
        let new_proc = self.visit_node(proc);

        // Check if children changed or if we need to attach documentation
        let children_changed = !Arc::ptr_eq(name, &new_name) || !Arc::ptr_eq(proc, &new_proc);
        let need_new_node = children_changed || structured_doc.is_some();

        if need_new_node {
            // Prepare metadata with documentation if needed
            let new_metadata = if let Some(structured) = structured_doc {
                let mut meta = if let Some(existing_meta) = metadata {
                    (**existing_meta).clone()
                } else {
                    HashMap::new()
                };

                // Phase 7: Store StructuredDocumentation instead of plain String
                // This enables richer doc display with @param, @return, @example, etc.
                meta.insert(
                    DOC_METADATA_KEY.to_string(),
                    Arc::new(structured) as Arc<dyn std::any::Any + Send + Sync>,
                );
                Some(Arc::new(meta))
            } else {
                metadata.clone()
            };

            Arc::new(RholangNode::Contract {
                base: base.clone(),
                name: new_name,
                formals: formals.clone(),
                formals_remainder: formals_remainder.clone(),
                proc: new_proc,
                metadata: new_metadata,
            })
        } else {
            Arc::clone(node)
        }
    }

    /// Override visit_new to attach documentation to new bindings (Phase 7: with structured docs)
    fn visit_new(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        decls: &RholangNodeVector,
        proc: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        // Phase 7: Extract structured documentation using new method
        let structured_doc = {
            let node_ptr = Arc::as_ptr(node) as usize;
            if let Some((node_pos, _node_end)) = self.positions.get(&node_ptr) {
                self.extract_structured_documentation(node_pos)
            } else {
                None
            }
        };

        // Visit children
        let new_proc = self.visit_node(proc);

        // Check if children changed or if we need to attach documentation
        let children_changed = !Arc::ptr_eq(proc, &new_proc);
        let need_new_node = children_changed || structured_doc.is_some();

        if need_new_node {
            // Prepare metadata with documentation if needed
            let new_metadata = if let Some(structured) = structured_doc {
                let mut meta = if let Some(existing_meta) = metadata {
                    (**existing_meta).clone()
                } else {
                    HashMap::new()
                };

                // Phase 7: Store StructuredDocumentation instead of plain String
                meta.insert(
                    DOC_METADATA_KEY.to_string(),
                    Arc::new(structured) as Arc<dyn std::any::Any + Send + Sync>,
                );
                Some(Arc::new(meta))
            } else {
                metadata.clone()
            };

            Arc::new(RholangNode::New {
                base: base.clone(),
                decls: decls.clone(),
                proc: new_proc,
                metadata: new_metadata,
            })
        } else {
            Arc::clone(node)
        }
    }

    /// Override visit_let to attach documentation to let bindings (Phase 7: with structured docs)
    fn visit_let(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        decls: &RholangNodeVector,
        proc: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        // Phase 7: Extract structured documentation using new method
        let structured_doc = {
            let node_ptr = Arc::as_ptr(node) as usize;
            if let Some((node_pos, _node_end)) = self.positions.get(&node_ptr) {
                self.extract_structured_documentation(node_pos)
            } else {
                None
            }
        };

        // Visit children
        let new_proc = self.visit_node(proc);

        // Check if children changed or if we need to attach documentation
        let children_changed = !Arc::ptr_eq(proc, &new_proc);
        let need_new_node = children_changed || structured_doc.is_some();

        if need_new_node {
            // Prepare metadata with documentation if needed
            let new_metadata = if let Some(structured) = structured_doc {
                let mut meta = if let Some(existing_meta) = metadata {
                    (**existing_meta).clone()
                } else {
                    HashMap::new()
                };

                // Phase 7: Store StructuredDocumentation instead of plain String
                meta.insert(
                    DOC_METADATA_KEY.to_string(),
                    Arc::new(structured) as Arc<dyn std::any::Any + Send + Sync>,
                );
                Some(Arc::new(meta))
            } else {
                metadata.clone()
            };

            Arc::new(RholangNode::Let {
                base: base.clone(),
                decls: decls.clone(),
                proc: new_proc,
                metadata: new_metadata,
            })
        } else {
            Arc::clone(node)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree_sitter::{parse_code, parse_to_document_ir};
    use ropey::Rope;

    #[test]
    fn test_attach_documentation_to_contract() {
        let source = r#"
/// This is a contract that does something
contract foo(@x) = {
    Nil
}
"#;
        let tree = parse_code(source);
        let rope = Rope::from_str(source);
        let document_ir = parse_to_document_ir(&tree, &rope);

        let attacher = DocumentationAttacher::new(document_ir.clone());
        let documented_ir = attacher.visit_node(&document_ir.root);

        // Find the contract node
        if let RholangNode::Contract { metadata, .. } = documented_ir.as_ref() {
            assert!(metadata.is_some(), "Contract should have metadata");
            let meta = metadata.as_ref().unwrap();
            assert!(
                meta.contains_key(DOC_METADATA_KEY),
                "Metadata should contain documentation"
            );

            // Phase 7: Documentation is now StructuredDocumentation instead of String
            let structured_doc = meta
                .get(DOC_METADATA_KEY)
                .unwrap()
                .downcast_ref::<StructuredDocumentation>()
                .expect("Documentation should be StructuredDocumentation");
            assert_eq!(structured_doc.summary, "This is a contract that does something");
        } else {
            panic!("Expected Contract node, got: {:?}", documented_ir);
        }
    }

    #[test]
    fn test_no_documentation_attached_without_doc_comment() {
        let source = r#"
// Regular comment
contract foo(@x) = {
    Nil
}
"#;
        let tree = parse_code(source);
        let rope = Rope::from_str(source);
        let document_ir = parse_to_document_ir(&tree, &rope);

        let attacher = DocumentationAttacher::new(document_ir.clone());
        let documented_ir = attacher.visit_node(&document_ir.root);

        // Find the contract node
        if let RholangNode::Contract { metadata, .. } = documented_ir.as_ref() {
            if let Some(meta) = metadata {
                assert!(
                    !meta.contains_key(DOC_METADATA_KEY),
                    "Metadata should not contain documentation for regular comments"
                );
            }
        } else {
            panic!("Expected Contract node");
        }
    }

    #[test]
    fn test_attach_documentation_to_new() {
        let source = r#"
/// Creates a new channel for communication
new x in {
    Nil
}
"#;
        let tree = parse_code(source);
        let rope = Rope::from_str(source);
        let document_ir = parse_to_document_ir(&tree, &rope);

        let attacher = DocumentationAttacher::new(document_ir.clone());
        let documented_ir = attacher.visit_node(&document_ir.root);

        // Find the New node
        if let RholangNode::New { metadata, .. } = documented_ir.as_ref() {
            assert!(metadata.is_some(), "New should have metadata");
            let meta = metadata.as_ref().unwrap();
            assert!(
                meta.contains_key(DOC_METADATA_KEY),
                "Metadata should contain documentation"
            );

            // Phase 7: Documentation is now StructuredDocumentation instead of String
            let structured_doc = meta
                .get(DOC_METADATA_KEY)
                .unwrap()
                .downcast_ref::<StructuredDocumentation>()
                .expect("Documentation should be StructuredDocumentation");
            assert_eq!(structured_doc.summary, "Creates a new channel for communication");
        } else {
            panic!("Expected New node, got: {:?}", documented_ir);
        }
    }

    #[test]
    fn test_multiline_documentation() {
        let source = r#"
/** This is a multiline
 * documentation comment
 * for a contract
 */
contract bar() = { Nil }
"#;
        let tree = parse_code(source);
        let rope = Rope::from_str(source);
        let document_ir = parse_to_document_ir(&tree, &rope);

        let attacher = DocumentationAttacher::new(document_ir.clone());
        let documented_ir = attacher.visit_node(&document_ir.root);

        // Find the contract node
        if let RholangNode::Contract { metadata, .. } = documented_ir.as_ref() {
            assert!(metadata.is_some(), "Contract should have metadata");
            let meta = metadata.as_ref().unwrap();

            // Phase 7: Documentation is now StructuredDocumentation instead of String
            let structured_doc = meta
                .get(DOC_METADATA_KEY)
                .unwrap()
                .downcast_ref::<StructuredDocumentation>()
                .expect("Documentation should be StructuredDocumentation");

            // Phase 7: Check that multiline doc is preserved in summary
            assert!(structured_doc.summary.contains("multiline"));
            assert!(structured_doc.summary.contains("documentation comment"));
            assert!(structured_doc.summary.contains("for a contract"));
        } else {
            panic!("Expected Contract node");
        }
    }
}
