use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;
use tree_sitter::Node as TSNode;
use rpds::Vector;
use archery::ArcK;
use tracing::trace;

pub type NodeVector<'a> = Vector<Arc<Node<'a>>, ArcK>;
pub type NodePairVector<'a> = Vector<(Arc<Node<'a>>, Arc<Node<'a>>), ArcK>;
pub type BranchVector<'a> = Vector<(NodeVector<'a>, Arc<Node<'a>>), ArcK>;
pub type ReceiptVector<'a> = Vector<NodeVector<'a>, ArcK>;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Position {
    pub row: usize,    // Line number (0-based)
    pub column: usize, // Column number (0-based)
    pub byte: usize,   // Byte offset from the start of the source code
}

/// Base structure for all Intermediate Representation (IR) nodes, encapsulating positional and textual metadata.
/// Provides the foundation for tracking node locations and source text.
#[derive(Debug, Clone)]
pub struct NodeBase<'a> {
    ts_node: Option<TSNode<'a>>,         // Optional reference to the Tree-Sitter node, if available
    relative_start: RelativePosition,    // Position relative to the previous node's end
    length: usize,                       // Length of the node's text in bytes
    text: Option<String>,                // Source text of the node, None if transformed
}

impl<'a> NodeBase<'a> {
    /// Creates a new `NodeBase` instance with the specified attributes.
    pub fn new(
        ts_node: Option<TSNode<'a>>,
        relative_start: RelativePosition,
        length: usize,
        text: Option<String>,
    ) -> Self {
        NodeBase {
            ts_node,
            relative_start,
            length,
            text,
        }
    }

    /// Returns the relative start position of the node.
    pub fn relative_start(&self) -> RelativePosition { self.relative_start }
    /// Returns the length of the node's text in bytes.
    pub fn length(&self) -> usize { self.length }
    /// Returns the source text of the node, if available.
    pub fn text(&self) -> Option<&String> { self.text.as_ref() }
    /// Returns the Tree-Sitter node reference, if present.
    pub fn ts_node(&self) -> Option<TSNode<'a>> { self.ts_node }
}

