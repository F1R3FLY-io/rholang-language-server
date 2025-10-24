/// Unified Intermediate Representation (Phase 2)
///
/// This module provides a language-agnostic IR that can represent common semantic constructs
/// across multiple languages (Rholang, MeTTa, etc.). This is inspired by ASR (Abstract Semantic
/// Representation) from LFortran.
///
/// The UnifiedIR sits at a higher abstraction level than language-specific IRs, focusing on
/// semantic meaning rather than syntactic details.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use super::node::{NodeBase, Node};
use super::semantic_node::{Metadata, NodeType, SemanticNode};

/// Common literal types across languages
#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Bool(bool),
    Integer(i64),
    Float(f64),
    String(String),
    Uri(String),
    Nil,
}

impl fmt::Display for Literal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Literal::Bool(b) => write!(f, "{}", b),
            Literal::Integer(i) => write!(f, "{}", i),
            Literal::Float(fl) => write!(f, "{}", fl),
            Literal::String(s) => write!(f, "\"{}\"", s),
            Literal::Uri(u) => write!(f, "`{}`", u),
            Literal::Nil => write!(f, "Nil"),
        }
    }
}

/// Binding types - how variables/names are bound
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingKind {
    /// New name creation (Rholang: new, MeTTa: fresh variable)
    NewBind,
    /// Let binding (both languages)
    LetBind,
    /// Pattern binding in match/case
    PatternBind,
    /// Function/Contract parameter
    Parameter,
    /// Input from channel (Rholang-specific but conceptually universal)
    InputBind,
}

/// Collection types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionKind {
    List,
    Set,
    Map,
    Tuple,
}

/// Unified IR node - common semantic constructs across languages
#[derive(Debug, Clone)]
pub enum UnifiedIR {
    /// Literal value
    Literal {
        base: NodeBase,
        value: Literal,
        metadata: Option<Arc<Metadata>>,
    },

    /// Variable reference
    Variable {
        base: NodeBase,
        name: String,
        metadata: Option<Arc<Metadata>>,
    },

    /// Binding construct (new, let, pattern, etc.)
    Binding {
        base: NodeBase,
        kind: BindingKind,
        names: Vec<Arc<UnifiedIR>>,
        values: Vec<Arc<UnifiedIR>>,
        body: Option<Arc<UnifiedIR>>,
        metadata: Option<Arc<Metadata>>,
    },

    /// Function/Contract/Process invocation
    Invocation {
        base: NodeBase,
        target: Arc<UnifiedIR>,
        args: Vec<Arc<UnifiedIR>>,
        metadata: Option<Arc<Metadata>>,
    },

    /// Pattern matching
    Match {
        base: NodeBase,
        scrutinee: Arc<UnifiedIR>,
        cases: Vec<(Arc<UnifiedIR>, Arc<UnifiedIR>)>, // (pattern, body)
        metadata: Option<Arc<Metadata>>,
    },

    /// Collection (list, set, map, tuple)
    Collection {
        base: NodeBase,
        kind: CollectionKind,
        elements: Vec<Arc<UnifiedIR>>,
        metadata: Option<Arc<Metadata>>,
    },

    /// Conditional (if-then-else, choice)
    Conditional {
        base: NodeBase,
        condition: Option<Arc<UnifiedIR>>, // None for non-deterministic choice
        consequence: Arc<UnifiedIR>,
        alternative: Option<Arc<UnifiedIR>>,
        metadata: Option<Arc<Metadata>>,
    },

    /// Block/Sequence of expressions
    Block {
        base: NodeBase,
        body: Arc<UnifiedIR>,
        metadata: Option<Arc<Metadata>>,
    },

    /// Composition - parallel (Rholang) or sequential (MeTTa)
    Composition {
        base: NodeBase,
        is_parallel: bool, // true for Rholang Par, false for MeTTa sequential
        left: Arc<UnifiedIR>,
        right: Arc<UnifiedIR>,
        metadata: Option<Arc<Metadata>>,
    },

    /// Language-specific extension for Rholang
    RholangExt {
        base: NodeBase,
        node: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
    },

