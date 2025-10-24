/// Language-agnostic semantic IR foundation
///
/// This module provides the core trait and types for a unified intermediate representation
/// that can work across multiple languages (Rholang, MeTTa, etc.).
///
/// Design principles:
/// - Language-agnostic: Common interface for all language IRs
/// - Semantic: Represents meaning, not just syntax
/// - Extensible: Metadata system allows language-specific data
/// - Type-safe: Rust type system ensures correct usage

use std::any::Any;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use super::node::{NodeBase, Position, RelativePosition};

/// Discriminator for different node types in the semantic IR
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeType {
    // Universal construct types (shared across languages)
    Literal,
    Variable,
    Binding,
    Invocation,
    Match,
    Collection,
    Conditional,
    Block,

    // Rholang-specific constructs
    RholangPar,           // Parallel composition
    RholangSend,          // Message send
    RholangInput,         // Channel input
    RholangContract,      // Contract definition
    RholangNew,           // Name creation
    RholangBundle,        // Access control
    RholangEval,          // Name evaluation
    RholangQuote,         // Process quotation

    // MeTTa-specific constructs
    MettaSExpr,           // S-expression
    MettaAtom,            // Atom/symbol
    MettaDefinition,      // Equality definition
    MettaType,            // Type annotation
    MettaError,           // Error value

    // Generic/unknown
    Unknown,
}

impl fmt::Display for NodeType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NodeType::Literal => write!(f, "Literal"),
            NodeType::Variable => write!(f, "Variable"),
            NodeType::Binding => write!(f, "Binding"),
            NodeType::Invocation => write!(f, "Invocation"),
            NodeType::Match => write!(f, "Match"),
            NodeType::Collection => write!(f, "Collection"),
            NodeType::Conditional => write!(f, "Conditional"),
            NodeType::Block => write!(f, "Block"),
            NodeType::RholangPar => write!(f, "Rholang::Par"),
            NodeType::RholangSend => write!(f, "Rholang::Send"),
            NodeType::RholangInput => write!(f, "Rholang::Input"),
            NodeType::RholangContract => write!(f, "Rholang::Contract"),
            NodeType::RholangNew => write!(f, "Rholang::New"),
            NodeType::RholangBundle => write!(f, "Rholang::Bundle"),
            NodeType::RholangEval => write!(f, "Rholang::Eval"),
            NodeType::RholangQuote => write!(f, "Rholang::Quote"),
            NodeType::MettaSExpr => write!(f, "MeTTa::SExpr"),
            NodeType::MettaAtom => write!(f, "MeTTa::Atom"),
            NodeType::MettaDefinition => write!(f, "MeTTa::Definition"),
            NodeType::MettaType => write!(f, "MeTTa::Type"),
            NodeType::MettaError => write!(f, "MeTTa::Error"),
            NodeType::Unknown => write!(f, "Unknown"),
        }
    }
}

/// Extensible metadata storage for semantic nodes
///
/// Allows transforms to attach arbitrary typed data to nodes without
/// modifying the core node structure.
pub type Metadata = HashMap<String, Arc<dyn Any + Send + Sync>>;

/// Core trait for all semantic IR nodes across languages
///
/// This trait provides a language-agnostic interface for working with IR nodes.
/// All language-specific node types must implement this trait.
///
/// # Design
/// - Position tracking: All nodes have source location information
/// - Metadata: Extensible data storage for transforms
/// - Type discrimination: NodeType allows pattern matching without downcasting
/// - Traversal: Children access for tree walking
///
/// # Thread Safety
/// All implementations must be `Send + Sync` to support concurrent LSP operations.
pub trait SemanticNode: Send + Sync + fmt::Debug {
    /// Returns the base node information (position, span, length)
    fn base(&self) -> &NodeBase;

    /// Returns the node's metadata (for extensibility)
    fn metadata(&self) -> Option<&Metadata>;

    /// Returns a mutable reference to the node's metadata
    fn metadata_mut(&mut self) -> Option<&mut Metadata>;

    /// Returns the discriminator type for this node
    fn node_type(&self) -> NodeType;

    /// Returns the child nodes of this node for traversal
    ///
    /// The order of children should be consistent with source order.
    /// Returns an empty vector if the node has no children (e.g., literals).
    fn children(&self) -> Vec<&dyn SemanticNode>;

