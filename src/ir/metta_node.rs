/// MeTTa Language IR Nodes
///
/// This module provides the intermediate representation for MeTTa code.
/// MeTTa is a meta-language with s-expression syntax, pattern matching,
/// and lazy evaluation.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use super::semantic_node::{NodeBase, Position};
use super::semantic_node::{Metadata, SemanticNode, SemanticCategory};

/// MeTTa-specific node types
#[derive(Debug, Clone)]
pub enum MettaNode {
    /// S-expression - the fundamental MeTTa construct (op arg1 arg2 ...)
    SExpr {
        base: NodeBase,
        elements: Vec<Arc<MettaNode>>,
        metadata: Option<Arc<Metadata>>,
    },

    /// Atom - symbol or identifier
    Atom {
        base: NodeBase,
        name: String,
        metadata: Option<Arc<Metadata>>,
    },

    /// Variable - starts with $ or & or '
    Variable {
        base: NodeBase,
        name: String,
        var_type: MettaVariableType,
        metadata: Option<Arc<Metadata>>,
    },

    /// Equality definition - (= pattern body)
    Definition {
        base: NodeBase,
        pattern: Arc<MettaNode>,
        body: Arc<MettaNode>,
        metadata: Option<Arc<Metadata>>,
    },

    /// Type annotation - (: expr type)
    TypeAnnotation {
        base: NodeBase,
        expr: Arc<MettaNode>,
        type_expr: Arc<MettaNode>,
        metadata: Option<Arc<Metadata>>,
    },

    /// Evaluation - !(expr)
    Eval {
        base: NodeBase,
        expr: Arc<MettaNode>,
        metadata: Option<Arc<Metadata>>,
    },

    /// Match expression - (match scrutinee (case1 result1) (case2 result2))
    Match {
        base: NodeBase,
        scrutinee: Arc<MettaNode>,
        cases: Vec<(Arc<MettaNode>, Arc<MettaNode>)>,
        metadata: Option<Arc<Metadata>>,
    },

    /// Let binding - (let ((var1 val1) (var2 val2)) body)
    Let {
        base: NodeBase,
        bindings: Vec<(Arc<MettaNode>, Arc<MettaNode>)>,
        body: Arc<MettaNode>,
        metadata: Option<Arc<Metadata>>,
    },

    /// Lambda - (λ (params) body) or (lambda (params) body)
    Lambda {
        base: NodeBase,
        params: Vec<Arc<MettaNode>>,
        body: Arc<MettaNode>,
        metadata: Option<Arc<Metadata>>,
    },

    /// If-then-else - (if cond then else)
    If {
        base: NodeBase,
        condition: Arc<MettaNode>,
        consequence: Arc<MettaNode>,
        alternative: Option<Arc<MettaNode>>,
        metadata: Option<Arc<Metadata>>,
    },

    /// Boolean literal
    Bool {
        base: NodeBase,
        value: bool,
        metadata: Option<Arc<Metadata>>,
    },

    /// Integer literal
    Integer {
        base: NodeBase,
        value: i64,
        metadata: Option<Arc<Metadata>>,
    },

    /// Float literal
    Float {
        base: NodeBase,
        value: f64,
        metadata: Option<Arc<Metadata>>,
    },

    /// String literal
    String {
        base: NodeBase,
        value: std::string::String,
        metadata: Option<Arc<Metadata>>,
    },

    /// Nil/empty
    Nil {
        base: NodeBase,
        metadata: Option<Arc<Metadata>>,
    },

    /// Error node
    Error {
        base: NodeBase,
        message: std::string::String,
        children: Vec<Arc<MettaNode>>,
        metadata: Option<Arc<Metadata>>,
    },

    /// Comment
    Comment {
        base: NodeBase,
        text: std::string::String,
        metadata: Option<Arc<Metadata>>,
    },
}