    /// Language-specific extension for MeTTa
    MettaExt {
        base: NodeBase,
        // Will be defined when we create MettaNode
        // For now, just store as Any
        node: Arc<dyn Any + Send + Sync>,
        metadata: Option<Arc<Metadata>>,
    },

    /// Error node
    Error {
        base: NodeBase,
        message: String,
        children: Vec<Arc<UnifiedIR>>,
        metadata: Option<Arc<Metadata>>,
    },
}

impl UnifiedIR {
    /// Convert a Rholang Node to UnifiedIR
    pub fn from_rholang(node: &Arc<Node>) -> Arc<UnifiedIR> {
        // Extract base from the node
        let base = node.base().clone();

        match &**node {
            // Literals
            Node::BoolLiteral { value, metadata, .. } => Arc::new(UnifiedIR::Literal {
                base,
                value: Literal::Bool(*value),
                metadata: metadata.clone(),
            }),
            Node::LongLiteral { value, metadata, .. } => Arc::new(UnifiedIR::Literal {
                base,
                value: Literal::Integer(*value),
                metadata: metadata.clone(),
            }),
            Node::StringLiteral { value, metadata, .. } => Arc::new(UnifiedIR::Literal {
                base,
                value: Literal::String(value.clone()),
                metadata: metadata.clone(),
            }),
            Node::UriLiteral { value, metadata, .. } => Arc::new(UnifiedIR::Literal {
                base,
                value: Literal::Uri(value.clone()),
                metadata: metadata.clone(),
            }),
            Node::Nil { metadata, .. } | Node::Wildcard { metadata, .. } => {
                Arc::new(UnifiedIR::Literal {
                    base,
                    value: Literal::Nil,
                    metadata: metadata.clone(),
                })
            }

            // Variables
            Node::Var { name, metadata, .. } => Arc::new(UnifiedIR::Variable {
                base,
                name: name.clone(),
                metadata: metadata.clone(),
            }),

            // Collections
            Node::List { elements, metadata, .. } => Arc::new(UnifiedIR::Collection {
                base,
                kind: CollectionKind::List,
                elements: elements.iter().map(UnifiedIR::from_rholang).collect(),
                metadata: metadata.clone(),
            }),
            Node::Set { elements, metadata, .. } => Arc::new(UnifiedIR::Collection {
                base,
                kind: CollectionKind::Set,
                elements: elements.iter().map(UnifiedIR::from_rholang).collect(),
                metadata: metadata.clone(),
            }),
            Node::Tuple { elements, metadata, .. } => Arc::new(UnifiedIR::Collection {
                base,
                kind: CollectionKind::Tuple,
                elements: elements.iter().map(UnifiedIR::from_rholang).collect(),
                metadata: metadata.clone(),
            }),

            // Parallel composition (Rholang-specific)
            Node::Par { left, right, metadata, .. } => Arc::new(UnifiedIR::Composition {
                base,
                is_parallel: true,
                left: UnifiedIR::from_rholang(left),
                right: UnifiedIR::from_rholang(right),
                metadata: metadata.clone(),
            }),

            // Match
            Node::Match { expression, cases, metadata, .. } => Arc::new(UnifiedIR::Match {
                base,
                scrutinee: UnifiedIR::from_rholang(expression),
                cases: cases
                    .iter()
                    .map(|(pat, body)| {
                        (UnifiedIR::from_rholang(pat), UnifiedIR::from_rholang(body))
                    })
                    .collect(),
                metadata: metadata.clone(),
            }),

            // Conditional
            Node::IfElse { condition, consequence, alternative, metadata, .. } => {
                Arc::new(UnifiedIR::Conditional {
                    base,
                    condition: Some(UnifiedIR::from_rholang(condition)),
                    consequence: UnifiedIR::from_rholang(consequence),
                    alternative: alternative.as_ref().map(UnifiedIR::from_rholang),
                    metadata: metadata.clone(),
                })
            }

            // Block
            Node::Block { proc, metadata, .. } | Node::Parenthesized { expr: proc, metadata, .. } => {
                Arc::new(UnifiedIR::Block {
                    base,
                    body: UnifiedIR::from_rholang(proc),
                    metadata: metadata.clone(),
                })
            }

            // For everything else, use the language-specific extension
            _ => Arc::new(UnifiedIR::RholangExt {
                base,
                node: node.clone(),
                metadata: None,
            }),
        }
    }

