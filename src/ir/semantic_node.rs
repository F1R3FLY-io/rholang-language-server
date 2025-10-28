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

/// Represents the position of a node relative to the previous node's end position in the source code.
/// Used to compute absolute positions dynamically during traversal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RelativePosition {
    pub delta_lines: i32,    // Difference in line numbers from the previous node's end
    pub delta_columns: i32,  // Difference in column numbers, or start column if on a new line
    pub delta_bytes: usize,  // Difference in byte offsets from the previous node's end
}

/// Represents an absolute position in the source code, computed when needed from relative positions.
/// Coordinates are zero-based (row, column, byte).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct Position {
    pub row: usize,    // Line number (0-based)
    pub column: usize, // Column number (0-based)
    pub byte: usize,   // Byte offset from the start of the source code
}

/// Base structure for all Intermediate Representation (IR) nodes, encapsulating positional and textual metadata.
/// Provides the foundation for tracking node locations and source text.
#[derive(Debug, Clone)]
pub struct NodeBase {
    relative_start: RelativePosition, // Position relative to the previous node's end
    content_length: usize,            // "Soft" length: content up to last child (for semantic operations)
    syntactic_length: usize,          // "Hard" length: includes closing delimiters (for reconstruction)
    span_lines: usize,                // Number of lines spanned by the node
    span_columns: usize,              // Columns on the last line
}

impl NodeBase {
    /// Creates a new NodeBase instance with the specified attributes.
    ///
    /// # Arguments
    /// * `relative_start` - Position relative to previous node's end
    /// * `content_length` - Soft length: content up to last child (for semantics)
    /// * `syntactic_length` - Hard length: includes closing delimiters (for reconstruction)
    /// * `span_lines` - Number of lines spanned
    /// * `span_columns` - Columns on the last line
    pub fn new(
        relative_start: RelativePosition,
        content_length: usize,
        syntactic_length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        NodeBase {
            relative_start,
            content_length,
            syntactic_length,
            span_lines,
            span_columns,
        }
    }

    /// Convenience constructor for nodes without closing delimiters.
    /// Sets syntactic_length = content_length.
    pub fn new_simple(
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        NodeBase {
            relative_start,
            content_length: length,
            syntactic_length: length,
            span_lines,
            span_columns,
        }
    }

    /// Returns the relative start position of the node.
    pub fn relative_start(&self) -> RelativePosition {
        self.relative_start
    }

    /// Returns the content length (soft length) - content up to last child.
    /// Use this for semantic operations and understanding node structure.
    pub fn content_length(&self) -> usize {
        self.content_length
    }

    /// Returns the syntactic length (hard length) - includes closing delimiters.
    /// Use this for position reconstruction to compute next sibling's start.
    pub fn syntactic_length(&self) -> usize {
        self.syntactic_length
    }

    /// Returns the length of the node's text in bytes.
    /// DEPRECATED: Use content_length() or syntactic_length() instead.
    /// Defaults to syntactic_length for backward compatibility.
    #[deprecated(since = "0.1.0", note = "Use content_length() or syntactic_length() instead")]
    pub fn length(&self) -> usize {
        self.syntactic_length
    }

    /// Returns the number of lines spanned by the node.
    pub fn span_lines(&self) -> usize {
        self.span_lines
    }

    /// Returns the number of columns on the last line spanned by the node.
    pub fn span_columns(&self) -> usize {
        self.span_columns
    }

    /// Returns the delta in bytes from the previous node's end.
    pub fn delta_bytes(&self) -> usize {
        self.relative_start.delta_bytes
    }

    /// Returns the delta in lines from the previous node's end.
    pub fn delta_lines(&self) -> i32 {
        self.relative_start.delta_lines
    }

    /// Returns the delta in columns from the previous node's end.
    pub fn delta_columns(&self) -> i32 {
        self.relative_start.delta_columns
    }
}