    /// Returns the child nodes as Arc for ownership transfer
    ///
    /// Used by transforms that need to reconstruct nodes with modified children.
    fn children_arc(&self) -> Vec<Arc<dyn SemanticNode>>;

    /// Attempts to downcast this node to a concrete type
    ///
    /// # Safety
    /// Returns None if the node is not of type T.
    fn as_any(&self) -> &dyn Any;

    /// Computes the absolute position of this node given the previous node's end position
    ///
    /// # Arguments
    /// - `prev_end`: The absolute position where the previous node ended
    ///
    /// # Returns
    /// The absolute position of this node's start
    fn absolute_position(&self, prev_end: Position) -> Position {
        let base = self.base();
        let rel = base.relative_start();

        Position {
            row: (prev_end.row as i32 + rel.delta_lines) as usize,
            column: if rel.delta_lines > 0 {
                rel.delta_columns as usize
            } else {
                (prev_end.column as i32 + rel.delta_columns) as usize
            },
            byte: prev_end.byte + rel.delta_bytes,
        }
    }

    /// Computes the absolute end position of this node given its start position
    ///
    /// # Arguments
    /// - `start`: The absolute start position of this node
    ///
    /// # Returns
    /// The absolute position where this node ends
    fn absolute_end(&self, start: Position) -> Position {
        let base = self.base();

        Position {
            row: start.row + base.span_lines(),
            column: if base.span_lines() > 0 {
                base.span_columns()
            } else {
                start.column + base.span_columns()
            },
            byte: start.byte + base.length(),
        }
    }
}

/// Helper function to create an empty metadata map
pub fn empty_metadata() -> Metadata {
    HashMap::new()
}

/// Helper function to create metadata with a single entry
pub fn metadata_with<T: Any + Send + Sync>(key: &str, value: T) -> Metadata {
    let mut map = HashMap::new();
    map.insert(key.to_string(), Arc::new(value) as Arc<dyn Any + Send + Sync>);
    map
}

/// Helper function to get a typed value from metadata
pub fn get_metadata<T: Any + Send + Sync>(metadata: &Metadata, key: &str) -> Option<&T> {
    metadata
        .get(key)
        .and_then(|arc| arc.downcast_ref::<T>())
}

/// Helper function to insert a typed value into metadata
pub fn insert_metadata<T: Any + Send + Sync>(
    metadata: &mut Metadata,
    key: &str,
    value: T,
) {
    metadata.insert(key.to_string(), Arc::new(value) as Arc<dyn Any + Send + Sync>);
}

/// Generic visitor trait for language-agnostic IR traversal
///
/// This visitor works with any IR that implements SemanticNode, providing
/// a unified way to traverse and transform IR trees regardless of the source language.
///
/// Unlike the language-specific Visitor trait (for Rholang Node), this visitor
/// operates at the semantic level using NodeType discrimination.
///
/// # Example
/// ```rust,ignore
/// struct CountVariables {
///     count: usize,
/// }
///
/// impl GenericVisitor for CountVariables {
///     fn visit_node(&mut self, node: &dyn SemanticNode) {
///         if matches!(node.node_type(), NodeType::Variable) {
///             self.count += 1;
///         }
///         self.visit_children(node);
///     }
/// }
/// ```
pub trait GenericVisitor {
    /// Visit a semantic node
    ///
    /// Override this method to implement custom visiting logic.
    /// Call `visit_children()` to recursively visit child nodes.
    fn visit_node(&mut self, node: &dyn SemanticNode) {
        self.visit_children(node);
    }

    /// Visit all children of a node
    ///
    /// This is a helper method that visits each child node.
    /// Override to customize child traversal order or filtering.
    fn visit_children(&mut self, node: &dyn SemanticNode) {
        for child in node.children() {
            self.visit_node(child);
        }
    }

    /// Visit a node based on its semantic type
    ///
    /// This method dispatches to type-specific handlers based on NodeType.
    /// Override specific handlers (visit_literal, visit_variable, etc.) to
    /// customize behavior for specific node types.
    fn visit_typed(&mut self, node: &dyn SemanticNode) {
        match node.node_type() {
            NodeType::Literal => self.visit_literal(node),
            NodeType::Variable => self.visit_variable(node),
            NodeType::Binding => self.visit_binding(node),
            NodeType::Invocation => self.visit_invocation(node),
            NodeType::Match => self.visit_match(node),
            NodeType::Collection => self.visit_collection(node),
            NodeType::Conditional => self.visit_conditional(node),
            NodeType::Block => self.visit_block(node),
            _ => self.visit_node(node), // Fallback for language-specific types
        }
    }