    /// Get the base node information
    pub fn base(&self) -> &NodeBase {
        match self {
            UnifiedIR::Literal { base, .. } => base,
            UnifiedIR::Variable { base, .. } => base,
            UnifiedIR::Binding { base, .. } => base,
            UnifiedIR::Invocation { base, .. } => base,
            UnifiedIR::Match { base, .. } => base,
            UnifiedIR::Collection { base, .. } => base,
            UnifiedIR::Conditional { base, .. } => base,
            UnifiedIR::Block { base, .. } => base,
            UnifiedIR::Composition { base, .. } => base,
            UnifiedIR::RholangExt { base, .. } => base,
            UnifiedIR::MettaExt { base, .. } => base,
            UnifiedIR::Error { base, .. } => base,
        }
    }

    /// Get metadata
    pub fn metadata(&self) -> Option<&Metadata> {
        match self {
            UnifiedIR::Literal { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            UnifiedIR::Variable { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            UnifiedIR::Binding { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            UnifiedIR::Invocation { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            UnifiedIR::Match { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            UnifiedIR::Collection { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            UnifiedIR::Conditional { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            UnifiedIR::Block { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            UnifiedIR::Composition { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            UnifiedIR::RholangExt { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            UnifiedIR::MettaExt { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            UnifiedIR::Error { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
        }
    }

    /// Get the semantic node type
    pub fn node_type(&self) -> NodeType {
        match self {
            UnifiedIR::Literal { .. } => NodeType::Literal,
            UnifiedIR::Variable { .. } => NodeType::Variable,
            UnifiedIR::Binding { .. } => NodeType::Binding,
            UnifiedIR::Invocation { .. } => NodeType::Invocation,
            UnifiedIR::Match { .. } => NodeType::Match,
            UnifiedIR::Collection { .. } => NodeType::Collection,
            UnifiedIR::Conditional { .. } => NodeType::Conditional,
            UnifiedIR::Block { .. } => NodeType::Block,
            UnifiedIR::Composition { is_parallel: true, .. } => NodeType::RholangPar,
            UnifiedIR::Composition { is_parallel: false, .. } => NodeType::Block,
            UnifiedIR::RholangExt { .. } => NodeType::Unknown,
            UnifiedIR::MettaExt { .. } => NodeType::Unknown,
            UnifiedIR::Error { .. } => NodeType::Unknown,
        }
    }
}

// Implement SemanticNode for UnifiedIR
impl SemanticNode for UnifiedIR {
    fn base(&self) -> &NodeBase {
        UnifiedIR::base(self)
    }

    fn metadata(&self) -> Option<&Metadata> {
        UnifiedIR::metadata(self)
    }

    fn metadata_mut(&mut self) -> Option<&mut Metadata> {
        // Similar to Node, metadata is Arc-wrapped so mutation not supported
        None
    }

    fn node_type(&self) -> NodeType {
        UnifiedIR::node_type(self)
    }

    fn children(&self) -> Vec<&dyn SemanticNode> {
        // Simplified for now
        vec![]
    }

    fn children_arc(&self) -> Vec<Arc<dyn SemanticNode>> {
        // Simplified for now
        vec![]
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_literal_display() {
        assert_eq!(Literal::Bool(true).to_string(), "true");
        assert_eq!(Literal::Integer(42).to_string(), "42");
        assert_eq!(Literal::String("hello".to_string()).to_string(), "\"hello\"");
    }

    #[test]
    fn test_binding_kind() {
        assert_eq!(BindingKind::NewBind, BindingKind::NewBind);
        assert_ne!(BindingKind::NewBind, BindingKind::LetBind);
    }
}