/// High-level semantic categories for language-agnostic IR traversal
///
/// These categories represent universal programming language constructs that exist
/// across most languages. Language-specific nodes should map to one of these categories
/// to enable generic analysis and transformation.
///
/// # Design Philosophy
/// - Language-agnostic: No language-specific variants
/// - Semantic: Based on meaning, not syntax
/// - Coarse-grained: High-level categorization, not exhaustive
/// - Extensible: LanguageSpecific for constructs that don't fit
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SemanticCategory {
    /// Literal values (numbers, strings, booleans, nil)
    Literal,

    /// Variable references
    Variable,

    /// Name/variable binding (let, new, contract params, etc.)
    Binding,

    /// Function/method invocation
    Invocation,

    /// Pattern matching (match, case)
    Match,

    /// Collections (lists, sets, maps, tuples)
    Collection,

    /// Conditional expressions (if/then/else)
    Conditional,

    /// Block/sequential composition
    Block,

    /// Language-specific construct that doesn't fit universal categories
    LanguageSpecific,

    /// Unknown or error node
    Unknown,
}

impl fmt::Display for SemanticCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SemanticCategory::Literal => write!(f, "Literal"),
            SemanticCategory::Variable => write!(f, "Variable"),
            SemanticCategory::Binding => write!(f, "Binding"),
            SemanticCategory::Invocation => write!(f, "Invocation"),
            SemanticCategory::Match => write!(f, "Match"),
            SemanticCategory::Collection => write!(f, "Collection"),
            SemanticCategory::Conditional => write!(f, "Conditional"),
            SemanticCategory::Block => write!(f, "Block"),
            SemanticCategory::LanguageSpecific => write!(f, "LanguageSpecific"),
            SemanticCategory::Unknown => write!(f, "Unknown"),
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
/// # Design Philosophy
/// - **Language-agnostic**: No language-specific enums or types
/// - **Semantic**: Focuses on meaning rather than syntax
/// - **Type-safe downcasting**: Use Rust's `Any` trait for concrete types
/// - **Category-based**: High-level semantic categories for generic code
///
/// # Usage Patterns
///
/// ## Generic/Language-Agnostic Code
/// Use `semantic_category()` to work with nodes without knowing their concrete type:
/// ```rust,ignore
/// fn count_variables(node: &dyn SemanticNode) -> usize {
///     match node.semantic_category() {
///         SemanticCategory::Variable => 1,
///         _ => node.children().iter().map(|c| count_variables(*c)).sum(),
///     }
/// }
/// ```
///
/// ## Language-Specific Code
/// Use downcasting to access language-specific structure:
/// ```rust,ignore
/// if let Some(rho_node) = node.as_rholang() {
///     match rho_node {
///         RholangNode::Par { procs, .. } => { /* handle parallel composition */ }
///         _ => {}
///     }
/// }
/// ```
///
/// # Thread Safety
/// All implementations must be `Send + Sync` to support concurrent LSP operations.
pub trait SemanticNode: Send + Sync + fmt::Debug + Any {
    /// Returns the base node information (position, span, length)
    fn base(&self) -> &NodeBase;

    /// Returns the node's metadata (for extensibility)
    fn metadata(&self) -> Option<&Metadata>;

    /// Returns a mutable reference to the node's metadata
    fn metadata_mut(&mut self) -> Option<&mut Metadata>;

    /// Returns the high-level semantic category for this node
    ///
    /// This enables language-agnostic traversal and analysis. Language-specific
    /// nodes should map to the most appropriate universal category, or return
    /// `SemanticCategory::LanguageSpecific` for unique constructs.
    fn semantic_category(&self) -> SemanticCategory {
        SemanticCategory::Unknown
    }

    /// Returns a human-readable type name for this node
    ///
    /// Format: "Language::NodeType" (e.g., "Rholang::Par", "MeTTa::SExpr")
    /// or just "NodeType" for universal constructs.
    fn type_name(&self) -> &'static str {
        "Unknown"
    }

    /// Returns the number of child nodes
    ///
    /// This enables index-based traversal without lifetime issues.
    /// Returns 0 for leaf nodes (e.g., literals, variables).
    fn children_count(&self) -> usize {
        0
    }

    /// Returns the child node at the specified index
    ///
    /// # Arguments
    /// - `index`: Zero-based index of the child to retrieve
    ///
    /// # Returns
    /// - `Some(&dyn SemanticNode)` if the index is valid
    /// - `None` if the index is out of bounds
    ///
    /// Children are ordered consistently with source order.
    fn child_at(&self, index: usize) -> Option<&dyn SemanticNode> {
        let _ = index;
        None
    }

    /// Downcasts this node to `&dyn Any` for type-safe casting
    ///
    /// Use this with `downcast_ref::<ConcreteType>()` to access language-specific structure.
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
pub fn get_metadata<'a, T: Any + Send + Sync>(metadata: &'a Metadata, key: &str) -> Option<&'a T> {
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