/// Type of MeTTa variable
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MettaVariableType {
    /// Regular variable ($x)
    Regular,
    /// Grounded variable (&x) - matches only ground terms
    Grounded,
    /// Quoted variable ('x)
    Quoted,
}

impl fmt::Display for MettaVariableType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MettaVariableType::Regular => write!(f, "$"),
            MettaVariableType::Grounded => write!(f, "&"),
            MettaVariableType::Quoted => write!(f, "'"),
        }
    }
}

impl MettaNode {
    /// Create a simple atom node
    pub fn atom(name: &str, base: NodeBase) -> Arc<Self> {
        Arc::new(MettaNode::Atom {
            base,
            name: name.to_string(),
            metadata: None,
        })
    }

    /// Create a variable node
    pub fn variable(name: &str, var_type: MettaVariableType, base: NodeBase) -> Arc<Self> {
        Arc::new(MettaNode::Variable {
            base,
            name: name.to_string(),
            var_type,
            metadata: None,
        })
    }

    /// Create an s-expression node
    pub fn sexpr(elements: Vec<Arc<MettaNode>>, base: NodeBase) -> Arc<Self> {
        Arc::new(MettaNode::SExpr {
            base,
            elements,
            metadata: None,
        })
    }

    /// Check if this node is a literal (ground term)
    pub fn is_literal(&self) -> bool {
        matches!(
            self,
            MettaNode::Bool { .. }
                | MettaNode::Integer { .. }
                | MettaNode::Float { .. }
                | MettaNode::String { .. }
                | MettaNode::Nil { .. }
        )
    }

    /// Check if this node is a variable
    pub fn is_variable(&self) -> bool {
        matches!(self, MettaNode::Variable { .. })
    }

    /// Get the name if this is an atom or variable
    pub fn name(&self) -> Option<&str> {
        match self {
            MettaNode::Atom { name, .. } => Some(name),
            MettaNode::Variable { name, .. } => Some(name),
            _ => None,
        }
    }
}

// Implement SemanticNode for MettaNode
impl SemanticNode for MettaNode {
    fn base(&self) -> &NodeBase {
        match self {
            MettaNode::SExpr { base, .. } => base,
            MettaNode::Atom { base, .. } => base,
            MettaNode::Variable { base, .. } => base,
            MettaNode::Definition { base, .. } => base,
            MettaNode::TypeAnnotation { base, .. } => base,
            MettaNode::Eval { base, .. } => base,
            MettaNode::Match { base, .. } => base,
            MettaNode::Let { base, .. } => base,
            MettaNode::Lambda { base, .. } => base,
            MettaNode::If { base, .. } => base,
            MettaNode::Bool { base, .. } => base,
            MettaNode::Integer { base, .. } => base,
            MettaNode::Float { base, .. } => base,
            MettaNode::String { base, .. } => base,
            MettaNode::Nil { base, .. } => base,
            MettaNode::Error { base, .. } => base,
            MettaNode::Comment { base, .. } => base,
        }
    }

