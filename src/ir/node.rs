use std::any::Any;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;

use rpds::Vector;
use archery::ArcK;

use ropey::{Rope, RopeSlice};

use tracing::{debug, warn};

pub type NodeVector = Vector<Arc<Node>, ArcK>;
pub type NodePairVector = Vector<(Arc<Node>, Arc<Node>), ArcK>;
pub type BranchVector = Vector<(NodeVector, Arc<Node>), ArcK>;
pub type ReceiptVector = Vector<NodeVector, ArcK>;

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
    length: usize,                    // Length of the node's text in bytes
    span_lines: usize,                // Number of lines spanned by the node
    span_columns: usize,              // Columns on the last line
}

impl NodeBase {
    /// Creates a new NodeBase instance with the specified attributes.
    pub fn new(
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        NodeBase {
            relative_start,
            length,
            span_lines,
            span_columns,
        }
    }

    /// Returns the relative start position of the node.
    pub fn relative_start(&self) -> RelativePosition {
        self.relative_start
    }

    /// Returns the length of the node's text in bytes.
    pub fn length(&self) -> usize {
        self.length
    }

    /// Returns the number of lines spanned by the node.
    pub fn span_lines(&self) -> usize {
        self.span_lines
    }

    /// Returns the number of columns on the last line spanned by the node.
    pub fn span_columns(&self) -> usize {
        self.span_columns
    }
}