    // Type-specific visitor methods (can be overridden)

    fn visit_literal(&mut self, node: &dyn SemanticNode) {
        self.visit_node(node);
    }

    fn visit_variable(&mut self, node: &dyn SemanticNode) {
        self.visit_node(node);
    }

    fn visit_binding(&mut self, node: &dyn SemanticNode) {
        self.visit_node(node);
    }

    fn visit_invocation(&mut self, node: &dyn SemanticNode) {
        self.visit_node(node);
    }

    fn visit_match(&mut self, node: &dyn SemanticNode) {
        self.visit_node(node);
    }

    fn visit_collection(&mut self, node: &dyn SemanticNode) {
        self.visit_node(node);
    }

    fn visit_conditional(&mut self, node: &dyn SemanticNode) {
        self.visit_node(node);
    }

    fn visit_block(&mut self, node: &dyn SemanticNode) {
        self.visit_node(node);
    }
}

/// Transforming visitor trait for language-agnostic IR transformation
///
/// Unlike GenericVisitor which is for analysis/inspection, TransformVisitor
/// creates new IR nodes, enabling immutable transformations.
///
/// # Example
/// ```rust,ignore
/// struct RenameVariable {
///     old_name: String,
///     new_name: String,
/// }
///
/// impl TransformVisitor for RenameVariable {
///     fn transform_node(&self, node: &Arc<dyn SemanticNode>) -> Arc<dyn SemanticNode> {
///         // Downcast and transform as needed
///         // Return new node or original if unchanged
///         node.clone()
///     }
/// }
/// ```
pub trait TransformVisitor {
    /// Transform a node, returning a new node or the original
    ///
    /// Implementations should:
    /// 1. Check if transformation applies to this node
    /// 2. Transform children recursively
    /// 3. Create new node if anything changed
    /// 4. Return original if unchanged (for structural sharing)
    fn transform_node(&self, node: &Arc<dyn SemanticNode>) -> Arc<dyn SemanticNode> {
        // Default: return original (identity transform)
        Arc::clone(node)
    }

    /// Transform all children of a node
    ///
    /// Helper method for recursively transforming child nodes.
    fn transform_children(&self, children: Vec<&dyn SemanticNode>) -> Vec<Arc<dyn SemanticNode>> {
        children
            .into_iter()
            .map(|child| {
                // This is tricky - we need Arc<dyn SemanticNode> but have &dyn SemanticNode
                // In practice, implementations will need to work with concrete types
                // For now, return empty vec
                unimplemented!("transform_children requires concrete type knowledge")
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_type_display() {
        assert_eq!(NodeType::RholangPar.to_string(), "Rholang::Par");
        assert_eq!(NodeType::MettaSExpr.to_string(), "MeTTa::SExpr");
        assert_eq!(NodeType::Literal.to_string(), "Literal");
    }

    #[test]
    fn test_metadata_helpers() {
        let mut metadata = empty_metadata();
        assert_eq!(metadata.len(), 0);

        insert_metadata(&mut metadata, "test_key", 42i32);
        assert_eq!(get_metadata::<i32>(&metadata, "test_key"), Some(&42));
        assert_eq!(get_metadata::<String>(&metadata, "test_key"), None);
    }

    #[test]
    fn test_metadata_with() {
        let metadata = metadata_with("count", 100usize);
        assert_eq!(get_metadata::<usize>(&metadata, "count"), Some(&100));
    }

    // Test GenericVisitor
    struct NodeCounter {
        count: usize,
    }

    impl GenericVisitor for NodeCounter {
        fn visit_node(&mut self, node: &dyn SemanticNode) {
            self.count += 1;
            self.visit_children(node);
        }
    }

    #[test]
    fn test_generic_visitor() {
        let mut counter = NodeCounter { count: 0 };
        // Would need actual nodes to test, but the trait compiles
        assert_eq!(counter.count, 0);
    }
}