    fn metadata(&self) -> Option<&Metadata> {
        match self {
            MettaNode::SExpr { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            MettaNode::Atom { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            MettaNode::Variable { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            MettaNode::Definition { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            MettaNode::TypeAnnotation { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            MettaNode::Eval { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            MettaNode::Match { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            MettaNode::Let { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            MettaNode::Lambda { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            MettaNode::If { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            MettaNode::Bool { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            MettaNode::Integer { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            MettaNode::Float { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            MettaNode::String { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            MettaNode::Nil { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            MettaNode::Error { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            MettaNode::Comment { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
        }
    }

    fn metadata_mut(&mut self) -> Option<&mut Metadata> {
        // Metadata is Arc-wrapped, so mutation not supported
        None
    }

    fn semantic_category(&self) -> SemanticCategory {
        match self {
            MettaNode::SExpr { .. } => SemanticCategory::LanguageSpecific,
            MettaNode::Atom { .. } => SemanticCategory::LanguageSpecific,
            MettaNode::Variable { .. } => SemanticCategory::Variable,
            MettaNode::Definition { .. } => SemanticCategory::Binding,
            MettaNode::TypeAnnotation { .. } => SemanticCategory::LanguageSpecific,
            MettaNode::Eval { .. } => SemanticCategory::Invocation,
            MettaNode::Match { .. } => SemanticCategory::Match,
            MettaNode::Let { .. } | MettaNode::Lambda { .. } => SemanticCategory::Binding,
            MettaNode::If { .. } => SemanticCategory::Conditional,
            MettaNode::Bool { .. }
            | MettaNode::Integer { .. }
            | MettaNode::Float { .. }
            | MettaNode::String { .. }
            | MettaNode::Nil { .. } => SemanticCategory::Literal,
            MettaNode::Error { .. } => SemanticCategory::Unknown,
            MettaNode::Comment { .. } => SemanticCategory::Unknown,
        }
    }

    fn type_name(&self) -> &'static str {
        match self {
            MettaNode::SExpr { .. } => "MeTTa::SExpr",
            MettaNode::Atom { .. } => "MeTTa::Atom",
            MettaNode::Variable { .. } => "MeTTa::Variable",
            MettaNode::Definition { .. } => "MeTTa::Definition",
            MettaNode::TypeAnnotation { .. } => "MeTTa::TypeAnnotation",
            MettaNode::Eval { .. } => "MeTTa::Eval",
            MettaNode::Match { .. } => "MeTTa::Match",
            MettaNode::Let { .. } => "MeTTa::Let",
            MettaNode::Lambda { .. } => "MeTTa::Lambda",
            MettaNode::If { .. } => "MeTTa::If",
            MettaNode::Bool { .. } => "MeTTa::Bool",
            MettaNode::Integer { .. } => "MeTTa::Integer",
            MettaNode::Float { .. } => "MeTTa::Float",
            MettaNode::String { .. } => "MeTTa::String",
            MettaNode::Nil { .. } => "MeTTa::Nil",
            MettaNode::Error { .. } => "MeTTa::Error",
            MettaNode::Comment { .. } => "MeTTa::Comment",
        }
    }

    fn children_count(&self) -> usize {
        match self {
            // Nodes with vector children
            MettaNode::SExpr { elements, .. } => elements.len(),

            // Nodes with 2 children
            MettaNode::Definition { pattern, body, .. } => {
                let _ = (pattern, body);
                2
            }
            MettaNode::TypeAnnotation { expr, type_expr, .. } => {
                let _ = (expr, type_expr);
                2
            }

            // Nodes with 1 child
            MettaNode::Eval { expr, .. } => {
                let _ = expr;
                1
            }

            // Match has scrutinee + cases
            MettaNode::Match { scrutinee, cases, .. } => {
                let _ = scrutinee;
                1 + cases.len() * 2  // scrutinee + (pattern, result) for each case
            }

            // Let has bindings + body
            MettaNode::Let { bindings, body, .. } => {
                let _ = body;
                bindings.len() * 2 + 1  // (var, val) for each binding + body
            }

            // Lambda has params + body
            MettaNode::Lambda { params, body, .. } => {
                let _ = body;
                params.len() + 1
            }

            // If has 2 or 3 children
            MettaNode::If { condition, consequence, alternative, .. } => {
                let _ = (condition, consequence);
                if alternative.is_some() { 3 } else { 2 }
            }

            // Leaf nodes
            MettaNode::Atom { .. } |
            MettaNode::Variable { .. } |
            MettaNode::Bool { .. } |
            MettaNode::Integer { .. } |
            MettaNode::Float { .. } |
            MettaNode::String { .. } |
            MettaNode::Nil { .. } |
            MettaNode::Error { .. } |
            MettaNode::Comment { .. } => 0,
        }
    }

    fn child_at(&self, index: usize) -> Option<&dyn SemanticNode> {
        match self {
            // Nodes with vector children
            MettaNode::SExpr { elements, .. } => {
                elements.get(index).map(|e| &**e as &dyn SemanticNode)
            }

            // Nodes with 2 children
            MettaNode::Definition { pattern, body, .. } => match index {
                0 => Some(&**pattern),
                1 => Some(&**body),
                _ => None,
            },
            MettaNode::TypeAnnotation { expr, type_expr, .. } => match index {
                0 => Some(&**expr),
                1 => Some(&**type_expr),
                _ => None,
            },

            // Nodes with 1 child
            MettaNode::Eval { expr, .. } if index == 0 => Some(&**expr),

            // Match
            MettaNode::Match { scrutinee, cases, .. } => {
                if index == 0 {
                    Some(&**scrutinee)
                } else {
                    let case_index = (index - 1) / 2;
                    if case_index < cases.len() {
                        let (pattern, result) = &cases[case_index];
                        if (index - 1) % 2 == 0 {
                            Some(&**pattern)
                        } else {
                            Some(&**result)
                        }
                    } else {
                        None
                    }
                }
            }

            // Let
            MettaNode::Let { bindings, body, .. } => {
                let binding_index = index / 2;
                if binding_index < bindings.len() {
                    let (var, val) = &bindings[binding_index];
                    if index % 2 == 0 {
                        Some(&**var)
                    } else {
                        Some(&**val)
                    }
                } else if index == bindings.len() * 2 {
                    Some(&**body)
                } else {
                    None
                }
            }

            // Lambda
            MettaNode::Lambda { params, body, .. } => {
                if index < params.len() {
                    Some(&**params.get(index)?)
                } else if index == params.len() {
                    Some(&**body)
                } else {
                    None
                }
            }

            // If
            MettaNode::If { condition, consequence, alternative, .. } => match index {
                0 => Some(&**condition),
                1 => Some(&**consequence),
                2 => alternative.as_ref().map(|alt| &**alt as &dyn SemanticNode),
                _ => None,
            },

            // Leaf nodes
            _ => None,
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

use std::collections::HashMap;

/// Computes absolute positions for all nodes in the MeTTa IR tree.
///
/// This function walks the entire tree and converts relative positions (deltas) to absolute
/// positions (line, column, byte). Returns a HashMap mapping each node's address to its
/// (start, end) position.
pub fn compute_absolute_positions(root: &Arc<MettaNode>) -> HashMap<usize, (Position, Position)> {
    let mut positions = HashMap::new();
    let initial_prev_end = Position {
        row: 0,
        column: 0,
        byte: 0,
    };
    compute_positions_helper(root, initial_prev_end, &mut positions);
    positions
}

/// Computes absolute positions for a node tree starting from a given previous end position.
///
/// This is useful when processing multiple top-level nodes where each node's position
/// is relative to the previous node's end.
///
/// # Returns
/// A tuple of (positions HashMap, final prev_end) where the final prev_end is the end
/// position of the last node processed.
pub fn compute_positions_with_prev_end(
    root: &Arc<MettaNode>,
    prev_end: Position,
) -> (HashMap<usize, (Position, Position)>, Position) {
    let mut positions = HashMap::new();
    let final_prev_end = compute_positions_helper(root, prev_end, &mut positions);
    (positions, final_prev_end)
}

/// Recursively computes absolute positions for all MeTTa node types in the IR tree.
///
/// # Arguments
/// * node - The current node being processed.
/// * prev_end - The absolute end position of the previous sibling or parent's start if first child.
/// * positions - The HashMap storing computed (start, end) positions.
///
/// # Returns
/// The absolute end position of the current node.
fn compute_positions_helper(
    node: &Arc<MettaNode>,
    prev_end: Position,
    positions: &mut HashMap<usize, (Position, Position)>,
) -> Position {
    let base = node.base();
    let key = &**node as *const MettaNode as usize;

    // NodeBase now stores absolute positions
    let start = base.start();
    let end = base.end();

    // Store this node's position
    positions.insert(key, (start, end));

    let mut current_prev = start;

    // Process children based on node type
    match &**node {
        MettaNode::SExpr { elements, .. } => {
            for elem in elements {
                current_prev = compute_positions_helper(elem, current_prev, positions);
            }
        }
        MettaNode::Definition { pattern, body, .. } => {
            current_prev = compute_positions_helper(pattern, current_prev, positions);
            current_prev = compute_positions_helper(body, current_prev, positions);
        }
        MettaNode::TypeAnnotation { expr, type_expr, .. } => {
            current_prev = compute_positions_helper(expr, current_prev, positions);
            current_prev = compute_positions_helper(type_expr, current_prev, positions);
        }
        MettaNode::Eval { expr, .. } => {
            current_prev = compute_positions_helper(expr, current_prev, positions);
        }
        MettaNode::Match { scrutinee, cases, .. } => {
            current_prev = compute_positions_helper(scrutinee, current_prev, positions);
            for (pattern, result) in cases {
                current_prev = compute_positions_helper(pattern, current_prev, positions);
                current_prev = compute_positions_helper(result, current_prev, positions);
            }
        }
        MettaNode::Let { bindings, body, .. } => {
            for (var, val) in bindings {
                current_prev = compute_positions_helper(var, current_prev, positions);
                current_prev = compute_positions_helper(val, current_prev, positions);
            }
            current_prev = compute_positions_helper(body, current_prev, positions);
        }
        MettaNode::Lambda { params, body, .. } => {
            for param in params {
                current_prev = compute_positions_helper(param, current_prev, positions);
            }
            current_prev = compute_positions_helper(body, current_prev, positions);
        }
        MettaNode::If { condition, consequence, alternative, .. } => {
            current_prev = compute_positions_helper(condition, current_prev, positions);
            current_prev = compute_positions_helper(consequence, current_prev, positions);
            if let Some(alt) = alternative {
                current_prev = compute_positions_helper(alt, current_prev, positions);
            }
        }
        MettaNode::Error { children, .. } => {
            for child in children {
                current_prev = compute_positions_helper(child, current_prev, positions);
            }
        }
        // Leaf nodes - no children to process
        MettaNode::Atom { .. }
        | MettaNode::Variable { .. }
        | MettaNode::Bool { .. }
        | MettaNode::Integer { .. }
        | MettaNode::Float { .. }
        | MettaNode::String { .. }
        | MettaNode::Nil { .. }
        | MettaNode::Comment { .. } => {
            // No children to process
        }
    }

    end
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::semantic_node::Position;

    fn test_base() -> NodeBase {
        NodeBase::new_simple(
            Position {
                row: 0,
                column: 0,
                byte: 0,
            },
            10,
            0,
            10,
        )
    }

    #[test]
    fn test_atom_creation() {
        let atom = MettaNode::atom("foo", test_base());
        assert!(matches!(&*atom, MettaNode::Atom { name, .. } if name == "foo"));
        assert_eq!(atom.name(), Some("foo"));
    }

    #[test]
    fn test_variable_creation() {
        let var = MettaNode::variable("x", MettaVariableType::Regular, test_base());
        assert!(matches!(&*var, MettaNode::Variable { name, var_type, .. }
            if name == "x" && *var_type == MettaVariableType::Regular));
        assert!(var.is_variable());
    }

    #[test]
    fn test_literal_check() {
        let int_node = Arc::new(MettaNode::Integer {
            base: test_base(),
            value: 42,
            metadata: None,
        });
        assert!(int_node.is_literal());

        let atom = MettaNode::atom("foo", test_base());
        assert!(!atom.is_literal());
    }

    #[test]
    fn test_semantic_node_impl() {
        let atom = MettaNode::atom("test", test_base());
        assert_eq!(atom.semantic_category(), SemanticCategory::LanguageSpecific);
        assert_eq!(atom.type_name(), "MeTTa::Atom");
        assert!(atom.metadata().is_none());
    }
}