/// Extension trait providing convenient downcasting helpers for SemanticNode
///
/// This trait is automatically implemented for all types that implement `SemanticNode`.
/// It provides type-safe downcasting to concrete language-specific node types.
///
/// # Example
/// ```rust,ignore
/// use rholang_language_server::ir::semantic_node::SemanticNodeExt;
///
/// fn analyze_node(node: &dyn SemanticNode) {
///     if let Some(rho_node) = node.as_rholang() {
///         match rho_node {
///             RholangNode::Par { procs, .. } => {
///                 println!("Found parallel composition with {} processes", procs.len());
///             }
///             _ => {}
///         }
///     } else if let Some(metta_node) = node.as_metta() {
///         match metta_node {
///             MettaNode::SExpr { elements, .. } => {
///                 println!("Found s-expr with {} elements", elements.len());
///             }
///             _ => {}
///         }
///     }
/// }
/// ```
pub trait SemanticNodeExt: SemanticNode {
    /// Attempts to downcast to RholangNode
    ///
    /// Returns `Some(&RholangNode)` if this node is a Rholang node, `None` otherwise.
    fn as_rholang(&self) -> Option<&crate::ir::rholang_node::RholangNode> {
        self.as_any().downcast_ref()
    }

    /// Attempts to downcast to MettaNode
    ///
    /// Returns `Some(&MettaNode)` if this node is a MeTTa node, `None` otherwise.
    fn as_metta(&self) -> Option<&crate::ir::metta_node::MettaNode> {
        self.as_any().downcast_ref()
    }

    /// Checks if this node is a Rholang node
    fn is_rholang(&self) -> bool {
        self.as_rholang().is_some()
    }

    /// Checks if this node is a MeTTa node
    fn is_metta(&self) -> bool {
        self.as_metta().is_some()
    }
}

/// Blanket implementation of SemanticNodeExt for all SemanticNode types
impl<T: SemanticNode + ?Sized> SemanticNodeExt for T {}

