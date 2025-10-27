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

use super::semantic_node::{NodeBase, Metadata, SemanticNode, SemanticCategory};
use super::rholang_node::RholangNode;
use super::metta_node::MettaNode;

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
        node: Arc<RholangNode>,
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
    /// Convert a Rholang RholangNode to UnifiedIR
    pub fn from_rholang(node: &Arc<RholangNode>) -> Arc<UnifiedIR> {
        // Extract base from the node
        let base = node.base().clone();

        match &**node {
            // Literals
            RholangNode::BoolLiteral { value, metadata, .. } => Arc::new(UnifiedIR::Literal {
                base,
                value: Literal::Bool(*value),
                metadata: metadata.clone(),
            }),
            RholangNode::LongLiteral { value, metadata, .. } => Arc::new(UnifiedIR::Literal {
                base,
                value: Literal::Integer(*value),
                metadata: metadata.clone(),
            }),
            RholangNode::StringLiteral { value, metadata, .. } => Arc::new(UnifiedIR::Literal {
                base,
                value: Literal::String(value.clone()),
                metadata: metadata.clone(),
            }),
            RholangNode::UriLiteral { value, metadata, .. } => Arc::new(UnifiedIR::Literal {
                base,
                value: Literal::Uri(value.clone()),
                metadata: metadata.clone(),
            }),
            RholangNode::Nil { metadata, .. } | RholangNode::Wildcard { metadata, .. } => {
                Arc::new(UnifiedIR::Literal {
                    base,
                    value: Literal::Nil,
                    metadata: metadata.clone(),
                })
            }

            // Variables
            RholangNode::Var { name, metadata, .. } => Arc::new(UnifiedIR::Variable {
                base,
                name: name.clone(),
                metadata: metadata.clone(),
            }),

            // Collections
            RholangNode::List { elements, metadata, .. } => Arc::new(UnifiedIR::Collection {
                base,
                kind: CollectionKind::List,
                elements: elements.iter().map(UnifiedIR::from_rholang).collect(),
                metadata: metadata.clone(),
            }),
            RholangNode::Set { elements, metadata, .. } => Arc::new(UnifiedIR::Collection {
                base,
                kind: CollectionKind::Set,
                elements: elements.iter().map(UnifiedIR::from_rholang).collect(),
                metadata: metadata.clone(),
            }),
            RholangNode::Tuple { elements, metadata, .. } => Arc::new(UnifiedIR::Collection {
                base,
                kind: CollectionKind::Tuple,
                elements: elements.iter().map(UnifiedIR::from_rholang).collect(),
                metadata: metadata.clone(),
            }),

            // Parallel composition (Rholang-specific)
            RholangNode::Par { left: Some(left), right: Some(right), metadata, .. } => Arc::new(UnifiedIR::Composition {
                base,
                is_parallel: true,
                left: UnifiedIR::from_rholang(left),
                right: UnifiedIR::from_rholang(right),
                metadata: metadata.clone(),
            }),

            // Match
            RholangNode::Match { expression, cases, metadata, .. } => Arc::new(UnifiedIR::Match {
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
            RholangNode::IfElse { condition, consequence, alternative, metadata, .. } => {
                Arc::new(UnifiedIR::Conditional {
                    base,
                    condition: Some(UnifiedIR::from_rholang(condition)),
                    consequence: UnifiedIR::from_rholang(consequence),
                    alternative: alternative.as_ref().map(UnifiedIR::from_rholang),
                    metadata: metadata.clone(),
                })
            }

            // Block
            RholangNode::Block { proc, metadata, .. } | RholangNode::Parenthesized { expr: proc, metadata, .. } => {
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

    /// Convert a MeTTa RholangNode to UnifiedIR
    pub fn from_metta(node: &Arc<MettaNode>) -> Arc<UnifiedIR> {
        let base = node.base().clone();

        match &**node {
            // Literals
            MettaNode::Bool { value, metadata, .. } => Arc::new(UnifiedIR::Literal {
                base,
                value: Literal::Bool(*value),
                metadata: metadata.clone(),
            }),
            MettaNode::Integer { value, metadata, .. } => Arc::new(UnifiedIR::Literal {
                base,
                value: Literal::Integer(*value),
                metadata: metadata.clone(),
            }),
            MettaNode::Float { value, metadata, .. } => Arc::new(UnifiedIR::Literal {
                base,
                value: Literal::Float(*value),
                metadata: metadata.clone(),
            }),
            MettaNode::String { value, metadata, .. } => Arc::new(UnifiedIR::Literal {
                base,
                value: Literal::String(value.clone()),
                metadata: metadata.clone(),
            }),
            MettaNode::Nil { metadata, .. } => Arc::new(UnifiedIR::Literal {
                base,
                value: Literal::Nil,
                metadata: metadata.clone(),
            }),

            // Atom - treat as variable or literal depending on context
            MettaNode::Atom { name, metadata, .. } => Arc::new(UnifiedIR::Variable {
                base,
                name: name.clone(),
                metadata: metadata.clone(),
            }),

            // Variables
            MettaNode::Variable { name, metadata, .. } => Arc::new(UnifiedIR::Variable {
                base,
                name: name.clone(),
                metadata: metadata.clone(),
            }),

            // Definition (= pattern body) is a binding
            MettaNode::Definition { pattern, body, metadata, .. } => {
                Arc::new(UnifiedIR::Binding {
                    base,
                    kind: BindingKind::LetBind,
                    names: vec![UnifiedIR::from_metta(pattern)],
                    values: vec![UnifiedIR::from_metta(body)],
                    body: None,
                    metadata: metadata.clone(),
                })
            }

            // Let binding
            MettaNode::Let { bindings, body, metadata, .. } => Arc::new(UnifiedIR::Binding {
                base,
                kind: BindingKind::LetBind,
                names: bindings.iter().map(|(name, _)| UnifiedIR::from_metta(name)).collect(),
                values: bindings.iter().map(|(_, val)| UnifiedIR::from_metta(val)).collect(),
                body: Some(UnifiedIR::from_metta(body)),
                metadata: metadata.clone(),
            }),

            // Lambda
            MettaNode::Lambda { params, body, metadata, .. } => Arc::new(UnifiedIR::Binding {
                base,
                kind: BindingKind::Parameter,
                names: params.iter().map(UnifiedIR::from_metta).collect(),
                values: vec![],
                body: Some(UnifiedIR::from_metta(body)),
                metadata: metadata.clone(),
            }),

            // Match
            MettaNode::Match { scrutinee, cases, metadata, .. } => Arc::new(UnifiedIR::Match {
                base,
                scrutinee: UnifiedIR::from_metta(scrutinee),
                cases: cases
                    .iter()
                    .map(|(pat, body)| {
                        (UnifiedIR::from_metta(pat), UnifiedIR::from_metta(body))
                    })
                    .collect(),
                metadata: metadata.clone(),
            }),

            // If-then-else
            MettaNode::If { condition, consequence, alternative, metadata, .. } => {
                Arc::new(UnifiedIR::Conditional {
                    base,
                    condition: Some(UnifiedIR::from_metta(condition)),
                    consequence: UnifiedIR::from_metta(consequence),
                    alternative: alternative.as_ref().map(UnifiedIR::from_metta),
                    metadata: metadata.clone(),
                })
            }

            // Evaluation
            MettaNode::Eval { expr, metadata, .. } => Arc::new(UnifiedIR::Invocation {
                base,
                target: UnifiedIR::from_metta(expr),
                args: vec![],
                metadata: metadata.clone(),
            }),

            // S-Expression - treat as invocation if first element is a function
            MettaNode::SExpr { elements, metadata, .. } => {
                if elements.is_empty() {
                    Arc::new(UnifiedIR::Literal {
                        base,
                        value: Literal::Nil,
                        metadata: metadata.clone(),
                    })
                } else if elements.len() == 1 {
                    UnifiedIR::from_metta(&elements[0])
                } else {
                    // Treat as invocation: (f arg1 arg2 ...)
                    Arc::new(UnifiedIR::Invocation {
                        base,
                        target: UnifiedIR::from_metta(&elements[0]),
                        args: elements[1..].iter().map(UnifiedIR::from_metta).collect(),
                        metadata: metadata.clone(),
                    })
                }
            }

            // Type annotation - treat as binding with type info
            MettaNode::TypeAnnotation { expr, type_expr, metadata, .. } => {
                Arc::new(UnifiedIR::Binding {
                    base,
                    kind: BindingKind::PatternBind,
                    names: vec![UnifiedIR::from_metta(expr)],
                    values: vec![UnifiedIR::from_metta(type_expr)],
                    body: None,
                    metadata: metadata.clone(),
                })
            }

            // Error
            MettaNode::Error { message, children, metadata, .. } => Arc::new(UnifiedIR::Error {
                base,
                message: message.clone(),
                children: children.iter().map(UnifiedIR::from_metta).collect(),
                metadata: metadata.clone(),
            }),

            // Comment - treat as unknown
            MettaNode::Comment { metadata, .. } => Arc::new(UnifiedIR::MettaExt {
                base,
                node: node.clone() as Arc<dyn Any + Send + Sync>,
                metadata: metadata.clone(),
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

    /// Get the semantic category
    pub fn semantic_category(&self) -> SemanticCategory {
        match self {
            UnifiedIR::Literal { .. } => SemanticCategory::Literal,
            UnifiedIR::Variable { .. } => SemanticCategory::Variable,
            UnifiedIR::Binding { .. } => SemanticCategory::Binding,
            UnifiedIR::Invocation { .. } => SemanticCategory::Invocation,
            UnifiedIR::Match { .. } => SemanticCategory::Match,
            UnifiedIR::Collection { .. } => SemanticCategory::Collection,
            UnifiedIR::Conditional { .. } => SemanticCategory::Conditional,
            UnifiedIR::Block { .. } => SemanticCategory::Block,
            UnifiedIR::Composition { is_parallel: true, .. } => SemanticCategory::LanguageSpecific,
            UnifiedIR::Composition { is_parallel: false, .. } => SemanticCategory::Block,
            UnifiedIR::RholangExt { .. } | UnifiedIR::MettaExt { .. } => SemanticCategory::LanguageSpecific,
            UnifiedIR::Error { .. } => SemanticCategory::Unknown,
        }
    }

    /// Get the type name
    pub fn type_name_str(&self) -> &'static str {
        match self {
            UnifiedIR::Literal { .. } => "UnifiedIR::Literal",
            UnifiedIR::Variable { .. } => "UnifiedIR::Variable",
            UnifiedIR::Binding { .. } => "UnifiedIR::Binding",
            UnifiedIR::Invocation { .. } => "UnifiedIR::Invocation",
            UnifiedIR::Match { .. } => "UnifiedIR::Match",
            UnifiedIR::Collection { .. } => "UnifiedIR::Collection",
            UnifiedIR::Conditional { .. } => "UnifiedIR::Conditional",
            UnifiedIR::Block { .. } => "UnifiedIR::Block",
            UnifiedIR::Composition { is_parallel: true, .. } => "UnifiedIR::ParallelComposition",
            UnifiedIR::Composition { is_parallel: false, .. } => "UnifiedIR::SequentialComposition",
            UnifiedIR::RholangExt { .. } => "UnifiedIR::RholangExt",
            UnifiedIR::MettaExt { .. } => "UnifiedIR::MettaExt",
            UnifiedIR::Error { .. } => "UnifiedIR::Error",
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
        // Similar to RholangNode, metadata is Arc-wrapped so mutation not supported
        None
    }

    fn semantic_category(&self) -> SemanticCategory {
        UnifiedIR::semantic_category(self)
    }

    fn type_name(&self) -> &'static str {
        UnifiedIR::type_name_str(self)
    }

    fn children_count(&self) -> usize {
        match self {
            // Leaf nodes
            UnifiedIR::Literal { .. } | UnifiedIR::Variable { .. } => 0,

            // Nodes with dynamic children
            UnifiedIR::Binding { names, values, body, .. } => {
                names.len() + values.len() + if body.is_some() { 1 } else { 0 }
            }
            UnifiedIR::Invocation { target, args, .. } => {
                let _ = target;
                1 + args.len()
            }
            UnifiedIR::Match { scrutinee, cases, .. } => {
                let _ = scrutinee;
                1 + cases.len() * 2  // scrutinee + (pattern, body) for each case
            }
            UnifiedIR::Collection { elements, .. } => elements.len(),
            UnifiedIR::Conditional { condition, consequence, alternative, .. } => {
                let _ = consequence;
                (if condition.is_some() { 1 } else { 0 })
                    + 1  // consequence
                    + (if alternative.is_some() { 1 } else { 0 })
            }
            UnifiedIR::Block { body, .. } => {
                let _ = body;
                1
            }
            UnifiedIR::Composition { left, right, .. } => {
                let _ = (left, right);
                2
            }
            UnifiedIR::Error { children, .. } => children.len(),

            // Extension nodes - delegate to wrapped nodes
            UnifiedIR::RholangExt { node, .. } => node.children_count(),
            UnifiedIR::MettaExt { node, .. } => {
                // Downcast from Arc<dyn Any> to Arc<MettaNode>
                if let Some(metta_node) = node.downcast_ref::<MettaNode>() {
                    metta_node.children_count()
                } else {
                    0
                }
            }
        }
    }

    fn child_at(&self, index: usize) -> Option<&dyn SemanticNode> {
        match self {
            // Binding
            UnifiedIR::Binding { names, values, body, .. } => {
                if index < names.len() {
                    Some(&**names.get(index)?)
                } else if index < names.len() + values.len() {
                    Some(&**values.get(index - names.len())?)
                } else if index == names.len() + values.len() && body.is_some() {
                    body.as_ref().map(|b| &**b as &dyn SemanticNode)
                } else {
                    None
                }
            }

            // Invocation
            UnifiedIR::Invocation { target, args, .. } => {
                if index == 0 {
                    Some(&**target)
                } else if index <= args.len() {
                    Some(&**args.get(index - 1)?)
                } else {
                    None
                }
            }

            // Match
            UnifiedIR::Match { scrutinee, cases, .. } => {
                if index == 0 {
                    Some(&**scrutinee)
                } else {
                    let case_index = (index - 1) / 2;
                    if case_index < cases.len() {
                        let (pattern, body) = &cases[case_index];
                        if (index - 1) % 2 == 0 {
                            Some(&**pattern)
                        } else {
                            Some(&**body)
                        }
                    } else {
                        None
                    }
                }
            }

            // Collection
            UnifiedIR::Collection { elements, .. } => {
                elements.get(index).map(|e| &**e as &dyn SemanticNode)
            }

            // Conditional
            UnifiedIR::Conditional { condition, consequence, alternative, .. } => {
                let mut curr_index = 0;
                if condition.is_some() {
                    if index == curr_index {
                        return condition.as_ref().map(|c| &**c as &dyn SemanticNode);
                    }
                    curr_index += 1;
                }
                if index == curr_index {
                    return Some(&**consequence);
                }
                curr_index += 1;
                if alternative.is_some() && index == curr_index {
                    return alternative.as_ref().map(|alt| &**alt as &dyn SemanticNode);
                }
                None
            }

            // Block
            UnifiedIR::Block { body, .. } if index == 0 => Some(&**body),

            // Composition
            UnifiedIR::Composition { left, right, .. } => match index {
                0 => Some(&**left),
                1 => Some(&**right),
                _ => None,
            },

            // Error
            UnifiedIR::Error { children, .. } => {
                children.get(index).map(|c| &**c as &dyn SemanticNode)
            }

            // Extension nodes - delegate to wrapped nodes
            UnifiedIR::RholangExt { node, .. } => node.child_at(index),
            UnifiedIR::MettaExt { node, .. } => {
                // Downcast from Arc<dyn Any> to Arc<MettaNode>
                if let Some(metta_node) = node.downcast_ref::<MettaNode>() {
                    metta_node.child_at(index)
                } else {
                    None
                }
            }

            // Leaf nodes
            _ => None,
        }
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
