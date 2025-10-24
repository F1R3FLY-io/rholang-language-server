/// MeTTa Language IR Nodes
///
/// This module provides the intermediate representation for MeTTa code.
/// MeTTa is a meta-language with s-expression syntax, pattern matching,
/// and lazy evaluation.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use super::rholang_node::{NodeBase, Position};
use super::semantic_node::{Metadata, NodeType, SemanticNode};

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
        var_type: VariableType,
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
pub enum VariableType {
    /// Regular variable ($x)
    Regular,
    /// Grounded variable (&x) - matches only ground terms
    Grounded,
    /// Quoted variable ('x)
    Quoted,
}

impl fmt::Display for VariableType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VariableType::Regular => write!(f, "$"),
            VariableType::Grounded => write!(f, "&"),
            VariableType::Quoted => write!(f, "'"),
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
    pub fn variable(name: &str, var_type: VariableType, base: NodeBase) -> Arc<Self> {
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

    fn node_type(&self) -> NodeType {
        match self {
            MettaNode::SExpr { .. } => NodeType::MettaSExpr,
            MettaNode::Atom { .. } => NodeType::MettaAtom,
            MettaNode::Variable { .. } => NodeType::Variable,
            MettaNode::Definition { .. } => NodeType::MettaDefinition,
            MettaNode::TypeAnnotation { .. } => NodeType::MettaType,
            MettaNode::Eval { .. } => NodeType::Invocation,
            MettaNode::Match { .. } => NodeType::Match,
            MettaNode::Let { .. } => NodeType::Binding,
            MettaNode::Lambda { .. } => NodeType::Binding,
            MettaNode::If { .. } => NodeType::Conditional,
            MettaNode::Bool { .. }
            | MettaNode::Integer { .. }
            | MettaNode::Float { .. }
            | MettaNode::String { .. }
            | MettaNode::Nil { .. } => NodeType::Literal,
            MettaNode::Error { .. } => NodeType::MettaError,
            MettaNode::Comment { .. } => NodeType::Unknown,
        }
    }

    fn children(&self) -> Vec<&dyn SemanticNode> {
        // Simplified - full implementation would return actual children
        vec![]
    }

    fn children_arc(&self) -> Vec<Arc<dyn SemanticNode>> {
        // Simplified - full implementation would return actual children
        vec![]
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::rholang_node::RelativePosition;

    fn test_base() -> NodeBase {
        NodeBase::new(
            RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
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
        let var = MettaNode::variable("x", VariableType::Regular, test_base());
        assert!(matches!(&*var, MettaNode::Variable { name, var_type, .. }
            if name == "x" && *var_type == VariableType::Regular));
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
        assert_eq!(atom.node_type(), NodeType::MettaAtom);
        assert!(atom.metadata().is_none());
    }
}