/// Represents all possible constructs in the Rholang Intermediate Representation (IR).
/// Each variant corresponds to a syntactic element in Rholang, such as processes, expressions, or bindings.
///
/// # Examples
/// - Par: Parallel composition of two processes (e.g., P | Q).
/// - Send: Asynchronous message send (e.g., ch!("msg")).
/// - Var: Variable reference (e.g., x in x!()).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum Node {
    /// Parallel composition of two processes.
    Par {
        base: NodeBase,
        left: Arc<Node>,
        right: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Synchronous send with a continuation process.
    SendSync {
        base: NodeBase,
        channel: Arc<Node>,
        inputs: NodeVector,
        cont: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Asynchronous send operation on a channel.
    Send {
        base: NodeBase,
        channel: Arc<Node>,
        send_type: SendType,
        send_type_delta: RelativePosition,
        inputs: NodeVector,
        metadata: Option<Arc<Metadata>>,
    },
    /// Declaration of new names with a scoped process
    New {
        base: NodeBase,
        decls: NodeVector,
        proc: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Conditional branching with optional else clause.
    IfElse {
        base: NodeBase,
        condition: Arc<Node>,
        consequence: Arc<Node>,
        alternative: Option<Arc<Node>>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Variable binding with a subsequent process.
    Let {
        base: NodeBase,
        decls: NodeVector,
        proc: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Access-controlled process with a bundle type.
    Bundle {
        base: NodeBase,
        bundle_type: BundleType,
        proc: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Pattern matching construct with cases.
    Match {
        base: NodeBase,
        expression: Arc<Node>,
        cases: NodePairVector,
        metadata: Option<Arc<Metadata>>,
    },
    /// Non-deterministic choice among branches.
    Choice {
        base: NodeBase,
        branches: BranchVector,
        metadata: Option<Arc<Metadata>>,
    },
    /// Contract definition with name, parameters, and body.
    Contract {
        base: NodeBase,
        name: Arc<Node>,
        formals: NodeVector,
        formals_remainder: Option<Arc<Node>>,
        proc: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Input binding from channels with a process.
    Input {
        base: NodeBase,
        receipts: ReceiptVector,
        proc: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Block of a single process (e.g., { P }).
    Block {
        base: NodeBase,
        proc: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Parenthesized expression (e.g., (P)).
    Parenthesized {
        base: NodeBase,
        expr: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Binary operation (e.g., P + Q).
    BinOp {
        base: NodeBase,
        op: BinOperator,
        left: Arc<Node>,
        right: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Unary operation (e.g., -P or not P).
    UnaryOp {
        base: NodeBase,
        op: UnaryOperator,
        operand: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Method call on a receiver (e.g., obj.method(args)).
    Method {
        base: NodeBase,
        receiver: Arc<Node>,
        name: String,
        args: NodeVector,
        metadata: Option<Arc<Metadata>>,
    },
    /// Evaluation of a name (e.g., *name).
    Eval {
        base: NodeBase,
        name: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Quotation of a process (e.g., @P).
    Quote {
        base: NodeBase,
        quotable: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Variable reference with assignment kind.
    VarRef {
        base: NodeBase,
        kind: VarRefKind,
        var: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Boolean literal (e.g., true or false).
    BoolLiteral {
        base: NodeBase,
        value: bool,
        metadata: Option<Arc<Metadata>>,
    },
    /// Integer literal (e.g., 42).
    LongLiteral {
        base: NodeBase,
        value: i64,
        metadata: Option<Arc<Metadata>>,
    },
    /// String literal (e.g., "hello").
    StringLiteral {
        base: NodeBase,
        value: String,
        metadata: Option<Arc<Metadata>>,
    },
    /// URI literal (e.g., `` http://example.com ``).
    UriLiteral {
        base: NodeBase,
        value: String,
        metadata: Option<Arc<Metadata>>,
    },
    /// Empty process (e.g., Nil).
    Nil {
        base: NodeBase,
        metadata: Option<Arc<Metadata>>,
    },
    /// List collection (e.g., [1, 2, 3]).
    List {
        base: NodeBase,
        elements: NodeVector,
        remainder: Option<Arc<Node>>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Set collection (e.g., Set(1, 2, 3)).
    Set {
        base: NodeBase,
        elements: NodeVector,
        remainder: Option<Arc<Node>>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Map collection (e.g., {k: v}).
    Map {
        base: NodeBase,
        pairs: NodePairVector,
        remainder: Option<Arc<Node>>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Tuple collection (e.g., (1, 2)).
    Tuple {
        base: NodeBase,
        elements: NodeVector,
        metadata: Option<Arc<Metadata>>,
    },
    /// Variable identifier (e.g., x).
    Var {
        base: NodeBase,
        name: String,
        metadata: Option<Arc<Metadata>>,
    },
    /// Name declaration in a new construct (e.g., x or x(uri)).
    NameDecl {
        base: NodeBase,
        var: Arc<Node>,
        uri: Option<Arc<Node>>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Declaration in a let statement (e.g., x = P).
    Decl {
        base: NodeBase,
        names: NodeVector,
        names_remainder: Option<Arc<Node>>,
        procs: NodeVector,
        metadata: Option<Arc<Metadata>>,
    },
    /// Linear binding in a for (e.g., x <- ch).
    LinearBind {
        base: NodeBase,
        names: NodeVector,
        remainder: Option<Arc<Node>>,
        source: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Repeated binding in a for (e.g., x <= ch).
    RepeatedBind {
        base: NodeBase,
        names: NodeVector,
        remainder: Option<Arc<Node>>,
        source: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Peek binding in a for (e.g., x <<- ch).
    PeekBind {
        base: NodeBase,
        names: NodeVector,
        remainder: Option<Arc<Node>>,
        source: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Comment in the source code (e.g., // text or /* text */).
    Comment {
        base: NodeBase,
        kind: CommentKind,
        metadata: Option<Arc<Metadata>>,
    },
    /// Wildcard pattern (e.g., _).
    Wildcard {
        base: NodeBase,
        metadata: Option<Arc<Metadata>>,
    },
    /// Simple type annotation (e.g., Bool).
    SimpleType {
        base: NodeBase,
        value: String,
        metadata: Option<Arc<Metadata>>,
    },
    /// Receive-send source (e.g., ch?!).
    ReceiveSendSource {
        base: NodeBase,
        name: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Send-receive source (e.g., ch!?(args)).
    SendReceiveSource {
        base: NodeBase,
        name: Arc<Node>,
        inputs: NodeVector,
        metadata: Option<Arc<Metadata>>,
    },
    /// Represents a syntax error in the source code with its erroneous subtree.
    Error {
        base: NodeBase,
        children: NodeVector,
        metadata: Option<Arc<Metadata>>,
    },
    /// Pattern disjunction (e.g., P | Q in patterns).
    Disjunction {
        base: NodeBase,
        left: Arc<Node>,
        right: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Pattern conjunction (e.g., P & Q in patterns).
    Conjunction {
        base: NodeBase,
        left: Arc<Node>,
        right: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Pattern negation (e.g., ~P in patterns).
    Negation {
        base: NodeBase,
        operand: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Unit value (e.g., ()).
    Unit {
        base: NodeBase,
        metadata: Option<Arc<Metadata>>,
    },
}

#[derive(Clone, PartialEq, Debug, Hash)]
pub enum BundleType {
    Read,
    Write,
    Equiv,
    ReadWrite,
}

#[derive(Clone, PartialEq, Debug, Hash)]
pub enum SendType {
    Single,
    Multiple,
}

#[derive(Clone, PartialEq, Debug, Hash)]
pub enum BinOperator {
    Or,
    And,
    Matches,
    Eq,
    Neq,
    Lt,
    Lte,
    Gt,
    Gte,
    Concat,
    Diff,
    Add,
    Sub,
    Interpolation,
    Mult,
    Div,
    Mod,
    Disjunction,
    Conjunction,
}

#[derive(Clone, PartialEq, Debug, Hash)]
pub enum UnaryOperator {
    Not,
    Neg,
    Negation,
}

#[derive(Clone, PartialEq, Debug, Hash, Eq, Ord, PartialOrd)]
pub enum VarRefKind {
    Bind,
    Unforgeable,
}

#[derive(Clone, PartialEq, Debug, Hash)]
pub enum CommentKind {
    Line,
    Block,
}

#[derive(Clone, Debug)]
pub struct Metadata {
    pub data: HashMap<String, Arc<dyn Any + Send + Sync>>,
}

impl Metadata {
    /// Retrieves the version from the metadata data map, defaulting to 0 if absent.
    pub fn get_version(&self) -> usize {
        self.data
            .get("version")
            .and_then(|v| v.downcast_ref::<usize>())
            .cloned()
            .unwrap_or(0)
    }

    /// Sets the version in the metadata data map.
    pub fn set_version(&mut self, version: usize) {
        self.data.insert(
            "version".to_string(),
            Arc::new(version) as Arc<dyn Any + Send + Sync>,
        );
    }
}

/// Computes absolute positions for all nodes in the IR tree, storing them in a HashMap.
/// Positions are keyed by the raw pointer to the Node cast to usize.
///
/// # Arguments
/// * root - The root node of the IR tree.
///
/// # Returns
/// A HashMap mapping node pointers (as usize) to tuples of (start, end) Positions.
pub fn compute_absolute_positions(root: &Arc<Node>) -> HashMap<usize, (Position, Position)> {
    let mut positions = HashMap::new();
    let initial_prev_end = Position {
        row: 0,
        column: 0,
        byte: 0,
    };
    compute_positions_helper(root, initial_prev_end, &mut positions);
    positions
}

/// Recursively computes absolute positions for all node types in the IR tree.
/// - Computes positions from relative offsets and child nodes.
///
/// # Arguments
/// * node - The current node being processed.
/// * prev_end - The absolute end position of the previous sibling or parent’s start if first child.
/// * positions - The HashMap storing computed (start, end) positions.
///
/// # Returns
/// The absolute end position of the current node.
#[allow(unused_assignments)]
fn compute_positions_helper(
    node: &Arc<Node>,
    prev_end: Position,
    positions: &mut HashMap<usize, (Position, Position)>,
) -> Position {
    let base = node.base();
    let key = &**node as *const Node as usize;
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
    let end = compute_end_position(start, base.span_lines(), base.span_columns(), base.length());

    // Debug logging for Block nodes to track position issues
    if matches!(&**node, Node::Block { .. }) {
        debug!("Block compute: prev_end={:?}, delta_bytes={}, computed start={:?}, length={}",
               prev_end, relative_start.delta_bytes, start, base.length());
    }

    // Debug logging for Send nodes to track position issues
    if matches!(&**node, Node::Send { .. }) {
        debug!("Send compute: prev_end={:?}, delta_bytes={}, computed start={:?}",
               prev_end, relative_start.delta_bytes, start);
    }

    // Debug logging for Var nodes to track position issues
    if let Node::Var { name, .. } = &**node {
        debug!("Var '{}': prev_end={:?}, start={:?}, length={}, end={:?}",
               name, prev_end, start, base.length(), end);
    }

    // Debug logging for Contract nodes
    if let Node::Contract { name, .. } = &**node {
        if let Node::Var { name: contract_name, .. } = &**name {
            debug!("Contract '{}': prev_end={:?}, delta=({},{},{}), computed start={:?}, end={:?}",
                contract_name, prev_end,
                relative_start.delta_lines, relative_start.delta_columns, relative_start.delta_bytes,
                start, end);
        }
    }

    let mut current_prev = start;

    // Process children
    match &**node {
        Node::Par { left, right, .. } => {
            debug!("Processing Par: current_prev before left = {:?}", current_prev);
            current_prev = compute_positions_helper(left, current_prev, positions);
            debug!("Processing Par: current_prev after left = {:?}", current_prev);
            current_prev = compute_positions_helper(right, current_prev, positions);
            debug!("Processing Par: current_prev after right = {:?}", current_prev);
        }
        Node::SendSync {
            channel, inputs, cont, ..
        } => {
            current_prev = compute_positions_helper(channel, current_prev, positions);
            for input in inputs {
                current_prev = compute_positions_helper(input, current_prev, positions);
            }
            current_prev = compute_positions_helper(cont, current_prev, positions);
        }
        Node::Send {
            channel,
            inputs,
            send_type_delta,
            ..
        } => {
            debug!("Send: start={:?}, current_prev={:?}", start, current_prev);
            let channel_end = compute_positions_helper(channel, current_prev, positions);
            debug!("Send: channel_end={:?}, send_type_delta=({},{},{})",
                   channel_end, send_type_delta.delta_lines, send_type_delta.delta_columns, send_type_delta.delta_bytes);
            let send_type_end = Position {
                row: (channel_end.row as i32 + send_type_delta.delta_lines) as usize,
                column: if send_type_delta.delta_lines == 0 {
                    (channel_end.column as i32 + send_type_delta.delta_columns) as usize
                } else {
                    send_type_delta.delta_columns as usize
                },
                byte: channel_end.byte + send_type_delta.delta_bytes,
            };
            debug!("Send: send_type_end={:?}", send_type_end);
            let mut temp_prev = send_type_end;
            for (i, input) in inputs.iter().enumerate() {
                debug!("Send: Processing input {} with temp_prev={:?}", i, temp_prev);
                temp_prev = compute_positions_helper(input, temp_prev, positions);
                debug!("Send: After input {}, temp_prev={:?}", i, temp_prev);
            }
            current_prev = temp_prev;
        }
        Node::New { decls, proc, .. } => {
            for decl in decls {
                current_prev = compute_positions_helper(decl, current_prev, positions);
            }
            current_prev = compute_positions_helper(proc, current_prev, positions);
        }
        Node::IfElse {
            condition,
            consequence,
            alternative,
            ..
        } => {
            current_prev = compute_positions_helper(condition, current_prev, positions);
            current_prev = compute_positions_helper(consequence, current_prev, positions);
            if let Some(alt) = alternative {
                current_prev = compute_positions_helper(alt, current_prev, positions);
            }
        }
        Node::Let { decls, proc, .. } => {
            for decl in decls {
                current_prev = compute_positions_helper(decl, current_prev, positions);
            }
            current_prev = compute_positions_helper(proc, current_prev, positions);
        }
        Node::Bundle { proc, .. } => {
            current_prev = compute_positions_helper(proc, current_prev, positions);
        }
        Node::Match { expression, cases, .. } => {
            current_prev = compute_positions_helper(expression, current_prev, positions);
            for (pattern, proc) in cases {
                current_prev = compute_positions_helper(pattern, current_prev, positions);
                current_prev = compute_positions_helper(proc, current_prev, positions);
            }
        }
        Node::Choice { branches, .. } => {
            for (inputs, proc) in branches {
                let mut temp_prev = current_prev;
                for input in inputs {
                    temp_prev = compute_positions_helper(input, temp_prev, positions);
                }
                current_prev = compute_positions_helper(proc, temp_prev, positions);
            }
        }
        Node::Contract {
            name,
            formals,
            formals_remainder,
            proc,
            ..
        } => {
            current_prev = compute_positions_helper(name, current_prev, positions);
            for formal in formals {
                current_prev = compute_positions_helper(formal, current_prev, positions);
            }
            if let Some(rem) = formals_remainder {
                current_prev = compute_positions_helper(rem, current_prev, positions);
            }
            current_prev = compute_positions_helper(proc, current_prev, positions);
        }
        Node::Input { receipts, proc, .. } => {
            for receipt in receipts {
                for bind in receipt {
                    current_prev = compute_positions_helper(bind, current_prev, positions);
                }
            }
            current_prev = compute_positions_helper(proc, current_prev, positions);
        }
        Node::Block { proc, .. } => {
            current_prev = compute_positions_helper(proc, current_prev, positions);
        }
        Node::Parenthesized { expr, .. } => {
            current_prev = compute_positions_helper(expr, current_prev, positions);
        }
        Node::BinOp { left, right, .. } => {
            current_prev = compute_positions_helper(left, current_prev, positions);
            current_prev = compute_positions_helper(right, current_prev, positions);
        }
        Node::UnaryOp { operand, .. } => {
            current_prev = compute_positions_helper(operand, current_prev, positions);
        }
        Node::Method { receiver, args, .. } => {
            current_prev = compute_positions_helper(receiver, current_prev, positions);
            for arg in args {
                current_prev = compute_positions_helper(arg, current_prev, positions);
            }
        }
        Node::Eval { name, .. } => {
            current_prev = compute_positions_helper(name, current_prev, positions);
        }
        Node::Quote { quotable, .. } => {
            // The quotable's delta in the IR was calculated from the end of '@',
            // so we need to start from after the '@' character (Quote start + 1 byte).
            let after_at = Position {
                row: start.row,
                column: start.column + 1,
                byte: start.byte + 1,
            };
            current_prev = compute_positions_helper(quotable, after_at, positions);
        }
        Node::VarRef { var, .. } => {
            current_prev = compute_positions_helper(var, current_prev, positions);
        }
        Node::List {
            elements,
            remainder,
            ..
        } => {
            for elem in elements {
                current_prev = compute_positions_helper(elem, current_prev, positions);
            }
            if let Some(rem) = remainder {
                current_prev = compute_positions_helper(rem, current_prev, positions);
            }
        }
        Node::Set {
            elements,
            remainder,
            ..
        } => {
            for elem in elements {
                current_prev = compute_positions_helper(elem, current_prev, positions);
            }
            if let Some(rem) = remainder {
                current_prev = compute_positions_helper(rem, current_prev, positions);
            }
        }
        Node::Map { pairs, remainder, .. } => {
            for (key, value) in pairs {
                current_prev = compute_positions_helper(key, current_prev, positions);
                current_prev = compute_positions_helper(value, current_prev, positions);
            }
            if let Some(rem) = remainder {
                current_prev = compute_positions_helper(rem, current_prev, positions);
            }
        }
        Node::Tuple { elements, .. } => {
            for elem in elements {
                current_prev = compute_positions_helper(elem, current_prev, positions);
            }
        }
        Node::NameDecl { var, uri, .. } => {
            current_prev = compute_positions_helper(var, current_prev, positions);
            if let Some(u) = uri {
                current_prev = compute_positions_helper(u, current_prev, positions);
            }
        }
        Node::Decl {
            names,
            names_remainder,
            procs,
            ..
        } => {
            for name in names {
                current_prev = compute_positions_helper(name, current_prev, positions);
            }
            if let Some(rem) = names_remainder {
                current_prev = compute_positions_helper(rem, current_prev, positions);
            }
            for proc in procs {
                current_prev = compute_positions_helper(proc, current_prev, positions);
            }
        }
        Node::LinearBind {
            names,
            remainder,
            source,
            ..
        } => {
            debug!("LinearBind: start={:?}, length={}, computed end={:?}",
                   start, base.length(), end);
            for name in names {
                current_prev = compute_positions_helper(name, current_prev, positions);
            }
            if let Some(rem) = remainder {
                current_prev = compute_positions_helper(rem, current_prev, positions);
            }
            current_prev = compute_positions_helper(source, current_prev, positions);
            debug!("LinearBind: after processing children, current_prev={:?}", current_prev);
        }
        Node::RepeatedBind {
            names,
            remainder,
            source,
            ..
        } => {
            for name in names {
                current_prev = compute_positions_helper(name, current_prev, positions);
            }
            if let Some(rem) = remainder {
                current_prev = compute_positions_helper(rem, current_prev, positions);
            }
            current_prev = compute_positions_helper(source, current_prev, positions);
        }
        Node::PeekBind {
            names,
            remainder,
            source,
            ..
        } => {
            for name in names {
                current_prev = compute_positions_helper(name, current_prev, positions);
            }
            if let Some(rem) = remainder {
                current_prev = compute_positions_helper(rem, current_prev, positions);
            }
            current_prev = compute_positions_helper(source, current_prev, positions);
        }
        Node::ReceiveSendSource { name, .. } => {
            current_prev = compute_positions_helper(name, current_prev, positions);
        }
        Node::SendReceiveSource { name, inputs, .. } => {
            current_prev = compute_positions_helper(name, current_prev, positions);
            for input in inputs {
                current_prev = compute_positions_helper(input, current_prev, positions);
            }
        }
        Node::Error { children, .. } => {
            for child in children {
                current_prev = compute_positions_helper(child, current_prev, positions);
            }
        }
        Node::Disjunction { left, right, .. } => {
            current_prev = compute_positions_helper(left, current_prev, positions);
            current_prev = compute_positions_helper(right, current_prev, positions);
        }
        Node::Conjunction { left, right, .. } => {
            current_prev = compute_positions_helper(left, current_prev, positions);
            current_prev = compute_positions_helper(right, current_prev, positions);
        }
        Node::Negation { operand, .. } => {
            current_prev = compute_positions_helper(operand, current_prev, positions);
        }
        Node::Unit { .. } => {}
        _ => {}
    }

    // Compute the actual end position as the maximum of computed end and last child's end
    // to handle structural nodes like Par that have no content of their own
    let actual_end = if current_prev.byte > end.byte {
        current_prev
    } else {
        end
    };

    // Insert position with the corrected end
    positions.insert(key, (start, actual_end));

    actual_end
}

/// Computes the absolute end position of a node given its start position, span lines, span columns, and length.
/// Adjusts row and column based on span information.
///
/// # Arguments
/// * start - The absolute start position.
/// * span_lines - The number of lines spanned by the node.
/// * span_columns - The number of columns on the last line.
/// * length - The length of the node’s text in bytes.
///
/// # Returns
/// The computed absolute end position.
pub fn compute_end_position(
    start: Position,
    span_lines: usize,
    span_columns: usize,
    length: usize,
) -> Position {
    Position {
        row: start.row + span_lines,
        column: if span_lines == 0 {
            start.column + span_columns
        } else {
            span_columns
        },
        byte: start.byte + length,
    }
}

/// Matches a pattern against a concrete node, with substitution for variables.
pub fn match_pat(pat: &Arc<Node>, concrete: &Arc<Node>, subst: &mut HashMap<String, Arc<Node>>) -> bool {
    match (&**pat, &**concrete) {
        (Node::Wildcard { .. }, _) => true,
        (Node::Var { name: p_name, .. }, _) => {
            if let Some(bound) = subst.get(p_name) {
                **bound == **concrete
            } else {
                subst.insert(p_name.clone(), concrete.clone());
                true
            }
        }
        (
            Node::Quote {
                quotable: p_q, ..
            },
            Node::Quote {
                quotable: c_q, ..
            },
        ) => match_pat(p_q, c_q, subst),
        (Node::Eval { name: p_n, .. }, Node::Eval { name: c_n, .. }) => match_pat(p_n, c_n, subst),
        (
            Node::VarRef {
                kind: p_k,
                var: p_v,
                ..
            },
            Node::VarRef {
                kind: c_k,
                var: c_v,
                ..
            },
        ) => p_k == c_k && match_pat(p_v, c_v, subst),
        (
            Node::List {
                elements: p_e,
                remainder: p_r,
                ..
            },
            Node::List {
                elements: c_e,
                remainder: c_r,
                ..
            },
        ) => {
            if p_e.len() > c_e.len() {
                return false;
            }
            for (p, c) in p_e.iter().zip(c_e.iter()) {
                if !match_pat(p, c, subst) {
                    return false;
                }
            }
            let rem_c_elements = c_e.iter().skip(p_e.len()).cloned().collect::<Vector<_, ArcK>>();
            let rem_base = NodeBase::new(
                RelativePosition {
                    delta_lines: 0,
                    delta_columns: 0,
                    delta_bytes: 0,
                },
                0,
                0,
                0,
            );
            let rem_list = Arc::new(Node::List {
                base: rem_base,
                elements: rem_c_elements,
                remainder: c_r.clone(),
                metadata: None,
            });
            if let Some(r) = p_r {
                match_pat(r, &rem_list, subst)
            } else if let Node::List {
                elements,
                remainder,
                ..
            } = &*rem_list {
                elements.is_empty() && remainder.is_none()
            } else {
                false
            }
        }
        (Node::Tuple { elements: p_e, .. }, Node::Tuple { elements: c_e, .. }) => {
            if p_e.len() != c_e.len() {
                false
            } else {
                p_e.iter()
                    .zip(c_e.iter())
                    .all(|(p, c)| match_pat(p, c, subst))
            }
        }
        (
            Node::Set {
                elements: p_e,
                remainder: p_r,
                ..
            },
            Node::Set {
                elements: c_e,
                remainder: c_r,
                ..
            },
        ) => {
            let mut p_sorted: Vec<&Arc<Node>> = p_e.iter().collect();
            p_sorted.sort_by(|a, b| Node::node_cmp(a, b));
            let mut c_sorted: Vec<&Arc<Node>> = c_e.iter().collect();
            c_sorted.sort_by(|a, b| Node::node_cmp(a, b));
            if p_sorted.len() > c_sorted.len() {
                return false;
            }
            for (p, c) in p_sorted.iter().zip(c_sorted.iter()) {
                if !match_pat(p, c, subst) {
                    return false;
                }
            }
            let rem_c_elements = c_e.iter().skip(p_e.len()).cloned().collect::<Vector<_, ArcK>>();
            let rem_base = NodeBase::new(
                RelativePosition {
                    delta_lines: 0,
                    delta_columns: 0,
                    delta_bytes: 0,
                },
                0,
                0,
                0,
            );
            let rem_set = Arc::new(Node::Set {
                base: rem_base,
                elements: rem_c_elements,
                remainder: c_r.clone(),
                metadata: None,
            });
            if let Some(r) = p_r {
                match_pat(r, &rem_set, subst)
            } else if let Node::Set {
                elements,
                remainder,
                ..
            } = &*rem_set {
                elements.is_empty() && remainder.is_none()
            } else {
                false
            }
        }
        (
            Node::Map {
                pairs: p_pairs,
                remainder: p_r,
                ..
            },
            Node::Map {
                pairs: c_pairs,
                remainder: c_r,
                ..
            },
        ) => {
            let mut p_sorted: Vec<(&Arc<Node>, &Arc<Node>)> =
                p_pairs.iter().map(|(k, v)| (k, v)).collect();
            p_sorted.sort_by(|(ka, _), (kb, _)| Node::node_cmp(ka, kb));
            let mut c_sorted: Vec<(&Arc<Node>, &Arc<Node>)> =
                c_pairs.iter().map(|(k, v)| (k, v)).collect();
            c_sorted.sort_by(|(ka, _), (kb, _)| Node::node_cmp(ka, kb));
            if p_sorted.len() > c_sorted.len() {
                return false;
            }
            for ((p_k, p_v), (c_k, c_v)) in p_sorted.iter().zip(c_sorted.iter()) {
                if !match_pat(p_k, c_k, subst) || !match_pat(p_v, c_v, subst) {
                    return false;
                }
            }
            let rem_c_pairs = c_pairs.iter().skip(p_pairs.len()).cloned().collect::<Vector<_, ArcK>>();
            let rem_base = NodeBase::new(
                RelativePosition {
                    delta_lines: 0,
                    delta_columns: 0,
                    delta_bytes: 0,
                },
                0,
                0,
                0,
            );
            let rem_map = Arc::new(Node::Map {
                base: rem_base,
                pairs: rem_c_pairs,
                remainder: c_r.clone(),
                metadata: None,
            });
            if let Some(r) = p_r {
                match_pat(r, &rem_map, subst)
            } else if let Node::Map {
                pairs,
                remainder,
                ..
            } = &*rem_map {
                pairs.is_empty() && remainder.is_none()
            } else {
                false
            }
        }
        (Node::BoolLiteral { value: p, .. }, Node::BoolLiteral { value: c, .. }) => p == c,
        (Node::LongLiteral { value: p, .. }, Node::LongLiteral { value: c, .. }) => p == c,
        (Node::StringLiteral { value: p, .. }, Node::StringLiteral { value: c, .. }) => p == c,
        (Node::UriLiteral { value: p, .. }, Node::UriLiteral { value: c, .. }) => p == c,
        (Node::SimpleType { value: p, .. }, Node::SimpleType { value: c, .. }) => p == c,
        (Node::Nil { .. }, Node::Nil { .. }) => true,
        (Node::Unit { .. }, Node::Unit { .. }) => true,
        (Node::Disjunction { left: p_l, right: p_r, .. }, Node::Disjunction { left: c_l, right: c_r, .. }) => {
            match_pat(p_l, c_l, subst) && match_pat(p_r, c_r, subst)
        }
        (Node::Conjunction { left: p_l, right: p_r, .. }, Node::Conjunction { left: c_l, right: c_r, .. }) => {
            match_pat(p_l, c_l, subst) && match_pat(p_r, c_r, subst)
        }
        (Node::Negation { operand: p_o, .. }, Node::Negation { operand: c_o, .. }) => {
            match_pat(p_o, c_o, subst)
        }
        (Node::Parenthesized { expr: p_e, .. }, Node::Parenthesized { expr: c_e, .. }) => {
            match_pat(p_e, c_e, subst)
        }
        _ => false,
    }
}

/// Matches a contract against a call's channel and inputs.
/// Check if two nodes are equal for contract name matching (avoids pattern matching's Var unification)
fn contract_names_equal(a: &Arc<Node>, b: &Arc<Node>) -> bool {
    match (&**a, &**b) {
        // Fast path: pointer equality
        _ if Arc::ptr_eq(a, b) => true,
        // Var nodes: compare names by reference (cheap since names are strings in Arc)
        (Node::Var { name: a_name, .. }, Node::Var { name: b_name, .. }) => a_name == b_name,
        // Quote nodes: recursively check quotable
        (Node::Quote { quotable: a_q, .. }, Node::Quote { quotable: b_q, .. }) => contract_names_equal(a_q, b_q),
        // Eval nodes: recursively check name
        (Node::Eval { name: a_n, .. }, Node::Eval { name: b_n, .. }) => contract_names_equal(a_n, b_n),
        // VarRef nodes: check kind and var
        (Node::VarRef { kind: a_k, var: a_v, .. }, Node::VarRef { kind: b_k, var: b_v, .. }) => {
            a_k == b_k && contract_names_equal(a_v, b_v)
        }
        // Different node types or other cases: not equal
        _ => false,
    }
}

pub fn match_contract(channel: &Arc<Node>, inputs: &NodeVector, contract: &Arc<Node>) -> bool {
    if let Node::Contract {
        name,
        formals,
        formals_remainder,
        ..
    } = &**contract
    {
        // For contract name matching, use exact equality instead of pattern matching
        // This avoids the issue where match_pat treats any Var as a pattern variable
        if !contract_names_equal(name, channel) {
            return false;
        }

        // For parameters, use pattern matching as before
        let mut subst = HashMap::new();
        let min_len = formals.len();
        if formals_remainder.is_none() && inputs.len() != min_len {
            return false;
        }
        if inputs.len() < min_len {
            return false;
        }
        for (f, a) in formals.iter().zip(inputs.iter()) {
            if !match_pat(f, a, &mut subst) {
                return false;
            }
        }
        if let Some(rem) = formals_remainder {
            let remaining_elements = inputs
                .iter()
                .skip(min_len)
                .cloned()
                .collect::<Vector<_, ArcK>>();
            let rem_base = NodeBase::new(
                RelativePosition {
                    delta_lines: 0,
                    delta_columns: 0,
                    delta_bytes: 0,
                },
                0,
                0,
                0,
            );
            let remaining_list = Arc::new(Node::List {
                base: rem_base,
                elements: remaining_elements,
                remainder: None,
                metadata: None,
            });
            match_pat(rem, &remaining_list, &mut subst)
        } else {
            true
        }
    } else {
        false
    }
}

/// Collects all contract nodes from the IR tree.
pub fn collect_contracts(node: &Arc<Node>, contracts: &mut Vec<Arc<Node>>) {
    match &**node {
        Node::Contract { .. } => contracts.push(node.clone()),
        Node::Par { left, right, .. } => {
            collect_contracts(left, contracts);
            collect_contracts(right, contracts);
        }
        Node::SendSync {
            channel, inputs, cont, ..
        } => {
            collect_contracts(channel, contracts);
            for input in inputs {
                collect_contracts(input, contracts);
            }
            collect_contracts(cont, contracts);
        }
        Node::Send { channel, inputs, .. } => {
            collect_contracts(channel, contracts);
            for input in inputs {
                collect_contracts(input, contracts);
            }
        }
        Node::New { decls, proc, .. } => {
            for decl in decls {
                collect_contracts(decl, contracts);
            }
            collect_contracts(proc, contracts);
        }
        Node::IfElse {
            condition,
            consequence,
            alternative,
            ..
        } => {
            collect_contracts(condition, contracts);
            collect_contracts(consequence, contracts);
            if let Some(alt) = alternative {
                collect_contracts(alt, contracts);
            }
        }
        Node::Let { decls, proc, .. } => {
            for decl in decls {
                collect_contracts(decl, contracts);
            }
            collect_contracts(proc, contracts);
        }
        Node::Bundle { proc, .. } => collect_contracts(proc, contracts),
        Node::Match {
            expression, cases, ..
        } => {
            collect_contracts(expression, contracts);
            for (pat, proc) in cases {
                collect_contracts(pat, contracts);
                collect_contracts(proc, contracts);
            }
        }
        Node::Choice { branches, .. } => {
            for (inputs, proc) in branches {
                for input in inputs {
                    collect_contracts(input, contracts);
                }
                collect_contracts(proc, contracts);
            }
        }
        Node::Input { receipts, proc, .. } => {
            for receipt in receipts {
                for bind in receipt {
                    collect_contracts(bind, contracts);
                }
            }
            collect_contracts(proc, contracts);
        }
        Node::Block { proc, .. } => collect_contracts(proc, contracts),
        Node::Parenthesized { expr, .. } => collect_contracts(expr, contracts),
        Node::BinOp { left, right, .. } => {
            collect_contracts(left, contracts);
            collect_contracts(right, contracts);
        }
        Node::UnaryOp { operand, .. } => collect_contracts(operand, contracts),
        Node::Method {
            receiver, args, ..
        } => {
            collect_contracts(receiver, contracts);
            for arg in args {
                collect_contracts(arg, contracts);
            }
        }
        Node::Eval { name, .. } => collect_contracts(name, contracts),
        Node::Quote { quotable, .. } => collect_contracts(quotable, contracts),
        Node::VarRef { var, .. } => collect_contracts(var, contracts),
        Node::List {
            elements,
            remainder,
            ..
        } => {
            for elem in elements {
                collect_contracts(elem, contracts);
            }
            if let Some(rem) = remainder {
                collect_contracts(rem, contracts);
            }
        }
        Node::Set {
            elements,
            remainder,
            ..
        } => {
            for elem in elements {
                collect_contracts(elem, contracts);
            }
            if let Some(rem) = remainder {
                collect_contracts(rem, contracts);
            }
        }
        Node::Map {
            pairs, remainder, ..
        } => {
            for (key, value) in pairs {
                collect_contracts(key, contracts);
                collect_contracts(value, contracts);
            }
            if let Some(rem) = remainder {
                collect_contracts(rem, contracts);
            }
        }
        Node::Tuple { elements, .. } => {
            for elem in elements {
                collect_contracts(elem, contracts);
            }
        }
        Node::NameDecl { var, uri, .. } => {
            collect_contracts(var, contracts);
            if let Some(u) = uri {
                collect_contracts(u, contracts);
            }
        }
        Node::Decl {
            names,
            names_remainder,
            procs,
            ..
        } => {
            for name in names {
                collect_contracts(name, contracts);
            }
            if let Some(rem) = names_remainder {
                collect_contracts(rem, contracts);
            }
            for proc in procs {
                collect_contracts(proc, contracts);
            }
        }
        Node::LinearBind {
            names,
            remainder,
            source,
            ..
        } => {
            for name in names {
                collect_contracts(name, contracts);
            }
            if let Some(rem) = remainder {
                collect_contracts(rem, contracts);
            }
            collect_contracts(source, contracts);
        }
        Node::RepeatedBind {
            names,
            remainder,
            source,
            ..
        } => {
            for name in names {
                collect_contracts(name, contracts);
            }
            if let Some(rem) = remainder {
                collect_contracts(rem, contracts);
            }
            collect_contracts(source, contracts);
        }
        Node::PeekBind {
            names,
            remainder,
            source,
            ..
        } => {
            for name in names {
                collect_contracts(name, contracts);
            }
            if let Some(rem) = remainder {
                collect_contracts(rem, contracts);
            }
            collect_contracts(source, contracts);
        }
        Node::ReceiveSendSource { name, .. } => collect_contracts(name, contracts),
        Node::SendReceiveSource { name, inputs, .. } => {
            collect_contracts(name, contracts);
            for input in inputs {
                collect_contracts(input, contracts);
            }
        }
        Node::Error { children, .. } => {
            for child in children {
                collect_contracts(child, contracts);
            }
        }
        Node::Disjunction { left, right, .. } => {
            collect_contracts(left, contracts);
            collect_contracts(right, contracts);
        }
        Node::Conjunction { left, right, .. } => {
            collect_contracts(left, contracts);
            collect_contracts(right, contracts);
        }
        Node::Negation { operand, .. } => collect_contracts(operand, contracts),
        Node::Unit { .. } => {}
        _ => {}
    }
}

/// Collects all call nodes (Send and SendSync) from the IR tree.
pub fn collect_calls(node: &Arc<Node>, calls: &mut Vec<Arc<Node>>) {
    match &**node {
        Node::Send { .. } | Node::SendSync { .. } => calls.push(node.clone()),
        Node::Par { left, right, .. } => {
            collect_calls(left, calls);
            collect_calls(right, calls);
        }
        Node::New { decls, proc, .. } => {
            for decl in decls {
                collect_calls(decl, calls);
            }
            collect_calls(proc, calls);
        }
        Node::IfElse {
            condition,
            consequence,
            alternative,
            ..
        } => {
            collect_calls(condition, calls);
            collect_calls(consequence, calls);
            if let Some(alt) = alternative {
                collect_calls(alt, calls);
            }
        }
        Node::Let { decls, proc, .. } => {
            for decl in decls {
                collect_calls(decl, calls);
            }
            collect_calls(proc, calls);
        }
        Node::Bundle { proc, .. } => collect_calls(proc, calls),
        Node::Match {
            expression, cases, ..
        } => {
            collect_calls(expression, calls);
            for (pat, proc) in cases {
                collect_calls(pat, calls);
                collect_calls(proc, calls);
            }
        }
        Node::Choice { branches, .. } => {
            for (inputs, proc) in branches {
                for input in inputs {
                    collect_calls(input, calls);
                }
                collect_calls(proc, calls);
            }
        }
        Node::Contract {
            name,
            formals,
            formals_remainder,
            proc,
            ..
        } => {
            collect_calls(name, calls);
            for formal in formals {
                collect_calls(formal, calls);
            }
            if let Some(rem) = formals_remainder {
                collect_calls(rem, calls);
            }
            collect_calls(proc, calls);
        }
        Node::Input { receipts, proc, .. } => {
            for receipt in receipts {
                for bind in receipt {
                    collect_calls(bind, calls);
                }
            }
            collect_calls(proc, calls);
        }
        Node::Block { proc, .. } => collect_calls(proc, calls),
        Node::Parenthesized { expr, .. } => collect_calls(expr, calls),
        Node::BinOp { left, right, .. } => {
            collect_calls(left, calls);
            collect_calls(right, calls);
        }
        Node::UnaryOp { operand, .. } => collect_calls(operand, calls),
        Node::Method {
            receiver, args, ..
        } => {
            collect_calls(receiver, calls);
            for arg in args {
                collect_calls(arg, calls);
            }
        }
        Node::Eval { name, .. } => collect_calls(name, calls),
        Node::Quote { quotable, .. } => collect_calls(quotable, calls),
        Node::VarRef { var, .. } => collect_calls(var, calls),
        Node::List {
            elements,
            remainder,
            ..
        } => {
            for elem in elements {
                collect_calls(elem, calls);
            }
            if let Some(rem) = remainder {
                collect_calls(rem, calls);
            }
        }
        Node::Set {
            elements,
            remainder,
            ..
        } => {
            for elem in elements {
                collect_calls(elem, calls);
            }
            if let Some(rem) = remainder {
                collect_calls(rem, calls);
            }
        }
        Node::Map {
            pairs, remainder, ..
        } => {
            for (key, value) in pairs {
                collect_calls(key, calls);
                collect_calls(value, calls);
            }
            if let Some(rem) = remainder {
                collect_calls(rem, calls);
            }
        }
        Node::Tuple { elements, .. } => {
            for elem in elements {
                collect_calls(elem, calls);
            }
        }
        Node::NameDecl { var, uri, .. } => {
            collect_calls(var, calls);
            if let Some(u) = uri {
                collect_calls(u, calls);
            }
        }
        Node::Decl {
            names,
            names_remainder,
            procs,
            ..
        } => {
            for name in names {
                collect_calls(name, calls);
            }
            if let Some(rem) = names_remainder {
                collect_calls(rem, calls);
            }
            for proc in procs {
                collect_calls(proc, calls);
            }
        }
        Node::LinearBind {
            names,
            remainder,
            source,
            ..
        } => {
            for name in names {
                collect_calls(name, calls);
            }
            if let Some(rem) = remainder {
                collect_calls(rem, calls);
            }
            collect_calls(source, calls);
        }
        Node::RepeatedBind {
            names,
            remainder,
            source,
            ..
        } => {
            for name in names {
                collect_calls(name, calls);
            }
            if let Some(rem) = remainder {
                collect_calls(rem, calls);
            }
            collect_calls(source, calls);
        }
        Node::PeekBind {
            names,
            remainder,
            source,
            ..
        } => {
            for name in names {
                collect_calls(name, calls);
            }
            if let Some(rem) = remainder {
                collect_calls(rem, calls);
            }
            collect_calls(source, calls);
        }
        Node::ReceiveSendSource { name, .. } => collect_calls(name, calls),
        Node::SendReceiveSource { name, inputs, .. } => {
            collect_calls(name, calls);
            for input in inputs {
                collect_calls(input, calls);
            }
        }
        Node::Error { children, .. } => {
            for child in children {
                collect_calls(child, calls);
            }
        }
        Node::Disjunction { left, right, .. } => {
            collect_calls(left, calls);
            collect_calls(right, calls);
        }
        Node::Conjunction { left, right, .. } => {
            collect_calls(left, calls);
            collect_calls(right, calls);
        }
        Node::Negation { operand, .. } => collect_calls(operand, calls),
        Node::Unit { .. } => {},
        _ => {},
    }
}

/// Traverses the tree with path tracking for finding node at position.
pub fn find_node_at_position_with_path(
    root: &Arc<Node>,
    positions: &HashMap<usize, (Position, Position)>,
    position: Position,
) -> Option<(Arc<Node>, Vec<Arc<Node>>)> {
    let mut path = Vec::new();
    let mut best: Option<(Arc<Node>, Vec<Arc<Node>>, usize)> = None;
    traverse_with_path(root, position, positions, &mut path, &mut best, 0);
    best.map(|(node, p, _)| (node, p))
}

fn traverse_with_path(
    node: &Arc<Node>,
    pos: Position,
    positions: &HashMap<usize, (Position, Position)>,
    path: &mut Vec<Arc<Node>>,
    best: &mut Option<(Arc<Node>, Vec<Arc<Node>>, usize)>,
    depth: usize,
) {
    path.push(node.clone());
    let key = &**node as *const Node as usize;
    if let Some(&(start, end)) = positions.get(&key) {
        debug!("traverse_with_path: Checking node {:p} at depth {} with range [{}, {}] for position {}",
               &**node, depth, start.byte, end.byte, pos.byte);
        if start.byte <= pos.byte && pos.byte <= end.byte {
            let is_better = best.as_ref().map_or(true, |(_, _, b_depth)| depth > *b_depth);
            debug!("  traverse_with_path: Node {:p} contains position. is_better={} (current depth={}, best depth={:?})",
                   &**node, is_better, depth, best.as_ref().map(|(_, _, d)| d));
            if is_better {
                debug!("  traverse_with_path: Setting node {:p} as new best at depth {}", &**node, depth);
                *best = Some((node.clone(), path.clone(), depth));
            }
        }
    }
    match &**node {
        Node::Par { left, right, .. } => {
            traverse_with_path(left, pos, positions, path, best, depth + 1);
            traverse_with_path(right, pos, positions, path, best, depth + 1);
        }
        Node::SendSync {
            channel, inputs, cont, ..
        } => {
            traverse_with_path(channel, pos, positions, path, best, depth + 1);
            for input in inputs {
                traverse_with_path(input, pos, positions, path, best, depth + 1);
            }
            traverse_with_path(cont, pos, positions, path, best, depth + 1);
        }
        Node::Send { channel, inputs, .. } => {
            traverse_with_path(channel, pos, positions, path, best, depth + 1);
            for input in inputs {
                traverse_with_path(input, pos, positions, path, best, depth + 1);
            }
        }
        Node::New { decls, proc, .. } => {
            for decl in decls {
                traverse_with_path(decl, pos, positions, path, best, depth + 1);
            }
            traverse_with_path(proc, pos, positions, path, best, depth + 1);
        }
        Node::IfElse {
            condition,
            consequence,
            alternative,
            ..
        } => {
            traverse_with_path(condition, pos, positions, path, best, depth + 1);
            traverse_with_path(consequence, pos, positions, path, best, depth + 1);
            if let Some(alt) = alternative {
                traverse_with_path(alt, pos, positions, path, best, depth + 1);
            }
        }
        Node::Let { decls, proc, .. } => {
            for decl in decls {
                traverse_with_path(decl, pos, positions, path, best, depth + 1);
            }
            traverse_with_path(proc, pos, positions, path, best, depth + 1);
        }
        Node::Bundle { proc, .. } => traverse_with_path(proc, pos, positions, path, best, depth + 1),
        Node::Match { expression, cases, .. } => {
            traverse_with_path(expression, pos, positions, path, best, depth + 1);
            for (pat, proc) in cases {
                traverse_with_path(pat, pos, positions, path, best, depth + 1);
                traverse_with_path(proc, pos, positions, path, best, depth + 1);
            }
        }
        Node::Choice { branches, .. } => {
            for (inputs, proc) in branches {
                for input in inputs {
                    traverse_with_path(input, pos, positions, path, best, depth + 1);
                }
                traverse_with_path(proc, pos, positions, path, best, depth + 1);
            }
        }
        Node::Contract { name, formals, formals_remainder, proc, .. } => {
            traverse_with_path(name, pos, positions, path, best, depth + 1);
            for formal in formals {
                traverse_with_path(formal, pos, positions, path, best, depth + 1);
            }
            if let Some(rem) = formals_remainder {
                traverse_with_path(rem, pos, positions, path, best, depth + 1);
            }
            traverse_with_path(proc, pos, positions, path, best, depth + 1);
        }
        Node::Input { receipts, proc, .. } => {
            for receipt in receipts {
                for bind in receipt {
                    traverse_with_path(bind, pos, positions, path, best, depth + 1);
                }
            }
            traverse_with_path(proc, pos, positions, path, best, depth + 1);
        }
        Node::Block { proc, .. } => traverse_with_path(proc, pos, positions, path, best, depth + 1),
        Node::Parenthesized { expr, .. } => traverse_with_path(expr, pos, positions, path, best, depth + 1),
        Node::BinOp { left, right, .. } => {
            traverse_with_path(left, pos, positions, path, best, depth + 1);
            traverse_with_path(right, pos, positions, path, best, depth + 1);
        }
        Node::UnaryOp { operand, .. } => traverse_with_path(operand, pos, positions, path, best, depth + 1),
        Node::Method { receiver, args, .. } => {
            traverse_with_path(receiver, pos, positions, path, best, depth + 1);
            for arg in args {
                traverse_with_path(arg, pos, positions, path, best, depth + 1);
            }
        }
        Node::Eval { name, .. } => traverse_with_path(name, pos, positions, path, best, depth + 1),
        Node::Quote { quotable, .. } => traverse_with_path(quotable, pos, positions, path, best, depth + 1),
        Node::VarRef { var, .. } => traverse_with_path(var, pos, positions, path, best, depth + 1),
        Node::List { elements, remainder, .. } => {
            for elem in elements {
                traverse_with_path(elem, pos, positions, path, best, depth + 1);
            }
            if let Some(rem) = remainder {
                traverse_with_path(rem, pos, positions, path, best, depth + 1);
            }
        }
        Node::Set { elements, remainder, .. } => {
            for elem in elements {
                traverse_with_path(elem, pos, positions, path, best, depth + 1);
            }
            if let Some(rem) = remainder {
                traverse_with_path(rem, pos, positions, path, best, depth + 1);
            }
        }
        Node::Map { pairs, remainder, .. } => {
            for (key, value) in pairs {
                traverse_with_path(key, pos, positions, path, best, depth + 1);
                traverse_with_path(value, pos, positions, path, best, depth + 1);
            }
            if let Some(rem) = remainder {
                traverse_with_path(rem, pos, positions, path, best, depth + 1);
            }
        }
        Node::Tuple { elements, .. } => {
            for elem in elements {
                traverse_with_path(elem, pos, positions, path, best, depth + 1);
            }
        }
        Node::NameDecl { var, uri, .. } => {
            traverse_with_path(var, pos, positions, path, best, depth + 1);
            if let Some(u) = uri {
                traverse_with_path(u, pos, positions, path, best, depth + 1);
            }
        }
        Node::Decl { names, names_remainder, procs, .. } => {
            for name in names {
                traverse_with_path(name, pos, positions, path, best, depth + 1);
            }
            if let Some(rem) = names_remainder {
                traverse_with_path(rem, pos, positions, path, best, depth + 1);
            }
            for proc in procs {
                traverse_with_path(proc, pos, positions, path, best, depth + 1);
            }
        }
        Node::LinearBind { names, remainder, source, .. } => {
            for name in names {
                traverse_with_path(name, pos, positions, path, best, depth + 1);
            }
            if let Some(rem) = remainder {
                traverse_with_path(rem, pos, positions, path, best, depth + 1);
            }
            traverse_with_path(source, pos, positions, path, best, depth + 1);
        }
        Node::RepeatedBind { names, remainder, source, .. } => {
            for name in names {
                traverse_with_path(name, pos, positions, path, best, depth + 1);
            }
            if let Some(rem) = remainder {
                traverse_with_path(rem, pos, positions, path, best, depth + 1);
            }
            traverse_with_path(source, pos, positions, path, best, depth + 1);
        }
        Node::PeekBind { names, remainder, source, .. } => {
            for name in names {
                traverse_with_path(name, pos, positions, path, best, depth + 1);
            }
            if let Some(rem) = remainder {
                traverse_with_path(rem, pos, positions, path, best, depth + 1);
            }
            traverse_with_path(source, pos, positions, path, best, depth + 1);
        }
        Node::ReceiveSendSource { name, .. } => traverse_with_path(name, pos, positions, path, best, depth + 1),
        Node::SendReceiveSource { name, inputs, .. } => {
            traverse_with_path(name, pos, positions, path, best, depth + 1);
            for input in inputs {
                traverse_with_path(input, pos, positions, path, best, depth + 1);
            }
        }
        Node::Error { children, .. } => {
            for child in children {
                traverse_with_path(child, pos, positions, path, best, depth + 1);
            }
        }
        Node::Disjunction { left, right, .. } => {
            traverse_with_path(left, pos, positions, path, best, depth + 1);
            traverse_with_path(right, pos, positions, path, best, depth + 1);
        }
        Node::Conjunction { left, right, .. } => {
            traverse_with_path(left, pos, positions, path, best, depth + 1);
            traverse_with_path(right, pos, positions, path, best, depth + 1);
        }
        Node::Negation { operand, .. } => traverse_with_path(operand, pos, positions, path, best, depth + 1),
        Node::Unit { .. } => {}
        _ => {}
    }
    path.pop();
}

fn traverse(
    node: &Arc<Node>,
    pos: Position,
    positions: &HashMap<usize, (Position, Position)>,
    best: &mut Option<(Arc<Node>, Position, usize)>,
    depth: usize,
) {
    let key = &**node as *const Node as usize;
    if let Some(&(start, end)) = positions.get(&key) {
        debug!("traverse: Checking node {:p} at depth {} with range [{}, {}] for position {}",
               &**node, depth, start.byte, end.byte, pos.byte);
        if start.byte <= pos.byte && pos.byte <= end.byte {
            let is_better = best.as_ref().map_or(true, |(_, _, b_depth)| depth > *b_depth);
            debug!("  traverse: Node {:p} contains position. is_better={} (current depth={}, best depth={:?})",
                   &**node, is_better, depth, best.as_ref().map(|(_, _, d)| d));
            if is_better {
                debug!("  traverse: Setting node {:p} as new best at depth {}", &**node, depth);
                *best = Some((node.clone(), start, depth));
            }
        }
    }
    match &**node {
        Node::Par { left, right, .. } => {
            traverse(left, pos, positions, best, depth + 1);
            traverse(right, pos, positions, best, depth + 1);
        }
        Node::SendSync { channel, inputs, cont, .. } => {
            traverse(channel, pos, positions, best, depth + 1);
            for input in inputs {
                traverse(input, pos, positions, best, depth + 1);
            }
            traverse(cont, pos, positions, best, depth + 1);
        }
        Node::Send { channel, inputs, .. } => {
            traverse(channel, pos, positions, best, depth + 1);
            for input in inputs {
                traverse(input, pos, positions, best, depth + 1);
            }
        }
        Node::New { decls, proc, .. } => {
            for decl in decls {
                traverse(decl, pos, positions, best, depth + 1);
            }
            traverse(proc, pos, positions, best, depth + 1);
        }
        Node::IfElse {
            condition,
            consequence,
            alternative,
            ..
        } => {
            traverse(condition, pos, positions, best, depth + 1);
            traverse(consequence, pos, positions, best, depth + 1);
            if let Some(alt) = alternative {
                traverse(alt, pos, positions, best, depth + 1);
            }
        }
        Node::Let { decls, proc, .. } => {
            for decl in decls {
                traverse(decl, pos, positions, best, depth + 1);
            }
            traverse(proc, pos, positions, best, depth + 1);
        }
        Node::Bundle { proc, .. } => traverse(proc, pos, positions, best, depth + 1),
        Node::Match { expression, cases, .. } => {
            traverse(expression, pos, positions, best, depth + 1);
            for (pat, proc) in cases {
                traverse(pat, pos, positions, best, depth + 1);
                traverse(proc, pos, positions, best, depth + 1);
            }
        }
        Node::Choice { branches, .. } => {
            for (inputs, proc) in branches {
                for input in inputs {
                    traverse(input, pos, positions, best, depth + 1);
                }
                traverse(proc, pos, positions, best, depth + 1);
            }
        }
        Node::Contract { name, formals, formals_remainder, proc, .. } => {
            traverse(name, pos, positions, best, depth + 1);
            for formal in formals {
                traverse(formal, pos, positions, best, depth + 1);
            }
            if let Some(rem) = formals_remainder {
                traverse(rem, pos, positions, best, depth + 1);
            }
            traverse(proc, pos, positions, best, depth + 1);
        }
        Node::Input { receipts, proc, .. } => {
            for receipt in receipts {
                for bind in receipt {
                    traverse(bind, pos, positions, best, depth + 1);
                }
            }
            traverse(proc, pos, positions, best, depth + 1);
        }
        Node::Block { proc, .. } => traverse(proc, pos, positions, best, depth + 1),
        Node::Parenthesized { expr, .. } => traverse(expr, pos, positions, best, depth + 1),
        Node::BinOp { left, right, .. } => {
            traverse(left, pos, positions, best, depth + 1);
            traverse(right, pos, positions, best, depth + 1);
        }
        Node::UnaryOp { operand, .. } => traverse(operand, pos, positions, best, depth + 1),
        Node::Method { receiver, args, .. } => {
            traverse(receiver, pos, positions, best, depth + 1);
            for arg in args {
                traverse(arg, pos, positions, best, depth + 1);
            }
        }
        Node::Eval { name, .. } => traverse(name, pos, positions, best, depth + 1),
        Node::Quote { quotable, .. } => traverse(quotable, pos, positions, best, depth + 1),
        Node::VarRef { var, .. } => traverse(var, pos, positions, best, depth + 1),
        Node::List { elements, remainder, .. } => {
            for elem in elements {
                traverse(elem, pos, positions, best, depth + 1);
            }
            if let Some(rem) = remainder {
                traverse(rem, pos, positions, best, depth + 1);
            }
        }
        Node::Set { elements, remainder, .. } => {
            for elem in elements {
                traverse(elem, pos, positions, best, depth + 1);
            }
            if let Some(rem) = remainder {
                traverse(rem, pos, positions, best, depth + 1);
            }
        }
        Node::Map { pairs, remainder, .. } => {
            for (key, value) in pairs {
                traverse(key, pos, positions, best, depth + 1);
                traverse(value, pos, positions, best, depth + 1);
            }
            if let Some(rem) = remainder {
                traverse(rem, pos, positions, best, depth + 1);
            }
        }
        Node::Tuple { elements, .. } => {
            for elem in elements {
                traverse(elem, pos, positions, best, depth + 1);
            }
        }
        Node::NameDecl { var, uri, .. } => {
            traverse(var, pos, positions, best, depth + 1);
            if let Some(u) = uri {
                traverse(u, pos, positions, best, depth + 1);
            }
        }
        Node::Decl { names, names_remainder, procs, .. } => {
            for name in names {
                traverse(name, pos, positions, best, depth + 1);
            }
            if let Some(rem) = names_remainder {
                traverse(rem, pos, positions, best, depth + 1);
            }
            for proc in procs {
                traverse(proc, pos, positions, best, depth + 1);
            }
        }
        Node::LinearBind { names, remainder, source, .. } => {
            for name in names {
                traverse(name, pos, positions, best, depth + 1);
            }
            if let Some(rem) = remainder {
                traverse(rem, pos, positions, best, depth + 1);
            }
            traverse(source, pos, positions, best, depth + 1);
        }
        Node::RepeatedBind { names, remainder, source, .. } => {
            for name in names {
                traverse(name, pos, positions, best, depth + 1);
            }
            if let Some(rem) = remainder {
                traverse(rem, pos, positions, best, depth + 1);
            }
            traverse(source, pos, positions, best, depth + 1);
        }
        Node::PeekBind { names, remainder, source, .. } => {
            for name in names {
                traverse(name, pos, positions, best, depth + 1);
            }
            if let Some(rem) = remainder {
                traverse(rem, pos, positions, best, depth + 1);
            }
            traverse(source, pos, positions, best, depth + 1);
        }
        Node::ReceiveSendSource { name, .. } => traverse(name, pos, positions, best, depth + 1),
        Node::SendReceiveSource { name, inputs, .. } => {
            traverse(name, pos, positions, best, depth + 1);
            for input in inputs {
                traverse(input, pos, positions, best, depth + 1);
            }
        }
        Node::Error { children, .. } => {
            for child in children {
                traverse(child, pos, positions, best, depth + 1);
            }
        }
        Node::Disjunction { left, right, .. } => {
            traverse(left, pos, positions, best, depth + 1);
            traverse(right, pos, positions, best, depth + 1);
        }
        Node::Conjunction { left, right, .. } => {
            traverse(left, pos, positions, best, depth + 1);
            traverse(right, pos, positions, best, depth + 1);
        }
        Node::Negation { operand, .. } => traverse(operand, pos, positions, best, depth + 1),
        Node::Unit { .. } => {},
        _ => {},
    }
}

pub fn find_node_at_position(
    root: &Arc<Node>,
    positions: &HashMap<usize, (Position, Position)>,
    position: Position,
) -> Option<Arc<Node>> {
    let mut best: Option<(Arc<Node>, Position, usize)> = None;
    traverse(root, position, positions, &mut best, 0);
    if let Some(node) = best.map(|(node, _, _) | node) {
        debug!("Found best match");
        Some(node)
    } else {
        debug!("No node found at position {:?}", position);
        None
    }
}

impl Node {
    /// Returns the starting line number of the node within the source code.
    ///
    /// # Arguments
    /// * root - The root node of the IR tree, used for position computation.
    pub fn start_line(&self, root: &Arc<Node>) -> usize {
        let positions = compute_absolute_positions(root);
        let key = self as *const Node as usize;
        positions.get(&key).expect("Node not found").0.row
    }

    /// Returns the starting column number of the node within the source code.
    ///
    /// # Arguments
    /// * root - The root node of the IR tree, used for position computation.
    pub fn start_column(&self, root: &Arc<Node>) -> usize {
        let positions = compute_absolute_positions(root);
        let key = self as *const Node as usize;
        positions.get(&key).expect("Node not found").0.column
    }

    /// Returns the ending line number of the node within the source code.
    ///
    /// # Arguments
    /// * root - The root node of the IR tree, used for position computation.
    pub fn end_line(&self, root: &Arc<Node>) -> usize {
        let positions = compute_absolute_positions(root);
        let key = self as *const Node as usize;
        positions.get(&key).expect("Node not found").1.row
    }

    /// Returns the ending column number of the node within the source code.
    ///
    /// # Arguments
    /// * root - The root node of the IR tree, used for position computation.
    pub fn end_column(&self, root: &Arc<Node>) -> usize {
        let positions = compute_absolute_positions(root);
        let key = self as *const Node as usize;
        positions.get(&key).expect("Node not found").1.column
    }

    /// Returns the byte offset of the node’s start position in the source code.
    ///
    /// # Arguments
    /// * root - The root node of the IR tree, used for position computation.
    pub fn position(&self, root: &Arc<Node>) -> usize {
        let positions = compute_absolute_positions(root);
        let key = self as *const Node as usize;
        positions.get(&key).expect("Node not found").0.byte
    }

    /// Returns the length of the node’s text in bytes.
    pub fn length(&self) -> usize {
        self.base().length
    }

    /// Returns the absolute start position of the node in the source code.
    ///
    /// # Arguments
    /// * root - The root node of the IR tree, used for position computation.
    pub fn absolute_start(&self, root: &Arc<Node>) -> Position {
        let positions = compute_absolute_positions(root);
        let key = self as *const Node as usize;
        positions.get(&key).expect("Node not found").0
    }

    /// Returns the absolute end position of the node in the source code.
    ///
    /// # Arguments
    /// * root - The root node of the IR tree, used for position computation.
    pub fn absolute_end(&self, root: &Arc<Node>) -> Position {
        let positions = compute_absolute_positions(root);
        let key = self as *const Node as usize;
        positions.get(&key).expect("Node not found").1
    }

    /// Creates a new node with the same fields but a different NodeBase.
    ///
    /// # Arguments
    /// * new_base - The new NodeBase to apply to the node.
    ///
    /// # Returns
    /// A new Arc<Node> with the updated base.
    pub fn with_base(&self, new_base: NodeBase) -> Arc<Node> {
        match self {
            Node::Par {
                metadata,
                left,
                right,
                ..
            } => Arc::new(Node::Par {
                base: new_base,
                left: left.clone(),
                right: right.clone(),
                metadata: metadata.clone(),
            }),
            Node::SendSync {
                metadata,
                channel,
                inputs,
                cont,
                ..
            } => Arc::new(Node::SendSync {
                base: new_base,
                channel: channel.clone(),
                inputs: inputs.clone(),
                cont: cont.clone(),
                metadata: metadata.clone(),
            }),
            Node::Send {
                metadata,
                channel,
                send_type,
                send_type_delta,
                inputs,
                ..
            } => Arc::new(Node::Send {
                base: new_base,
                channel: channel.clone(),
                send_type: send_type.clone(),
                send_type_delta: *send_type_delta,
                inputs: inputs.clone(),
                metadata: metadata.clone(),
            }),
            Node::New { decls, proc, metadata, .. } => Arc::new(Node::New {
                base: new_base,
                decls: decls.clone(),
                proc: proc.clone(),
                metadata: metadata.clone(),
            }),
            Node::IfElse {
                condition,
                consequence,
                alternative,
                metadata,
                ..
            } => Arc::new(Node::IfElse {
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
            Node::Bundle {
                bundle_type,
                proc,
                metadata,
                ..
            } => Arc::new(Node::Bundle {
                base: new_base,
                bundle_type: bundle_type.clone(),
                proc: proc.clone(),
                metadata: metadata.clone(),
            }),
            Node::Match {
                expression,
                cases,
                metadata,
                ..
            } => Arc::new(Node::Match {
                base: new_base,
                expression: expression.clone(),
                cases: cases.clone(),
                metadata: metadata.clone(),
            }),
            Node::Choice {
                branches, metadata, ..
            } => Arc::new(Node::Choice {
                base: new_base,
                branches: branches.clone(),
                metadata: metadata.clone(),
            }),
            Node::Contract {
                name,
                formals,
                formals_remainder,
                proc,
                metadata,
                ..
            } => Arc::new(Node::Contract {
                base: new_base,
                name: name.clone(),
                formals: formals.clone(),
                formals_remainder: formals_remainder.clone(),
                proc: proc.clone(),
                metadata: metadata.clone(),
            }),
            Node::Input {
                receipts, proc, metadata, ..
            } => Arc::new(Node::Input {
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
            Node::BinOp {
                op,
                left,
                right,
                metadata,
                ..
            } => Arc::new(Node::BinOp {
                base: new_base,
                op: op.clone(),
                left: left.clone(),
                right: right.clone(),
                metadata: metadata.clone(),
            }),
            Node::UnaryOp {
                op, operand, metadata, ..
            } => Arc::new(Node::UnaryOp {
                base: new_base,
                op: op.clone(),
                operand: operand.clone(),
                metadata: metadata.clone(),
            }),
            Node::Method {
                receiver,
                name,
                args,
                metadata,
                ..
            } => Arc::new(Node::Method {
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
            Node::Quote {
                quotable, metadata, ..
            } => Arc::new(Node::Quote {
                base: new_base,
                quotable: quotable.clone(),
                metadata: metadata.clone(),
            }),
            Node::VarRef {
                kind, var, metadata, ..
            } => Arc::new(Node::VarRef {
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
            Node::List {
                elements,
                remainder,
                metadata,
                ..
            } => Arc::new(Node::List {
                base: new_base,
                elements: elements.clone(),
                remainder: remainder.clone(),
                metadata: metadata.clone(),
            }),
            Node::Set {
                elements,
                remainder,
                metadata,
                ..
            } => Arc::new(Node::Set {
                base: new_base,
                elements: elements.clone(),
                remainder: remainder.clone(),
                metadata: metadata.clone(),
            }),
            Node::Map {
                pairs,
                remainder,
                metadata,
                ..
            } => Arc::new(Node::Map {
                base: new_base,
                pairs: pairs.clone(),
                remainder: remainder.clone(),
                metadata: metadata.clone(),
            }),
            Node::Tuple {
                elements, metadata, ..
            } => Arc::new(Node::Tuple {
                base: new_base,
                elements: elements.clone(),
                metadata: metadata.clone(),
            }),
            Node::Var { name, metadata, .. } => Arc::new(Node::Var {
                base: new_base,
                name: name.clone(),
                metadata: metadata.clone(),
            }),
            Node::NameDecl {
                var, uri, metadata, ..
            } => Arc::new(Node::NameDecl {
                base: new_base,
                var: var.clone(),
                uri: uri.clone(),
                metadata: metadata.clone(),
            }),
            Node::Decl {
                names,
                names_remainder,
                procs,
                metadata,
                ..
            } => Arc::new(Node::Decl {
                base: new_base,
                names: names.clone(),
                names_remainder: names_remainder.clone(),
                procs: procs.clone(),
                metadata: metadata.clone(),
            }),
            Node::LinearBind {
                names,
                remainder,
                source,
                metadata,
                ..
            } => Arc::new(Node::LinearBind {
                base: new_base,
                names: names.clone(),
                remainder: remainder.clone(),
                source: source.clone(),
                metadata: metadata.clone(),
            }),
            Node::RepeatedBind {
                names,
                remainder,
                source,
                metadata,
                ..
            } => Arc::new(Node::RepeatedBind {
                base: new_base,
                names: names.clone(),
                remainder: remainder.clone(),
                source: source.clone(),
                metadata: metadata.clone(),
            }),
            Node::PeekBind {
                names,
                remainder,
                source,
                metadata,
                ..
            } => Arc::new(Node::PeekBind {
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
            Node::SendReceiveSource {
                name,
                inputs,
                metadata,
                ..
            } => Arc::new(Node::SendReceiveSource {
                base: new_base,
                name: name.clone(),
                inputs: inputs.clone(),
                metadata: metadata.clone(),
            }),
            Node::Error {
                children, metadata, ..
            } => Arc::new(Node::Error {
                base: new_base,
                children: children.clone(),
                metadata: metadata.clone(),
            }),
            Node::Disjunction {
                left, right, metadata, ..
            } => Arc::new(Node::Disjunction {
                base: new_base,
                left: left.clone(),
                right: right.clone(),
                metadata: metadata.clone(),
            }),
            Node::Conjunction {
                left, right, metadata, ..
            } => Arc::new(Node::Conjunction {
                base: new_base,
                left: left.clone(),
                right: right.clone(),
                metadata: metadata.clone(),
            }),
            Node::Negation {
                operand, metadata, ..
            } => Arc::new(Node::Negation {
                base: new_base,
                operand: operand.clone(),
                metadata: metadata.clone(),
            }),
            Node::Unit { metadata, .. } => Arc::new(Node::Unit {
                base: new_base,
                metadata: metadata.clone(),
            }),
        }
    }

    /// Validates the node by checking for reserved keyword usage in variable names.
    ///
    /// # Returns
    /// * Ok(()) if validation passes.
    /// * Err(String) with an error message if a reserved keyword is misused.
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
                        return Err(format!("Channel name '{name}' is a reserved keyword"));
                    }
                }
            }
            Node::Par { left, right, .. } => {
                left.validate()?;
                right.validate()?;
            }
            Node::New { decls, proc, .. } => {
                for decl in decls {
                    decl.validate()?;
                }
                proc.validate()?;
            }
            Node::IfElse {
                condition,
                consequence,
                alternative,
                ..
            } => {
                condition.validate()?;
                consequence.validate()?;
                if let Some(alt) = alternative {
                    alt.validate()?;
                }
            }
            Node::Let { decls, proc, .. } => {
                for decl in decls {
                    decl.validate()?;
                }
                proc.validate()?;
            }
            Node::Bundle { proc, .. } => proc.validate()?,
            Node::Match {
                expression, cases, ..
            } => {
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
                        if let Node::LinearBind {
                            names, remainder, ..
                        } = &**input
                        {
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
                        }
                        input.validate()?;
                    }
                    proc.validate()?;
                }
            }
            Node::Contract {
                name,
                formals,
                formals_remainder,
                proc,
                ..
            } => {
                name.validate()?;
                for formal in formals {
                    formal.validate()?;
                }
                if let Some(rem) = formals_remainder {
                    rem.validate()?;
                }
                proc.validate()?;
            }
            Node::Input { receipts, proc, .. } => {
                for receipt in receipts {
                    for bind in receipt {
                        bind.validate()?;
                    }
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
                for arg in args {
                    arg.validate()?;
                }
            }
            Node::Eval { name, .. } => name.validate()?,
            Node::Quote { quotable, .. } => quotable.validate()?,
            Node::VarRef { var, .. } => var.validate()?,
            Node::List {
                elements,
                remainder,
                ..
            } => {
                for elem in elements {
                    elem.validate()?;
                }
                if let Some(rem) = remainder {
                    rem.validate()?;
                }
            }
            Node::Set {
                elements,
                remainder,
                ..
            } => {
                for elem in elements {
                    elem.validate()?;
                }
                if let Some(rem) = remainder {
                    rem.validate()?;
                }
            }
            Node::Map { pairs, remainder, .. } => {
                for (key, value) in pairs {
                    key.validate()?;
                    value.validate()?;
                }
                if let Some(rem) = remainder {
                    rem.validate()?;
                }
            }
            Node::Tuple { elements, .. } => {
                for elem in elements {
                    elem.validate()?;
                }
            }
            Node::NameDecl { var, uri, .. } => {
                var.validate()?;
                if let Some(u) = uri {
                    u.validate()?;
                }
            }
            Node::Decl {
                names,
                names_remainder,
                procs,
                ..
            } => {
                for name in names {
                    name.validate()?;
                }
                if let Some(rem) = names_remainder {
                    rem.validate()?;
                }
                for proc in procs {
                    proc.validate()?;
                }
            }
            Node::LinearBind {
                names,
                remainder,
                source,
                ..
            } => {
                for name in names {
                    name.validate()?;
                }
                if let Some(rem) = remainder {
                    rem.validate()?;
                }
                source.validate()?;
            }
            Node::RepeatedBind {
                names,
                remainder,
                source,
                ..
            } => {
                for name in names {
                    name.validate()?;
                }
                if let Some(rem) = remainder {
                    rem.validate()?;
                }
                source.validate()?;
            }
            Node::PeekBind {
                names,
                remainder,
                source,
                ..
            } => {
                for name in names {
                    name.validate()?;
                }
                if let Some(rem) = remainder {
                    rem.validate()?;
                }
                source.validate()?;
            }
            Node::ReceiveSendSource { name, .. } => name.validate()?,
            Node::SendReceiveSource { name, inputs, .. } => {
                name.validate()?;
                for input in inputs {
                    input.validate()?;
                }
            }
            Node::Error { children, .. } => {
                for child in children {
                    child.validate()?;
                }
            }
            Node::Disjunction { left, right, .. } => {
                left.validate()?;
                right.validate()?;
            }
            Node::Conjunction { left, right, .. } => {
                left.validate()?;
                right.validate()?;
            }
            Node::Negation { operand, .. } => operand.validate()?,
            Node::Unit { .. } => {},
            _ => {},
        }
        Ok(())
    }

    /// Updates the node's metadata with a new value.
    ///
    /// # Arguments
    /// * new_metadata - The new metadata to apply to the node.
    ///
    /// # Returns
    /// A new Arc<Node> with the updated metadata.
    pub fn with_metadata(&self, new_metadata: Option<Arc<Metadata>>) -> Arc<Node> {
        match self {
            Node::Par { base, left, right, .. } => Arc::new(Node::Par {
                base: base.clone(),
                left: left.clone(),
                right: right.clone(),
                metadata: new_metadata,
            }),
            Node::SendSync {
                base,
                channel,
                inputs,
                cont,
                ..
            } => Arc::new(Node::SendSync {
                base: base.clone(),
                channel: channel.clone(),
                inputs: inputs.clone(),
                cont: cont.clone(),
                metadata: new_metadata,
            }),
            Node::Send {
                base,
                channel,
                send_type,
                send_type_delta,
                inputs,
                ..
            } => Arc::new(Node::Send {
                base: base.clone(),
                channel: channel.clone(),
                send_type: send_type.clone(),
                send_type_delta: *send_type_delta,
                inputs: inputs.clone(),
                metadata: new_metadata,
            }),
            Node::New { base, decls, proc, .. } => Arc::new(Node::New {
                base: base.clone(),
                decls: decls.clone(),
                proc: proc.clone(),
                metadata: new_metadata,
            }),
            Node::IfElse {
                base,
                condition,
                consequence,
                alternative,
                ..
            } => Arc::new(Node::IfElse {
                base: base.clone(),
                condition: condition.clone(),
                consequence: consequence.clone(),
                alternative: alternative.clone(),
                metadata: new_metadata,
            }),
            Node::Let { base, decls, proc, .. } => Arc::new(Node::Let {
                base: base.clone(),
                decls: decls.clone(),
                proc: proc.clone(),
                metadata: new_metadata,
            }),
            Node::Bundle {
                base, bundle_type, proc, ..
            } => Arc::new(Node::Bundle {
                base: base.clone(),
                bundle_type: bundle_type.clone(),
                proc: proc.clone(),
                metadata: new_metadata,
            }),
            Node::Match {
                base, expression, cases, ..
            } => Arc::new(Node::Match {
                base: base.clone(),
                expression: expression.clone(),
                cases: cases.clone(),
                metadata: new_metadata,
            }),
            Node::Choice { base, branches, .. } => Arc::new(Node::Choice {
                base: base.clone(),
                branches: branches.clone(),
                metadata: new_metadata,
            }),
            Node::Contract {
                base,
                name,
                formals,
                formals_remainder,
                proc,
                ..
            } => Arc::new(Node::Contract {
                base: base.clone(),
                name: name.clone(),
                formals: formals.clone(),
                formals_remainder: formals_remainder.clone(),
                proc: proc.clone(),
                metadata: new_metadata,
            }),
            Node::Input {
                base, receipts, proc, ..
            } => Arc::new(Node::Input {
                base: base.clone(),
                receipts: receipts.clone(),
                proc: proc.clone(),
                metadata: new_metadata,
            }),
            Node::Block { base, proc, .. } => Arc::new(Node::Block {
                base: base.clone(),
                proc: proc.clone(),
                metadata: new_metadata,
            }),
            Node::Parenthesized { base, expr, .. } => Arc::new(Node::Parenthesized {
                base: base.clone(),
                expr: expr.clone(),
                metadata: new_metadata,
            }),
            Node::BinOp {
                base,
                op,
                left,
                right,
                ..
            } => Arc::new(Node::BinOp {
                base: base.clone(),
                op: op.clone(),
                left: left.clone(),
                right: right.clone(),
                metadata: new_metadata,
            }),
            Node::UnaryOp {
                base, op, operand, ..
            } => Arc::new(Node::UnaryOp {
                base: base.clone(),
                op: op.clone(),
                operand: operand.clone(),
                metadata: new_metadata,
            }),
            Node::Method {
                base,
                receiver,
                name,
                args,
                ..
            } => Arc::new(Node::Method {
                base: base.clone(),
                receiver: receiver.clone(),
                name: name.clone(),
                args: args.clone(),
                metadata: new_metadata,
            }),
            Node::Eval { base, name, .. } => Arc::new(Node::Eval {
                base: base.clone(),
                name: name.clone(),
                metadata: new_metadata,
            }),
            Node::Quote { base, quotable, .. } => Arc::new(Node::Quote {
                base: base.clone(),
                quotable: quotable.clone(),
                metadata: new_metadata,
            }),
            Node::VarRef {
                base, kind, var, ..
            } => Arc::new(Node::VarRef {
                base: base.clone(),
                kind: kind.clone(),
                var: var.clone(),
                metadata: new_metadata,
            }),
            Node::BoolLiteral { base, value, .. } => Arc::new(Node::BoolLiteral {
                base: base.clone(),
                value: *value,
                metadata: new_metadata,
            }),
            Node::LongLiteral { base, value, .. } => Arc::new(Node::LongLiteral {
                base: base.clone(),
                value: *value,
                metadata: new_metadata,
            }),
            Node::StringLiteral { base, value, .. } => Arc::new(Node::StringLiteral {
                base: base.clone(),
                value: value.clone(),
                metadata: new_metadata,
            }),
            Node::UriLiteral { base, value, .. } => Arc::new(Node::UriLiteral {
                base: base.clone(),
                value: value.clone(),
                metadata: new_metadata,
            }),
            Node::Nil { base, .. } => Arc::new(Node::Nil {
                base: base.clone(),
                metadata: new_metadata,
            }),
            Node::List {
                base,
                elements,
                remainder,
                ..
            } => Arc::new(Node::List {
                base: base.clone(),
                elements: elements.clone(),
                remainder: remainder.clone(),
                metadata: new_metadata,
            }),
            Node::Set {
                base,
                elements,
                remainder,
                ..
            } => Arc::new(Node::Set {
                base: base.clone(),
                elements: elements.clone(),
                remainder: remainder.clone(),
                metadata: new_metadata,
            }),
            Node::Map {
                base,
                pairs,
                remainder,
                ..
            } => Arc::new(Node::Map {
                base: base.clone(),
                pairs: pairs.clone(),
                remainder: remainder.clone(),
                metadata: new_metadata,
            }),
            Node::Tuple { base, elements, .. } => Arc::new(Node::Tuple {
                base: base.clone(),
                elements: elements.clone(),
                metadata: new_metadata,
            }),
            Node::Var { base, name, .. } => Arc::new(Node::Var {
                base: base.clone(),
                name: name.clone(),
                metadata: new_metadata,
            }),
            Node::NameDecl {
                base, var, uri, ..
            } => Arc::new(Node::NameDecl {
                base: base.clone(),
                var: var.clone(),
                uri: uri.clone(),
                metadata: new_metadata,
            }),
            Node::Decl {
                base,
                names,
                names_remainder,
                procs,
                ..
            } => Arc::new(Node::Decl {
                base: base.clone(),
                names: names.clone(),
                names_remainder: names_remainder.clone(),
                procs: procs.clone(),
                metadata: new_metadata,
            }),
            Node::LinearBind {
                base,
                names,
                remainder,
                source,
                ..
            } => Arc::new(Node::LinearBind {
                base: base.clone(),
                names: names.clone(),
                remainder: remainder.clone(),
                source: source.clone(),
                metadata: new_metadata,
            }),
            Node::RepeatedBind {
                base,
                names,
                remainder,
                source,
                ..
            } => Arc::new(Node::RepeatedBind {
                base: base.clone(),
                names: names.clone(),
                remainder: remainder.clone(),
                source: source.clone(),
                metadata: new_metadata,
            }),
            Node::PeekBind {
                base,
                names,
                remainder,
                source,
                ..
            } => Arc::new(Node::PeekBind {
                base: base.clone(),
                names: names.clone(),
                remainder: remainder.clone(),
                source: source.clone(),
                metadata: new_metadata,
            }),
            Node::Comment { base, kind, .. } => Arc::new(Node::Comment {
                base: base.clone(),
                kind: kind.clone(),
                metadata: new_metadata,
            }),
            Node::Wildcard { base, .. } => Arc::new(Node::Wildcard {
                base: base.clone(),
                metadata: new_metadata,
            }),
            Node::SimpleType { base, value, .. } => Arc::new(Node::SimpleType {
                base: base.clone(),
                value: value.clone(),
                metadata: new_metadata,
            }),
            Node::ReceiveSendSource { base, name, .. } => Arc::new(Node::ReceiveSendSource {
                base: base.clone(),
                name: name.clone(),
                metadata: new_metadata,
            }),
            Node::SendReceiveSource {
                base, name, inputs, ..
            } => Arc::new(Node::SendReceiveSource {
                base: base.clone(),
                name: name.clone(),
                inputs: inputs.clone(),
                metadata: new_metadata,
            }),
            Node::Error {
                base, children, ..
            } => Arc::new(Node::Error {
                base: base.clone(),
                children: children.clone(),
                metadata: new_metadata,
            }),
            Node::Disjunction {
                base, left, right, ..
            } => Arc::new(Node::Disjunction {
                base: base.clone(),
                left: left.clone(),
                right: right.clone(),
                metadata: new_metadata,
            }),
            Node::Conjunction {
                base, left, right, ..
            } => Arc::new(Node::Conjunction {
                base: base.clone(),
                left: left.clone(),
                right: right.clone(),
                metadata: new_metadata,
            }),
            Node::Negation { base, operand, .. } => Arc::new(Node::Negation {
                base: base.clone(),
                operand: operand.clone(),
                metadata: new_metadata,
            }),
            Node::Unit { base, .. } => Arc::new(Node::Unit {
                base: base.clone(),
                metadata: new_metadata,
            }),
        }
    }

    /// Returns the textual representation of the node by slicing the Rope.
    /// The slice is based on the node's absolute start and end byte offsets in the source.
    pub fn text<'a>(&self, rope: &'a Rope, root: &Arc<Node>) -> RopeSlice<'a> {
        let start = self.absolute_start(root).byte;
        let end = self.absolute_end(root).byte;

        // Comprehensive bounds check to prevent panic
        let rope_len = rope.len_bytes();

        // Check basic invariants
        if start > end {
            warn!("Invalid text slice: start {} > end {} (rope len={}). Returning empty slice.", start, end, rope_len);
            return rope.slice(0..0);
        }

        // Check bounds
        if start > rope_len || end > rope_len {
            warn!("Text slice out of bounds: {}-{} (rope len={}). Clamping to rope length.", start, end, rope_len);
            let safe_start = start.min(rope_len);
            let safe_end = end.min(rope_len);
            if safe_start == safe_end {
                return rope.slice(0..0);
            }
            return rope.slice(safe_start..safe_end);
        }

        // Ropey requires char boundary alignment. Use char-based slicing to be safe.
        // Catch any potential panics from byte_to_char (e.g., invalid UTF-8 boundaries)
        let start_char = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            rope.byte_to_char(start)
        })).unwrap_or_else(|_| {
            warn!("byte_to_char panicked for start={}, using 0", start);
            0
        });

        let end_char = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            rope.byte_to_char(end)
        })).unwrap_or_else(|_| {
            warn!("byte_to_char panicked for end={}, using len_chars", end);
            rope.len_chars()
        });

        if start_char >= end_char {
            debug!("Empty slice after char conversion: start_char={}, end_char={}", start_char, end_char);
            return rope.slice(0..0);
        }

        // Use char-based slicing which is always safe
        let text = rope.slice(start_char..end_char);
        debug!(r#"rope.slice(char {}..{}) = "{text}""#, start_char, end_char);
        text
    }

    /// Returns a reference to the node’s NodeBase.
    pub fn base(&self) -> &NodeBase {
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
            Node::Disjunction { base, .. } => base,
            Node::Conjunction { base, .. } => base,
            Node::Negation { base, .. } => base,
            Node::Unit { base, .. } => base,
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
            Node::Disjunction { metadata, .. } => metadata.as_ref(),
            Node::Conjunction { metadata, .. } => metadata.as_ref(),
            Node::Negation { metadata, .. } => metadata.as_ref(),
            Node::Unit { metadata, .. } => metadata.as_ref(),
        }
    }

    pub fn node_cmp(a: &Node, b: &Node) -> Ordering {
        let tag_a = a.tag();
        let tag_b = b.tag();
        if tag_a != tag_b {
            return tag_a.cmp(&tag_b);
        }
        match (a, b) {
            (Node::Var { name: na, .. }, Node::Var { name: nb, .. }) => na.cmp(nb),
            (Node::BoolLiteral { value: va, .. }, Node::BoolLiteral { value: vb, .. }) => va.cmp(vb),
            (Node::LongLiteral { value: va, .. }, Node::LongLiteral { value: vb, .. }) => va.cmp(vb),
            (Node::StringLiteral { value: va, .. }, Node::StringLiteral { value: vb, .. }) => va.cmp(vb),
            (Node::UriLiteral { value: va, .. }, Node::UriLiteral { value: vb, .. }) => va.cmp(vb),
            (Node::SimpleType { value: va, .. }, Node::SimpleType { value: vb, .. }) => va.cmp(vb),
            (Node::Nil { .. }, Node::Nil { .. }) => Ordering::Equal,
            (Node::Unit { .. }, Node::Unit { .. }) => Ordering::Equal,
            (Node::Quote { quotable: qa, .. }, Node::Quote { quotable: qb, .. }) => {
                Node::node_cmp(&*qa, &*qb)
            }
            (Node::Eval { name: na, .. }, Node::Eval { name: nb, .. }) => Node::node_cmp(&*na, &*nb),
            (
                Node::VarRef {
                    kind: ka,
                    var: va,
                    ..
                },
                Node::VarRef {
                    kind: kb,
                    var: vb,
                    ..
                },
            ) => ka.cmp(kb).then_with(|| Node::node_cmp(&*va, &*vb)),
            (Node::Disjunction { left: p_l, right: p_r, .. }, Node::Disjunction { left: c_l, right: c_r, .. }) => {
                Node::node_cmp(p_l, c_l).then_with(|| Node::node_cmp(p_r, c_r))
            }
            (Node::Conjunction { left: p_l, right: p_r, .. }, Node::Conjunction { left: c_l, right: c_r, .. }) => {
                Node::node_cmp(p_l, c_l).then_with(|| Node::node_cmp(p_r, c_r))
            }
            (Node::Negation { operand: p_o, .. }, Node::Negation { operand: c_o, .. }) => {
                Node::node_cmp(p_o, c_o)
            }
            (Node::Parenthesized { expr: p_e, .. }, Node::Parenthesized { expr: c_e, .. }) => {
                Node::node_cmp(p_e, c_e)
            }
            (
                Node::List {
                    elements: ea,
                    remainder: ra,
                    ..
                },
                Node::List {
                    elements: eb,
                    remainder: rb,
                    ..
                },
            ) => {
                let mut ea_sorted: Vec<&Arc<Node>> = ea.iter().collect();
                ea_sorted.sort_by(|a, b| Node::node_cmp(a, b));
                let mut eb_sorted: Vec<&Arc<Node>> = eb.iter().collect();
                eb_sorted.sort_by(|a, b| Node::node_cmp(a, b));
                ea_sorted.cmp(&eb_sorted).then_with(|| ra.cmp(rb))
            }
            (Node::Tuple { elements: ea, .. }, Node::Tuple { elements: eb, .. }) => ea.cmp(eb),
            (
                Node::Set {
                    elements: ea,
                    remainder: ra,
                    ..
                },
                Node::Set {
                    elements: eb,
                    remainder: rb,
                    ..
                },
            ) => {
                let mut ea_sorted: Vec<&Arc<Node>> = ea.iter().collect();
                ea_sorted.sort_by(|a, b| Node::node_cmp(a, b));
                let mut eb_sorted: Vec<&Arc<Node>> = eb.iter().collect();
                eb_sorted.sort_by(|a, b| Node::node_cmp(a, b));
                ea_sorted.cmp(&eb_sorted).then_with(|| ra.cmp(rb))
            }
            (
                Node::Map {
                    pairs: pa,
                    remainder: ra,
                    ..
                },
                Node::Map {
                    pairs: pb,
                    remainder: rb,
                    ..
                },
            ) => {
                let mut pa_sorted: Vec<(&Arc<Node>, &Arc<Node>)> =
                    pa.iter().map(|(k, v)| (k, v)).collect();
                pa_sorted.sort_by(|(ka, _), (kb, _)| Node::node_cmp(ka, kb));
                let mut pb_sorted: Vec<(&Arc<Node>, &Arc<Node>)> =
                    pb.iter().map(|(k, v)| (k, v)).collect();
                pb_sorted.sort_by(|(ka, _), (kb, _)| Node::node_cmp(ka, kb));
                pa_sorted.cmp(&pb_sorted).then_with(|| ra.cmp(rb))
            }
            _ => Ordering::Equal, // For unmatched or leaf variants without comparable fields
        }
    }

    pub fn tag(&self) -> u32 {
        match self {
            Node::Par { .. } => 0,
            Node::SendSync { .. } => 1,
            Node::Send { .. } => 2,
            Node::New { .. } => 3,
            Node::IfElse { .. } => 4,
            Node::Let { .. } => 5,
            Node::Bundle { .. } => 6,
            Node::Match { .. } => 7,
            Node::Choice { .. } => 8,
            Node::Contract { .. } => 9,
            Node::Input { .. } => 10,
            Node::Block { .. } => 11,
            Node::Parenthesized { .. } => 12,
            Node::BinOp { .. } => 13,
            Node::UnaryOp { .. } => 14,
            Node::Method { .. } => 15,
            Node::Eval { .. } => 16,
            Node::Quote { .. } => 17,
            Node::VarRef { .. } => 18,
            Node::BoolLiteral { .. } => 19,
            Node::LongLiteral { .. } => 20,
            Node::StringLiteral { .. } => 21,
            Node::UriLiteral { .. } => 22,
            Node::Nil { .. } => 23,
            Node::List { .. } => 24,
            Node::Set { .. } => 25,
            Node::Map { .. } => 26,
            Node::Tuple { .. } => 27,
            Node::Var { .. } => 28,
            Node::NameDecl { .. } => 29,
            Node::Decl { .. } => 30,
            Node::LinearBind { .. } => 31,
            Node::RepeatedBind { .. } => 32,
            Node::PeekBind { .. } => 33,
            Node::Comment { .. } => 34,
            Node::Wildcard { .. } => 35,
            Node::SimpleType { .. } => 36,
            Node::ReceiveSendSource { .. } => 37,
            Node::SendReceiveSource { .. } => 38,
            Node::Error { .. } => 39,
            Node::Disjunction { .. } => 40,
            Node::Conjunction { .. } => 41,
            Node::Negation { .. } => 42,
            Node::Unit { .. } => 43,
        }
    }

    /// Constructs a new Par node with the given attributes.
    pub fn new_par(
        left: Arc<Node>,
        right: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::Par {
            base,
            left,
            right,
            metadata,
        }
    }

    /// Constructs a new SendSync node with the given attributes.
    pub fn new_send_sync(
        channel: Arc<Node>,
        inputs: NodeVector,
        cont: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::SendSync {
            base,
            channel,
            inputs,
            cont,
            metadata,
        }
    }

    /// Constructs a new Send node with the given attributes.
    pub fn new_send(
        channel: Arc<Node>,
        send_type: SendType,
        send_type_delta: RelativePosition,
        inputs: NodeVector,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::Send {
            base,
            channel,
            send_type,
            send_type_delta,
            inputs,
            metadata,
        }
    }

    /// Constructs a new New node with the given attributes.
    pub fn new_new(
        decls: NodeVector,
        proc: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::New {
            base,
            decls,
            proc,
            metadata,
        }
    }

    /// Constructs a new IfElse node with the given attributes.
    pub fn new_if_else(
        condition: Arc<Node>,
        consequence: Arc<Node>,
        alternative: Option<Arc<Node>>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::IfElse {
            base,
            condition,
            consequence,
            alternative,
            metadata,
        }
    }

    /// Constructs a new Let node with the given attributes.
    pub fn new_let(
        decls: NodeVector,
        proc: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::Let {
            base,
            decls,
            proc,
            metadata,
        }
    }

    /// Constructs a new Bundle node with the given attributes.
    pub fn new_bundle(
        bundle_type: BundleType,
        proc: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::Bundle {
            base,
            bundle_type,
            proc,
            metadata,
        }
    }

    /// Constructs a new Match node with the given attributes.
    pub fn new_match(
        expression: Arc<Node>,
        cases: NodePairVector,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::Match {
            base,
            expression,
            cases,
            metadata,
        }
    }

    /// Constructs a new Choice node with the given attributes.
    pub fn new_choice(
        branches: BranchVector,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::Choice {
            base,
            branches,
            metadata,
        }
    }

    /// Constructs a new Contract node with the given attributes.
    pub fn new_contract(
        name: Arc<Node>,
        formals: NodeVector,
        formals_remainder: Option<Arc<Node>>,
        proc: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::Contract {
            base,
            name,
            formals,
            formals_remainder,
            proc,
            metadata,
        }
    }

    /// Constructs a new Input node with the given attributes.
    pub fn new_input(
        receipts: ReceiptVector,
        proc: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::Input {
            base,
            receipts,
            proc,
            metadata,
        }
    }

    /// Constructs a new Block node with the given attributes.
    pub fn new_block(
        proc: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::Block {
            base,
            proc,
            metadata,
        }
    }

    /// Constructs a new Parenthesized node with the given attributes.
    pub fn new_parenthesized(
        expr: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::Parenthesized {
            base,
            expr,
            metadata,
        }
    }

    /// Constructs a new BinOp node with the given attributes.
    pub fn new_bin_op(
        op: BinOperator,
        left: Arc<Node>,
        right: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::BinOp {
            base,
            op,
            left,
            right,
            metadata,
        }
    }

    /// Constructs a new UnaryOp node with the given attributes.
    pub fn new_unary_op(
        op: UnaryOperator,
        operand: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::UnaryOp {
            base,
            op,
            operand,
            metadata,
        }
    }

    /// Constructs a new Method node with the given attributes.
    pub fn new_method(
        receiver: Arc<Node>,
        name: String,
        args: NodeVector,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::Method {
            base,
            receiver,
            name,
            args,
            metadata,
        }
    }

    /// Constructs a new Eval node with the given attributes.
    pub fn new_eval(
        name: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::Eval {
            base,
            name,
            metadata,
        }
    }

    /// Constructs a new Quote node with the given attributes.
    pub fn new_quote(
        quotable: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::Quote {
            base,
            quotable,
            metadata,
        }
    }

    /// Constructs a new VarRef node with the given attributes.
    pub fn new_var_ref(
        kind: VarRefKind,
        var: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::VarRef {
            base,
            kind,
            var,
            metadata,
        }
    }

    /// Constructs a new BoolLiteral node with the given attributes.
    pub fn new_bool_literal(
        value: bool,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::BoolLiteral {
            base,
            value,
            metadata,
        }
    }

    /// Constructs a new LongLiteral node with the given attributes.
    pub fn new_long_literal(
        value: i64,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::LongLiteral {
            base,
            value,
            metadata,
        }
    }

    /// Constructs a new StringLiteral node with the given attributes.
    pub fn new_string_literal(
        value: String,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::StringLiteral {
            base,
            value,
            metadata,
        }
    }

    /// Constructs a new UriLiteral node with the given attributes.
    pub fn new_uri_literal(
        value: String,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::UriLiteral {
            base,
            value,
            metadata,
        }
    }

    /// Constructs a new Nil node with the given attributes.
    pub fn new_nil(
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::Nil { base, metadata }
    }

    /// Constructs a new List node with the given attributes.
    pub fn new_list(
        elements: NodeVector,
        remainder: Option<Arc<Node>>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::List {
            base,
            elements,
            remainder,
            metadata,
        }
    }

    /// Constructs a new Set node with the given attributes.
    pub fn new_set(
        elements: NodeVector,
        remainder: Option<Arc<Node>>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::Set {
            base,
            elements,
            remainder,
            metadata,
        }
    }

    /// Constructs a new Map node with the given attributes.
    pub fn new_map(
        pairs: NodePairVector,
        remainder: Option<Arc<Node>>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::Map {
            base,
            pairs,
            remainder,
            metadata,
        }
    }

    /// Constructs a new Tuple node with the given attributes.
    pub fn new_tuple(
        elements: NodeVector,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::Tuple {
            base,
            elements,
            metadata,
        }
    }

    /// Constructs a new Var node with the given attributes.
    pub fn new_var(
        name: String,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::Var { base, name, metadata }
    }

    /// Constructs a new NameDecl node with the given attributes.
    pub fn new_name_decl(
        var: Arc<Node>,
        uri: Option<Arc<Node>>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::NameDecl {
            base,
            var,
            uri,
            metadata,
        }
    }

    /// Constructs a new Decl node with the given attributes.
    pub fn new_decl(
        names: NodeVector,
        names_remainder: Option<Arc<Node>>,
        procs: NodeVector,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::Decl {
            base,
            names,
            names_remainder,
            procs,
            metadata,
        }
    }

    /// Constructs a new LinearBind node with the given attributes.
    pub fn new_linear_bind(
        names: NodeVector,
        remainder: Option<Arc<Node>>,
        source: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::LinearBind {
            base,
            names,
            remainder,
            source,
            metadata,
        }
    }

    /// Constructs a new RepeatedBind node with the given attributes.
    pub fn new_repeated_bind(
        names: NodeVector,
        remainder: Option<Arc<Node>>,
        source: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::RepeatedBind {
            base,
            names,
            remainder,
            source,
            metadata,
        }
    }

    /// Constructs a new PeekBind node with the given attributes.
    pub fn new_peek_bind(
        names: NodeVector,
        remainder: Option<Arc<Node>>,
        source: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::PeekBind {
            base,
            names,
            remainder,
            source,
            metadata,
        }
    }

    /// Constructs a new Comment node with the given attributes.
    pub fn new_comment(
        kind: CommentKind,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::Comment {
            base,
            kind,
            metadata,
        }
    }

    /// Constructs a new Wildcard node with the given attributes.
    pub fn new_wildcard(
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::Wildcard { base, metadata }
    }

    /// Constructs a new SimpleType node with the given attributes.
    pub fn new_simple_type(
        value: String,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::SimpleType {
            base,
            value,
            metadata,
        }
    }

    /// Constructs a new ReceiveSendSource node with the given attributes.
    pub fn new_receive_send_source(
        name: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::ReceiveSendSource {
            base,
            name,
            metadata,
        }
    }

    /// Constructs a new SendReceiveSource node with the given attributes.
    pub fn new_send_receive_source(
        name: Arc<Node>,
        inputs: NodeVector,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::SendReceiveSource {
            base,
            name,
            inputs,
            metadata,
        }
    }

    /// Constructs a new Error node with the given attributes.
    pub fn new_error(
        children: NodeVector,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::Error {
            base,
            children,
            metadata,
        }
    }

    /// Constructs a new Disjunction node with the given attributes.
    pub fn new_disjunction(
        left: Arc<Node>,
        right: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::Disjunction {
            base,
            left,
            right,
            metadata,
        }
    }

    /// Constructs a new Conjunction node with the given attributes.
    pub fn new_conjunction(
        left: Arc<Node>,
        right: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::Conjunction {
            base,
            left,
            right,
            metadata,
        }
    }

    /// Constructs a new Negation node with the given attributes.
    pub fn new_negation(
        operand: Arc<Node>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::Negation {
            base,
            operand,
            metadata,
        }
    }

    /// Constructs a new Unit node with the given attributes.
    pub fn new_unit(
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        Node::Unit { base, metadata }
    }
}

impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        Node::node_cmp(self, other) == Ordering::Equal
    }
}

impl Eq for Node {}

impl PartialOrd for Node {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(Node::node_cmp(self, other))
    }
}

impl Ord for Node {
    fn cmp(&self, other: &Self) -> Ordering {
        Node::node_cmp(self, other)
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
        let _ = crate::logging::init_logger(false, Some("warn"));
        let code = "ch!(\"msg\")\nNil";
        let tree = parse_code(code);
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree, &rope);
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
        let _ = crate::logging::init_logger(false, Some("warn"));
        let code = r#"new x in { x!("msg") }"#;
        let tree = parse_code(code);
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree, &rope);
        let root = Arc::new(ir.clone());
        if let Node::New { decls, proc, .. } = &*ir {
            let decl_start = decls[0].absolute_start(&root);
            assert_eq!(decl_start.row, 0);
            assert_eq!(decl_start.column, 4);
            assert_eq!(decl_start.byte, 4);
            if let Node::Block { proc: inner, .. } = &**proc {
                if let Node::Send { channel, .. } = &**inner {
                    let chan_start = channel.absolute_start(&root);
                    assert_eq!(chan_start.row, 0);
                    assert_eq!(chan_start.column, 11);
                    assert_eq!(chan_start.byte, 11);
                } else {
                    panic!("Expected Send node");
                }
            } else {
                panic!("Expected Block node");
            }
        } else {
            panic!("Expected New node");
        }
    }

    #[test]
    fn test_prop_position_consistency() {
        fn prop(proc: RholangProc) -> TestResult {
            let code = proc.to_code();
            let tree = parse_code(&code);
            if tree.root_node().has_error() {
                return TestResult::discard();
            }
            let rope = Rope::from_str(&code);
            let ir = parse_to_ir(&tree, &rope);
            let root = Arc::new(ir.clone());
            let start = ir.absolute_start(&root);
            let end = ir.absolute_end(&root);
            assert!(start.byte <= end.byte);
            assert!(start.row <= end.row);
            if start.row == end.row {
                assert!(start.column <= end.column);
            }
            TestResult::passed()
        }
        QuickCheck::new().tests(100).max_tests(1000).quickcheck(prop as fn(RholangProc) -> TestResult);
    }

    #[test]
    fn test_multi_line_positions() {
        let _ = crate::logging::init_logger(false, Some("warn"));
        let code = "ch!(\n\"msg\"\n)";
        let tree = parse_code(code);
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree, &rope);
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
        let _ = crate::logging::init_logger(false, Some("warn"));
        let code = r#"match "target" { "pat" => Nil }"#;
        let tree = parse_code(code);
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree, &rope);
        let root = Arc::new(ir.clone());
        if let Node::Match { expression, cases, .. } = &*ir {
            let expr_start = expression.absolute_start(&root);
            assert_eq!(expr_start.row, 0);
            assert_eq!(expr_start.column, 6);
            assert_eq!(expr_start.byte, 6);
            let (pattern, proc) = &cases[0];
            let pat_start = pattern.absolute_start(&root);
            assert_eq!(pat_start.row, 0);
            assert_eq!(pat_start.column, 17);
            assert_eq!(pat_start.byte, 17);
            let proc_start = proc.absolute_start(&root);
            assert_eq!(proc_start.row, 0);
            assert_eq!(proc_start.column, 26);
            assert_eq!(proc_start.byte, 26);
        } else {
            panic!("Expected Match node");
        }
    }

    #[test]
    fn test_metadata_dynamic() {
        let mut data = HashMap::new();
        data.insert(
            "version".to_string(),
            Arc::new(1_usize) as Arc<dyn Any + Send + Sync>,
        );
        data.insert(
            "custom".to_string(),
            Arc::new("test".to_string()) as Arc<dyn Any + Send + Sync>,
        );
        let metadata = Arc::new(Metadata { data });
        let base = NodeBase::new(
            RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            0,
            0,
            0,
        );
        let node = Node::Nil {
            base,
            metadata: Some(metadata.clone()),
        };
        assert_eq!(
            node.metadata()
                .unwrap()
                .data
                .get("version")
                .unwrap()
                .downcast_ref::<usize>(),
            Some(&1)
        );
        assert_eq!(
            node.metadata()
                .unwrap()
                .data
                .get("custom")
                .unwrap()
                .downcast_ref::<String>(),
            Some(&"test".to_string())
        );
    }

    #[test]
    fn test_error_node_with_children() {
        let code = r#"new x { x!("") }"#;
        let tree = parse_code(code);
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree, &rope);
        if let Node::Par { left, .. } = &*ir {
            if let Node::Error { children, .. } = left.as_ref() {
                assert!(!children.is_empty(), "Error node should have children");
            }
        }
    }

    #[test]
    fn test_match_pat_simple() {
        let wild = Arc::new(Node::new_wildcard(
            None,
            RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            1,
            0,
            1,
        ));
        let var_pat = Arc::new(Node::new_var(
            "x".to_string(),
            None,
            RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            1,
            0,
            1,
        ));
        let var_con = Arc::new(Node::new_var(
            "y".to_string(),
            None,
            RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            1,
            0,
            1,
        ));
        let string_pat = Arc::new(Node::new_string_literal(
            "foo".to_string(),
            None,
            RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            5,
            0,
            5,
        ));
        let string_con = Arc::new(Node::new_string_literal(
            "foo".to_string(),
            None,
            RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            5,
            0,
            5,
        ));
        let string_con_diff = Arc::new(Node::new_string_literal(
            "bar".to_string(),
            None,
            RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            5,
            0,
            5,
        ));
        let mut subst = HashMap::new();
        assert!(match_pat(&wild, &var_con, &mut subst));
        assert!(match_pat(&var_pat, &var_con, &mut subst));
        assert_eq!(subst.get("x"), Some(&var_con));
        assert!(match_pat(&string_pat, &string_con, &mut subst));
        assert!(!match_pat(&string_pat, &string_con_diff, &mut subst));
    }

    #[test]
    fn test_match_pat_repeat() {
        let var_pat = Arc::new(Node::new_var(
            "x".to_string(),
            None,
            RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            1,
            0,
            1,
        ));
        let con1 = Arc::new(Node::new_long_literal(
            1,
            None,
            RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            1,
            0,
            1,
        ));
        let con2 = Arc::new(Node::new_long_literal(
            1,
            None,
            RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            1,
            0,
            1,
        ));
        let con_diff = Arc::new(Node::new_long_literal(
            2,
            None,
            RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            1,
            0,
            1,
        ));
        let mut subst = HashMap::new();
        assert!(match_pat(&var_pat, &con1, &mut subst));
        assert!(match_pat(&var_pat, &con2, &mut subst));
        assert!(!match_pat(&var_pat, &con_diff, &mut subst));
    }

    #[test]
    fn test_match_contract_basic() {
        let channel = Arc::new(Node::new_var(
            "foo".to_string(),
            None,
            RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            3,
            0,
            3,
        ));
        let inputs = Vector::new_with_ptr_kind().push_back(Arc::new(Node::new_long_literal(
            1,
            None,
            RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            1,
            0,
            1,
        )));
        let contract_name = Arc::new(Node::new_var(
            "foo".to_string(),
            None,
            RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            3,
            0,
            3,
        ));
        let contract_formals = Vector::new_with_ptr_kind().push_back(Arc::new(Node::new_var(
            "x".to_string(),
            None,
            RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            1,
            0,
            1,
        )));
        let contract = Arc::new(Node::new_contract(
            contract_name,
            contract_formals,
            None,
            Arc::new(Node::new_nil(
                None,
                RelativePosition {
                    delta_lines: 0,
                    delta_columns: 0,
                    delta_bytes: 0,
                },
                3,
                0,
                3,
            )),
            None,
            RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            0,
            0,
            0,
        ));
        assert!(match_contract(&channel, &inputs, &contract));
        let bad_inputs = Vector::new_with_ptr_kind();
        assert!(!match_contract(&channel, &bad_inputs, &contract));
    }

    #[test]
    fn test_match_pat_set() {
        let p_e = Vector::new_with_ptr_kind()
            .push_back(Arc::new(Node::LongLiteral {
                base: NodeBase::new(
                    RelativePosition {
                        delta_lines: 0,
                        delta_columns: 0,
                        delta_bytes: 0,
                    },
                    1,
                    0,
                    1,
                ),
                value: 1,
                metadata: None,
            }))
            .push_back(Arc::new(Node::LongLiteral {
                base: NodeBase::new(
                    RelativePosition {
                        delta_lines: 0,
                        delta_columns: 0,
                        delta_bytes: 0,
                    },
                    1,
                    0,
                    1,
                ),
                value: 2,
                metadata: None,
            }));
        let pat = Arc::new(Node::Set {
            base: NodeBase::new(
                RelativePosition {
                    delta_lines: 0,
                    delta_columns: 0,
                    delta_bytes: 0,
                },
                0,
                0,
                0,
            ),
            elements: p_e,
            remainder: None,
            metadata: None,
        });
        let c_e = Vector::new_with_ptr_kind()
            .push_back(Arc::new(Node::LongLiteral {
                base: NodeBase::new(
                    RelativePosition {
                        delta_lines: 0,
                        delta_columns: 0,
                        delta_bytes: 0,
                    },
                    1,
                    0,
                    1,
                ),
                value: 2,
                metadata: None,
            }))
            .push_back(Arc::new(Node::LongLiteral {
                base: NodeBase::new(
                    RelativePosition {
                        delta_lines: 0,
                        delta_columns: 0,
                        delta_bytes: 0,
                    },
                    1,
                    0,
                    1,
                ),
                value: 1,
                metadata: None,
            }));
        let concrete = Arc::new(Node::Set {
            base: NodeBase::new(
                RelativePosition {
                    delta_lines: 0,
                    delta_columns: 0,
                    delta_bytes: 0,
                },
                0,
                0,
                0,
            ),
            elements: c_e,
            remainder: None,
            metadata: None,
        });
        let mut subst = HashMap::new();
        assert!(crate::ir::node::match_pat(&pat, &concrete, &mut subst));
    }

    #[test]
    fn test_match_pat_map() {
        let p_pair1 = (
            Arc::new(Node::StringLiteral {
                base: NodeBase::new(
                    RelativePosition {
                        delta_lines: 0,
                        delta_columns: 0,
                        delta_bytes: 0,
                    },
                    3,
                    0,
                    3,
                ),
                value: "a".to_string(),
                metadata: None,
            }),
            Arc::new(Node::LongLiteral {
                base: NodeBase::new(
                    RelativePosition {
                        delta_lines: 0,
                        delta_columns: 0,
                        delta_bytes: 0,
                    },
                    1,
                    0,
                    1,
                ),
                value: 1,
                metadata: None,
            }),
        );
        let p_pair2 = (
            Arc::new(Node::StringLiteral {
                base: NodeBase::new(
                    RelativePosition {
                        delta_lines: 0,
                        delta_columns: 0,
                        delta_bytes: 0,
                    },
                    3,
                    0,
                    3,
                ),
                value: "b".to_string(),
                metadata: None,
            }),
            Arc::new(Node::LongLiteral {
                base: NodeBase::new(
                    RelativePosition {
                        delta_lines: 0,
                        delta_columns: 0,
                        delta_bytes: 0,
                    },
                    1,
                    0,
                    1,
                ),
                value: 2,
                metadata: None,
            }),
        );
        let p_pairs = Vector::new_with_ptr_kind().push_back(p_pair1).push_back(p_pair2);
        let pat = Arc::new(Node::Map {
            base: NodeBase::new(
                RelativePosition {
                    delta_lines: 0,
                    delta_columns: 0,
                    delta_bytes: 0,
                },
                0,
                0,
                0,
            ),
            pairs: p_pairs,
            remainder: None,
            metadata: None,
        });
        let c_pair1 = (
            Arc::new(Node::StringLiteral {
                base: NodeBase::new(
                    RelativePosition {
                        delta_lines: 0,
                        delta_columns: 0,
                        delta_bytes: 0,
                    },
                    3,
                    0,
                    3,
                ),
                value: "b".to_string(),
                metadata: None,
            }),
            Arc::new(Node::LongLiteral {
                base: NodeBase::new(
                    RelativePosition {
                        delta_lines: 0,
                        delta_columns: 0,
                        delta_bytes: 0,
                    },
                    1,
                    0,
                    1,
                ),
                value: 2,
                metadata: None,
            }),
        );
        let c_pair2 = (
            Arc::new(Node::StringLiteral {
                base: NodeBase::new(
                    RelativePosition {
                        delta_lines: 0,
                        delta_columns: 0,
                        delta_bytes: 0,
                    },
                    3,
                    0,
                    3,
                ),
                value: "a".to_string(),
                metadata: None,
            }),
            Arc::new(Node::LongLiteral {
                base: NodeBase::new(
                    RelativePosition {
                        delta_lines: 0,
                        delta_columns: 0,
                        delta_bytes: 0,
                    },
                    1,
                    0,
                    1,
                ),
                value: 1,
                metadata: None,
            }),
        );
        let c_pairs = Vector::new_with_ptr_kind().push_back(c_pair1).push_back(c_pair2);
        let concrete = Arc::new(Node::Map {
            base: NodeBase::new(
                RelativePosition {
                    delta_lines: 0,
                    delta_columns: 0,
                    delta_bytes: 0,
                },
                0,
                0,
                0,
            ),
            pairs: c_pairs,
            remainder: None,
            metadata: None,
        });
        let mut subst = HashMap::new();
        assert!(crate::ir::node::match_pat(&pat, &concrete, &mut subst));
    }

    #[test]
    fn test_match_pat_disjunction() {
        let p_left = Arc::new(Node::LongLiteral {
            base: NodeBase::new(
                RelativePosition {
                    delta_lines: 0,
                    delta_columns: 0,
                    delta_bytes: 0,
                },
                1,
                0,
                1,
            ),
            value: 1,
            metadata: None,
        });
        let p_right = Arc::new(Node::LongLiteral {
            base: NodeBase::new(
                RelativePosition {
                    delta_lines: 0,
                    delta_columns: 0,
                    delta_bytes: 0,
                },
                1,
                0,
                1,
            ),
            value: 2,
            metadata: None,
        });
        let pat = Arc::new(Node::Disjunction {
            base: NodeBase::new(
                RelativePosition {
                    delta_lines: 0,
                    delta_columns: 0,
                    delta_bytes: 0,
                },
                0,
                0,
                0,
            ),
            left: p_left,
            right: p_right,
            metadata: None,
        });
        let c_left = Arc::new(Node::LongLiteral {
            base: NodeBase::new(
                RelativePosition {
                    delta_lines: 0,
                    delta_columns: 0,
                    delta_bytes: 0,
                },
                1,
                0,
                1,
            ),
            value: 1,
            metadata: None,
        });
        let c_right = Arc::new(Node::LongLiteral {
            base: NodeBase::new(
                RelativePosition {
                    delta_lines: 0,
                    delta_columns: 0,
                    delta_bytes: 0,
                },
                1,
                0,
                1,
            ),
            value: 2,
            metadata: None,
        });
        let concrete = Arc::new(Node::Disjunction {
            base: NodeBase::new(
                RelativePosition {
                    delta_lines: 0,
                    delta_columns: 0,
                    delta_bytes: 0,
                },
                0,
                0,
                0,
            ),
            left: c_left,
            right: c_right,
            metadata: None,
        });
        let mut subst = HashMap::new();
        assert!(crate::ir::node::match_pat(&pat, &concrete, &mut subst));
    }

    #[test]
    fn test_prop_match_pat_no_crash() {
        fn prop(pat: RholangProc, concrete: RholangProc) -> TestResult {
            let pat_code = pat.to_code();
            let concrete_code = concrete.to_code();
            let pat_tree = parse_code(&pat_code);
            let concrete_tree = parse_code(&concrete_code);
            if pat_tree.root_node().has_error() || concrete_tree.root_node().has_error() {
                return TestResult::discard();
            }
            let pat_rope = Rope::from_str(&pat_code);
            let concrete_rope = Rope::from_str(&concrete_code);
            let pat_ir = parse_to_ir(&pat_tree, &pat_rope);
            let concrete_ir = parse_to_ir(&concrete_tree, &concrete_rope);
            let mut subst = HashMap::new();
            let _ = match_pat(&pat_ir, &concrete_ir, &mut subst);
            TestResult::passed()
        }
        QuickCheck::new().quickcheck(prop as fn(RholangProc, RholangProc) -> TestResult);
    }
}
