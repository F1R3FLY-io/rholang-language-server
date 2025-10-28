use std::sync::{Arc, Mutex};
use petgraph::Graph;
use petgraph::graph::NodeIndex;
use petgraph::algo::toposort;
use super::rholang_node::RholangNode;
use super::visitor::Visitor;
use super::semantic_node::{GenericVisitor, SemanticNode};

/// Enum representing either a language-specific or language-agnostic visitor.
pub enum TransformKind {
    /// Language-specific visitor working with RholangNode
    Specific(Arc<dyn Visitor + Send + Sync>),
    /// Language-agnostic visitor working with SemanticNode (requires Mutex for interior mutability)
    Generic(Arc<Mutex<dyn GenericVisitor + Send>>),
}

/// Manages a pipeline of transformations applied to the Rholang IR tree.
/// Transformations are organized in a dependency graph and executed in topological order,
/// ensuring that dependent transformations run after their prerequisites.
///
/// The pipeline now supports both language-specific (Visitor) and language-agnostic
/// (GenericVisitor) transforms, enabling gradual migration to the unified IR system.
pub struct Pipeline {
    /// The dependency graph of transformations.
    graph: Graph<Transform, ()>,
    /// Maps transformation IDs to their indices in the graph.
    node_indices: std::collections::HashMap<String, NodeIndex>,
}

/// Represents a single transformation in the pipeline, including its visitor and dependencies.
pub struct Transform {
    /// Unique identifier for the transformation.
    pub id: String,
    /// List of transformation IDs this transform depends on.
    pub dependencies: Vec<String>,
    /// The visitor implementing the transformation logic.
    pub kind: TransformKind,
}

impl Clone for Transform {
    fn clone(&self) -> Self {
        Transform {
            id: self.id.clone(),
            dependencies: self.dependencies.clone(),
            kind: match &self.kind {
                TransformKind::Specific(v) => TransformKind::Specific(Arc::clone(v)),
                TransformKind::Generic(v) => TransformKind::Generic(Arc::clone(v)),
            },
        }
    }
}

#[allow(dead_code)]
impl Pipeline {
    /// Creates a new, empty transformation pipeline.
    pub fn new() -> Self {
        Pipeline {
            graph: Graph::new(),
            node_indices: std::collections::HashMap::new(),
        }
    }

    /// Adds a transformation to the pipeline, establishing its dependencies.
    ///
    /// # Arguments
    /// * `transform` - The transformation to add.
    pub fn add_transform(&mut self, transform: Transform) {
        let node = self.graph.add_node(transform.clone());
        self.node_indices.insert(transform.id.clone(), node);
        for dep_id in &transform.dependencies {
            if let Some(dep_node) = self.node_indices.get(dep_id) {
                self.graph.add_edge(*dep_node, node, ());
            }
        }
    }

    /// Removes a transformation from the pipeline by its ID.
    ///
    /// # Arguments
    /// * `id` - The ID of the transformation to remove.
    pub fn remove_transform(&mut self, id: &str) {
        if let Some(node) = self.node_indices.remove(id) {
            self.graph.remove_node(node);
        }
    }

    /// Applies all transformations in the pipeline to the given IR tree in topological order.
    ///
    /// # Arguments
    /// * `tree` - The input IR tree to transform.
    ///
    /// # Returns
    /// The transformed IR tree after all transformations (Specific visitors may modify it).
    ///
    /// # Note
    /// - Specific transforms (Visitor) transform the tree and return a new version
    /// - Generic transforms (GenericVisitor) observe/analyze the tree without modifying it
    pub fn apply(&self, tree: &Arc<RholangNode>) -> Arc<RholangNode> {
        let order = toposort(&self.graph, None).unwrap_or_default();
        let mut current = Arc::clone(tree);
        for node_idx in order {
            let transform = &self.graph[node_idx];
            match &transform.kind {
                TransformKind::Specific(visitor) => {
                    // Language-specific transformation - may modify tree
                    current = visitor.visit_node(&current);
                }
                TransformKind::Generic(visitor) => {
                    // Language-agnostic observation - doesn't modify tree
                    // Lock the mutex to access the visitor mutably
                    visitor.lock().unwrap().visit_node(&*current as &dyn SemanticNode);
                }
            }
        }
        current
    }
}

#[cfg(test)]
mod tests {
    use std::any::Any;
    use std::collections::HashMap;
    use std::sync::Arc;
    use super::*;
    use crate::ir::rholang_node::{Metadata, RholangNode, NodeBase, RelativePosition};
    use crate::ir::visitor::Visitor;

    // Define an IdentityVisitor that preserves the node
    struct IdentityVisitor;

    impl Visitor for IdentityVisitor {
        // Default implementations preserve the node
    }

    #[test]
    fn test_pipeline_apply() {
        let mut pipeline = Pipeline::new();
        let transform = Transform {
            id: "identity".to_string(),
            dependencies: vec![],
            kind: TransformKind::Specific(Arc::new(IdentityVisitor)),
        };
        pipeline.add_transform(transform);
        let base = NodeBase::new_simple(RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 }, 0, 0, 0);
        let mut data = HashMap::new();
        data.insert("version".to_string(), Arc::new(0_usize) as Arc<dyn Any + Send + Sync>);
        let metadata = Some(Arc::new(data));
        let node = Arc::new(RholangNode::Nil { base, metadata });
        let result = pipeline.apply(&node);
        assert!(Arc::ptr_eq(&node, &result));
    }

    #[test]
    fn test_pipeline_with_generic_visitor() {
        use crate::ir::transforms::generic_symbol_collector::GenericSymbolCollector;
        use tower_lsp::lsp_types::Url;

        let _ = crate::logging::init_logger(false, Some("warn"), false);

        // Parse some Rholang code
        let code = r#"new x in { x!(42) }"#;
        let tree = crate::tree_sitter::parse_code(code);
        let rope = ropey::Rope::from_str(code);
        let ir = crate::tree_sitter::parse_to_ir(&tree, &rope);

        // Create a pipeline with a Generic visitor
        let mut pipeline = Pipeline::new();
        let uri = Url::parse("file:///test.rho").unwrap();
        let collector = Arc::new(Mutex::new(GenericSymbolCollector::new(uri)));

        // Add the generic visitor to the pipeline
        pipeline.add_transform(Transform {
            id: "symbol_collector".to_string(),
            dependencies: vec![],
            kind: TransformKind::Generic(collector.clone()),
        });

        // Apply the pipeline
        let result = pipeline.apply(&ir);

        // The tree should be unchanged (GenericVisitor doesn't transform)
        assert!(Arc::ptr_eq(&ir, &result));

        // But the collector should have collected symbols
        let collector = collector.lock().unwrap();
        assert!(collector.symbols().len() > 0, "Should have collected symbols");
        println!("Collected {} symbols", collector.symbols().len());
    }
}