/// Generic visitor trait for language-agnostic IR traversal
///
/// This visitor works with any IR that implements SemanticNode, providing
/// a unified way to traverse and transform IR trees regardless of the source language.
///
/// Unlike the language-specific Visitor trait (for Rholang RholangNode), this visitor
/// operates at the semantic level using SemanticCategory discrimination.
///
/// # Example
/// ```rust,ignore
/// struct CountVariables {
///     count: usize,
/// }
///
/// impl GenericVisitor for CountVariables {
///     fn visit_node(&mut self, node: &dyn SemanticNode) {
///         if matches!(node.semantic_category(), SemanticCategory::Variable) {
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
    /// This is a helper method that visits each child node using index-based traversal.
    /// Override to customize child traversal order or filtering.
    fn visit_children(&mut self, node: &dyn SemanticNode) {
        let count = node.children_count();
        for i in 0..count {
            if let Some(child) = node.child_at(i) {
                self.visit_node(child);
            }
        }
    }

    /// Visit a node based on its semantic category
    ///
    /// This method dispatches to type-specific handlers based on SemanticCategory.
    /// Override specific handlers (visit_literal, visit_variable, etc.) to
    /// customize behavior for specific semantic categories.
    fn visit_typed(&mut self, node: &dyn SemanticNode) {
        match node.semantic_category() {
            SemanticCategory::Literal => self.visit_literal(node),
            SemanticCategory::Variable => self.visit_variable(node),
            SemanticCategory::Binding => self.visit_binding(node),
            SemanticCategory::Invocation => self.visit_invocation(node),
            SemanticCategory::Match => self.visit_match(node),
            SemanticCategory::Collection => self.visit_collection(node),
            SemanticCategory::Conditional => self.visit_conditional(node),
            SemanticCategory::Block => self.visit_block(node),
            SemanticCategory::LanguageSpecific => self.visit_language_specific(node),
            SemanticCategory::Unknown => self.visit_node(node),
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

    fn visit_language_specific(&mut self, node: &dyn SemanticNode) {
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
/// struct ConstantFolder;
///
/// impl TransformVisitor for ConstantFolder {
///     fn transform_node(&mut self, node: &dyn SemanticNode) -> Option<Arc<dyn SemanticNode>> {
///         // Check if this is a binary operation with literal operands
///         if let Some(rho) = node.as_any().downcast_ref::<RholangNode>() {
///             if let RholangNode::BinOp { op: BinOperator::Add, left, right, .. } = rho {
///                 // Try to fold constants
///                 // Return Some(new_node) if folded, None to use default behavior
///             }
///         }
///         None  // Use default recursive transformation
///     }
/// }
/// ```
pub trait TransformVisitor {
    /// Transform a single node without recursing to children
    ///
    /// Implementations should:
    /// 1. Check if transformation applies to this node
    /// 2. Return Some(new_node) if transformation applied
    /// 3. Return None to use default recursive transformation
    ///
    /// This method is called BEFORE transforming children, allowing
    /// early exit for optimizations.
    fn transform_node(&mut self, _node: &dyn SemanticNode) -> Option<Arc<dyn SemanticNode>> {
        None  // Default: use recursive transformation
    }

    /// Transform a node and all its children recursively
    ///
    /// This is the main entry point for transformations. It:
    /// 1. Calls transform_node() to check for custom transformation
    /// 2. If None, recursively transforms all children
    /// 3. Rebuilds the node if any children changed
    /// 4. Returns original node if nothing changed (structural sharing)
    fn transform_with_children(&mut self, node: &dyn SemanticNode) -> Arc<dyn SemanticNode> {
        // First, check if custom transformation applies
        if let Some(transformed) = self.transform_node(node) {
            return transformed;
        }

        // Otherwise, recursively transform children using index-based traversal
        let child_count = node.children_count();

        if child_count == 0 {
            // Leaf node - no children to transform
            // Need to Arc-wrap the node reference
            // We'll use downcasting to get the concrete Arc
            return self.wrap_node_in_arc(node);
        }

        // Transform all children
        let mut transformed_children = Vec::with_capacity(child_count);

        for i in 0..child_count {
            if let Some(child) = node.child_at(i) {
                let transformed_child = self.transform_with_children(child);
                transformed_children.push(transformed_child);
            }
        }

        // Always reconstruct - transformations may have occurred deep in the tree
        // The reconstruction will propagate transformed children up
        self.reconstruct_node_with_children(node, transformed_children)
    }

    /// Helper: Wraps a &dyn SemanticNode in Arc
    ///
    /// Uses downcasting to get the concrete Arc from the trait object reference.
    fn wrap_node_in_arc(&self, node: &dyn SemanticNode) -> Arc<dyn SemanticNode> {
        use crate::ir::rholang_node::RholangNode;
        use crate::ir::metta_node::MettaNode;
        use crate::ir::unified_ir::UnifiedIR;

        // Try RholangNode
        if let Some(rho) = node.as_any().downcast_ref::<RholangNode>() {
            return Arc::new(rho.clone()) as Arc<dyn SemanticNode>;
        }

        // Try MettaNode
        if let Some(metta) = node.as_any().downcast_ref::<MettaNode>() {
            return Arc::new(metta.clone()) as Arc<dyn SemanticNode>;
        }

        // Try UnifiedIR
        if let Some(unified) = node.as_any().downcast_ref::<UnifiedIR>() {
            return Arc::new(unified.clone()) as Arc<dyn SemanticNode>;
        }

        // Unknown type - this shouldn't happen
        panic!("Unknown SemanticNode type: {}", node.type_name());
    }

    /// Reconstructs a node with transformed children
    ///
    /// This is the complex part - requires knowledge of concrete node types
    /// to rebuild them with new children.
    fn reconstruct_node_with_children(
        &self,
        node: &dyn SemanticNode,
        transformed_children: Vec<Arc<dyn SemanticNode>>,
    ) -> Arc<dyn SemanticNode> {
        use crate::ir::rholang_node::RholangNode;

        // Try to reconstruct RholangNode variants
        if let Some(rho) = node.as_any().downcast_ref::<RholangNode>() {
            return self.reconstruct_rholang_node(rho, transformed_children);
        }

        // For other types (MettaNode, UnifiedIR), fall back to wrapping original
        // TODO: Implement reconstruction for MettaNode and UnifiedIR
        self.wrap_node_in_arc(node)
    }

    /// Reconstructs a RholangNode with transformed children
    ///
    /// Handles common RholangNode variants. For variants not handled,
    /// returns the original node.
    fn reconstruct_rholang_node(
        &self,
        node: &crate::ir::rholang_node::RholangNode,
        transformed_children: Vec<Arc<dyn SemanticNode>>,
    ) -> Arc<dyn SemanticNode> {
        use crate::ir::rholang_node::RholangNode;

        // Helper to downcast transformed child back to RholangNode
        fn to_rholang(child: &Arc<dyn SemanticNode>) -> Arc<crate::ir::rholang_node::RholangNode> {
            if let Some(rho) = child.as_any().downcast_ref::<RholangNode>() {
                Arc::new(rho.clone())
            } else {
                panic!("Expected RholangNode in transformed children");
            }
        }

        match node {
            // Binary nodes (2 children)
            RholangNode::Par { base, metadata, .. } if transformed_children.len() == 2 => {
                Arc::new(RholangNode::Par {
                    base: base.clone(),
                    left: Some(to_rholang(&transformed_children[0])),
                    right: Some(to_rholang(&transformed_children[1])),
                    processes: None,
                    metadata: metadata.clone(),
                }) as Arc<dyn SemanticNode>
            }

            // Lists (variable children)
            RholangNode::List { base, remainder, metadata, .. } => {
                let element_count = if remainder.is_some() {
                    transformed_children.len() - 1
                } else {
                    transformed_children.len()
                };

                let new_elements = transformed_children[..element_count]
                    .iter()
                    .map(to_rholang)
                    .collect();

                let new_remainder = if remainder.is_some() {
                    Some(to_rholang(&transformed_children[element_count]))
                } else {
                    None
                };

                Arc::new(RholangNode::List {
                    base: base.clone(),
                    elements: new_elements,
                    remainder: new_remainder,
                    metadata: metadata.clone(),
                }) as Arc<dyn SemanticNode>
            }

            // Tuples (variable children)
            RholangNode::Tuple { base, metadata, .. } => {
                let new_elements = transformed_children.iter().map(to_rholang).collect();

                Arc::new(RholangNode::Tuple {
                    base: base.clone(),
                    elements: new_elements,
                    metadata: metadata.clone(),
                }) as Arc<dyn SemanticNode>
            }

            // Sets (variable children)
            RholangNode::Set { base, remainder, metadata, .. } => {
                let element_count = if remainder.is_some() {
                    transformed_children.len() - 1
                } else {
                    transformed_children.len()
                };

                let new_elements = transformed_children[..element_count]
                    .iter()
                    .map(to_rholang)
                    .collect();

                let new_remainder = if remainder.is_some() {
                    Some(to_rholang(&transformed_children[element_count]))
                } else {
                    None
                };

                Arc::new(RholangNode::Set {
                    base: base.clone(),
                    elements: new_elements,
                    remainder: new_remainder,
                    metadata: metadata.clone(),
                }) as Arc<dyn SemanticNode>
            }

            // Send (channel + inputs)
            RholangNode::Send { base, send_type, send_type_delta, metadata, .. } if !transformed_children.is_empty() => {
                let channel = to_rholang(&transformed_children[0]);
                let inputs = transformed_children[1..].iter().map(to_rholang).collect();

                Arc::new(RholangNode::Send {
                    base: base.clone(),
                    send_type: send_type.clone(),
                    send_type_delta: *send_type_delta,
                    channel,
                    inputs,
                    metadata: metadata.clone(),
                }) as Arc<dyn SemanticNode>
            }

            // SendSync (channel + inputs + cont)
            RholangNode::SendSync { base, metadata, .. } if transformed_children.len() >= 2 => {
                let channel = to_rholang(&transformed_children[0]);
                let cont_index = transformed_children.len() - 1;
                let cont = to_rholang(&transformed_children[cont_index]);
                let inputs = transformed_children[1..cont_index].iter().map(to_rholang).collect();

                Arc::new(RholangNode::SendSync {
                    base: base.clone(),
                    channel,
                    inputs,
                    cont,
                    metadata: metadata.clone(),
                }) as Arc<dyn SemanticNode>
            }

            // Block (1 child)
            RholangNode::Block { base, metadata, .. } if transformed_children.len() == 1 => {
                Arc::new(RholangNode::Block {
                    base: base.clone(),
                    proc: to_rholang(&transformed_children[0]),
                    metadata: metadata.clone(),
                }) as Arc<dyn SemanticNode>
            }

            // Parenthesized (1 child)
            RholangNode::Parenthesized { base, metadata, .. } if transformed_children.len() == 1 => {
                Arc::new(RholangNode::Parenthesized {
                    base: base.clone(),
                    expr: to_rholang(&transformed_children[0]),
                    metadata: metadata.clone(),
                }) as Arc<dyn SemanticNode>
            }

            // BinOp (2 children)
            RholangNode::BinOp { base, op, metadata, .. } if transformed_children.len() == 2 => {
                Arc::new(RholangNode::BinOp {
                    base: base.clone(),
                    op: op.clone(),
                    left: to_rholang(&transformed_children[0]),
                    right: to_rholang(&transformed_children[1]),
                    metadata: metadata.clone(),
                }) as Arc<dyn SemanticNode>
            }

            // UnaryOp (1 child)
            RholangNode::UnaryOp { base, op, metadata, .. } if transformed_children.len() == 1 => {
                Arc::new(RholangNode::UnaryOp {
                    base: base.clone(),
                    op: op.clone(),
                    operand: to_rholang(&transformed_children[0]),
                    metadata: metadata.clone(),
                }) as Arc<dyn SemanticNode>
            }

            // New (decls + proc)
            RholangNode::New { base, metadata, decls, .. } if transformed_children.len() > 0 => {
                let decl_count = decls.len();
                let new_decls: crate::ir::rholang_node::RholangNodeVector = transformed_children[..decl_count]
                    .iter()
                    .map(to_rholang)
                    .map(|arc| (*arc).clone())
                    .map(Arc::new)
                    .collect();
                let new_proc = to_rholang(&transformed_children[decl_count]);

                Arc::new(RholangNode::New {
                    base: base.clone(),
                    decls: new_decls,
                    proc: new_proc,
                    metadata: metadata.clone(),
                }) as Arc<dyn SemanticNode>
            }

            // Let (decls + proc)
            RholangNode::Let { base, metadata, decls, .. } if transformed_children.len() > 0 => {
                let decl_count = decls.len();
                let new_decls: crate::ir::rholang_node::RholangNodeVector = transformed_children[..decl_count]
                    .iter()
                    .map(to_rholang)
                    .map(|arc| (*arc).clone())
                    .map(Arc::new)
                    .collect();
                let new_proc = to_rholang(&transformed_children[decl_count]);

                Arc::new(RholangNode::Let {
                    base: base.clone(),
                    decls: new_decls,
                    proc: new_proc,
                    metadata: metadata.clone(),
                }) as Arc<dyn SemanticNode>
            }

            // Input (receipts + proc)
            RholangNode::Input { base, metadata, receipts, .. } if transformed_children.len() > 0 => {
                // Each receipt is a vector of patterns
                // For now, just wrap original as this is complex
                self.wrap_node_in_arc(node as &dyn SemanticNode)
            }

            // Match (expression + cases)
            RholangNode::Match { base, metadata, cases, .. } if transformed_children.len() > 0 => {
                let _expression = to_rholang(&transformed_children[0]);
                // Cases are pairs - complex to reconstruct
                // For now, wrap original
                self.wrap_node_in_arc(node as &dyn SemanticNode)
            }

            // Contract (name + formals + proc)
            RholangNode::Contract { base, metadata, formals, formals_remainder, .. }
                if transformed_children.len() > 0 => {
                let name = to_rholang(&transformed_children[0]);
                let formal_count = formals.len();
                let new_formals: crate::ir::rholang_node::RholangNodeVector = transformed_children[1..=formal_count]
                    .iter()
                    .map(to_rholang)
                    .map(|arc| (*arc).clone())
                    .map(Arc::new)
                    .collect();
                let proc_index = 1 + formal_count + if formals_remainder.is_some() { 1 } else { 0 };
                let new_proc = to_rholang(&transformed_children[proc_index]);

                Arc::new(RholangNode::Contract {
                    base: base.clone(),
                    name,
                    formals: new_formals,
                    formals_remainder: formals_remainder.clone(),
                    proc: new_proc,
                    metadata: metadata.clone(),
                }) as Arc<dyn SemanticNode>
            }

            // For unhandled variants or mismatched child counts, return original
            _ => self.wrap_node_in_arc(node as &dyn SemanticNode),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_semantic_category_display() {
        assert_eq!(SemanticCategory::Literal.to_string(), "Literal");
        assert_eq!(SemanticCategory::Variable.to_string(), "Variable");
        assert_eq!(SemanticCategory::LanguageSpecific.to_string(), "LanguageSpecific");
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
