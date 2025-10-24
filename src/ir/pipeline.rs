use std::sync::Arc;
use petgraph::Graph;
use petgraph::graph::NodeIndex;
use petgraph::algo::toposort;
use super::rholang_node::Node;
use super::visitor::Visitor;

/// Manages a pipeline of transformations applied to the Rholang IR tree.
/// Transformations are organized in a dependency graph and executed in topological order,
/// ensuring that dependent transformations run after their prerequisites.
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
    pub visitor: Arc<dyn Visitor + Send + Sync>,
}

impl Clone for Transform {
    fn clone(&self) -> Self {
        Transform {
            id: self.id.clone(),
            dependencies: self.dependencies.clone(),
            visitor: Arc::clone(&self.visitor),
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
    /// The transformed IR tree after all transformations.
    pub fn apply(&self, tree: &Arc<Node>) -> Arc<Node> {
        let order = toposort(&self.graph, None).unwrap_or_default();
        let mut current = Arc::clone(tree);
        for node_idx in order {
            let transform = &self.graph[node_idx];
            current = transform.visitor.visit_node(&current);
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
    use crate::ir::node::{Metadata, Node, NodeBase, RelativePosition};
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
            visitor: Arc::new(IdentityVisitor),
        };
        pipeline.add_transform(transform);
        let base = NodeBase::new(RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 }, 0, 0, 0);
        let mut data = HashMap::new();
        data.insert("version".to_string(), Arc::new(0_usize) as Arc<dyn Any + Send + Sync>);
        let metadata = Some(Arc::new(Metadata { data }));
        let node = Arc::new(Node::Nil { base, metadata });
        let result = pipeline.apply(&node);
        assert!(Arc::ptr_eq(&node, &result));
    }
}