/// Represents all possible constructs in the Rholang Intermediate Representation (IR).
/// Each variant corresponds to a syntactic element in Rholang, such as processes, expressions, or bindings.
///
/// # Examples
/// - `Par`: Parallel composition of two processes (e.g., `P | Q`).
/// - `Send`: Asynchronous message send (e.g., `ch!("msg")`).
/// - `Var`: Variable reference (e.g., `x` in `x!()`).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum Node<'a> {
    /// Parallel composition of two processes.
    Par { base: NodeBase<'a>, left: Arc<Node<'a>>, right: Arc<Node<'a>>, metadata: Option<Arc<Metadata>> },
    /// Synchronous send with a continuation process.
    SendSync { base: NodeBase<'a>, channel: Arc<Node<'a>>, inputs: NodeVector<'a>, cont: Arc<Node<'a>>, metadata: Option<Arc<Metadata>> },
    /// Asynchronous send operation on a channel.
    Send { base: NodeBase<'a>, channel: Arc<Node<'a>>, send_type: SendType, send_type_end: Position, inputs: NodeVector<'a>, metadata: Option<Arc<Metadata>> },
    /// Declaration of new names with a scoped process.
    New { base: NodeBase<'a>, decls: NodeVector<'a>, proc: Arc<Node<'a>>, metadata: Option<Arc<Metadata>> },
    /// Conditional branching with optional else clause.
    IfElse { base: NodeBase<'a>, condition: Arc<Node<'a>>, consequence: Arc<Node<'a>>, alternative: Option<Arc<Node<'a>>>, metadata: Option<Arc<Metadata>> },
    /// Variable binding with a subsequent process.
    Let { base: NodeBase<'a>, decls: NodeVector<'a>, proc: Arc<Node<'a>>, metadata: Option<Arc<Metadata>> },
    /// Access-controlled process with a bundle type.
    Bundle { base: NodeBase<'a>, bundle_type: BundleType, proc: Arc<Node<'a>>, metadata: Option<Arc<Metadata>> },
    /// Pattern matching construct with cases.
    Match { base: NodeBase<'a>, expression: Arc<Node<'a>>, cases: NodePairVector<'a>, metadata: Option<Arc<Metadata>> },
    /// Non-deterministic choice among branches.
    Choice { base: NodeBase<'a>, branches: BranchVector<'a>, metadata: Option<Arc<Metadata>> },
    /// Contract definition with name, parameters, and body.
    Contract { base: NodeBase<'a>, name: Arc<Node<'a>>, formals: NodeVector<'a>, formals_remainder: Option<Arc<Node<'a>>>, proc: Arc<Node<'a>>, metadata: Option<Arc<Metadata>> },
    /// Input binding from channels with a process.
    Input { base: NodeBase<'a>, receipts: ReceiptVector<'a>, proc: Arc<Node<'a>>, metadata: Option<Arc<Metadata>> },
    /// Block of a single process (e.g., `{ P }`).
    Block { base: NodeBase<'a>, proc: Arc<Node<'a>>, metadata: Option<Arc<Metadata>> },
    /// Parenthesized expression (e.g., `(P)`).
    Parenthesized { base: NodeBase<'a>, expr: Arc<Node<'a>>, metadata: Option<Arc<Metadata>> },
    /// Binary operation (e.g., `P + Q`).
    BinOp { base: NodeBase<'a>, op: BinOperator, left: Arc<Node<'a>>, right: Arc<Node<'a>>, metadata: Option<Arc<Metadata>> },
    /// Unary operation (e.g., `-P` or `not P`).
    UnaryOp { base: NodeBase<'a>, op: UnaryOperator, operand: Arc<Node<'a>>, metadata: Option<Arc<Metadata>> },
    /// Method call on a receiver (e.g., `obj.method(args)`).
    Method { base: NodeBase<'a>, receiver: Arc<Node<'a>>, name: String, args: NodeVector<'a>, metadata: Option<Arc<Metadata>> },
    /// Evaluation of a name (e.g., `*name`).
    Eval { base: NodeBase<'a>, name: Arc<Node<'a>>, metadata: Option<Arc<Metadata>> },
    /// Quotation of a process (e.g., `@P`).
    Quote { base: NodeBase<'a>, quotable: Arc<Node<'a>>, metadata: Option<Arc<Metadata>> },
    /// Variable reference with assignment kind.
    VarRef { base: NodeBase<'a>, kind: VarRefKind, var: Arc<Node<'a>>, metadata: Option<Arc<Metadata>> },
    /// Boolean literal (e.g., `true` or `false`).
    BoolLiteral { base: NodeBase<'a>, value: bool, metadata: Option<Arc<Metadata>> },
    /// Integer literal (e.g., `42`).
    LongLiteral { base: NodeBase<'a>, value: i64, metadata: Option<Arc<Metadata>> },
    /// String literal (e.g., `"hello"`).
    StringLiteral { base: NodeBase<'a>, value: String, metadata: Option<Arc<Metadata>> },
    /// URI literal (e.g., `` `http://example.com` ``).
    UriLiteral { base: NodeBase<'a>, value: String, metadata: Option<Arc<Metadata>> },
    /// Empty process (e.g., `Nil`).
    Nil { base: NodeBase<'a>, metadata: Option<Arc<Metadata>> },
    /// List collection (e.g., `[1, 2, 3]`).
    List { base: NodeBase<'a>, elements: NodeVector<'a>, remainder: Option<Arc<Node<'a>>>, metadata: Option<Arc<Metadata>> },
    /// Set collection (e.g., `Set(1, 2, 3)`).
    Set { base: NodeBase<'a>, elements: NodeVector<'a>, remainder: Option<Arc<Node<'a>>>, metadata: Option<Arc<Metadata>> },
    /// Map collection (e.g., `{k: v}`).
    Map { base: NodeBase<'a>, pairs: NodePairVector<'a>, remainder: Option<Arc<Node<'a>>>, metadata: Option<Arc<Metadata>> },
    /// Tuple collection (e.g., `(1, 2)`).
    Tuple { base: NodeBase<'a>, elements: NodeVector<'a>, metadata: Option<Arc<Metadata>> },
    /// Variable identifier (e.g., `x`).
    Var { base: NodeBase<'a>, name: String, metadata: Option<Arc<Metadata>> },
    /// Name declaration in a `new` construct (e.g., `x` or `x(uri)`).
    NameDecl { base: NodeBase<'a>, var: Arc<Node<'a>>, uri: Option<Arc<Node<'a>>>, metadata: Option<Arc<Metadata>> },
    /// Declaration in a `let` statement (e.g., `x = P`).
    Decl { base: NodeBase<'a>, names: NodeVector<'a>, names_remainder: Option<Arc<Node<'a>>>, procs: NodeVector<'a>, metadata: Option<Arc<Metadata>> },
    /// Linear binding in a `for` (e.g., `x <- ch`).
    LinearBind { base: NodeBase<'a>, names: NodeVector<'a>, remainder: Option<Arc<Node<'a>>>, source: Arc<Node<'a>>, metadata: Option<Arc<Metadata>> },
    /// Repeated binding in a `for` (e.g., `x <= ch`).
    RepeatedBind { base: NodeBase<'a>, names: NodeVector<'a>, remainder: Option<Arc<Node<'a>>>, source: Arc<Node<'a>>, metadata: Option<Arc<Metadata>> },
    /// Peek binding in a `for` (e.g., `x <<- ch`).
    PeekBind { base: NodeBase<'a>, names: NodeVector<'a>, remainder: Option<Arc<Node<'a>>>, source: Arc<Node<'a>>, metadata: Option<Arc<Metadata>> },
    /// Comment in the source code (e.g., `// text` or `/* text */`).
    Comment { base: NodeBase<'a>, kind: CommentKind, metadata: Option<Arc<Metadata>> },
    /// Wildcard pattern (e.g., `_`).
    Wildcard { base: NodeBase<'a>, metadata: Option<Arc<Metadata>> },
    /// Simple type annotation (e.g., `Bool`).
    SimpleType { base: NodeBase<'a>, value: String, metadata: Option<Arc<Metadata>> },
    /// Receive-send source (e.g., `ch?!`).
    ReceiveSendSource { base: NodeBase<'a>, name: Arc<Node<'a>>, metadata: Option<Arc<Metadata>> },
    /// Send-receive source (e.g., `ch!?(args)`).
    SendReceiveSource { base: NodeBase<'a>, name: Arc<Node<'a>>, inputs: NodeVector<'a>, metadata: Option<Arc<Metadata>> },
    /// Represents a syntax error in the source code with its erroneous subtree.
    Error { base: NodeBase<'a>, children: NodeVector<'a>, metadata: Option<Arc<Metadata>> },
}

#[derive(Clone, PartialEq, Debug, Hash)]
pub enum BundleType { Read, Write, Equiv, ReadWrite }

#[derive(Clone, PartialEq, Debug, Hash)]
pub enum SendType { Single, Multiple }

#[derive(Clone, PartialEq, Debug, Hash)]
pub enum BinOperator { Or, And, Matches, Eq, Neq, Lt, Lte, Gt, Gte, Concat, Diff, Add, Sub, Interpolation, Mult, Div, Mod, Disjunction, Conjunction }

#[derive(Clone, PartialEq, Debug, Hash)]
pub enum UnaryOperator { Not, Neg, Negation }

#[derive(Clone, PartialEq, Debug, Hash)]
pub enum VarRefKind { Bind, Unforgeable }

#[derive(Clone, PartialEq, Debug, Hash)]
pub enum CommentKind { Line, Block }

#[derive(Clone, Debug)]
pub struct Metadata {
    pub data: HashMap<String, Arc<dyn Any + Send + Sync>>,
}

impl Metadata {
    /// Retrieves the version from the metadata data map, defaulting to 0 if absent.
    pub fn get_version(&self) -> usize {
        self.data.get("version")
            .and_then(|v| v.downcast_ref::<usize>())
            .cloned()
            .unwrap_or(0)
    }

    /// Sets the version in the metadata data map.
    pub fn set_version(&mut self, version: usize) {
        self.data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
    }
}

/// Computes absolute positions for all nodes in the IR tree, storing them in a HashMap.
/// Positions are keyed by the Tree-Sitter node ID or 0 if no Tree-Sitter node exists.
///
/// # Arguments
/// * `root` - The root node of the IR tree.
///
/// # Returns
/// A HashMap mapping node keys to tuples of (start, end) `Position`s.
pub fn compute_absolute_positions<'a>(root: &Arc<Node<'a>>) -> HashMap<usize, (Position, Position)> {
    let mut positions = HashMap::new();
    let initial_prev_end = Position { row: 0, column: 0, byte: 0 };
    compute_positions_helper(root, initial_prev_end, &mut positions);
    trace!("Computed positions for {} nodes", positions.len());
    positions
}

/// Recursively computes absolute positions for all node types in the IR tree.
/// - Uses Tree-Sitter positions directly if available.
/// - Otherwise, computes positions from relative offsets and child nodes.
///
/// # Arguments
/// * `node` - The current node being processed.
/// * `prev_end` - The absolute end position of the previous sibling or parent’s start if first child.
/// * `positions` - The HashMap storing computed (start, end) positions.
///
/// # Returns
/// The absolute end position of the current node.
#[allow(unused_assignments)]
fn compute_positions_helper<'a>(
    node: &Arc<Node<'a>>,
    prev_end: Position,
    positions: &mut HashMap<usize, (Position, Position)>,
) -> Position {
    let base = node.base();
    let key = base.ts_node().map_or(0, |n| n.id());

    if let Some(ts_node) = base.ts_node() {
        let start = Position {
            row: ts_node.start_position().row,
            column: ts_node.start_position().column,
            byte: ts_node.start_byte(),
        };
        let end = Position {
            row: ts_node.end_position().row,
            column: ts_node.end_position().column,
            byte: ts_node.end_byte(),
        };
        positions.insert(key, (start, end));
        trace!(
            "Node '{}': key={}, ts_node positions: start={:?}, end={:?}",
            base.text().map_or("Unknown", |v| v), key, start, end
        );

        let mut current_end = start;
        match &**node {
            Node::Par { left, right, .. } => {
                current_end = compute_positions_helper(left, start, positions);
                compute_positions_helper(right, current_end, positions)
            }
            Node::SendSync { channel, inputs, cont, .. } => {
                current_end = compute_positions_helper(channel, start, positions);
                current_end = inputs.iter().fold(current_end, |prev, input| {
                    compute_positions_helper(input, prev, positions)
                });
                compute_positions_helper(cont, current_end, positions)
            }
            Node::Send { channel, inputs, send_type_end, .. } => {
                compute_positions_helper(channel, start, positions);
                inputs.iter().fold(*send_type_end, |prev, input| {
                    compute_positions_helper(input, prev, positions)
                })
            }
            Node::New { decls, proc, .. } => {
                current_end = decls.iter().fold(start, |prev, decl| {
                    compute_positions_helper(decl, prev, positions)
                });
                compute_positions_helper(proc, current_end, positions)
            }
            Node::IfElse { condition, consequence, alternative, .. } => {
                current_end = compute_positions_helper(condition, start, positions);
                current_end = compute_positions_helper(consequence, current_end, positions);
                alternative.as_ref().map_or(end, |alt| {
                    compute_positions_helper(alt, current_end, positions)
                })
            }
            Node::Let { decls, proc, .. } => {
                current_end = decls.iter().fold(start, |prev, decl| {
                    compute_positions_helper(decl, prev, positions)
                });
                compute_positions_helper(proc, current_end, positions)
            }
            Node::Bundle { proc, .. } => {
                compute_positions_helper(proc, start, positions)
            }
            Node::Match { expression, cases, .. } => {
                current_end = compute_positions_helper(expression, start, positions);
                cases.iter().fold(current_end, |prev, (pattern, proc)| {
                    let pat_end = compute_positions_helper(pattern, prev, positions);
                    compute_positions_helper(proc, pat_end, positions)
                })
            }
            Node::Choice { branches, .. } => {
                branches.iter().fold(start, |prev, (inputs, proc)| {
                    let inputs_end = inputs.iter().fold(prev, |acc, input| {
                        compute_positions_helper(input, acc, positions)
                    });
                    compute_positions_helper(proc, inputs_end, positions)
                })
            }
            Node::Contract { name, formals, formals_remainder, proc, .. } => {
                current_end = compute_positions_helper(name, start, positions);
                current_end = formals.iter().fold(current_end, |prev, formal| {
                    compute_positions_helper(formal, prev, positions)
                });
                current_end = if let Some(rem) = formals_remainder {
                    compute_positions_helper(rem, current_end, positions)
                } else {
                    current_end
                };
                compute_positions_helper(proc, current_end, positions)
            }
            Node::Input { receipts, proc, .. } => {
                current_end = receipts.iter().fold(start, |prev, receipt| {
                    receipt.iter().fold(prev, |acc, bind| {
                        compute_positions_helper(bind, acc, positions)
                    })
                });
                compute_positions_helper(proc, current_end, positions)
            }
            Node::Block { proc, .. } => {
                compute_positions_helper(proc, start, positions)
            }
            Node::Parenthesized { expr, .. } => {
                compute_positions_helper(expr, start, positions)
            }
            Node::BinOp { left, right, .. } => {
                current_end = compute_positions_helper(left, start, positions);
                compute_positions_helper(right, current_end, positions)
            }
            Node::UnaryOp { operand, .. } => {
                compute_positions_helper(operand, start, positions)
            }
            Node::Method { receiver, args, .. } => {
                current_end = compute_positions_helper(receiver, start, positions);
                args.iter().fold(current_end, |prev, arg| {
                    compute_positions_helper(arg, prev, positions)
                })
            }
            Node::Eval { name, .. } => {
                compute_positions_helper(name, start, positions)
            }
            Node::Quote { quotable, .. } => {
                compute_positions_helper(quotable, start, positions)
            }
            Node::VarRef { var, .. } => {
                compute_positions_helper(var, start, positions)
            }
            Node::List { elements, remainder, .. } => {
                current_end = elements.iter().fold(start, |prev, elem| {
                    compute_positions_helper(elem, prev, positions)
                });
                remainder.as_ref().map_or(end, |rem| {
                    compute_positions_helper(rem, current_end, positions)
                })
            }
            Node::Set { elements, remainder, .. } => {
                current_end = elements.iter().fold(start, |prev, elem| {
                    compute_positions_helper(elem, prev, positions)
                });
                remainder.as_ref().map_or(end, |rem| {
                    compute_positions_helper(rem, current_end, positions)
                })
            }
            Node::Map { pairs, remainder, .. } => {
                current_end = pairs.iter().fold(start, |prev, (key, value)| {
                    let key_end = compute_positions_helper(key, prev, positions);
                    compute_positions_helper(value, key_end, positions)
                });
                remainder.as_ref().map_or(end, |rem| {
                    compute_positions_helper(rem, current_end, positions)
                })
            }
            Node::Tuple { elements, .. } => {
                elements.iter().fold(start, |prev, elem| {
                    compute_positions_helper(elem, prev, positions)
                })
            }
            Node::NameDecl { var, uri, .. } => {
                current_end = compute_positions_helper(var, start, positions);
                uri.as_ref().map_or(end, |u| {
                    compute_positions_helper(u, current_end, positions)
                })
            }
            Node::Decl { names, names_remainder, procs, .. } => {
                current_end = names.iter().fold(start, |prev, name| {
                    compute_positions_helper(name, prev, positions)
                });
                current_end = if let Some(rem) = names_remainder {
                    compute_positions_helper(rem, current_end, positions)
                } else {
                    current_end
                };
                procs.iter().fold(current_end, |prev, proc| {
                    compute_positions_helper(proc, prev, positions)
                })
            }
            Node::LinearBind { names, remainder, source, .. } => {
                current_end = names.iter().fold(start, |prev, name| {
                    compute_positions_helper(name, prev, positions)
                });
                current_end = if let Some(rem) = remainder {
                    compute_positions_helper(rem, current_end, positions)
                } else {
                    current_end
                };
                compute_positions_helper(source, current_end, positions)
            }
            Node::RepeatedBind { names, remainder, source, .. } => {
                current_end = names.iter().fold(start, |prev, name| {
                    compute_positions_helper(name, prev, positions)
                });
                current_end = if let Some(rem) = remainder {
                    compute_positions_helper(rem, current_end, positions)
                } else {
                    current_end
                };
                compute_positions_helper(source, current_end, positions)
            }
            Node::PeekBind { names, remainder, source, .. } => {
                current_end = names.iter().fold(start, |prev, name| {
                    compute_positions_helper(name, prev, positions)
                });
                current_end = if let Some(rem) = remainder {
                    compute_positions_helper(rem, current_end, positions)
                } else {
                    current_end
                };
                compute_positions_helper(source, current_end, positions)
            }
            Node::ReceiveSendSource { name, .. } => {
                compute_positions_helper(name, start, positions)
            }
            Node::SendReceiveSource { name, inputs, .. } => {
                current_end = compute_positions_helper(name, start, positions);
                inputs.iter().fold(current_end, |prev, input| {
                    compute_positions_helper(input, prev, positions)
                })
            }
            Node::Error { children, .. } => {
                children.iter().fold(start, |prev, child| {
                    compute_positions_helper(child, prev, positions)
                })
            }
            _ => end, // Leaf nodes use Tree-Sitter’s end
        }
    } else {
        let relative_start = base.relative_start();
        let start = Position {
            row: (prev_end.row as i32 + relative_start.delta_lines) as usize,
            column: if relative_start.delta_lines == 0 {
                (prev_end.column as i32 + relative_start.delta_columns) as usize
            } else {
                relative_start.delta_columns as usize
            },
            byte: prev_end.byte + relative_start.delta_bytes,
        };
        let end = match &**node {
            Node::Par { left, right, .. } => {
                let left_end = compute_positions_helper(left, start, positions);
                compute_positions_helper(right, left_end, positions)
            }
            Node::SendSync { channel, inputs, cont, .. } => {
                let chan_end = compute_positions_helper(channel, start, positions);
                let inputs_end = inputs.iter().fold(chan_end, |prev, input| {
                    compute_positions_helper(input, prev, positions)
                });
                compute_positions_helper(cont, inputs_end, positions)
            }
            Node::Send { channel, inputs, send_type_end, .. } => {
                let _chan_end = compute_positions_helper(channel, start, positions);
                inputs.iter().fold(*send_type_end, |prev, input| {
                    compute_positions_helper(input, prev, positions)
                })
            }
            Node::New { decls, proc, .. } => {
                let decls_end = decls.iter().fold(start, |prev, decl| {
                    compute_positions_helper(decl, prev, positions)
                });
                compute_positions_helper(proc, decls_end, positions)
            }
            Node::IfElse { condition, consequence, alternative, .. } => {
                let cond_end = compute_positions_helper(condition, start, positions);
                let cons_end = compute_positions_helper(consequence, cond_end, positions);
                alternative.as_ref().map_or(cons_end, |alt| {
                    compute_positions_helper(alt, cons_end, positions)
                })
            }
            Node::Let { decls, proc, .. } => {
                let decls_end = decls.iter().fold(start, |prev, decl| {
                    compute_positions_helper(decl, prev, positions)
                });
                compute_positions_helper(proc, decls_end, positions)
            }
            Node::Bundle { proc, .. } => {
                compute_positions_helper(proc, start, positions)
            }
            Node::Match { expression, cases, .. } => {
                let expr_end = compute_positions_helper(expression, start, positions);
                cases.iter().fold(expr_end, |prev, (pattern, proc)| {
                    let pat_end = compute_positions_helper(pattern, prev, positions);
                    compute_positions_helper(proc, pat_end, positions)
                })
            }
            Node::Choice { branches, .. } => {
                branches.iter().fold(start, |prev, (inputs, proc)| {
                    let inputs_end = inputs.iter().fold(prev, |acc, input| {
                        compute_positions_helper(input, acc, positions)
                    });
                    compute_positions_helper(proc, inputs_end, positions)
                })
            }
            Node::Contract { name, formals, formals_remainder, proc, .. } => {
                let name_end = compute_positions_helper(name, start, positions);
                let formals_end = formals.iter().fold(name_end, |prev, formal| {
                    compute_positions_helper(formal, prev, positions)
                });
                let formals_rem_end = formals_remainder.as_ref().map_or(formals_end, |rem| {
                    compute_positions_helper(rem, formals_end, positions)
                });
                compute_positions_helper(proc, formals_rem_end, positions)
            }
            Node::Input { receipts, proc, .. } => {
                let receipts_end = receipts.iter().fold(start, |prev, receipt| {
                    receipt.iter().fold(prev, |acc, bind| {
                        compute_positions_helper(bind, acc, positions)
                    })
                });
                compute_positions_helper(proc, receipts_end, positions)
            }
            Node::Block { proc, .. } => {
                compute_positions_helper(proc, start, positions)
            }
            Node::Parenthesized { expr, .. } => {
                compute_positions_helper(expr, start, positions)
            }
            Node::BinOp { left, right, .. } => {
                let left_end = compute_positions_helper(left, start, positions);
                compute_positions_helper(right, left_end, positions)
            }
            Node::UnaryOp { operand, .. } => {
                compute_positions_helper(operand, start, positions)
            }
            Node::Method { receiver, args, .. } => {
                let recv_end = compute_positions_helper(receiver, start, positions);
                args.iter().fold(recv_end, |prev, arg| {
                    compute_positions_helper(arg, prev, positions)
                })
            }
            Node::Eval { name, .. } => {
                compute_positions_helper(name, start, positions)
            }
            Node::Quote { quotable, .. } => {
                compute_positions_helper(quotable, start, positions)
            }
            Node::VarRef { var, .. } => {
                compute_positions_helper(var, start, positions)
            }
            Node::List { elements, remainder, .. } => {
                let elems_end = elements.iter().fold(start, |prev, elem| {
                    compute_positions_helper(elem, prev, positions)
                });
                remainder.as_ref().map_or(elems_end, |rem| {
                    compute_positions_helper(rem, elems_end, positions)
                })
            }
            Node::Set { elements, remainder, .. } => {
                let elems_end = elements.iter().fold(start, |prev, elem| {
                    compute_positions_helper(elem, prev, positions)
                });
                remainder.as_ref().map_or(elems_end, |rem| {
                    compute_positions_helper(rem, elems_end, positions)
                })
            }
            Node::Map { pairs, remainder, .. } => {
                let pairs_end = pairs.iter().fold(start, |prev, (key, value)| {
                    let key_end = compute_positions_helper(key, prev, positions);
                    compute_positions_helper(value, key_end, positions)
                });
                remainder.as_ref().map_or(pairs_end, |rem| {
                    compute_positions_helper(rem, pairs_end, positions)
                })
            }
            Node::Tuple { elements, .. } => {
                elements.iter().fold(start, |prev, elem| {
                    compute_positions_helper(elem, prev, positions)
                })
            }
            Node::NameDecl { var, uri, .. } => {
                let var_end = compute_positions_helper(var, start, positions);
                uri.as_ref().map_or(var_end, |u| {
                    compute_positions_helper(u, var_end, positions)
                })
            }
            Node::Decl { names, names_remainder, procs, .. } => {
                let names_end = names.iter().fold(start, |prev, name| {
                    compute_positions_helper(name, prev, positions)
                });
                let names_rem_end = names_remainder.as_ref().map_or(names_end, |rem| {
                    compute_positions_helper(rem, names_end, positions)
                });
                procs.iter().fold(names_rem_end, |prev, proc| {
                    compute_positions_helper(proc, prev, positions)
                })
            }
            Node::LinearBind { names, remainder, source, .. } => {
                let names_end = names.iter().fold(start, |prev, name| {
                    compute_positions_helper(name, prev, positions)
                });
                let rem_end = remainder.as_ref().map_or(names_end, |rem| {
                    compute_positions_helper(rem, names_end, positions)
                });
                compute_positions_helper(source, rem_end, positions)
            }
            Node::RepeatedBind { names, remainder, source, .. } => {
                let names_end = names.iter().fold(start, |prev, name| {
                    compute_positions_helper(name, prev, positions)
                });
                let rem_end = remainder.as_ref().map_or(names_end, |rem| {
                    compute_positions_helper(rem, names_end, positions)
                });
                compute_positions_helper(source, rem_end, positions)
            }
            Node::PeekBind { names, remainder, source, .. } => {
                let names_end = names.iter().fold(start, |prev, name| {
                    compute_positions_helper(name, prev, positions)
                });
                let rem_end = remainder.as_ref().map_or(names_end, |rem| {
                    compute_positions_helper(rem, names_end, positions)
                });
                compute_positions_helper(source, rem_end, positions)
            }
            Node::ReceiveSendSource { name, .. } => {
                compute_positions_helper(name, start, positions)
            }
            Node::SendReceiveSource { name, inputs, .. } => {
                let name_end = compute_positions_helper(name, start, positions);
                inputs.iter().fold(name_end, |prev, input| {
                    compute_positions_helper(input, prev, positions)
                })
            }
            Node::Error { children, .. } => {
                children.iter().fold(start, |prev, child| {
                    compute_positions_helper(child, prev, positions)
                })
            }
            _ => compute_end_position(start, base.length, base.text()),
        };
        positions.insert(key, (start, end));
        trace!(
            "Node '{}': key={}, computed positions: start={:?}, end={:?}",
            base.text().map_or("Unknown", |v| v), key, start, end
        );
        end
    }
}

/// Computes the absolute end position of a node given its start position, length, and optional text.
/// Adjusts row and column based on newlines in the text.
///
/// # Arguments
/// * `start` - The absolute start position.
/// * `length` - The length of the node’s text in bytes.
/// * `text` - Optional source text for precise newline handling.
///
/// # Returns
/// The computed absolute end position.
pub fn compute_end_position(start: Position, length: usize, text: Option<&String>) -> Position {
    let mut row = start.row;
    let mut column = start.column;
    let byte = start.byte + length;

    if let Some(text) = text {
        for c in text.chars() {
            if c == '\n' {
                row += 1;
                column = 0;
            } else {
                column += c.len_utf8(); // Accurate column increment for UTF-8 chars
            }
        }
    } else {
        column += length; // Fallback assumes single-byte chars
    }

    Position { row, column, byte }
}

impl<'a> Node<'a> {
    /// Computes the byte offset from a line and character position in the source text.
    pub fn byte_offset_from_position(text: &str, line: usize, character: usize) -> Option<usize> {
        let mut byte_offset = 0;
        for (i, line_str) in text.lines().enumerate() {
            if i == line {
                let char_index = character.min(line_str.len());
                byte_offset += line_str.as_bytes()[..char_index].len();
                return Some(byte_offset);
            }
            byte_offset += line_str.len() + 1; // +1 for newline
        }
        None
    }

    /// Returns the starting line number of the node within the source code.
    ///
    /// # Arguments
    /// * `root` - The root node of the IR tree, used for position computation.
    pub fn start_line(&self, root: &Arc<Node<'a>>) -> usize {
        let positions = compute_absolute_positions(root);
        let key = self.base().ts_node().map_or(0, |n| n.id());
        positions.get(&key).expect("Node not found").0.row
    }

    /// Returns the starting column number of the node within the source code.
    ///
    /// # Arguments
    /// * `root` - The root node of the IR tree, used for position computation.
    pub fn start_column(&self, root: &Arc<Node<'a>>) -> usize {
        let positions = compute_absolute_positions(root);
        let key = self.base().ts_node().map_or(0, |n| n.id());
        positions.get(&key).expect("Node not found").0.column
    }

    /// Returns the ending line number of the node within the source code.
    ///
    /// # Arguments
    /// * `root` - The root node of the IR tree, used for position computation.
    pub fn end_line(&self, root: &Arc<Node<'a>>) -> usize {
        let positions = compute_absolute_positions(root);
        let key = self.base().ts_node().map_or(0, |n| n.id());
        positions.get(&key).expect("Node not found").1.row
    }

    /// Returns the ending column number of the node within the source code.
    ///
    /// # Arguments
    /// * `root` - The root node of the IR tree, used for position computation.
    pub fn end_column(&self, root: &Arc<Node<'a>>) -> usize {
        let positions = compute_absolute_positions(root);
        let key = self.base().ts_node().map_or(0, |n| n.id());
        positions.get(&key).expect("Node not found").1.column
    }

    /// Returns the byte offset of the node’s start position in the source code.
    ///
    /// # Arguments
    /// * `root` - The root node of the IR tree, used for position computation.
    pub fn position(&self, root: &Arc<Node<'a>>) -> usize {
        let positions = compute_absolute_positions(root);
        let key = self.base().ts_node().map_or(0, |n| n.id());
        positions.get(&key).expect("Node not found").0.byte
    }

    /// Returns the length of the node’s text in bytes.
    pub fn length(&self) -> usize { self.base().length }

    /// Returns the absolute start position of the node in the source code.
    ///
    /// # Arguments
    /// * `root` - The root node of the IR tree, used for position computation.
    pub fn absolute_start(&self, root: &Arc<Node<'a>>) -> Position {
        let positions = compute_absolute_positions(root);
        let key = self.base().ts_node().map_or(0, |n| n.id());
        positions.get(&key).expect("Node not found").0
    }

    /// Returns the absolute end position of the node in the source code.
    ///
    /// # Arguments
    /// * `root` - The root node of the IR tree, used for position computation.
    pub fn absolute_end(&self, root: &Arc<Node<'a>>) -> Position {
        let positions = compute_absolute_positions(root);
        let key = self.base().ts_node().map_or(0, |n| n.id());
        positions.get(&key).expect("Node not found").1
    }

    /// Creates a new node with the same fields but a different `NodeBase`.
    ///
    /// # Arguments
    /// * `new_base` - The new `NodeBase` to apply to the node.
    ///
    /// # Returns
    /// A new `Arc<Node>` with the updated base.
    pub fn with_base(&self, new_base: NodeBase<'a>) -> Arc<Node<'a>> {
        match self {
            Node::Par { metadata, left, right, .. } => Arc::new(Node::Par {
                base: new_base,
                left: left.clone(),
                right: right.clone(),
                metadata: metadata.clone(),
            }),
            Node::SendSync { channel, inputs, cont, metadata, .. } => Arc::new(Node::SendSync {
                base: new_base,
                channel: channel.clone(),
                inputs: inputs.clone(),
                cont: cont.clone(),
                metadata: metadata.clone(),
            }),
            Node::Send { metadata, channel, send_type, send_type_end, inputs, .. } => Arc::new(Node::Send {
                base: new_base,
                channel: channel.clone(),
                send_type: send_type.clone(),
                send_type_end: *send_type_end,
                inputs: inputs.clone(),
                metadata: metadata.clone(),
            }),
            Node::New { decls, proc, metadata, .. } => Arc::new(Node::New {
                base: new_base,
                decls: decls.clone(),
                proc: proc.clone(),
                metadata: metadata.clone(),
            }),
            Node::IfElse { condition, consequence, alternative, metadata, .. } => Arc::new(Node::IfElse {
                base: new_base,
                condition: condition.clone(),
                consequence: consequence.clone(),
                alternative: alternative.clone(),
                metadata: metadata.clone(),
            }),
            Node::Let { decls, proc, metadata, .. } => Arc::new(Node::Let {
                base: new_base,
                decls: decls.clone(),
                proc: proc.clone(),
                metadata: metadata.clone(),
            }),
            Node::Bundle { bundle_type, proc, metadata, .. } => Arc::new(Node::Bundle {
                base: new_base,
                bundle_type: bundle_type.clone(),
                proc: proc.clone(),
                metadata: metadata.clone(),
            }),
            Node::Match { expression, cases, metadata, .. } => Arc::new(Node::Match {
                base: new_base,
                expression: expression.clone(),
                cases: cases.clone(),
                metadata: metadata.clone(),
            }),
            Node::Choice { branches, metadata, .. } => Arc::new(Node::Choice {
                base: new_base,
                branches: branches.clone(),
                metadata: metadata.clone(),
            }),
            Node::Contract { name, formals, formals_remainder, proc, metadata, .. } => Arc::new(Node::Contract {
                base: new_base,
                name: name.clone(),
                formals: formals.clone(),
                formals_remainder: formals_remainder.clone(),
                proc: proc.clone(),
                metadata: metadata.clone(),
            }),
            Node::Input { receipts, proc, metadata, .. } => Arc::new(Node::Input {
                base: new_base,
                receipts: receipts.clone(),
                proc: proc.clone(),
                metadata: metadata.clone(),
            }),
            Node::Block { proc, metadata, .. } => Arc::new(Node::Block {
                base: new_base,
                proc: proc.clone(),
                metadata: metadata.clone(),
            }),
            Node::Parenthesized { expr, metadata, .. } => Arc::new(Node::Parenthesized {
                base: new_base,
                expr: expr.clone(),
                metadata: metadata.clone(),
            }),
            Node::BinOp { op, left, right, metadata, .. } => Arc::new(Node::BinOp {
                base: new_base,
                op: op.clone(),
                left: left.clone(),
                right: right.clone(),
                metadata: metadata.clone(),
            }),
            Node::UnaryOp { op, operand, metadata, .. } => Arc::new(Node::UnaryOp {
                base: new_base,
                op: op.clone(),
                operand: operand.clone(),
                metadata: metadata.clone(),
            }),
            Node::Method { receiver, name, args, metadata, .. } => Arc::new(Node::Method {
                base: new_base,
                receiver: receiver.clone(),
                name: name.clone(),
                args: args.clone(),
                metadata: metadata.clone(),
            }),
            Node::Eval { name, metadata, .. } => Arc::new(Node::Eval {
                base: new_base,
                name: name.clone(),
                metadata: metadata.clone(),
            }),
            Node::Quote { quotable, metadata, .. } => Arc::new(Node::Quote {
                base: new_base,
                quotable: quotable.clone(),
                metadata: metadata.clone(),
            }),
            Node::VarRef { kind, var, metadata, .. } => Arc::new(Node::VarRef {
                base: new_base,
                kind: kind.clone(),
                var: var.clone(),
                metadata: metadata.clone(),
            }),
            Node::BoolLiteral { value, metadata, .. } => Arc::new(Node::BoolLiteral {
                base: new_base,
                value: *value,
                metadata: metadata.clone(),
            }),
            Node::LongLiteral { value, metadata, .. } => Arc::new(Node::LongLiteral {
                base: new_base,
                value: *value,
                metadata: metadata.clone(),
            }),
            Node::StringLiteral { value, metadata, .. } => Arc::new(Node::StringLiteral {
                base: new_base,
                value: value.clone(),
                metadata: metadata.clone(),
            }),
            Node::UriLiteral { value, metadata, .. } => Arc::new(Node::UriLiteral {
                base: new_base,
                value: value.clone(),
                metadata: metadata.clone(),
            }),
            Node::Nil { metadata, .. } => Arc::new(Node::Nil {
                base: new_base,
                metadata: metadata.clone(),
            }),
            Node::List { elements, remainder, metadata, .. } => Arc::new(Node::List {
                base: new_base,
                elements: elements.clone(),
                remainder: remainder.clone(),
                metadata: metadata.clone(),
            }),
            Node::Set { elements, remainder, metadata, .. } => Arc::new(Node::Set {
                base: new_base,
                elements: elements.clone(),
                remainder: remainder.clone(),
                metadata: metadata.clone(),
            }),
            Node::Map { pairs, remainder, metadata, .. } => Arc::new(Node::Map {
                base: new_base,
                pairs: pairs.clone(),
                remainder: remainder.clone(),
                metadata: metadata.clone(),
            }),
            Node::Tuple { elements, metadata, .. } => Arc::new(Node::Tuple {
                base: new_base,
                elements: elements.clone(),
                metadata: metadata.clone(),
            }),
            Node::Var { name, metadata, .. } => Arc::new(Node::Var {
                base: new_base,
                name: name.clone(),
                metadata: metadata.clone(),
            }),
            Node::NameDecl { var, uri, metadata, .. } => Arc::new(Node::NameDecl {
                base: new_base,
                var: var.clone(),
                uri: uri.clone(),
                metadata: metadata.clone(),
            }),
            Node::Decl { names, names_remainder, procs, metadata, .. } => Arc::new(Node::Decl {
                base: new_base,
                names: names.clone(),
                names_remainder: names_remainder.clone(),
                procs: procs.clone(),
                metadata: metadata.clone(),
            }),
            Node::LinearBind { names, remainder, source, metadata, .. } => Arc::new(Node::LinearBind {
                base: new_base,
                names: names.clone(),
                remainder: remainder.clone(),
                source: source.clone(),
                metadata: metadata.clone(),
            }),
            Node::RepeatedBind { names, remainder, source, metadata, .. } => Arc::new(Node::RepeatedBind {
                base: new_base,
                names: names.clone(),
                remainder: remainder.clone(),
                source: source.clone(),
                metadata: metadata.clone(),
            }),
            Node::PeekBind { names, remainder, source, metadata, .. } => Arc::new(Node::PeekBind {
                base: new_base,
                names: names.clone(),
                remainder: remainder.clone(),
                source: source.clone(),
                metadata: metadata.clone(),
            }),
            Node::Comment { kind, metadata, .. } => Arc::new(Node::Comment {
                base: new_base,
                kind: kind.clone(),
                metadata: metadata.clone(),
            }),
            Node::Wildcard { metadata, .. } => Arc::new(Node::Wildcard {
                base: new_base,
                metadata: metadata.clone(),
            }),
            Node::SimpleType { value, metadata, .. } => Arc::new(Node::SimpleType {
                base: new_base,
                value: value.clone(),
                metadata: metadata.clone(),
            }),
            Node::ReceiveSendSource { name, metadata, .. } => Arc::new(Node::ReceiveSendSource {
                base: new_base,
                name: name.clone(),
                metadata: metadata.clone(),
            }),
            Node::SendReceiveSource { name, inputs, metadata, .. } => Arc::new(Node::SendReceiveSource {
                base: new_base,
                name: name.clone(),
                inputs: inputs.clone(),
                metadata: metadata.clone(),
            }),
            Node::Error { children, metadata, .. } => Arc::new(Node::Error {
                base: new_base,
                children: children.clone(),
                metadata: metadata.clone(),
            }),
        }
    }

    /// Validates the node by checking for reserved keyword usage in variable names.
    ///
    /// # Returns
    /// * `Ok(())` if validation passes.
    /// * `Err(String)` with an error message if a reserved keyword is misused.
    pub fn validate(&self) -> Result<(), String> {
        const RESERVED_KEYWORDS: &[&str] = &[
            "if", "else", "new", "in", "match", "contract", "select", "for", "let",
            "bundle", "bundle+", "bundle-", "bundle0", "true", "false", "Nil",
            "or", "and", "not", "matches",
        ];

        match self {
            Node::Send { channel, .. } | Node::SendSync { channel, .. } => {
                if let Node::Var { name, .. } = &**channel {
                    if RESERVED_KEYWORDS.contains(&name.as_str()) {
                        return Err(format!("Channel name '{}' is a reserved keyword", name));
                    }
                }
            }
            Node::Par { left, right, .. } => {
                left.validate()?;
                right.validate()?;
            }
            Node::New { decls, proc, .. } => {
                for decl in decls { decl.validate()?; }
                proc.validate()?;
            }
            Node::IfElse { condition, consequence, alternative, .. } => {
                condition.validate()?;
                consequence.validate()?;
                if let Some(alt) = alternative { alt.validate()?; }
            }
            Node::Let { decls, proc, .. } => {
                for decl in decls { decl.validate()?; }
                proc.validate()?;
            }
            Node::Bundle { proc, .. } => proc.validate()?,
            Node::Match { expression, cases, .. } => {
                expression.validate()?;
                for (pattern, proc) in cases {
                    if let Node::Var { name, .. } = &**pattern {
                        if RESERVED_KEYWORDS.contains(&name.as_str()) {
                            let pos = pattern.absolute_start(&Arc::new(self.clone()));
                            return Err(format!(
                                "Match pattern '{}' uses reserved keyword at {}:{}",
                                name, pos.row, pos.column
                            ));
                        }
                    }
                    pattern.validate()?;
                    proc.validate()?;
                }
            }
            Node::Choice { branches, .. } => {
                for (inputs, proc) in branches {
                    for input in inputs {
                        if let Node::LinearBind { names, remainder, source, .. } = &**input {
                            for name in names {
                                if let Node::Var { name: var_name, .. } = &**name {
                                    if RESERVED_KEYWORDS.contains(&var_name.as_str()) {
                                        let pos = name.absolute_start(&Arc::new(self.clone()));
                                        return Err(format!(
                                            "Select variable '{}' uses reserved keyword at {}:{}",
                                            var_name, pos.row, pos.column
                                        ));
                                    }
                                }
                            }
                            if let Some(rem) = remainder {
                                rem.validate()?;
                            }
                            source.validate()?;
                        }
                        input.validate()?;
                    }
                    proc.validate()?;
                }
            }
            Node::Contract { name, formals, formals_remainder, proc, .. } => {
                name.validate()?;
                for formal in formals { formal.validate()?; }
                if let Some(rem) = formals_remainder { rem.validate()?; }
                proc.validate()?;
            }
            Node::Input { receipts, proc, .. } => {
                for binds in receipts {
                    for bind in binds { bind.validate()?; }
                }
                proc.validate()?;
            }
            Node::Block { proc, .. } => proc.validate()?,
            Node::Parenthesized { expr, .. } => expr.validate()?,
            Node::BinOp { left, right, .. } => {
                left.validate()?;
                right.validate()?;
            }
            Node::UnaryOp { operand, .. } => operand.validate()?,
            Node::Method { receiver, args, .. } => {
                receiver.validate()?;
                for arg in args { arg.validate()?; }
            }
            Node::Eval { name, .. } => name.validate()?,
            Node::Quote { quotable, .. } => quotable.validate()?,
            Node::VarRef { var, .. } => var.validate()?,
            Node::List { elements, remainder, .. } => {
                for elem in elements { elem.validate()?; }
                if let Some(rem) = remainder { rem.validate()?; }
            }
            Node::Set { elements, remainder, .. } => {
                for elem in elements { elem.validate()?; }
                if let Some(rem) = remainder { rem.validate()?; }
            }
            Node::Map { pairs, remainder, .. } => {
                for (key, value) in pairs {
                    key.validate()?;
                    value.validate()?;
                }
                if let Some(rem) = remainder { rem.validate()?; }
            }
            Node::Tuple { elements, .. } => {
                for elem in elements { elem.validate()?; }
            }
            Node::NameDecl { var, uri, .. } => {
                var.validate()?;
                if let Some(u) = uri { u.validate()?; }
            }
            Node::Decl { names, names_remainder, procs, .. } => {
                for name in names { name.validate()?; }
                if let Some(rem) = names_remainder { rem.validate()?; }
                for proc in procs { proc.validate()?; }
            }
            Node::LinearBind { names, remainder, source, .. } => {
                for name in names { name.validate()?; }
                if let Some(rem) = remainder { rem.validate()?; }
                source.validate()?;
            }
            Node::RepeatedBind { names, remainder, source, .. } => {
                for name in names { name.validate()?; }
                if let Some(rem) = remainder { rem.validate()?; }
                source.validate()?;
            }
            Node::PeekBind { names, remainder, source, .. } => {
                for name in names { name.validate()?; }
                if let Some(rem) = remainder { rem.validate()?; }
                source.validate()?;
            }
            Node::ReceiveSendSource { name, .. } => name.validate()?,
            Node::SendReceiveSource { name, inputs, .. } => {
                name.validate()?;
                for input in inputs { input.validate()?; }
            }
            Node::Error { children, .. } => {
                for child in children { child.validate()?; }
            }
            _ => {}
        }
        Ok(())
    }

    /// Updates the node's metadata with a new value.
    ///
    /// # Arguments
    /// * `new_metadata` - The new metadata to apply to the node.
    ///
    /// # Returns
    /// A new `Arc<Node>` with the updated metadata.
    pub fn with_metadata(&self, new_metadata: Option<Arc<Metadata>>) -> Arc<Node<'a>> {
        match self {
            Node::Par { base, left, right, .. } => Arc::new(Node::Par {
                base: base.clone(), left: left.clone(), right: right.clone(), metadata: new_metadata }),
            Node::SendSync { base, channel, inputs, cont, .. } => Arc::new(Node::SendSync {
                base: base.clone(), channel: channel.clone(), inputs: inputs.clone(), cont: cont.clone(), metadata: new_metadata }),
            Node::Send { base, channel, send_type, send_type_end, inputs, .. } => Arc::new(Node::Send {
                base: base.clone(),
                channel: channel.clone(),
                send_type: send_type.clone(),
                send_type_end: *send_type_end,
                inputs: inputs.clone(),
                metadata: new_metadata,
            }),
            Node::New { base, decls, proc, .. } => Arc::new(Node::New {
                base: base.clone(), decls: decls.clone(), proc: proc.clone(), metadata: new_metadata }),
            Node::IfElse { base, condition, consequence, alternative, .. } => Arc::new(Node::IfElse {
                base: base.clone(), condition: condition.clone(), consequence: consequence.clone(), alternative: alternative.clone(), metadata: new_metadata }),
            Node::Let { base, decls, proc, .. } => Arc::new(Node::Let {
                base: base.clone(), decls: decls.clone(), proc: proc.clone(), metadata: new_metadata }),
            Node::Bundle { base, bundle_type, proc, .. } => Arc::new(Node::Bundle {
                base: base.clone(), bundle_type: bundle_type.clone(), proc: proc.clone(), metadata: new_metadata }),
            Node::Match { base, expression, cases, .. } => Arc::new(Node::Match {
                base: base.clone(), expression: expression.clone(), cases: cases.clone(), metadata: new_metadata }),
            Node::Choice { base, branches, .. } => Arc::new(Node::Choice {
                base: base.clone(), branches: branches.clone(), metadata: new_metadata }),
            Node::Contract { base, name, formals, formals_remainder, proc, .. } => Arc::new(Node::Contract {
                base: base.clone(), name: name.clone(), formals: formals.clone(), formals_remainder: formals_remainder.clone(), proc: proc.clone(), metadata: new_metadata }),
            Node::Input { base, receipts, proc, .. } => Arc::new(Node::Input {
                base: base.clone(), receipts: receipts.clone(), proc: proc.clone(), metadata: new_metadata }),
            Node::Block { base, proc, .. } => Arc::new(Node::Block {
                base: base.clone(), proc: proc.clone(), metadata: new_metadata }),
            Node::Parenthesized { base, expr, .. } => Arc::new(Node::Parenthesized {
                base: base.clone(), expr: expr.clone(), metadata: new_metadata }),
            Node::BinOp { base, op, left, right, .. } => Arc::new(Node::BinOp {
                base: base.clone(), op: op.clone(), left: left.clone(), right: right.clone(), metadata: new_metadata }),
            Node::UnaryOp { base, op, operand, .. } => Arc::new(Node::UnaryOp {
                base: base.clone(), op: op.clone(), operand: operand.clone(), metadata: new_metadata }),
            Node::Method { base, receiver, name, args, .. } => Arc::new(Node::Method {
                base: base.clone(), receiver: receiver.clone(), name: name.clone(), args: args.clone(), metadata: new_metadata }),
            Node::Eval { base, name, .. } => Arc::new(Node::Eval {
                base: base.clone(), name: name.clone(), metadata: new_metadata }),
            Node::Quote { base, quotable, .. } => Arc::new(Node::Quote {
                base: base.clone(), quotable: quotable.clone(), metadata: new_metadata }),
            Node::VarRef { base, kind, var, .. } => Arc::new(Node::VarRef {
                base: base.clone(), kind: kind.clone(), var: var.clone(), metadata: new_metadata }),
            Node::BoolLiteral { base, value, .. } => Arc::new(Node::BoolLiteral {
                base: base.clone(), value: *value, metadata: new_metadata }),
            Node::LongLiteral { base, value, .. } => Arc::new(Node::LongLiteral {
                base: base.clone(), value: *value, metadata: new_metadata }),
            Node::StringLiteral { base, value, .. } => Arc::new(Node::StringLiteral {
                base: base.clone(), value: value.clone(), metadata: new_metadata }),
            Node::UriLiteral { base, value, .. } => Arc::new(Node::UriLiteral {
                base: base.clone(), value: value.clone(), metadata: new_metadata }),
            Node::Nil { base, .. } => Arc::new(Node::Nil {
                base: base.clone(), metadata: new_metadata }),
            Node::List { base, elements, remainder, .. } => Arc::new(Node::List {
                base: base.clone(), elements: elements.clone(), remainder: remainder.clone(), metadata: new_metadata }),
            Node::Set { base, elements, remainder, .. } => Arc::new(Node::Set {
                base: base.clone(), elements: elements.clone(), remainder: remainder.clone(), metadata: new_metadata }),
            Node::Map { base, pairs, remainder, .. } => Arc::new(Node::Map {
                base: base.clone(), pairs: pairs.clone(), remainder: remainder.clone(), metadata: new_metadata }),
            Node::Tuple { base, elements, .. } => Arc::new(Node::Tuple {
                base: base.clone(), elements: elements.clone(), metadata: new_metadata }),
            Node::Var { base, name, .. } => Arc::new(Node::Var {
                base: base.clone(), name: name.clone(), metadata: new_metadata }),
            Node::NameDecl { base, var, uri, .. } => Arc::new(Node::NameDecl {
                base: base.clone(), var: var.clone(), uri: uri.clone(), metadata: new_metadata }),
            Node::Decl { base, names, names_remainder, procs, .. } => Arc::new(Node::Decl {
                base: base.clone(), names: names.clone(), names_remainder: names_remainder.clone(), procs: procs.clone(), metadata: new_metadata }),
            Node::LinearBind { base, names, remainder, source, .. } => Arc::new(Node::LinearBind {
                base: base.clone(), names: names.clone(), remainder: remainder.clone(), source: source.clone(), metadata: new_metadata }),
            Node::RepeatedBind { base, names, remainder, source, .. } => Arc::new(Node::RepeatedBind {
                base: base.clone(), names: names.clone(), remainder: remainder.clone(), source: source.clone(), metadata: new_metadata }),
            Node::PeekBind { base, names, remainder, source, .. } => Arc::new(Node::PeekBind {
                base: base.clone(), names: names.clone(), remainder: remainder.clone(), source: source.clone(), metadata: new_metadata }),
            Node::Comment { base, kind, .. } => Arc::new(Node::Comment {
                base: base.clone(), kind: kind.clone(), metadata: new_metadata }),
            Node::Wildcard { base, .. } => Arc::new(Node::Wildcard {
                base: base.clone(), metadata: new_metadata }),
            Node::SimpleType { base, value, .. } => Arc::new(Node::SimpleType {
                base: base.clone(), value: value.clone(), metadata: new_metadata }),
            Node::ReceiveSendSource { base, name, .. } => Arc::new(Node::ReceiveSendSource {
                base: base.clone(), name: name.clone(), metadata: new_metadata }),
            Node::SendReceiveSource { base, name, inputs, .. } => Arc::new(Node::SendReceiveSource {
                base: base.clone(), name: name.clone(), inputs: inputs.clone(), metadata: new_metadata }),
            Node::Error { base, children, .. } => Arc::new(Node::Error {
                base: base.clone(),
                children: children.clone(),
                metadata: new_metadata,
            }),
        }
    }

    /// Constructs a new `Par` node with the given attributes.
    pub fn new_par(ts_node: Option<TSNode<'a>>, left: Arc<Node<'a>>, right: Arc<Node<'a>>, metadata: Option<Arc<Metadata>>, relative_start: RelativePosition, length: usize, text: Option<String>) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::Par { base, left, right, metadata }
    }

    /// Constructs a new `SendSync` node with the given attributes.
    pub fn new_send_sync(ts_node: Option<TSNode<'a>>, channel: Arc<Node<'a>>, inputs: NodeVector<'a>, cont: Arc<Node<'a>>, metadata: Option<Arc<Metadata>>, relative_start: RelativePosition, length: usize, text: Option<String>) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::SendSync { base, channel, inputs, cont, metadata }
    }

    /// Constructs a new `Send` node with the given attributes.
    pub fn new_send(
        ts_node: Option<TSNode<'a>>,
        channel: Arc<Node<'a>>,
        send_type: SendType,
        send_type_end: Position,
        inputs: NodeVector<'a>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        text: Option<String>,
    ) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::Send { base, channel, send_type, send_type_end, inputs, metadata }
    }

    /// Constructs a new `New` node with the given attributes.
    pub fn new_new(ts_node: Option<TSNode<'a>>, decls: NodeVector<'a>, proc: Arc<Node<'a>>, metadata: Option<Arc<Metadata>>, relative_start: RelativePosition, length: usize, text: Option<String>) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::New { base, decls, proc, metadata }
    }

    /// Constructs a new `IfElse` node with the given attributes.
    pub fn new_if_else(ts_node: Option<TSNode<'a>>, condition: Arc<Node<'a>>, consequence: Arc<Node<'a>>, alternative: Option<Arc<Node<'a>>>, metadata: Option<Arc<Metadata>>, relative_start: RelativePosition, length: usize, text: Option<String>) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::IfElse { base, condition, consequence, alternative, metadata }
    }

    /// Constructs a new `Let` node with the given attributes.
    pub fn new_let(ts_node: Option<TSNode<'a>>, decls: NodeVector<'a>, proc: Arc<Node<'a>>, metadata: Option<Arc<Metadata>>, relative_start: RelativePosition, length: usize, text: Option<String>) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::Let { base, decls, proc, metadata }
    }

    /// Constructs a new `Bundle` node with the given attributes.
    pub fn new_bundle(ts_node: Option<TSNode<'a>>, bundle_type: BundleType, proc: Arc<Node<'a>>, metadata: Option<Arc<Metadata>>, relative_start: RelativePosition, length: usize, text: Option<String>) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::Bundle { base, bundle_type, proc, metadata }
    }

    /// Constructs a new `Match` node with the given attributes.
    pub fn new_match(ts_node: Option<TSNode<'a>>, expression: Arc<Node<'a>>, cases: NodePairVector<'a>, metadata: Option<Arc<Metadata>>, relative_start: RelativePosition, length: usize, text: Option<String>) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::Match { base, expression, cases, metadata }
    }

    /// Constructs a new `Choice` node with the given attributes.
    pub fn new_choice(ts_node: Option<TSNode<'a>>, branches: BranchVector<'a>, metadata: Option<Arc<Metadata>>, relative_start: RelativePosition, length: usize, text: Option<String>) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::Choice { base, branches, metadata }
    }

    /// Constructs a new `Contract` node with the given attributes.
    pub fn new_contract(ts_node: Option<TSNode<'a>>, name: Arc<Node<'a>>, formals: NodeVector<'a>, formals_remainder: Option<Arc<Node<'a>>>, proc: Arc<Node<'a>>, metadata: Option<Arc<Metadata>>, relative_start: RelativePosition, length: usize, text: Option<String>) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::Contract { base, name, formals, formals_remainder, proc, metadata }
    }

    /// Constructs a new `Input` node with the given attributes.
    pub fn new_input(ts_node: Option<TSNode<'a>>, receipts: ReceiptVector<'a>, proc: Arc<Node<'a>>, metadata: Option<Arc<Metadata>>, relative_start: RelativePosition, length: usize, text: Option<String>) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::Input { base, receipts, proc, metadata }
    }

    /// Constructs a new `Block` node with the given attributes.
    pub fn new_block(ts_node: Option<TSNode<'a>>, proc: Arc<Node<'a>>, metadata: Option<Arc<Metadata>>, relative_start: RelativePosition, length: usize, text: Option<String>) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::Block { base, proc, metadata }
    }

    /// Constructs a new `Parenthesized` node with the given attributes.
    pub fn new_parenthesized(ts_node: Option<TSNode<'a>>, expr: Arc<Node<'a>>, metadata: Option<Arc<Metadata>>, relative_start: RelativePosition, length: usize, text: Option<String>) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::Parenthesized { base, expr, metadata }
    }

    /// Constructs a new `BinOp` node with the given attributes.
    pub fn new_bin_op(ts_node: Option<TSNode<'a>>, op: BinOperator, left: Arc<Node<'a>>, right: Arc<Node<'a>>, metadata: Option<Arc<Metadata>>, relative_start: RelativePosition, length: usize, text: Option<String>) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::BinOp { base, op, left, right, metadata }
    }

    /// Constructs a new `UnaryOp` node with the given attributes.
    pub fn new_unary_op(ts_node: Option<TSNode<'a>>, op: UnaryOperator, operand: Arc<Node<'a>>, metadata: Option<Arc<Metadata>>, relative_start: RelativePosition, length: usize, text: Option<String>) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::UnaryOp { base, op, operand, metadata }
    }

    /// Constructs a new `Method` node with the given attributes.
    pub fn new_method(ts_node: Option<TSNode<'a>>, receiver: Arc<Node<'a>>, name: String, args: NodeVector<'a>, metadata: Option<Arc<Metadata>>, relative_start: RelativePosition, length: usize, text: Option<String>) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::Method { base, receiver, name, args, metadata }
    }

    /// Constructs a new `Eval` node with the given attributes.
    pub fn new_eval(ts_node: Option<TSNode<'a>>, name: Arc<Node<'a>>, metadata: Option<Arc<Metadata>>, relative_start: RelativePosition, length: usize, text: Option<String>) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::Eval { base, name, metadata }
    }

    /// Constructs a new `Quote` node with the given attributes.
    pub fn new_quote(ts_node: Option<TSNode<'a>>, quotable: Arc<Node<'a>>, metadata: Option<Arc<Metadata>>, relative_start: RelativePosition, length: usize, text: Option<String>) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::Quote { base, quotable, metadata }
    }

    /// Constructs a new `VarRef` node with the given attributes.
    pub fn new_var_ref(ts_node: Option<TSNode<'a>>, kind: VarRefKind, var: Arc<Node<'a>>, metadata: Option<Arc<Metadata>>, relative_start: RelativePosition, length: usize, text: Option<String>) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::VarRef { base, kind, var, metadata }
    }

    /// Constructs a new `BoolLiteral` node with the given attributes.
    pub fn new_bool_literal(ts_node: Option<TSNode<'a>>, value: bool, metadata: Option<Arc<Metadata>>, relative_start: RelativePosition, length: usize, text: Option<String>) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::BoolLiteral { base, value, metadata }
    }

    /// Constructs a new `LongLiteral` node with the given attributes.
    pub fn new_long_literal(ts_node: Option<TSNode<'a>>, value: i64, metadata: Option<Arc<Metadata>>, relative_start: RelativePosition, length: usize, text: Option<String>) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::LongLiteral { base, value, metadata }
    }

    /// Constructs a new `StringLiteral` node with the given attributes.
    pub fn new_string_literal(ts_node: Option<TSNode<'a>>, value: String, metadata: Option<Arc<Metadata>>, relative_start: RelativePosition, length: usize, text: Option<String>) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::StringLiteral { base, value, metadata }
    }

    /// Constructs a new `UriLiteral` node with the given attributes.
    pub fn new_uri_literal(ts_node: Option<TSNode<'a>>, value: String, metadata: Option<Arc<Metadata>>, relative_start: RelativePosition, length: usize, text: Option<String>) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::UriLiteral { base, value, metadata }
    }

    /// Constructs a new `Nil` node with the given attributes.
    pub fn new_nil(ts_node: Option<TSNode<'a>>, metadata: Option<Arc<Metadata>>, relative_start: RelativePosition, length: usize, text: Option<String>) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::Nil { base, metadata }
    }

    /// Constructs a new `List` node with the given attributes.
    pub fn new_list(ts_node: Option<TSNode<'a>>, elements: NodeVector<'a>, remainder: Option<Arc<Node<'a>>>, metadata: Option<Arc<Metadata>>, relative_start: RelativePosition, length: usize, text: Option<String>) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::List { base, elements, remainder, metadata }
    }

    /// Constructs a new `Set` node with the given attributes.
    pub fn new_set(ts_node: Option<TSNode<'a>>, elements: NodeVector<'a>, remainder: Option<Arc<Node<'a>>>, metadata: Option<Arc<Metadata>>, relative_start: RelativePosition, length: usize, text: Option<String>) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::Set { base, elements, remainder, metadata }
    }

    /// Constructs a new `Map` node with the given attributes.
    pub fn new_map(ts_node: Option<TSNode<'a>>, pairs: NodePairVector<'a>, remainder: Option<Arc<Node<'a>>>, metadata: Option<Arc<Metadata>>, relative_start: RelativePosition, length: usize, text: Option<String>) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::Map { base, pairs, remainder, metadata }
    }

    /// Constructs a new `Tuple` node with the given attributes.
    pub fn new_tuple(ts_node: Option<TSNode<'a>>, elements: NodeVector<'a>, metadata: Option<Arc<Metadata>>, relative_start: RelativePosition, length: usize, text: Option<String>) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::Tuple { base, elements, metadata }
    }

    /// Constructs a new `Var` node with the given attributes.
    pub fn new_var(ts_node: Option<TSNode<'a>>, name: String, metadata: Option<Arc<Metadata>>, relative_start: RelativePosition, length: usize, text: Option<String>) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::Var { base, name, metadata }
    }

    /// Constructs a new `NameDecl` node with the given attributes.
    pub fn new_name_decl(ts_node: Option<TSNode<'a>>, var: Arc<Node<'a>>, uri: Option<Arc<Node<'a>>>, metadata: Option<Arc<Metadata>>, relative_start: RelativePosition, length: usize, text: Option<String>) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::NameDecl { base, var, uri, metadata }
    }

    /// Constructs a new `Decl` node with the given attributes.
    pub fn new_decl(ts_node: Option<TSNode<'a>>, names: NodeVector<'a>, names_remainder: Option<Arc<Node<'a>>>, procs: NodeVector<'a>, metadata: Option<Arc<Metadata>>, relative_start: RelativePosition, length: usize, text: Option<String>) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::Decl { base, names, names_remainder, procs, metadata }
    }

    /// Constructs a new `LinearBind` node with the given attributes.
    pub fn new_linear_bind(ts_node: Option<TSNode<'a>>, names: NodeVector<'a>, remainder: Option<Arc<Node<'a>>>, source: Arc<Node<'a>>, metadata: Option<Arc<Metadata>>, relative_start: RelativePosition, length: usize, text: Option<String>) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::LinearBind { base, names, remainder, source, metadata }
    }

    /// Constructs a new `RepeatedBind` node with the given attributes.
    pub fn new_repeated_bind(ts_node: Option<TSNode<'a>>, names: NodeVector<'a>, remainder: Option<Arc<Node<'a>>>, source: Arc<Node<'a>>, metadata: Option<Arc<Metadata>>, relative_start: RelativePosition, length: usize, text: Option<String>) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::RepeatedBind { base, names, remainder, source, metadata }
    }

    /// Constructs a new `PeekBind` node with the given attributes.
    pub fn new_peek_bind(ts_node: Option<TSNode<'a>>, names: NodeVector<'a>, remainder: Option<Arc<Node<'a>>>, source: Arc<Node<'a>>, metadata: Option<Arc<Metadata>>, relative_start: RelativePosition, length: usize, text: Option<String>) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::PeekBind { base, names, remainder, source, metadata }
    }

    /// Constructs a new `Comment` node with the given attributes.
    pub fn new_comment(ts_node: Option<TSNode<'a>>, kind: CommentKind, metadata: Option<Arc<Metadata>>, relative_start: RelativePosition, length: usize, text: Option<String>) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::Comment { base, kind, metadata }
    }

    /// Constructs a new `Wildcard` node with the given attributes.
    pub fn new_wildcard(ts_node: Option<TSNode<'a>>, metadata: Option<Arc<Metadata>>, relative_start: RelativePosition, length: usize, text: Option<String>) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::Wildcard { base, metadata }
    }

    /// Constructs a new `SimpleType` node with the given attributes.
    pub fn new_simple_type(ts_node: Option<TSNode<'a>>, value: String, metadata: Option<Arc<Metadata>>, relative_start: RelativePosition, length: usize, text: Option<String>) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::SimpleType { base, value, metadata }
    }

    /// Constructs a new `ReceiveSendSource` node with the given attributes.
    pub fn new_receive_send_source(ts_node: Option<TSNode<'a>>, name: Arc<Node<'a>>, metadata: Option<Arc<Metadata>>, relative_start: RelativePosition, length: usize, text: Option<String>) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::ReceiveSendSource { base, name, metadata }
    }

    /// Constructs a new `SendReceiveSource` node with the given attributes.
    pub fn new_send_receive_source(ts_node: Option<TSNode<'a>>, name: Arc<Node<'a>>, inputs: NodeVector<'a>, metadata: Option<Arc<Metadata>>, relative_start: RelativePosition, length: usize, text: Option<String>) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::SendReceiveSource { base, name, inputs, metadata }
    }

    /// Constructs a new `Error` node with the given attributes.
    pub fn new_error(
        ts_node: Option<TSNode<'a>>,
        children: NodeVector<'a>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        text: Option<String>,
    ) -> Self {
        let base = NodeBase::new(ts_node, relative_start, length, text);
        Node::Error { base, children, metadata }
    }

    /// Returns the textual representation of the node.
    /// If source text is available, returns it; otherwise, formats the node using the IR formatter.
    pub fn text(&self) -> String {
        if let Some(text) = self.base().text() {
            text.clone()
        } else {
            crate::ir::formatter::format_node(&Arc::new(self.clone()), false, None)
        }
    }

    /// Returns a reference to the node’s `NodeBase`.
    pub fn base(&self) -> &NodeBase<'a> {
        match self {
            Node::Par { base, .. } => base,
            Node::SendSync { base, .. } => base,
            Node::Send { base, .. } => base,
            Node::New { base, .. } => base,
            Node::IfElse { base, .. } => base,
            Node::Let { base, .. } => base,
            Node::Bundle { base, .. } => base,
            Node::Match { base, .. } => base,
            Node::Choice { base, .. } => base,
            Node::Contract { base, .. } => base,
            Node::Input { base, .. } => base,
            Node::Block { base, .. } => base,
            Node::Parenthesized { base, .. } => base,
            Node::BinOp { base, .. } => base,
            Node::UnaryOp { base, .. } => base,
            Node::Method { base, .. } => base,
            Node::Eval { base, .. } => base,
            Node::Quote { base, .. } => base,
            Node::VarRef { base, .. } => base,
            Node::BoolLiteral { base, .. } => base,
            Node::LongLiteral { base, .. } => base,
            Node::StringLiteral { base, .. } => base,
            Node::UriLiteral { base, .. } => base,
            Node::Nil { base, .. } => base,
            Node::List { base, .. } => base,
            Node::Set { base, .. } => base,
            Node::Map { base, .. } => base,
            Node::Tuple { base, .. } => base,
            Node::Var { base, .. } => base,
            Node::NameDecl { base, .. } => base,
            Node::Decl { base, .. } => base,
            Node::LinearBind { base, .. } => base,
            Node::RepeatedBind { base, .. } => base,
            Node::PeekBind { base, .. } => base,
            Node::Comment { base, .. } => base,
            Node::Wildcard { base, .. } => base,
            Node::SimpleType { base, .. } => base,
            Node::ReceiveSendSource { base, .. } => base,
            Node::SendReceiveSource { base, .. } => base,
            Node::Error { base, .. } => base,
        }
    }

    /// Returns an optional reference to the node’s metadata.
    pub fn metadata(&self) -> Option<&Arc<Metadata>> {
        match self {
            Node::Par { metadata, .. } => metadata.as_ref(),
            Node::SendSync { metadata, .. } => metadata.as_ref(),
            Node::Send { metadata, .. } => metadata.as_ref(),
            Node::New { metadata, .. } => metadata.as_ref(),
            Node::IfElse { metadata, .. } => metadata.as_ref(),
            Node::Let { metadata, .. } => metadata.as_ref(),
            Node::Bundle { metadata, .. } => metadata.as_ref(),
            Node::Match { metadata, .. } => metadata.as_ref(),
            Node::Choice { metadata, .. } => metadata.as_ref(),
            Node::Contract { metadata, .. } => metadata.as_ref(),
            Node::Input { metadata, .. } => metadata.as_ref(),
            Node::Block { metadata, .. } => metadata.as_ref(),
            Node::Parenthesized { metadata, .. } => metadata.as_ref(),
            Node::BinOp { metadata, .. } => metadata.as_ref(),
            Node::UnaryOp { metadata, .. } => metadata.as_ref(),
            Node::Method { metadata, .. } => metadata.as_ref(),
            Node::Eval { metadata, .. } => metadata.as_ref(),
            Node::Quote { metadata, .. } => metadata.as_ref(),
            Node::VarRef { metadata, .. } => metadata.as_ref(),
            Node::BoolLiteral { metadata, .. } => metadata.as_ref(),
            Node::LongLiteral { metadata, .. } => metadata.as_ref(),
            Node::StringLiteral { metadata, .. } => metadata.as_ref(),
            Node::UriLiteral { metadata, .. } => metadata.as_ref(),
            Node::Nil { metadata, .. } => metadata.as_ref(),
            Node::List { metadata, .. } => metadata.as_ref(),
            Node::Set { metadata, .. } => metadata.as_ref(),
            Node::Map { metadata, .. } => metadata.as_ref(),
            Node::Tuple { metadata, .. } => metadata.as_ref(),
            Node::Var { metadata, .. } => metadata.as_ref(),
            Node::NameDecl { metadata, .. } => metadata.as_ref(),
            Node::Decl { metadata, .. } => metadata.as_ref(),
            Node::LinearBind { metadata, .. } => metadata.as_ref(),
            Node::RepeatedBind { metadata, .. } => metadata.as_ref(),
            Node::PeekBind { metadata, .. } => metadata.as_ref(),
            Node::Comment { metadata, .. } => metadata.as_ref(),
            Node::Wildcard { metadata, .. } => metadata.as_ref(),
            Node::SimpleType { metadata, .. } => metadata.as_ref(),
            Node::ReceiveSendSource { metadata, .. } => metadata.as_ref(),
            Node::SendReceiveSource { metadata, .. } => metadata.as_ref(),
            Node::Error { metadata, .. } => metadata.as_ref(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quickcheck::{QuickCheck, TestResult};
    use test_utils::ir::generator::RholangProc;
    use crate::tree_sitter::{parse_code, parse_to_ir};

    #[test]
    fn test_position_computation() {
        let _ = crate::logging::init_logger(false, Some("debug"));
        let code = "ch!(\"msg\")\nNil";
        let tree = parse_code(code);
        let ir = parse_to_ir(&tree, code);
        let root = Arc::new(ir.clone());

        if let Node::Par { left, right, .. } = &*ir {
            let left_start = left.absolute_start(&root);
            assert_eq!(left_start.row, 0);
            assert_eq!(left_start.column, 0);
            assert_eq!(left_start.byte, 0);

            let left_end = left.absolute_end(&root);
            assert_eq!(left_end.row, 0);
            assert_eq!(left_end.column, 10);
            assert_eq!(left_end.byte, 10);

            let right_start = right.absolute_start(&root);
            assert_eq!(right_start.row, 1);
            assert_eq!(right_start.column, 0);
            assert_eq!(right_start.byte, 11);

            let right_end = right.absolute_end(&root);
            assert_eq!(right_end.row, 1);
            assert_eq!(right_end.column, 3);
            assert_eq!(right_end.byte, 14);
        } else {
            panic!("Expected Par node");
        }
    }

    #[test]
    fn test_nested_position() {
        let _ = crate::logging::init_logger(false, Some("debug"));
        let code = "new x in { x!(\"msg\") }";
        let tree = parse_code(code);
        let ir = parse_to_ir(&tree, code);
        let root = Arc::new(ir.clone());

        if let Node::New { decls, proc, .. } = &*ir {
            let decl_start = decls[0].absolute_start(&root);
            assert_eq!(decl_start.row, 0);
            assert_eq!(decl_start.column, 4);
            assert_eq!(decl_start.byte, 4);

            if let Node::Block { proc: inner_proc, .. } = &**proc {
                if let Node::Send { channel, inputs, .. } = &**inner_proc {
                    let channel_start = channel.absolute_start(&root);
                    assert_eq!(channel_start.row, 0);
                    assert_eq!(channel_start.column, 11);
                    assert_eq!(channel_start.byte, 11);

                    let input_start = inputs[0].absolute_start(&root);
                    assert_eq!(input_start.row, 0);
                    assert_eq!(input_start.column, 14);
                    assert_eq!(input_start.byte, 14);
                }
            }
        }
    }

    #[test]
    fn test_position_property() {
        fn prop(proc: RholangProc) -> TestResult {
            let code = proc.to_code();
            let tree = parse_code(&code);
            if tree.root_node().has_error() {
                return TestResult::discard();
            }
            let ir = parse_to_ir(&tree, &code);
            let root = Arc::new(ir.clone());
            let start = ir.absolute_start(&root);
            let end = ir.absolute_end(&root);
            assert!(start.byte <= end.byte, "Start byte should be <= end byte");
            assert!(start.row <= end.row, "Start row should be <= end row");
            if start.row == end.row {
                assert!(start.column <= end.column, "Start column should be <= end column on same row");
            }
            TestResult::passed()
        }
        QuickCheck::new().tests(100).max_tests(1000).quickcheck(prop as fn(RholangProc) -> TestResult);
    }

    #[test]
    fn test_multi_line_positions() {
        let _ = crate::logging::init_logger(false, Some("debug"));
        let code = "ch!(\n\"msg\"\n)";
        let tree = parse_code(code);
        let ir = parse_to_ir(&tree, code);
        let root = Arc::new(ir.clone());

        if let Node::Send { inputs, .. } = &*ir {
            let input_start = inputs[0].absolute_start(&root);
            assert_eq!(input_start.row, 1);
            assert_eq!(input_start.column, 0);
        } else {
            panic!("Expected Send node");
        }
    }

    #[test]
    fn test_match_positioning() {
        let _ = crate::logging::init_logger(false, Some("debug"));
        let code = r#"match "target" { "pat" => Nil }"#;
        let tree = parse_code(code);
        let ir = parse_to_ir(&tree, code);
        let root = Arc::new(ir.clone());

        if let Node::Match { expression, cases, .. } = &*ir {
            let expr_start = expression.absolute_start(&root);
            assert_eq!(expr_start.row, 0);
            assert_eq!(expr_start.column, 6); // After "match "
            assert_eq!(expr_start.byte, 6);

            let (pattern, proc) = &cases[0];
            let pat_start = pattern.absolute_start(&root);
            assert_eq!(pat_start.row, 0);
            assert_eq!(pat_start.column, 17); // After "{ "
            assert_eq!(pat_start.byte, 17);

            let proc_start = proc.absolute_start(&root);
            assert_eq!(proc_start.row, 0);
            assert_eq!(proc_start.column, 26); // After " => "
            assert_eq!(proc_start.byte, 26);
        } else {
            panic!("Expected Match node");
        }
    }

    #[test]
    fn test_metadata_dynamic() {
        let mut data = HashMap::new();
        data.insert("version".to_string(), Arc::new(1_usize) as Arc<dyn Any + Send + Sync>);
        data.insert("custom".to_string(), Arc::new("test".to_string()) as Arc<dyn Any + Send + Sync>);
        let metadata = Arc::new(Metadata { data });
        let base = NodeBase::new(None, RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 }, 0, None);
        let node = Node::Nil { base, metadata: Some(metadata.clone()) };

        assert_eq!(node.metadata().unwrap().data.get("version").unwrap().downcast_ref::<usize>(), Some(&1));
        assert_eq!(node.metadata().unwrap().data.get("custom").unwrap().downcast_ref::<String>(), Some(&"test".to_string()));
    }

    #[test]
    fn test_error_node_with_children() {
        let code = r#"new x { x!("") }"#;
        let tree = parse_code(code);
        let ir = parse_to_ir(&tree, code);
        if let Node::Par { left, .. } = &*ir {
            if let Node::Error { children, .. } = left.as_ref() {
                assert!(!children.is_empty(), "Error node should have children");
            }
        }
    }
}
