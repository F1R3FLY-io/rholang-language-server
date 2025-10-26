use std::any::Any;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;

use rpds::Vector;
use archery::ArcK;

use ropey::{Rope, RopeSlice};

use tracing::{debug, trace, warn};

pub use super::semantic_node::{Metadata, NodeBase, Position, RelativePosition};

pub type RholangNodeVector = Vector<Arc<RholangNode>, ArcK>;
pub type RholangNodePairVector = Vector<(Arc<RholangNode>, Arc<RholangNode>), ArcK>;
pub type RholangBranchVector = Vector<(RholangNodeVector, Arc<RholangNode>), ArcK>;
pub type RholangReceiptVector = Vector<RholangNodeVector, ArcK>;

/// Represents all possible constructs in the Rholang Intermediate Representation (IR).
/// Each variant corresponds to a syntactic element in Rholang, such as processes, expressions, or bindings.
///
/// # Examples
/// - Par: Parallel composition of two processes (e.g., P | Q).
/// - Send: Asynchronous message send (e.g., ch!("msg")).
/// - Var: Variable reference (e.g., x in x!()).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum RholangNode {
    /// Parallel composition of processes.
    /// Supports both binary (left/right) and n-ary (processes) forms for gradual migration.
    Par {
        base: NodeBase,
        // Legacy binary form (deprecated, will be removed after migration)
        left: Option<Arc<RholangNode>>,
        right: Option<Arc<RholangNode>>,
        // New n-ary form (preferred)
        processes: Option<RholangNodeVector>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Synchronous send with a continuation process.
    SendSync {
        base: NodeBase,
        channel: Arc<RholangNode>,
        inputs: RholangNodeVector,
        cont: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Asynchronous send operation on a channel.
    Send {
        base: NodeBase,
        channel: Arc<RholangNode>,
        send_type: RholangSendType,
        send_type_delta: RelativePosition,
        inputs: RholangNodeVector,
        metadata: Option<Arc<Metadata>>,
    },
    /// Declaration of new names with a scoped process
    New {
        base: NodeBase,
        decls: RholangNodeVector,
        proc: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Conditional branching with optional else clause.
    IfElse {
        base: NodeBase,
        condition: Arc<RholangNode>,
        consequence: Arc<RholangNode>,
        alternative: Option<Arc<RholangNode>>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Variable binding with a subsequent process.
    Let {
        base: NodeBase,
        decls: RholangNodeVector,
        proc: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Access-controlled process with a bundle type.
    Bundle {
        base: NodeBase,
        bundle_type: RholangBundleType,
        proc: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Pattern matching construct with cases.
    Match {
        base: NodeBase,
        expression: Arc<RholangNode>,
        cases: RholangNodePairVector,
        metadata: Option<Arc<Metadata>>,
    },
    /// Non-deterministic choice among branches.
    Choice {
        base: NodeBase,
        branches: RholangBranchVector,
        metadata: Option<Arc<Metadata>>,
    },
    /// Contract definition with name, parameters, and body.
    Contract {
        base: NodeBase,
        name: Arc<RholangNode>,
        formals: RholangNodeVector,
        formals_remainder: Option<Arc<RholangNode>>,
        proc: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Input binding from channels with a process.
    Input {
        base: NodeBase,
        receipts: RholangReceiptVector,
        proc: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Block of a single process (e.g., { P }).
    Block {
        base: NodeBase,
        proc: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Parenthesized expression (e.g., (P)).
    Parenthesized {
        base: NodeBase,
        expr: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Binary operation (e.g., P + Q).
    BinOp {
        base: NodeBase,
        op: BinOperator,
        left: Arc<RholangNode>,
        right: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Unary operation (e.g., -P or not P).
    UnaryOp {
        base: NodeBase,
        op: UnaryOperator,
        operand: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Method call on a receiver (e.g., obj.method(args)).
    Method {
        base: NodeBase,
        receiver: Arc<RholangNode>,
        name: String,
        args: RholangNodeVector,
        metadata: Option<Arc<Metadata>>,
    },
    /// Evaluation of a name (e.g., *name).
    Eval {
        base: NodeBase,
        name: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Quotation of a process (e.g., @P).
    Quote {
        base: NodeBase,
        quotable: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Variable reference with assignment kind.
    VarRef {
        base: NodeBase,
        kind: RholangVarRefKind,
        var: Arc<RholangNode>,
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
        elements: RholangNodeVector,
        remainder: Option<Arc<RholangNode>>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Set collection (e.g., Set(1, 2, 3)).
    Set {
        base: NodeBase,
        elements: RholangNodeVector,
        remainder: Option<Arc<RholangNode>>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Map collection (e.g., {k: v}).
    Map {
        base: NodeBase,
        pairs: RholangNodePairVector,
        remainder: Option<Arc<RholangNode>>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Tuple collection (e.g., (1, 2)).
    Tuple {
        base: NodeBase,
        elements: RholangNodeVector,
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
        var: Arc<RholangNode>,
        uri: Option<Arc<RholangNode>>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Declaration in a let statement (e.g., x = P).
    Decl {
        base: NodeBase,
        names: RholangNodeVector,
        names_remainder: Option<Arc<RholangNode>>,
        procs: RholangNodeVector,
        metadata: Option<Arc<Metadata>>,
    },
    /// Linear binding in a for (e.g., x <- ch).
    LinearBind {
        base: NodeBase,
        names: RholangNodeVector,
        remainder: Option<Arc<RholangNode>>,
        source: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Repeated binding in a for (e.g., x <= ch).
    RepeatedBind {
        base: NodeBase,
        names: RholangNodeVector,
        remainder: Option<Arc<RholangNode>>,
        source: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Peek binding in a for (e.g., x <<- ch).
    PeekBind {
        base: NodeBase,
        names: RholangNodeVector,
        remainder: Option<Arc<RholangNode>>,
        source: Arc<RholangNode>,
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
        name: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Send-receive source (e.g., ch!?(args)).
    SendReceiveSource {
        base: NodeBase,
        name: Arc<RholangNode>,
        inputs: RholangNodeVector,
        metadata: Option<Arc<Metadata>>,
    },
    /// Represents a syntax error in the source code with its erroneous subtree.
    Error {
        base: NodeBase,
        children: RholangNodeVector,
        metadata: Option<Arc<Metadata>>,
    },
    /// Pattern disjunction (e.g., P | Q in patterns).
    Disjunction {
        base: NodeBase,
        left: Arc<RholangNode>,
        right: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Pattern conjunction (e.g., P & Q in patterns).
    Conjunction {
        base: NodeBase,
        left: Arc<RholangNode>,
        right: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Pattern negation (e.g., ~P in patterns).
    Negation {
        base: NodeBase,
        operand: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
    },
    /// Unit value (e.g., ()).
    Unit {
        base: NodeBase,
        metadata: Option<Arc<Metadata>>,
    },
}

#[derive(Clone, PartialEq, Debug, Hash)]
pub enum RholangBundleType {
    Read,
    Write,
    Equiv,
    ReadWrite,
}

#[derive(Clone, PartialEq, Debug, Hash)]
pub enum RholangSendType {
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
pub enum RholangVarRefKind {
    Bind,
    Unforgeable,
}

#[derive(Clone, PartialEq, Debug, Hash)]
pub enum CommentKind {
    Line,
    Block,
}

/// Computes absolute positions for all nodes in the IR tree, storing them in a HashMap.
/// Positions are keyed by the raw pointer to the RholangNode cast to usize.
///
/// # Arguments
/// * root - The root node of the IR tree.
///
/// # Returns
/// A HashMap mapping node pointers (as usize) to tuples of (start, end) Positions.
pub fn compute_absolute_positions(root: &Arc<RholangNode>) -> HashMap<usize, (Position, Position)> {
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
    node: &Arc<RholangNode>,
    prev_end: Position,
    positions: &mut HashMap<usize, (Position, Position)>,
) -> Position {
    let base = node.base();
    let key = &**node as *const RholangNode as usize;
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

    // Hot path: Position computation runs during parsing for every node
    // Removed per-node debug logging to avoid excessive log volume
    // Use RUST_LOG=trace for detailed position tracking

    let mut current_prev = start;

    // Process children
    match &**node {
        RholangNode::Par { left: Some(left), right: Some(right), .. } => {
            current_prev = compute_positions_helper(left, current_prev, positions);
            current_prev = compute_positions_helper(right, current_prev, positions);
        }
        RholangNode::Par { processes: Some(procs), .. } => {
            for proc in procs.iter() {
                current_prev = compute_positions_helper(proc, current_prev, positions);
            }
        }
        RholangNode::SendSync {
            channel, inputs, cont, ..
        } => {
            current_prev = compute_positions_helper(channel, current_prev, positions);
            for input in inputs {
                current_prev = compute_positions_helper(input, current_prev, positions);
            }
            current_prev = compute_positions_helper(cont, current_prev, positions);
        }
        RholangNode::Send {
            channel,
            inputs,
            send_type_delta,
            ..
        } => {
            let channel_end = compute_positions_helper(channel, current_prev, positions);
            let send_type_end = Position {
                row: (channel_end.row as i32 + send_type_delta.delta_lines) as usize,
                column: if send_type_delta.delta_lines == 0 {
                    (channel_end.column as i32 + send_type_delta.delta_columns) as usize
                } else {
                    send_type_delta.delta_columns as usize
                },
                byte: channel_end.byte + send_type_delta.delta_bytes,
            };
            let mut temp_prev = send_type_end;
            for input in inputs.iter() {
                temp_prev = compute_positions_helper(input, temp_prev, positions);
            }
            current_prev = temp_prev;
        }
        RholangNode::New { decls, proc, .. } => {
            for decl in decls {
                current_prev = compute_positions_helper(decl, current_prev, positions);
            }
            current_prev = compute_positions_helper(proc, current_prev, positions);
        }
        RholangNode::IfElse {
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
        RholangNode::Let { decls, proc, .. } => {
            for decl in decls {
                current_prev = compute_positions_helper(decl, current_prev, positions);
            }
            current_prev = compute_positions_helper(proc, current_prev, positions);
        }
        RholangNode::Bundle { proc, .. } => {
            current_prev = compute_positions_helper(proc, current_prev, positions);
        }
        RholangNode::Match { expression, cases, .. } => {
            current_prev = compute_positions_helper(expression, current_prev, positions);
            for (pattern, proc) in cases {
                current_prev = compute_positions_helper(pattern, current_prev, positions);
                current_prev = compute_positions_helper(proc, current_prev, positions);
            }
        }
        RholangNode::Choice { branches, .. } => {
            for (inputs, proc) in branches {
                let mut temp_prev = current_prev;
                for input in inputs {
                    temp_prev = compute_positions_helper(input, temp_prev, positions);
                }
                current_prev = compute_positions_helper(proc, temp_prev, positions);
            }
        }
        RholangNode::Contract {
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
        RholangNode::Input { receipts, proc, .. } => {
            for receipt in receipts {
                for bind in receipt {
                    current_prev = compute_positions_helper(bind, current_prev, positions);
                }
            }
            current_prev = compute_positions_helper(proc, current_prev, positions);
        }
        RholangNode::Block { proc, .. } => {
            current_prev = compute_positions_helper(proc, current_prev, positions);
        }
        RholangNode::Parenthesized { expr, .. } => {
            current_prev = compute_positions_helper(expr, current_prev, positions);
        }
        RholangNode::BinOp { left, right, .. } => {
            current_prev = compute_positions_helper(left, current_prev, positions);
            current_prev = compute_positions_helper(right, current_prev, positions);
        }
        RholangNode::UnaryOp { operand, .. } => {
            current_prev = compute_positions_helper(operand, current_prev, positions);
        }
        RholangNode::Method { receiver, args, .. } => {
            current_prev = compute_positions_helper(receiver, current_prev, positions);
            for arg in args {
                current_prev = compute_positions_helper(arg, current_prev, positions);
            }
        }
        RholangNode::Eval { name, .. } => {
            current_prev = compute_positions_helper(name, current_prev, positions);
        }
        RholangNode::Quote { quotable, .. } => {
            // The quotable's delta was calculated from after the '@' symbol (see quote handler in tree_sitter.rs).
            // So we need to pass the position after '@' to match how the delta was computed.
            let after_at = Position {
                row: start.row,
                column: start.column + 1,
                byte: start.byte + 1,
            };
            current_prev = compute_positions_helper(quotable, after_at, positions);
        }
        RholangNode::VarRef { var, .. } => {
            current_prev = compute_positions_helper(var, current_prev, positions);
        }
        RholangNode::List {
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
        RholangNode::Set {
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
        RholangNode::Map { pairs, remainder, .. } => {
            for (key, value) in pairs {
                current_prev = compute_positions_helper(key, current_prev, positions);
                current_prev = compute_positions_helper(value, current_prev, positions);
            }
            if let Some(rem) = remainder {
                current_prev = compute_positions_helper(rem, current_prev, positions);
            }
        }
        RholangNode::Tuple { elements, .. } => {
            for elem in elements {
                current_prev = compute_positions_helper(elem, current_prev, positions);
            }
        }
        RholangNode::NameDecl { var, uri, .. } => {
            current_prev = compute_positions_helper(var, current_prev, positions);
            if let Some(u) = uri {
                current_prev = compute_positions_helper(u, current_prev, positions);
            }
        }
        RholangNode::Decl {
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
        RholangNode::LinearBind {
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
        RholangNode::RepeatedBind {
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
        RholangNode::PeekBind {
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
        RholangNode::ReceiveSendSource { name, .. } => {
            current_prev = compute_positions_helper(name, current_prev, positions);
        }
        RholangNode::SendReceiveSource { name, inputs, .. } => {
            current_prev = compute_positions_helper(name, current_prev, positions);
            for input in inputs {
                current_prev = compute_positions_helper(input, current_prev, positions);
            }
        }
        RholangNode::Error { children, .. } => {
            for child in children {
                current_prev = compute_positions_helper(child, current_prev, positions);
            }
        }
        RholangNode::Disjunction { left, right, .. } => {
            current_prev = compute_positions_helper(left, current_prev, positions);
            current_prev = compute_positions_helper(right, current_prev, positions);
        }
        RholangNode::Conjunction { left, right, .. } => {
            current_prev = compute_positions_helper(left, current_prev, positions);
            current_prev = compute_positions_helper(right, current_prev, positions);
        }
        RholangNode::Negation { operand, .. } => {
            current_prev = compute_positions_helper(operand, current_prev, positions);
        }
        RholangNode::Unit { .. } => {}
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
pub fn match_pat(pat: &Arc<RholangNode>, concrete: &Arc<RholangNode>, subst: &mut HashMap<String, Arc<RholangNode>>) -> bool {
    match (&**pat, &**concrete) {
        (RholangNode::Wildcard { .. }, _) => true,
        (RholangNode::Var { name: p_name, .. }, _) => {
            if let Some(bound) = subst.get(p_name) {
                **bound == **concrete
            } else {
                subst.insert(p_name.clone(), concrete.clone());
                true
            }
        }
        (
            RholangNode::Quote {
                quotable: p_q, ..
            },
            RholangNode::Quote {
                quotable: c_q, ..
            },
        ) => match_pat(p_q, c_q, subst),
        (RholangNode::Eval { name: p_n, .. }, RholangNode::Eval { name: c_n, .. }) => match_pat(p_n, c_n, subst),
        (
            RholangNode::VarRef {
                kind: p_k,
                var: p_v,
                ..
            },
            RholangNode::VarRef {
                kind: c_k,
                var: c_v,
                ..
            },
        ) => p_k == c_k && match_pat(p_v, c_v, subst),
        (
            RholangNode::List {
                elements: p_e,
                remainder: p_r,
                ..
            },
            RholangNode::List {
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
            let rem_list = Arc::new(RholangNode::List {
                base: rem_base,
                elements: rem_c_elements,
                remainder: c_r.clone(),
                metadata: None,
            });
            if let Some(r) = p_r {
                match_pat(r, &rem_list, subst)
            } else if let RholangNode::List {
                elements,
                remainder,
                ..
            } = &*rem_list {
                elements.is_empty() && remainder.is_none()
            } else {
                false
            }
        }
        (RholangNode::Tuple { elements: p_e, .. }, RholangNode::Tuple { elements: c_e, .. }) => {
            if p_e.len() != c_e.len() {
                false
            } else {
                p_e.iter()
                    .zip(c_e.iter())
                    .all(|(p, c)| match_pat(p, c, subst))
            }
        }
        (
            RholangNode::Set {
                elements: p_e,
                remainder: p_r,
                ..
            },
            RholangNode::Set {
                elements: c_e,
                remainder: c_r,
                ..
            },
        ) => {
            let mut p_sorted: Vec<&Arc<RholangNode>> = p_e.iter().collect();
            p_sorted.sort_by(|a, b| RholangNode::node_cmp(a, b));
            let mut c_sorted: Vec<&Arc<RholangNode>> = c_e.iter().collect();
            c_sorted.sort_by(|a, b| RholangNode::node_cmp(a, b));
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
            let rem_set = Arc::new(RholangNode::Set {
                base: rem_base,
                elements: rem_c_elements,
                remainder: c_r.clone(),
                metadata: None,
            });
            if let Some(r) = p_r {
                match_pat(r, &rem_set, subst)
            } else if let RholangNode::Set {
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
            RholangNode::Map {
                pairs: p_pairs,
                remainder: p_r,
                ..
            },
            RholangNode::Map {
                pairs: c_pairs,
                remainder: c_r,
                ..
            },
        ) => {
            let mut p_sorted: Vec<(&Arc<RholangNode>, &Arc<RholangNode>)> =
                p_pairs.iter().map(|(k, v)| (k, v)).collect();
            p_sorted.sort_by(|(ka, _), (kb, _)| RholangNode::node_cmp(ka, kb));
            let mut c_sorted: Vec<(&Arc<RholangNode>, &Arc<RholangNode>)> =
                c_pairs.iter().map(|(k, v)| (k, v)).collect();
            c_sorted.sort_by(|(ka, _), (kb, _)| RholangNode::node_cmp(ka, kb));
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
            let rem_map = Arc::new(RholangNode::Map {
                base: rem_base,
                pairs: rem_c_pairs,
                remainder: c_r.clone(),
                metadata: None,
            });
            if let Some(r) = p_r {
                match_pat(r, &rem_map, subst)
            } else if let RholangNode::Map {
                pairs,
                remainder,
                ..
            } = &*rem_map {
                pairs.is_empty() && remainder.is_none()
            } else {
                false
            }
        }
        (RholangNode::BoolLiteral { value: p, .. }, RholangNode::BoolLiteral { value: c, .. }) => p == c,
        (RholangNode::LongLiteral { value: p, .. }, RholangNode::LongLiteral { value: c, .. }) => p == c,
        (RholangNode::StringLiteral { value: p, .. }, RholangNode::StringLiteral { value: c, .. }) => p == c,
        (RholangNode::UriLiteral { value: p, .. }, RholangNode::UriLiteral { value: c, .. }) => p == c,
        (RholangNode::SimpleType { value: p, .. }, RholangNode::SimpleType { value: c, .. }) => p == c,
        (RholangNode::Nil { .. }, RholangNode::Nil { .. }) => true,
        (RholangNode::Unit { .. }, RholangNode::Unit { .. }) => true,
        (RholangNode::Disjunction { left: p_l, right: p_r, .. }, RholangNode::Disjunction { left: c_l, right: c_r, .. }) => {
            match_pat(p_l, c_l, subst) && match_pat(p_r, c_r, subst)
        }
        (RholangNode::Conjunction { left: p_l, right: p_r, .. }, RholangNode::Conjunction { left: c_l, right: c_r, .. }) => {
            match_pat(p_l, c_l, subst) && match_pat(p_r, c_r, subst)
        }
        (RholangNode::Negation { operand: p_o, .. }, RholangNode::Negation { operand: c_o, .. }) => {
            match_pat(p_o, c_o, subst)
        }
        (RholangNode::Parenthesized { expr: p_e, .. }, RholangNode::Parenthesized { expr: c_e, .. }) => {
            match_pat(p_e, c_e, subst)
        }
        _ => false,
    }
}

/// Matches a contract against a call's channel and inputs.
/// Check if two nodes are equal for contract name matching (avoids pattern matching's Var unification)
fn contract_names_equal(a: &Arc<RholangNode>, b: &Arc<RholangNode>) -> bool {
    match (&**a, &**b) {
        // Fast path: pointer equality
        _ if Arc::ptr_eq(a, b) => true,
        // Var nodes: compare names by reference (cheap since names are strings in Arc)
        (RholangNode::Var { name: a_name, .. }, RholangNode::Var { name: b_name, .. }) => a_name == b_name,
        // Quote nodes: recursively check quotable
        (RholangNode::Quote { quotable: a_q, .. }, RholangNode::Quote { quotable: b_q, .. }) => contract_names_equal(a_q, b_q),
        // Eval nodes: recursively check name
        (RholangNode::Eval { name: a_n, .. }, RholangNode::Eval { name: b_n, .. }) => contract_names_equal(a_n, b_n),
        // VarRef nodes: check kind and var
        (RholangNode::VarRef { kind: a_k, var: a_v, .. }, RholangNode::VarRef { kind: b_k, var: b_v, .. }) => {
            a_k == b_k && contract_names_equal(a_v, b_v)
        }
        // Different node types or other cases: not equal
        _ => false,
    }
}

pub fn match_contract(channel: &Arc<RholangNode>, inputs: &RholangNodeVector, contract: &Arc<RholangNode>) -> bool {
    if let RholangNode::Contract {
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
            let remaining_list = Arc::new(RholangNode::List {
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
pub fn collect_contracts(node: &Arc<RholangNode>, contracts: &mut Vec<Arc<RholangNode>>) {
    match &**node {
        RholangNode::Contract { .. } => contracts.push(node.clone()),
        RholangNode::Par { left: Some(left), right: Some(right), .. } => {
            collect_contracts(left, contracts);
            collect_contracts(right, contracts);
        }
        RholangNode::Par { processes: Some(procs), .. } => {
            for proc in procs.iter() {
                collect_contracts(proc, contracts);
            }
        }
        RholangNode::SendSync {
            channel, inputs, cont, ..
        } => {
            collect_contracts(channel, contracts);
            for input in inputs {
                collect_contracts(input, contracts);
            }
            collect_contracts(cont, contracts);
        }
        RholangNode::Send { channel, inputs, .. } => {
            collect_contracts(channel, contracts);
            for input in inputs {
                collect_contracts(input, contracts);
            }
        }
        RholangNode::New { decls, proc, .. } => {
            for decl in decls {
                collect_contracts(decl, contracts);
            }
            collect_contracts(proc, contracts);
        }
        RholangNode::IfElse {
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
        RholangNode::Let { decls, proc, .. } => {
            for decl in decls {
                collect_contracts(decl, contracts);
            }
            collect_contracts(proc, contracts);
        }
        RholangNode::Bundle { proc, .. } => collect_contracts(proc, contracts),
        RholangNode::Match {
            expression, cases, ..
        } => {
            collect_contracts(expression, contracts);
            for (pat, proc) in cases {
                collect_contracts(pat, contracts);
                collect_contracts(proc, contracts);
            }
        }
        RholangNode::Choice { branches, .. } => {
            for (inputs, proc) in branches {
                for input in inputs {
                    collect_contracts(input, contracts);
                }
                collect_contracts(proc, contracts);
            }
        }
        RholangNode::Input { receipts, proc, .. } => {
            for receipt in receipts {
                for bind in receipt {
                    collect_contracts(bind, contracts);
                }
            }
            collect_contracts(proc, contracts);
        }
        RholangNode::Block { proc, .. } => collect_contracts(proc, contracts),
        RholangNode::Parenthesized { expr, .. } => collect_contracts(expr, contracts),
        RholangNode::BinOp { left, right, .. } => {
            collect_contracts(left, contracts);
            collect_contracts(right, contracts);
        }
        RholangNode::UnaryOp { operand, .. } => collect_contracts(operand, contracts),
        RholangNode::Method {
            receiver, args, ..
        } => {
            collect_contracts(receiver, contracts);
            for arg in args {
                collect_contracts(arg, contracts);
            }
        }
        RholangNode::Eval { name, .. } => collect_contracts(name, contracts),
        RholangNode::Quote { quotable, .. } => collect_contracts(quotable, contracts),
        RholangNode::VarRef { var, .. } => collect_contracts(var, contracts),
        RholangNode::List {
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
        RholangNode::Set {
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
        RholangNode::Map {
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
        RholangNode::Tuple { elements, .. } => {
            for elem in elements {
                collect_contracts(elem, contracts);
            }
        }
        RholangNode::NameDecl { var, uri, .. } => {
            collect_contracts(var, contracts);
            if let Some(u) = uri {
                collect_contracts(u, contracts);
            }
        }
        RholangNode::Decl {
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
        RholangNode::LinearBind {
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
        RholangNode::RepeatedBind {
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
        RholangNode::PeekBind {
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
        RholangNode::ReceiveSendSource { name, .. } => collect_contracts(name, contracts),
        RholangNode::SendReceiveSource { name, inputs, .. } => {
            collect_contracts(name, contracts);
            for input in inputs {
                collect_contracts(input, contracts);
            }
        }
        RholangNode::Error { children, .. } => {
            for child in children {
                collect_contracts(child, contracts);
            }
        }
        RholangNode::Disjunction { left, right, .. } => {
            collect_contracts(left, contracts);
            collect_contracts(right, contracts);
        }
        RholangNode::Conjunction { left, right, .. } => {
            collect_contracts(left, contracts);
            collect_contracts(right, contracts);
        }
        RholangNode::Negation { operand, .. } => collect_contracts(operand, contracts),
        RholangNode::Unit { .. } => {}
        _ => {}
    }
}

/// Collects all call nodes (Send and SendSync) from the IR tree.
pub fn collect_calls(node: &Arc<RholangNode>, calls: &mut Vec<Arc<RholangNode>>) {
    match &**node {
        RholangNode::Send { .. } | RholangNode::SendSync { .. } => calls.push(node.clone()),
        RholangNode::Par { left: Some(left), right: Some(right), .. } => {
            collect_calls(left, calls);
            collect_calls(right, calls);
        }
        RholangNode::Par { processes: Some(procs), .. } => {
            for proc in procs.iter() {
                collect_calls(proc, calls);
            }
        }
        RholangNode::New { decls, proc, .. } => {
            for decl in decls {
                collect_calls(decl, calls);
            }
            collect_calls(proc, calls);
        }
        RholangNode::IfElse {
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
        RholangNode::Let { decls, proc, .. } => {
            for decl in decls {
                collect_calls(decl, calls);
            }
            collect_calls(proc, calls);
        }
        RholangNode::Bundle { proc, .. } => collect_calls(proc, calls),
        RholangNode::Match {
            expression, cases, ..
        } => {
            collect_calls(expression, calls);
            for (pat, proc) in cases {
                collect_calls(pat, calls);
                collect_calls(proc, calls);
            }
        }
        RholangNode::Choice { branches, .. } => {
            for (inputs, proc) in branches {
                for input in inputs {
                    collect_calls(input, calls);
                }
                collect_calls(proc, calls);
            }
        }
        RholangNode::Contract {
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
        RholangNode::Input { receipts, proc, .. } => {
            for receipt in receipts {
                for bind in receipt {
                    collect_calls(bind, calls);
                }
            }
            collect_calls(proc, calls);
        }
        RholangNode::Block { proc, .. } => collect_calls(proc, calls),
        RholangNode::Parenthesized { expr, .. } => collect_calls(expr, calls),
        RholangNode::BinOp { left, right, .. } => {
            collect_calls(left, calls);
            collect_calls(right, calls);
        }
        RholangNode::UnaryOp { operand, .. } => collect_calls(operand, calls),
        RholangNode::Method {
            receiver, args, ..
        } => {
            collect_calls(receiver, calls);
            for arg in args {
                collect_calls(arg, calls);
            }
        }
        RholangNode::Eval { name, .. } => collect_calls(name, calls),
        RholangNode::Quote { quotable, .. } => collect_calls(quotable, calls),
        RholangNode::VarRef { var, .. } => collect_calls(var, calls),
        RholangNode::List {
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
        RholangNode::Set {
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
        RholangNode::Map {
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
        RholangNode::Tuple { elements, .. } => {
            for elem in elements {
                collect_calls(elem, calls);
            }
        }
        RholangNode::NameDecl { var, uri, .. } => {
            collect_calls(var, calls);
            if let Some(u) = uri {
                collect_calls(u, calls);
            }
        }
        RholangNode::Decl {
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
        RholangNode::LinearBind {
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
        RholangNode::RepeatedBind {
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
        RholangNode::PeekBind {
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
        RholangNode::ReceiveSendSource { name, .. } => collect_calls(name, calls),
        RholangNode::SendReceiveSource { name, inputs, .. } => {
            collect_calls(name, calls);
            for input in inputs {
                collect_calls(input, calls);
            }
        }
        RholangNode::Error { children, .. } => {
            for child in children {
                collect_calls(child, calls);
            }
        }
        RholangNode::Disjunction { left, right, .. } => {
            collect_calls(left, calls);
            collect_calls(right, calls);
        }
        RholangNode::Conjunction { left, right, .. } => {
            collect_calls(left, calls);
            collect_calls(right, calls);
        }
        RholangNode::Negation { operand, .. } => collect_calls(operand, calls),
        RholangNode::Unit { .. } => {},
        _ => {},
    }
}

/// Traverses the tree with path tracking for finding node at position.
pub fn find_node_at_position_with_path(
    root: &Arc<RholangNode>,
    positions: &HashMap<usize, (Position, Position)>,
    position: Position,
) -> Option<(Arc<RholangNode>, Vec<Arc<RholangNode>>)> {
    let mut path = Vec::new();
    let mut best: Option<(Arc<RholangNode>, Vec<Arc<RholangNode>>, usize)> = None;
    traverse_with_path(root, position, positions, &mut path, &mut best, 0);
    best.map(|(node, p, _)| (node, p))
}

fn traverse_with_path(
    node: &Arc<RholangNode>,
    pos: Position,
    positions: &HashMap<usize, (Position, Position)>,
    path: &mut Vec<Arc<RholangNode>>,
    best: &mut Option<(Arc<RholangNode>, Vec<Arc<RholangNode>>, usize)>,
    depth: usize,
) {
    path.push(node.clone());
    let key = &**node as *const RholangNode as usize;
    if let Some(&(start, end)) = positions.get(&key) {
        // Hot path: removed per-node debug logging to avoid thousands of log lines per request
        // Enable with RUST_LOG=trace for deep debugging
        if start.byte <= pos.byte && pos.byte <= end.byte {
            let is_better = best.as_ref().map_or(true, |(_, _, b_depth)| depth > *b_depth);
            if is_better {
                trace!("Found better match at depth {} for position {}", depth, pos.byte);
                *best = Some((node.clone(), path.clone(), depth));
            }
        }
    }
    match &**node {
        RholangNode::Par { left: Some(left), right: Some(right), .. } => {
            traverse_with_path(left, pos, positions, path, best, depth + 1);
            traverse_with_path(right, pos, positions, path, best, depth + 1);
        }
        RholangNode::Par { processes: Some(procs), .. } => {
            for proc in procs.iter() {
                traverse_with_path(proc, pos, positions, path, best, depth + 1);
            }
        }
        RholangNode::SendSync {
            channel, inputs, cont, ..
        } => {
            traverse_with_path(channel, pos, positions, path, best, depth + 1);
            for input in inputs {
                traverse_with_path(input, pos, positions, path, best, depth + 1);
            }
            traverse_with_path(cont, pos, positions, path, best, depth + 1);
        }
        RholangNode::Send { channel, inputs, .. } => {
            traverse_with_path(channel, pos, positions, path, best, depth + 1);
            for input in inputs {
                traverse_with_path(input, pos, positions, path, best, depth + 1);
            }
        }
        RholangNode::New { decls, proc, .. } => {
            for decl in decls {
                traverse_with_path(decl, pos, positions, path, best, depth + 1);
            }
            traverse_with_path(proc, pos, positions, path, best, depth + 1);
        }
        RholangNode::IfElse {
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
        RholangNode::Let { decls, proc, .. } => {
            for decl in decls {
                traverse_with_path(decl, pos, positions, path, best, depth + 1);
            }
            traverse_with_path(proc, pos, positions, path, best, depth + 1);
        }
        RholangNode::Bundle { proc, .. } => traverse_with_path(proc, pos, positions, path, best, depth + 1),
        RholangNode::Match { expression, cases, .. } => {
            traverse_with_path(expression, pos, positions, path, best, depth + 1);
            for (pat, proc) in cases {
                traverse_with_path(pat, pos, positions, path, best, depth + 1);
                traverse_with_path(proc, pos, positions, path, best, depth + 1);
            }
        }
        RholangNode::Choice { branches, .. } => {
            for (inputs, proc) in branches {
                for input in inputs {
                    traverse_with_path(input, pos, positions, path, best, depth + 1);
                }
                traverse_with_path(proc, pos, positions, path, best, depth + 1);
            }
        }
        RholangNode::Contract { name, formals, formals_remainder, proc, .. } => {
            traverse_with_path(name, pos, positions, path, best, depth + 1);
            for formal in formals {
                traverse_with_path(formal, pos, positions, path, best, depth + 1);
            }
            if let Some(rem) = formals_remainder {
                traverse_with_path(rem, pos, positions, path, best, depth + 1);
            }
            traverse_with_path(proc, pos, positions, path, best, depth + 1);
        }
        RholangNode::Input { receipts, proc, .. } => {
            for receipt in receipts {
                for bind in receipt {
                    traverse_with_path(bind, pos, positions, path, best, depth + 1);
                }
            }
            traverse_with_path(proc, pos, positions, path, best, depth + 1);
        }
        RholangNode::Block { proc, .. } => traverse_with_path(proc, pos, positions, path, best, depth + 1),
        RholangNode::Parenthesized { expr, .. } => traverse_with_path(expr, pos, positions, path, best, depth + 1),
        RholangNode::BinOp { left, right, .. } => {
            traverse_with_path(left, pos, positions, path, best, depth + 1);
            traverse_with_path(right, pos, positions, path, best, depth + 1);
        }
        RholangNode::UnaryOp { operand, .. } => traverse_with_path(operand, pos, positions, path, best, depth + 1),
        RholangNode::Method { receiver, args, .. } => {
            traverse_with_path(receiver, pos, positions, path, best, depth + 1);
            for arg in args {
                traverse_with_path(arg, pos, positions, path, best, depth + 1);
            }
        }
        RholangNode::Eval { name, .. } => traverse_with_path(name, pos, positions, path, best, depth + 1),
        RholangNode::Quote { quotable, .. } => traverse_with_path(quotable, pos, positions, path, best, depth + 1),
        RholangNode::VarRef { var, .. } => traverse_with_path(var, pos, positions, path, best, depth + 1),
        RholangNode::List { elements, remainder, .. } => {
            for elem in elements {
                traverse_with_path(elem, pos, positions, path, best, depth + 1);
            }
            if let Some(rem) = remainder {
                traverse_with_path(rem, pos, positions, path, best, depth + 1);
            }
        }
        RholangNode::Set { elements, remainder, .. } => {
            for elem in elements {
                traverse_with_path(elem, pos, positions, path, best, depth + 1);
            }
            if let Some(rem) = remainder {
                traverse_with_path(rem, pos, positions, path, best, depth + 1);
            }
        }
        RholangNode::Map { pairs, remainder, .. } => {
            for (key, value) in pairs {
                traverse_with_path(key, pos, positions, path, best, depth + 1);
                traverse_with_path(value, pos, positions, path, best, depth + 1);
            }
            if let Some(rem) = remainder {
                traverse_with_path(rem, pos, positions, path, best, depth + 1);
            }
        }
        RholangNode::Tuple { elements, .. } => {
            for elem in elements {
                traverse_with_path(elem, pos, positions, path, best, depth + 1);
            }
        }
        RholangNode::NameDecl { var, uri, .. } => {
            traverse_with_path(var, pos, positions, path, best, depth + 1);
            if let Some(u) = uri {
                traverse_with_path(u, pos, positions, path, best, depth + 1);
            }
        }
        RholangNode::Decl { names, names_remainder, procs, .. } => {
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
        RholangNode::LinearBind { names, remainder, source, .. } => {
            for name in names {
                traverse_with_path(name, pos, positions, path, best, depth + 1);
            }
            if let Some(rem) = remainder {
                traverse_with_path(rem, pos, positions, path, best, depth + 1);
            }
            traverse_with_path(source, pos, positions, path, best, depth + 1);
        }
        RholangNode::RepeatedBind { names, remainder, source, .. } => {
            for name in names {
                traverse_with_path(name, pos, positions, path, best, depth + 1);
            }
            if let Some(rem) = remainder {
                traverse_with_path(rem, pos, positions, path, best, depth + 1);
            }
            traverse_with_path(source, pos, positions, path, best, depth + 1);
        }
        RholangNode::PeekBind { names, remainder, source, .. } => {
            for name in names {
                traverse_with_path(name, pos, positions, path, best, depth + 1);
            }
            if let Some(rem) = remainder {
                traverse_with_path(rem, pos, positions, path, best, depth + 1);
            }
            traverse_with_path(source, pos, positions, path, best, depth + 1);
        }
        RholangNode::ReceiveSendSource { name, .. } => traverse_with_path(name, pos, positions, path, best, depth + 1),
        RholangNode::SendReceiveSource { name, inputs, .. } => {
            traverse_with_path(name, pos, positions, path, best, depth + 1);
            for input in inputs {
                traverse_with_path(input, pos, positions, path, best, depth + 1);
            }
        }
        RholangNode::Error { children, .. } => {
            for child in children {
                traverse_with_path(child, pos, positions, path, best, depth + 1);
            }
        }
        RholangNode::Disjunction { left, right, .. } => {
            traverse_with_path(left, pos, positions, path, best, depth + 1);
            traverse_with_path(right, pos, positions, path, best, depth + 1);
        }
        RholangNode::Conjunction { left, right, .. } => {
            traverse_with_path(left, pos, positions, path, best, depth + 1);
            traverse_with_path(right, pos, positions, path, best, depth + 1);
        }
        RholangNode::Negation { operand, .. } => traverse_with_path(operand, pos, positions, path, best, depth + 1),
        RholangNode::Unit { .. } => {}
        _ => {}
    }
    path.pop();
}

fn traverse(
    node: &Arc<RholangNode>,
    pos: Position,
    positions: &HashMap<usize, (Position, Position)>,
    best: &mut Option<(Arc<RholangNode>, Position, usize)>,
    depth: usize,
) {
    let key = &**node as *const RholangNode as usize;
    if let Some(&(start, end)) = positions.get(&key) {
        // Hot path: removed per-node debug logging - same as traverse_with_path
        if start.byte <= pos.byte && pos.byte <= end.byte {
            let is_better = best.as_ref().map_or(true, |(_, _, b_depth)| depth > *b_depth);
            if is_better {
                trace!("Found better match at depth {} for position {}", depth, pos.byte);
                *best = Some((node.clone(), start, depth));
            }
        }
    }
    match &**node {
        RholangNode::Par { left: Some(left), right: Some(right), .. } => {
            traverse(left, pos, positions, best, depth + 1);
            traverse(right, pos, positions, best, depth + 1);
        }
        RholangNode::Par { processes: Some(procs), .. } => {
            for proc in procs.iter() {
                traverse(proc, pos, positions, best, depth + 1);
            }
        }
        RholangNode::SendSync { channel, inputs, cont, .. } => {
            traverse(channel, pos, positions, best, depth + 1);
            for input in inputs {
                traverse(input, pos, positions, best, depth + 1);
            }
            traverse(cont, pos, positions, best, depth + 1);
        }
        RholangNode::Send { channel, inputs, .. } => {
            traverse(channel, pos, positions, best, depth + 1);
            for input in inputs {
                traverse(input, pos, positions, best, depth + 1);
            }
        }
        RholangNode::New { decls, proc, .. } => {
            for decl in decls {
                traverse(decl, pos, positions, best, depth + 1);
            }
            traverse(proc, pos, positions, best, depth + 1);
        }
        RholangNode::IfElse {
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
        RholangNode::Let { decls, proc, .. } => {
            for decl in decls {
                traverse(decl, pos, positions, best, depth + 1);
            }
            traverse(proc, pos, positions, best, depth + 1);
        }
        RholangNode::Bundle { proc, .. } => traverse(proc, pos, positions, best, depth + 1),
        RholangNode::Match { expression, cases, .. } => {
            traverse(expression, pos, positions, best, depth + 1);
            for (pat, proc) in cases {
                traverse(pat, pos, positions, best, depth + 1);
                traverse(proc, pos, positions, best, depth + 1);
            }
        }
        RholangNode::Choice { branches, .. } => {
            for (inputs, proc) in branches {
                for input in inputs {
                    traverse(input, pos, positions, best, depth + 1);
                }
                traverse(proc, pos, positions, best, depth + 1);
            }
        }
        RholangNode::Contract { name, formals, formals_remainder, proc, .. } => {
            traverse(name, pos, positions, best, depth + 1);
            for formal in formals {
                traverse(formal, pos, positions, best, depth + 1);
            }
            if let Some(rem) = formals_remainder {
                traverse(rem, pos, positions, best, depth + 1);
            }
            traverse(proc, pos, positions, best, depth + 1);
        }
        RholangNode::Input { receipts, proc, .. } => {
            for receipt in receipts {
                for bind in receipt {
                    traverse(bind, pos, positions, best, depth + 1);
                }
            }
            traverse(proc, pos, positions, best, depth + 1);
        }
        RholangNode::Block { proc, .. } => traverse(proc, pos, positions, best, depth + 1),
        RholangNode::Parenthesized { expr, .. } => traverse(expr, pos, positions, best, depth + 1),
        RholangNode::BinOp { left, right, .. } => {
            traverse(left, pos, positions, best, depth + 1);
            traverse(right, pos, positions, best, depth + 1);
        }
        RholangNode::UnaryOp { operand, .. } => traverse(operand, pos, positions, best, depth + 1),
        RholangNode::Method { receiver, args, .. } => {
            traverse(receiver, pos, positions, best, depth + 1);
            for arg in args {
                traverse(arg, pos, positions, best, depth + 1);
            }
        }
        RholangNode::Eval { name, .. } => traverse(name, pos, positions, best, depth + 1),
        RholangNode::Quote { quotable, .. } => traverse(quotable, pos, positions, best, depth + 1),
        RholangNode::VarRef { var, .. } => traverse(var, pos, positions, best, depth + 1),
        RholangNode::List { elements, remainder, .. } => {
            for elem in elements {
                traverse(elem, pos, positions, best, depth + 1);
            }
            if let Some(rem) = remainder {
                traverse(rem, pos, positions, best, depth + 1);
            }
        }
        RholangNode::Set { elements, remainder, .. } => {
            for elem in elements {
                traverse(elem, pos, positions, best, depth + 1);
            }
            if let Some(rem) = remainder {
                traverse(rem, pos, positions, best, depth + 1);
            }
        }
        RholangNode::Map { pairs, remainder, .. } => {
            for (key, value) in pairs {
                traverse(key, pos, positions, best, depth + 1);
                traverse(value, pos, positions, best, depth + 1);
            }
            if let Some(rem) = remainder {
                traverse(rem, pos, positions, best, depth + 1);
            }
        }
        RholangNode::Tuple { elements, .. } => {
            for elem in elements {
                traverse(elem, pos, positions, best, depth + 1);
            }
        }
        RholangNode::NameDecl { var, uri, .. } => {
            traverse(var, pos, positions, best, depth + 1);
            if let Some(u) = uri {
                traverse(u, pos, positions, best, depth + 1);
            }
        }
        RholangNode::Decl { names, names_remainder, procs, .. } => {
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
        RholangNode::LinearBind { names, remainder, source, .. } => {
            for name in names {
                traverse(name, pos, positions, best, depth + 1);
            }
            if let Some(rem) = remainder {
                traverse(rem, pos, positions, best, depth + 1);
            }
            traverse(source, pos, positions, best, depth + 1);
        }
        RholangNode::RepeatedBind { names, remainder, source, .. } => {
            for name in names {
                traverse(name, pos, positions, best, depth + 1);
            }
            if let Some(rem) = remainder {
                traverse(rem, pos, positions, best, depth + 1);
            }
            traverse(source, pos, positions, best, depth + 1);
        }
        RholangNode::PeekBind { names, remainder, source, .. } => {
            for name in names {
                traverse(name, pos, positions, best, depth + 1);
            }
            if let Some(rem) = remainder {
                traverse(rem, pos, positions, best, depth + 1);
            }
            traverse(source, pos, positions, best, depth + 1);
        }
        RholangNode::ReceiveSendSource { name, .. } => traverse(name, pos, positions, best, depth + 1),
        RholangNode::SendReceiveSource { name, inputs, .. } => {
            traverse(name, pos, positions, best, depth + 1);
            for input in inputs {
                traverse(input, pos, positions, best, depth + 1);
            }
        }
        RholangNode::Error { children, .. } => {
            for child in children {
                traverse(child, pos, positions, best, depth + 1);
            }
        }
        RholangNode::Disjunction { left, right, .. } => {
            traverse(left, pos, positions, best, depth + 1);
            traverse(right, pos, positions, best, depth + 1);
        }
        RholangNode::Conjunction { left, right, .. } => {
            traverse(left, pos, positions, best, depth + 1);
            traverse(right, pos, positions, best, depth + 1);
        }
        RholangNode::Negation { operand, .. } => traverse(operand, pos, positions, best, depth + 1),
        RholangNode::Unit { .. } => {},
        _ => {},
    }
}

pub fn find_node_at_position(
    root: &Arc<RholangNode>,
    positions: &HashMap<usize, (Position, Position)>,
    position: Position,
) -> Option<Arc<RholangNode>> {
    let mut best: Option<(Arc<RholangNode>, Position, usize)> = None;
    traverse(root, position, positions, &mut best, 0);
    if let Some(node) = best.map(|(node, _, _) | node) {
        debug!("Found best match");
        Some(node)
    } else {
        debug!("No node found at position {:?}", position);
        None
    }
}

impl RholangNode {
    /// Returns the processes in a Par node, handling both binary and n-ary forms.
    ///
    /// This helper provides uniform access during the migration from binary to n-ary Par.
    /// Returns an empty vector if called on a non-Par node.
    pub fn par_processes(&self) -> Vec<Arc<RholangNode>> {
        match self {
            RholangNode::Par { processes: Some(procs), .. } => {
                // N-ary form - return all processes
                procs.iter().cloned().collect()
            }
            RholangNode::Par { left: Some(left), right: Some(right), .. } => {
                // Binary form - return left and right
                vec![left.clone(), right.clone()]
            }
            _ => vec![],
        }
    }

    /// Returns the starting line number of the node within the source code.
    ///
    /// # Arguments
    /// * root - The root node of the IR tree, used for position computation.
    pub fn start_line(&self, root: &Arc<RholangNode>) -> usize {
        let positions = compute_absolute_positions(root);
        let key = self as *const RholangNode as usize;
        positions.get(&key).expect("RholangNode not found").0.row
    }

    /// Returns the starting column number of the node within the source code.
    ///
    /// # Arguments
    /// * root - The root node of the IR tree, used for position computation.
    pub fn start_column(&self, root: &Arc<RholangNode>) -> usize {
        let positions = compute_absolute_positions(root);
        let key = self as *const RholangNode as usize;
        positions.get(&key).expect("RholangNode not found").0.column
    }

    /// Returns the ending line number of the node within the source code.
    ///
    /// # Arguments
    /// * root - The root node of the IR tree, used for position computation.
    pub fn end_line(&self, root: &Arc<RholangNode>) -> usize {
        let positions = compute_absolute_positions(root);
        let key = self as *const RholangNode as usize;
        positions.get(&key).expect("RholangNode not found").1.row
    }

    /// Returns the ending column number of the node within the source code.
    ///
    /// # Arguments
    /// * root - The root node of the IR tree, used for position computation.
    pub fn end_column(&self, root: &Arc<RholangNode>) -> usize {
        let positions = compute_absolute_positions(root);
        let key = self as *const RholangNode as usize;
        positions.get(&key).expect("RholangNode not found").1.column
    }

    /// Returns the byte offset of the node’s start position in the source code.
    ///
    /// # Arguments
    /// * root - The root node of the IR tree, used for position computation.
    pub fn position(&self, root: &Arc<RholangNode>) -> usize {
        let positions = compute_absolute_positions(root);
        let key = self as *const RholangNode as usize;
        positions.get(&key).expect("RholangNode not found").0.byte
    }

    /// Returns the length of the node's text in bytes.
    pub fn length(&self) -> usize {
        self.base().length()
    }

    /// Returns the absolute start position of the node in the source code.
    ///
    /// # Arguments
    /// * root - The root node of the IR tree, used for position computation.
    pub fn absolute_start(&self, root: &Arc<RholangNode>) -> Position {
        let positions = compute_absolute_positions(root);
        let key = self as *const RholangNode as usize;
        positions.get(&key).expect("RholangNode not found").0
    }

    /// Returns the absolute end position of the node in the source code.
    ///
    /// # Arguments
    /// * root - The root node of the IR tree, used for position computation.
    pub fn absolute_end(&self, root: &Arc<RholangNode>) -> Position {
        let positions = compute_absolute_positions(root);
        let key = self as *const RholangNode as usize;
        positions.get(&key).expect("RholangNode not found").1
    }

    /// Creates a new node with the same fields but a different NodeBase.
    ///
    /// # Arguments
    /// * new_base - The new NodeBase to apply to the node.
    ///
    /// # Returns
    /// A new Arc<RholangNode> with the updated base.
    pub fn with_base(&self, new_base: NodeBase) -> Arc<RholangNode> {
        match self {
            RholangNode::Par {
                processes: None,
                metadata,
                left,
                right,
                ..
            } => Arc::new(RholangNode::Par {
                processes: None,
                base: new_base,
                left: left.clone(),
                right: right.clone(),
                metadata: metadata.clone(),
            }),
            // N-ary Par case (currently not used but needed for exhaustiveness)
            RholangNode::Par {
                processes: Some(procs),
                metadata,
                ..
            } => Arc::new(RholangNode::Par {
                base: new_base,
                left: None,
                right: None,
                processes: Some(procs.clone()),
                metadata: metadata.clone(),
            }),
            RholangNode::SendSync {
                metadata,
                channel,
                inputs,
                cont,
                ..
            } => Arc::new(RholangNode::SendSync {
                base: new_base,
                channel: channel.clone(),
                inputs: inputs.clone(),
                cont: cont.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Send {
                metadata,
                channel,
                send_type,
                send_type_delta,
                inputs,
                ..
            } => Arc::new(RholangNode::Send {
                base: new_base,
                channel: channel.clone(),
                send_type: send_type.clone(),
                send_type_delta: *send_type_delta,
                inputs: inputs.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::New { decls, proc, metadata, .. } => Arc::new(RholangNode::New {
                base: new_base,
                decls: decls.clone(),
                proc: proc.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::IfElse {
                condition,
                consequence,
                alternative,
                metadata,
                ..
            } => Arc::new(RholangNode::IfElse {
                base: new_base,
                condition: condition.clone(),
                consequence: consequence.clone(),
                alternative: alternative.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Let { decls, proc, metadata, .. } => Arc::new(RholangNode::Let {
                base: new_base,
                decls: decls.clone(),
                proc: proc.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Bundle {
                bundle_type,
                proc,
                metadata,
                ..
            } => Arc::new(RholangNode::Bundle {
                base: new_base,
                bundle_type: bundle_type.clone(),
                proc: proc.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Match {
                expression,
                cases,
                metadata,
                ..
            } => Arc::new(RholangNode::Match {
                base: new_base,
                expression: expression.clone(),
                cases: cases.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Choice {
                branches, metadata, ..
            } => Arc::new(RholangNode::Choice {
                base: new_base,
                branches: branches.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Contract {
                name,
                formals,
                formals_remainder,
                proc,
                metadata,
                ..
            } => Arc::new(RholangNode::Contract {
                base: new_base,
                name: name.clone(),
                formals: formals.clone(),
                formals_remainder: formals_remainder.clone(),
                proc: proc.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Input {
                receipts, proc, metadata, ..
            } => Arc::new(RholangNode::Input {
                base: new_base,
                receipts: receipts.clone(),
                proc: proc.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Block { proc, metadata, .. } => Arc::new(RholangNode::Block {
                base: new_base,
                proc: proc.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Parenthesized { expr, metadata, .. } => Arc::new(RholangNode::Parenthesized {
                base: new_base,
                expr: expr.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::BinOp {
                op,
                left,
                right,
                metadata,
                ..
            } => Arc::new(RholangNode::BinOp {
                base: new_base,
                op: op.clone(),
                left: left.clone(),
                right: right.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::UnaryOp {
                op, operand, metadata, ..
            } => Arc::new(RholangNode::UnaryOp {
                base: new_base,
                op: op.clone(),
                operand: operand.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Method {
                receiver,
                name,
                args,
                metadata,
                ..
            } => Arc::new(RholangNode::Method {
                base: new_base,
                receiver: receiver.clone(),
                name: name.clone(),
                args: args.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Eval { name, metadata, .. } => Arc::new(RholangNode::Eval {
                base: new_base,
                name: name.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Quote {
                quotable, metadata, ..
            } => Arc::new(RholangNode::Quote {
                base: new_base,
                quotable: quotable.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::VarRef {
                kind, var, metadata, ..
            } => Arc::new(RholangNode::VarRef {
                base: new_base,
                kind: kind.clone(),
                var: var.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::BoolLiteral { value, metadata, .. } => Arc::new(RholangNode::BoolLiteral {
                base: new_base,
                value: *value,
                metadata: metadata.clone(),
            }),
            RholangNode::LongLiteral { value, metadata, .. } => Arc::new(RholangNode::LongLiteral {
                base: new_base,
                value: *value,
                metadata: metadata.clone(),
            }),
            RholangNode::StringLiteral { value, metadata, .. } => Arc::new(RholangNode::StringLiteral {
                base: new_base,
                value: value.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::UriLiteral { value, metadata, .. } => Arc::new(RholangNode::UriLiteral {
                base: new_base,
                value: value.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Nil { metadata, .. } => Arc::new(RholangNode::Nil {
                base: new_base,
                metadata: metadata.clone(),
            }),
            RholangNode::List {
                elements,
                remainder,
                metadata,
                ..
            } => Arc::new(RholangNode::List {
                base: new_base,
                elements: elements.clone(),
                remainder: remainder.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Set {
                elements,
                remainder,
                metadata,
                ..
            } => Arc::new(RholangNode::Set {
                base: new_base,
                elements: elements.clone(),
                remainder: remainder.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Map {
                pairs,
                remainder,
                metadata,
                ..
            } => Arc::new(RholangNode::Map {
                base: new_base,
                pairs: pairs.clone(),
                remainder: remainder.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Tuple {
                elements, metadata, ..
            } => Arc::new(RholangNode::Tuple {
                base: new_base,
                elements: elements.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Var { name, metadata, .. } => Arc::new(RholangNode::Var {
                base: new_base,
                name: name.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::NameDecl {
                var, uri, metadata, ..
            } => Arc::new(RholangNode::NameDecl {
                base: new_base,
                var: var.clone(),
                uri: uri.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Decl {
                names,
                names_remainder,
                procs,
                metadata,
                ..
            } => Arc::new(RholangNode::Decl {
                base: new_base,
                names: names.clone(),
                names_remainder: names_remainder.clone(),
                procs: procs.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::LinearBind {
                names,
                remainder,
                source,
                metadata,
                ..
            } => Arc::new(RholangNode::LinearBind {
                base: new_base,
                names: names.clone(),
                remainder: remainder.clone(),
                source: source.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::RepeatedBind {
                names,
                remainder,
                source,
                metadata,
                ..
            } => Arc::new(RholangNode::RepeatedBind {
                base: new_base,
                names: names.clone(),
                remainder: remainder.clone(),
                source: source.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::PeekBind {
                names,
                remainder,
                source,
                metadata,
                ..
            } => Arc::new(RholangNode::PeekBind {
                base: new_base,
                names: names.clone(),
                remainder: remainder.clone(),
                source: source.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Comment { kind, metadata, .. } => Arc::new(RholangNode::Comment {
                base: new_base,
                kind: kind.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Wildcard { metadata, .. } => Arc::new(RholangNode::Wildcard {
                base: new_base,
                metadata: metadata.clone(),
            }),
            RholangNode::SimpleType { value, metadata, .. } => Arc::new(RholangNode::SimpleType {
                base: new_base,
                value: value.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::ReceiveSendSource { name, metadata, .. } => Arc::new(RholangNode::ReceiveSendSource {
                base: new_base,
                name: name.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::SendReceiveSource {
                name,
                inputs,
                metadata,
                ..
            } => Arc::new(RholangNode::SendReceiveSource {
                base: new_base,
                name: name.clone(),
                inputs: inputs.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Error {
                children, metadata, ..
            } => Arc::new(RholangNode::Error {
                base: new_base,
                children: children.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Disjunction {
                left, right, metadata, ..
            } => Arc::new(RholangNode::Disjunction {
                base: new_base,
                left: left.clone(),
                right: right.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Conjunction {
                left, right, metadata, ..
            } => Arc::new(RholangNode::Conjunction {
                base: new_base,
                left: left.clone(),
                right: right.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Negation {
                operand, metadata, ..
            } => Arc::new(RholangNode::Negation {
                base: new_base,
                operand: operand.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Unit { metadata, .. } => Arc::new(RholangNode::Unit {
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
            RholangNode::Send { channel, .. } | RholangNode::SendSync { channel, .. } => {
                if let RholangNode::Var { name, .. } = &**channel {
                    if RESERVED_KEYWORDS.contains(&name.as_str()) {
                        return Err(format!("Channel name '{name}' is a reserved keyword"));
                    }
                }
            }
            RholangNode::Par { left: Some(left), right: Some(right), .. } => {
                left.validate()?;
                right.validate()?;
            }
            RholangNode::Par { processes: Some(procs), .. } => {
                for proc in procs.iter() {
                    proc.validate()?;
                }
            }
            RholangNode::New { decls, proc, .. } => {
                for decl in decls {
                    decl.validate()?;
                }
                proc.validate()?;
            }
            RholangNode::IfElse {
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
            RholangNode::Let { decls, proc, .. } => {
                for decl in decls {
                    decl.validate()?;
                }
                proc.validate()?;
            }
            RholangNode::Bundle { proc, .. } => proc.validate()?,
            RholangNode::Match {
                expression, cases, ..
            } => {
                expression.validate()?;
                for (pattern, proc) in cases {
                    if let RholangNode::Var { name, .. } = &**pattern {
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
            RholangNode::Choice { branches, .. } => {
                for (inputs, proc) in branches {
                    for input in inputs {
                        if let RholangNode::LinearBind {
                            names, remainder, ..
                        } = &**input
                        {
                            for name in names {
                                if let RholangNode::Var { name: var_name, .. } = &**name {
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
            RholangNode::Contract {
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
            RholangNode::Input { receipts, proc, .. } => {
                for receipt in receipts {
                    for bind in receipt {
                        bind.validate()?;
                    }
                }
                proc.validate()?;
            }
            RholangNode::Block { proc, .. } => proc.validate()?,
            RholangNode::Parenthesized { expr, .. } => expr.validate()?,
            RholangNode::BinOp { left, right, .. } => {
                left.validate()?;
                right.validate()?;
            }
            RholangNode::UnaryOp { operand, .. } => operand.validate()?,
            RholangNode::Method { receiver, args, .. } => {
                receiver.validate()?;
                for arg in args {
                    arg.validate()?;
                }
            }
            RholangNode::Eval { name, .. } => name.validate()?,
            RholangNode::Quote { quotable, .. } => quotable.validate()?,
            RholangNode::VarRef { var, .. } => var.validate()?,
            RholangNode::List {
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
            RholangNode::Set {
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
            RholangNode::Map { pairs, remainder, .. } => {
                for (key, value) in pairs {
                    key.validate()?;
                    value.validate()?;
                }
                if let Some(rem) = remainder {
                    rem.validate()?;
                }
            }
            RholangNode::Tuple { elements, .. } => {
                for elem in elements {
                    elem.validate()?;
                }
            }
            RholangNode::NameDecl { var, uri, .. } => {
                var.validate()?;
                if let Some(u) = uri {
                    u.validate()?;
                }
            }
            RholangNode::Decl {
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
            RholangNode::LinearBind {
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
            RholangNode::RepeatedBind {
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
            RholangNode::PeekBind {
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
            RholangNode::ReceiveSendSource { name, .. } => name.validate()?,
            RholangNode::SendReceiveSource { name, inputs, .. } => {
                name.validate()?;
                for input in inputs {
                    input.validate()?;
                }
            }
            RholangNode::Error { children, .. } => {
                for child in children {
                    child.validate()?;
                }
            }
            RholangNode::Disjunction { left, right, .. } => {
                left.validate()?;
                right.validate()?;
            }
            RholangNode::Conjunction { left, right, .. } => {
                left.validate()?;
                right.validate()?;
            }
            RholangNode::Negation { operand, .. } => operand.validate()?,
            RholangNode::Unit { .. } => {},
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
    /// A new Arc<RholangNode> with the updated metadata.
    pub fn with_metadata(&self, new_metadata: Option<Arc<Metadata>>) -> Arc<RholangNode> {
        match self {
            RholangNode::Par { base, left, right, processes, .. } => Arc::new(RholangNode::Par {
                base: base.clone(),
                left: left.clone(),
                right: right.clone(),
                processes: processes.clone(),
                metadata: new_metadata,
            }),
            RholangNode::SendSync {
                base,
                channel,
                inputs,
                cont,
                ..
            } => Arc::new(RholangNode::SendSync {
                base: base.clone(),
                channel: channel.clone(),
                inputs: inputs.clone(),
                cont: cont.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Send {
                base,
                channel,
                send_type,
                send_type_delta,
                inputs,
                ..
            } => Arc::new(RholangNode::Send {
                base: base.clone(),
                channel: channel.clone(),
                send_type: send_type.clone(),
                send_type_delta: *send_type_delta,
                inputs: inputs.clone(),
                metadata: new_metadata,
            }),
            RholangNode::New { base, decls, proc, .. } => Arc::new(RholangNode::New {
                base: base.clone(),
                decls: decls.clone(),
                proc: proc.clone(),
                metadata: new_metadata,
            }),
            RholangNode::IfElse {
                base,
                condition,
                consequence,
                alternative,
                ..
            } => Arc::new(RholangNode::IfElse {
                base: base.clone(),
                condition: condition.clone(),
                consequence: consequence.clone(),
                alternative: alternative.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Let { base, decls, proc, .. } => Arc::new(RholangNode::Let {
                base: base.clone(),
                decls: decls.clone(),
                proc: proc.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Bundle {
                base, bundle_type, proc, ..
            } => Arc::new(RholangNode::Bundle {
                base: base.clone(),
                bundle_type: bundle_type.clone(),
                proc: proc.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Match {
                base, expression, cases, ..
            } => Arc::new(RholangNode::Match {
                base: base.clone(),
                expression: expression.clone(),
                cases: cases.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Choice { base, branches, .. } => Arc::new(RholangNode::Choice {
                base: base.clone(),
                branches: branches.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Contract {
                base,
                name,
                formals,
                formals_remainder,
                proc,
                ..
            } => Arc::new(RholangNode::Contract {
                base: base.clone(),
                name: name.clone(),
                formals: formals.clone(),
                formals_remainder: formals_remainder.clone(),
                proc: proc.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Input {
                base, receipts, proc, ..
            } => Arc::new(RholangNode::Input {
                base: base.clone(),
                receipts: receipts.clone(),
                proc: proc.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Block { base, proc, .. } => Arc::new(RholangNode::Block {
                base: base.clone(),
                proc: proc.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Parenthesized { base, expr, .. } => Arc::new(RholangNode::Parenthesized {
                base: base.clone(),
                expr: expr.clone(),
                metadata: new_metadata,
            }),
            RholangNode::BinOp {
                base,
                op,
                left,
                right,
                ..
            } => Arc::new(RholangNode::BinOp {
                base: base.clone(),
                op: op.clone(),
                left: left.clone(),
                right: right.clone(),
                metadata: new_metadata,
            }),
            RholangNode::UnaryOp {
                base, op, operand, ..
            } => Arc::new(RholangNode::UnaryOp {
                base: base.clone(),
                op: op.clone(),
                operand: operand.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Method {
                base,
                receiver,
                name,
                args,
                ..
            } => Arc::new(RholangNode::Method {
                base: base.clone(),
                receiver: receiver.clone(),
                name: name.clone(),
                args: args.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Eval { base, name, .. } => Arc::new(RholangNode::Eval {
                base: base.clone(),
                name: name.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Quote { base, quotable, .. } => Arc::new(RholangNode::Quote {
                base: base.clone(),
                quotable: quotable.clone(),
                metadata: new_metadata,
            }),
            RholangNode::VarRef {
                base, kind, var, ..
            } => Arc::new(RholangNode::VarRef {
                base: base.clone(),
                kind: kind.clone(),
                var: var.clone(),
                metadata: new_metadata,
            }),
            RholangNode::BoolLiteral { base, value, .. } => Arc::new(RholangNode::BoolLiteral {
                base: base.clone(),
                value: *value,
                metadata: new_metadata,
            }),
            RholangNode::LongLiteral { base, value, .. } => Arc::new(RholangNode::LongLiteral {
                base: base.clone(),
                value: *value,
                metadata: new_metadata,
            }),
            RholangNode::StringLiteral { base, value, .. } => Arc::new(RholangNode::StringLiteral {
                base: base.clone(),
                value: value.clone(),
                metadata: new_metadata,
            }),
            RholangNode::UriLiteral { base, value, .. } => Arc::new(RholangNode::UriLiteral {
                base: base.clone(),
                value: value.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Nil { base, .. } => Arc::new(RholangNode::Nil {
                base: base.clone(),
                metadata: new_metadata,
            }),
            RholangNode::List {
                base,
                elements,
                remainder,
                ..
            } => Arc::new(RholangNode::List {
                base: base.clone(),
                elements: elements.clone(),
                remainder: remainder.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Set {
                base,
                elements,
                remainder,
                ..
            } => Arc::new(RholangNode::Set {
                base: base.clone(),
                elements: elements.clone(),
                remainder: remainder.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Map {
                base,
                pairs,
                remainder,
                ..
            } => Arc::new(RholangNode::Map {
                base: base.clone(),
                pairs: pairs.clone(),
                remainder: remainder.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Tuple { base, elements, .. } => Arc::new(RholangNode::Tuple {
                base: base.clone(),
                elements: elements.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Var { base, name, .. } => Arc::new(RholangNode::Var {
                base: base.clone(),
                name: name.clone(),
                metadata: new_metadata,
            }),
            RholangNode::NameDecl {
                base, var, uri, ..
            } => Arc::new(RholangNode::NameDecl {
                base: base.clone(),
                var: var.clone(),
                uri: uri.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Decl {
                base,
                names,
                names_remainder,
                procs,
                ..
            } => Arc::new(RholangNode::Decl {
                base: base.clone(),
                names: names.clone(),
                names_remainder: names_remainder.clone(),
                procs: procs.clone(),
                metadata: new_metadata,
            }),
            RholangNode::LinearBind {
                base,
                names,
                remainder,
                source,
                ..
            } => Arc::new(RholangNode::LinearBind {
                base: base.clone(),
                names: names.clone(),
                remainder: remainder.clone(),
                source: source.clone(),
                metadata: new_metadata,
            }),
            RholangNode::RepeatedBind {
                base,
                names,
                remainder,
                source,
                ..
            } => Arc::new(RholangNode::RepeatedBind {
                base: base.clone(),
                names: names.clone(),
                remainder: remainder.clone(),
                source: source.clone(),
                metadata: new_metadata,
            }),
            RholangNode::PeekBind {
                base,
                names,
                remainder,
                source,
                ..
            } => Arc::new(RholangNode::PeekBind {
                base: base.clone(),
                names: names.clone(),
                remainder: remainder.clone(),
                source: source.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Comment { base, kind, .. } => Arc::new(RholangNode::Comment {
                base: base.clone(),
                kind: kind.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Wildcard { base, .. } => Arc::new(RholangNode::Wildcard {
                base: base.clone(),
                metadata: new_metadata,
            }),
            RholangNode::SimpleType { base, value, .. } => Arc::new(RholangNode::SimpleType {
                base: base.clone(),
                value: value.clone(),
                metadata: new_metadata,
            }),
            RholangNode::ReceiveSendSource { base, name, .. } => Arc::new(RholangNode::ReceiveSendSource {
                base: base.clone(),
                name: name.clone(),
                metadata: new_metadata,
            }),
            RholangNode::SendReceiveSource {
                base, name, inputs, ..
            } => Arc::new(RholangNode::SendReceiveSource {
                base: base.clone(),
                name: name.clone(),
                inputs: inputs.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Error {
                base, children, ..
            } => Arc::new(RholangNode::Error {
                base: base.clone(),
                children: children.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Disjunction {
                base, left, right, ..
            } => Arc::new(RholangNode::Disjunction {
                base: base.clone(),
                left: left.clone(),
                right: right.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Conjunction {
                base, left, right, ..
            } => Arc::new(RholangNode::Conjunction {
                base: base.clone(),
                left: left.clone(),
                right: right.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Negation { base, operand, .. } => Arc::new(RholangNode::Negation {
                base: base.clone(),
                operand: operand.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Unit { base, .. } => Arc::new(RholangNode::Unit {
                base: base.clone(),
                metadata: new_metadata,
            }),
        }
    }

    /// Returns the textual representation of the node by slicing the Rope.
    /// The slice is based on the node's absolute start and end byte offsets in the source.
    pub fn text<'a>(&self, rope: &'a Rope, root: &Arc<RholangNode>) -> RopeSlice<'a> {
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
            RholangNode::Par { base, .. } => base,
            RholangNode::SendSync { base, .. } => base,
            RholangNode::Send { base, .. } => base,
            RholangNode::New { base, .. } => base,
            RholangNode::IfElse { base, .. } => base,
            RholangNode::Let { base, .. } => base,
            RholangNode::Bundle { base, .. } => base,
            RholangNode::Match { base, .. } => base,
            RholangNode::Choice { base, .. } => base,
            RholangNode::Contract { base, .. } => base,
            RholangNode::Input { base, .. } => base,
            RholangNode::Block { base, .. } => base,
            RholangNode::Parenthesized { base, .. } => base,
            RholangNode::BinOp { base, .. } => base,
            RholangNode::UnaryOp { base, .. } => base,
            RholangNode::Method { base, .. } => base,
            RholangNode::Eval { base, .. } => base,
            RholangNode::Quote { base, .. } => base,
            RholangNode::VarRef { base, .. } => base,
            RholangNode::BoolLiteral { base, .. } => base,
            RholangNode::LongLiteral { base, .. } => base,
            RholangNode::StringLiteral { base, .. } => base,
            RholangNode::UriLiteral { base, .. } => base,
            RholangNode::Nil { base, .. } => base,
            RholangNode::List { base, .. } => base,
            RholangNode::Set { base, .. } => base,
            RholangNode::Map { base, .. } => base,
            RholangNode::Tuple { base, .. } => base,
            RholangNode::Var { base, .. } => base,
            RholangNode::NameDecl { base, .. } => base,
            RholangNode::Decl { base, .. } => base,
            RholangNode::LinearBind { base, .. } => base,
            RholangNode::RepeatedBind { base, .. } => base,
            RholangNode::PeekBind { base, .. } => base,
            RholangNode::Comment { base, .. } => base,
            RholangNode::Wildcard { base, .. } => base,
            RholangNode::SimpleType { base, .. } => base,
            RholangNode::ReceiveSendSource { base, .. } => base,
            RholangNode::SendReceiveSource { base, .. } => base,
            RholangNode::Error { base, .. } => base,
            RholangNode::Disjunction { base, .. } => base,
            RholangNode::Conjunction { base, .. } => base,
            RholangNode::Negation { base, .. } => base,
            RholangNode::Unit { base, .. } => base,
        }
    }

    /// Returns an optional reference to the node’s metadata.
    pub fn metadata(&self) -> Option<&Arc<Metadata>> {
        match self {
            RholangNode::Par { metadata, .. } => metadata.as_ref(),
            RholangNode::SendSync { metadata, .. } => metadata.as_ref(),
            RholangNode::Send { metadata, .. } => metadata.as_ref(),
            RholangNode::New { metadata, .. } => metadata.as_ref(),
            RholangNode::IfElse { metadata, .. } => metadata.as_ref(),
            RholangNode::Let { metadata, .. } => metadata.as_ref(),
            RholangNode::Bundle { metadata, .. } => metadata.as_ref(),
            RholangNode::Match { metadata, .. } => metadata.as_ref(),
            RholangNode::Choice { metadata, .. } => metadata.as_ref(),
            RholangNode::Contract { metadata, .. } => metadata.as_ref(),
            RholangNode::Input { metadata, .. } => metadata.as_ref(),
            RholangNode::Block { metadata, .. } => metadata.as_ref(),
            RholangNode::Parenthesized { metadata, .. } => metadata.as_ref(),
            RholangNode::BinOp { metadata, .. } => metadata.as_ref(),
            RholangNode::UnaryOp { metadata, .. } => metadata.as_ref(),
            RholangNode::Method { metadata, .. } => metadata.as_ref(),
            RholangNode::Eval { metadata, .. } => metadata.as_ref(),
            RholangNode::Quote { metadata, .. } => metadata.as_ref(),
            RholangNode::VarRef { metadata, .. } => metadata.as_ref(),
            RholangNode::BoolLiteral { metadata, .. } => metadata.as_ref(),
            RholangNode::LongLiteral { metadata, .. } => metadata.as_ref(),
            RholangNode::StringLiteral { metadata, .. } => metadata.as_ref(),
            RholangNode::UriLiteral { metadata, .. } => metadata.as_ref(),
            RholangNode::Nil { metadata, .. } => metadata.as_ref(),
            RholangNode::List { metadata, .. } => metadata.as_ref(),
            RholangNode::Set { metadata, .. } => metadata.as_ref(),
            RholangNode::Map { metadata, .. } => metadata.as_ref(),
            RholangNode::Tuple { metadata, .. } => metadata.as_ref(),
            RholangNode::Var { metadata, .. } => metadata.as_ref(),
            RholangNode::NameDecl { metadata, .. } => metadata.as_ref(),
            RholangNode::Decl { metadata, .. } => metadata.as_ref(),
            RholangNode::LinearBind { metadata, .. } => metadata.as_ref(),
            RholangNode::RepeatedBind { metadata, .. } => metadata.as_ref(),
            RholangNode::PeekBind { metadata, .. } => metadata.as_ref(),
            RholangNode::Comment { metadata, .. } => metadata.as_ref(),
            RholangNode::Wildcard { metadata, .. } => metadata.as_ref(),
            RholangNode::SimpleType { metadata, .. } => metadata.as_ref(),
            RholangNode::ReceiveSendSource { metadata, .. } => metadata.as_ref(),
            RholangNode::SendReceiveSource { metadata, .. } => metadata.as_ref(),
            RholangNode::Error { metadata, .. } => metadata.as_ref(),
            RholangNode::Disjunction { metadata, .. } => metadata.as_ref(),
            RholangNode::Conjunction { metadata, .. } => metadata.as_ref(),
            RholangNode::Negation { metadata, .. } => metadata.as_ref(),
            RholangNode::Unit { metadata, .. } => metadata.as_ref(),
        }
    }

    pub fn node_cmp(a: &RholangNode, b: &RholangNode) -> Ordering {
        let tag_a = a.tag();
        let tag_b = b.tag();
        if tag_a != tag_b {
            return tag_a.cmp(&tag_b);
        }
        match (a, b) {
            (RholangNode::Var { name: na, .. }, RholangNode::Var { name: nb, .. }) => na.cmp(nb),
            (RholangNode::BoolLiteral { value: va, .. }, RholangNode::BoolLiteral { value: vb, .. }) => va.cmp(vb),
            (RholangNode::LongLiteral { value: va, .. }, RholangNode::LongLiteral { value: vb, .. }) => va.cmp(vb),
            (RholangNode::StringLiteral { value: va, .. }, RholangNode::StringLiteral { value: vb, .. }) => va.cmp(vb),
            (RholangNode::UriLiteral { value: va, .. }, RholangNode::UriLiteral { value: vb, .. }) => va.cmp(vb),
            (RholangNode::SimpleType { value: va, .. }, RholangNode::SimpleType { value: vb, .. }) => va.cmp(vb),
            (RholangNode::Nil { .. }, RholangNode::Nil { .. }) => Ordering::Equal,
            (RholangNode::Unit { .. }, RholangNode::Unit { .. }) => Ordering::Equal,
            (RholangNode::Quote { quotable: qa, .. }, RholangNode::Quote { quotable: qb, .. }) => {
                RholangNode::node_cmp(&*qa, &*qb)
            }
            (RholangNode::Eval { name: na, .. }, RholangNode::Eval { name: nb, .. }) => RholangNode::node_cmp(&*na, &*nb),
            (
                RholangNode::VarRef {
                    kind: ka,
                    var: va,
                    ..
                },
                RholangNode::VarRef {
                    kind: kb,
                    var: vb,
                    ..
                },
            ) => ka.cmp(kb).then_with(|| RholangNode::node_cmp(&*va, &*vb)),
            (RholangNode::Disjunction { left: p_l, right: p_r, .. }, RholangNode::Disjunction { left: c_l, right: c_r, .. }) => {
                RholangNode::node_cmp(p_l, c_l).then_with(|| RholangNode::node_cmp(p_r, c_r))
            }
            (RholangNode::Conjunction { left: p_l, right: p_r, .. }, RholangNode::Conjunction { left: c_l, right: c_r, .. }) => {
                RholangNode::node_cmp(p_l, c_l).then_with(|| RholangNode::node_cmp(p_r, c_r))
            }
            (RholangNode::Negation { operand: p_o, .. }, RholangNode::Negation { operand: c_o, .. }) => {
                RholangNode::node_cmp(p_o, c_o)
            }
            (RholangNode::Parenthesized { expr: p_e, .. }, RholangNode::Parenthesized { expr: c_e, .. }) => {
                RholangNode::node_cmp(p_e, c_e)
            }
            (
                RholangNode::List {
                    elements: ea,
                    remainder: ra,
                    ..
                },
                RholangNode::List {
                    elements: eb,
                    remainder: rb,
                    ..
                },
            ) => {
                let mut ea_sorted: Vec<&Arc<RholangNode>> = ea.iter().collect();
                ea_sorted.sort_by(|a, b| RholangNode::node_cmp(a, b));
                let mut eb_sorted: Vec<&Arc<RholangNode>> = eb.iter().collect();
                eb_sorted.sort_by(|a, b| RholangNode::node_cmp(a, b));
                ea_sorted.cmp(&eb_sorted).then_with(|| ra.cmp(rb))
            }
            (RholangNode::Tuple { elements: ea, .. }, RholangNode::Tuple { elements: eb, .. }) => ea.cmp(eb),
            (
                RholangNode::Set {
                    elements: ea,
                    remainder: ra,
                    ..
                },
                RholangNode::Set {
                    elements: eb,
                    remainder: rb,
                    ..
                },
            ) => {
                let mut ea_sorted: Vec<&Arc<RholangNode>> = ea.iter().collect();
                ea_sorted.sort_by(|a, b| RholangNode::node_cmp(a, b));
                let mut eb_sorted: Vec<&Arc<RholangNode>> = eb.iter().collect();
                eb_sorted.sort_by(|a, b| RholangNode::node_cmp(a, b));
                ea_sorted.cmp(&eb_sorted).then_with(|| ra.cmp(rb))
            }
            (
                RholangNode::Map {
                    pairs: pa,
                    remainder: ra,
                    ..
                },
                RholangNode::Map {
                    pairs: pb,
                    remainder: rb,
                    ..
                },
            ) => {
                let mut pa_sorted: Vec<(&Arc<RholangNode>, &Arc<RholangNode>)> =
                    pa.iter().map(|(k, v)| (k, v)).collect();
                pa_sorted.sort_by(|(ka, _), (kb, _)| RholangNode::node_cmp(ka, kb));
                let mut pb_sorted: Vec<(&Arc<RholangNode>, &Arc<RholangNode>)> =
                    pb.iter().map(|(k, v)| (k, v)).collect();
                pb_sorted.sort_by(|(ka, _), (kb, _)| RholangNode::node_cmp(ka, kb));
                pa_sorted.cmp(&pb_sorted).then_with(|| ra.cmp(rb))
            }
            _ => Ordering::Equal, // For unmatched or leaf variants without comparable fields
        }
    }

    pub fn tag(&self) -> u32 {
        match self {
            RholangNode::Par { .. } => 0,
            RholangNode::SendSync { .. } => 1,
            RholangNode::Send { .. } => 2,
            RholangNode::New { .. } => 3,
            RholangNode::IfElse { .. } => 4,
            RholangNode::Let { .. } => 5,
            RholangNode::Bundle { .. } => 6,
            RholangNode::Match { .. } => 7,
            RholangNode::Choice { .. } => 8,
            RholangNode::Contract { .. } => 9,
            RholangNode::Input { .. } => 10,
            RholangNode::Block { .. } => 11,
            RholangNode::Parenthesized { .. } => 12,
            RholangNode::BinOp { .. } => 13,
            RholangNode::UnaryOp { .. } => 14,
            RholangNode::Method { .. } => 15,
            RholangNode::Eval { .. } => 16,
            RholangNode::Quote { .. } => 17,
            RholangNode::VarRef { .. } => 18,
            RholangNode::BoolLiteral { .. } => 19,
            RholangNode::LongLiteral { .. } => 20,
            RholangNode::StringLiteral { .. } => 21,
            RholangNode::UriLiteral { .. } => 22,
            RholangNode::Nil { .. } => 23,
            RholangNode::List { .. } => 24,
            RholangNode::Set { .. } => 25,
            RholangNode::Map { .. } => 26,
            RholangNode::Tuple { .. } => 27,
            RholangNode::Var { .. } => 28,
            RholangNode::NameDecl { .. } => 29,
            RholangNode::Decl { .. } => 30,
            RholangNode::LinearBind { .. } => 31,
            RholangNode::RepeatedBind { .. } => 32,
            RholangNode::PeekBind { .. } => 33,
            RholangNode::Comment { .. } => 34,
            RholangNode::Wildcard { .. } => 35,
            RholangNode::SimpleType { .. } => 36,
            RholangNode::ReceiveSendSource { .. } => 37,
            RholangNode::SendReceiveSource { .. } => 38,
            RholangNode::Error { .. } => 39,
            RholangNode::Disjunction { .. } => 40,
            RholangNode::Conjunction { .. } => 41,
            RholangNode::Negation { .. } => 42,
            RholangNode::Unit { .. } => 43,
        }
    }

    /// Constructs a new Par node with the given attributes.
    pub fn new_par(
        left: Arc<RholangNode>,
        right: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Par {
                processes: None,
            base,
            left: Some(left),
            right: Some(right),
            metadata,
        }
    }

    /// Constructs a new SendSync node with the given attributes.
    pub fn new_send_sync(
        channel: Arc<RholangNode>,
        inputs: RholangNodeVector,
        cont: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::SendSync {
            base,
            channel,
            inputs,
            cont,
            metadata,
        }
    }

    /// Constructs a new Send node with the given attributes.
    pub fn new_send(
        channel: Arc<RholangNode>,
        send_type: RholangSendType,
        send_type_delta: RelativePosition,
        inputs: RholangNodeVector,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Send {
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
        decls: RholangNodeVector,
        proc: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::New {
            base,
            decls,
            proc,
            metadata,
        }
    }

    /// Constructs a new IfElse node with the given attributes.
    pub fn new_if_else(
        condition: Arc<RholangNode>,
        consequence: Arc<RholangNode>,
        alternative: Option<Arc<RholangNode>>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::IfElse {
            base,
            condition,
            consequence,
            alternative,
            metadata,
        }
    }

    /// Constructs a new Let node with the given attributes.
    pub fn new_let(
        decls: RholangNodeVector,
        proc: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Let {
            base,
            decls,
            proc,
            metadata,
        }
    }

    /// Constructs a new Bundle node with the given attributes.
    pub fn new_bundle(
        bundle_type: RholangBundleType,
        proc: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Bundle {
            base,
            bundle_type,
            proc,
            metadata,
        }
    }

    /// Constructs a new Match node with the given attributes.
    pub fn new_match(
        expression: Arc<RholangNode>,
        cases: RholangNodePairVector,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Match {
            base,
            expression,
            cases,
            metadata,
        }
    }

    /// Constructs a new Choice node with the given attributes.
    pub fn new_choice(
        branches: RholangBranchVector,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Choice {
            base,
            branches,
            metadata,
        }
    }

    /// Constructs a new Contract node with the given attributes.
    pub fn new_contract(
        name: Arc<RholangNode>,
        formals: RholangNodeVector,
        formals_remainder: Option<Arc<RholangNode>>,
        proc: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Contract {
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
        receipts: RholangReceiptVector,
        proc: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Input {
            base,
            receipts,
            proc,
            metadata,
        }
    }

    /// Constructs a new Block node with the given attributes.
    pub fn new_block(
        proc: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Block {
            base,
            proc,
            metadata,
        }
    }

    /// Constructs a new Parenthesized node with the given attributes.
    pub fn new_parenthesized(
        expr: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Parenthesized {
            base,
            expr,
            metadata,
        }
    }

    /// Constructs a new BinOp node with the given attributes.
    pub fn new_bin_op(
        op: BinOperator,
        left: Arc<RholangNode>,
        right: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::BinOp {
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
        operand: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::UnaryOp {
            base,
            op,
            operand,
            metadata,
        }
    }

    /// Constructs a new Method node with the given attributes.
    pub fn new_method(
        receiver: Arc<RholangNode>,
        name: String,
        args: RholangNodeVector,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Method {
            base,
            receiver,
            name,
            args,
            metadata,
        }
    }

    /// Constructs a new Eval node with the given attributes.
    pub fn new_eval(
        name: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Eval {
            base,
            name,
            metadata,
        }
    }

    /// Constructs a new Quote node with the given attributes.
    pub fn new_quote(
        quotable: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Quote {
            base,
            quotable,
            metadata,
        }
    }

    /// Constructs a new VarRef node with the given attributes.
    pub fn new_var_ref(
        kind: RholangVarRefKind,
        var: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::VarRef {
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
        RholangNode::BoolLiteral {
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
        RholangNode::LongLiteral {
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
        RholangNode::StringLiteral {
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
        RholangNode::UriLiteral {
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
        RholangNode::Nil { base, metadata }
    }

    /// Constructs a new List node with the given attributes.
    pub fn new_list(
        elements: RholangNodeVector,
        remainder: Option<Arc<RholangNode>>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::List {
            base,
            elements,
            remainder,
            metadata,
        }
    }

    /// Constructs a new Set node with the given attributes.
    pub fn new_set(
        elements: RholangNodeVector,
        remainder: Option<Arc<RholangNode>>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Set {
            base,
            elements,
            remainder,
            metadata,
        }
    }

    /// Constructs a new Map node with the given attributes.
    pub fn new_map(
        pairs: RholangNodePairVector,
        remainder: Option<Arc<RholangNode>>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Map {
            base,
            pairs,
            remainder,
            metadata,
        }
    }

    /// Constructs a new Tuple node with the given attributes.
    pub fn new_tuple(
        elements: RholangNodeVector,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Tuple {
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
        RholangNode::Var { base, name, metadata }
    }

    /// Constructs a new NameDecl node with the given attributes.
    pub fn new_name_decl(
        var: Arc<RholangNode>,
        uri: Option<Arc<RholangNode>>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::NameDecl {
            base,
            var,
            uri,
            metadata,
        }
    }

    /// Constructs a new Decl node with the given attributes.
    pub fn new_decl(
        names: RholangNodeVector,
        names_remainder: Option<Arc<RholangNode>>,
        procs: RholangNodeVector,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Decl {
            base,
            names,
            names_remainder,
            procs,
            metadata,
        }
    }

    /// Constructs a new LinearBind node with the given attributes.
    pub fn new_linear_bind(
        names: RholangNodeVector,
        remainder: Option<Arc<RholangNode>>,
        source: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::LinearBind {
            base,
            names,
            remainder,
            source,
            metadata,
        }
    }

    /// Constructs a new RepeatedBind node with the given attributes.
    pub fn new_repeated_bind(
        names: RholangNodeVector,
        remainder: Option<Arc<RholangNode>>,
        source: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::RepeatedBind {
            base,
            names,
            remainder,
            source,
            metadata,
        }
    }

    /// Constructs a new PeekBind node with the given attributes.
    pub fn new_peek_bind(
        names: RholangNodeVector,
        remainder: Option<Arc<RholangNode>>,
        source: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::PeekBind {
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
        RholangNode::Comment {
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
        RholangNode::Wildcard { base, metadata }
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
        RholangNode::SimpleType {
            base,
            value,
            metadata,
        }
    }

    /// Constructs a new ReceiveSendSource node with the given attributes.
    pub fn new_receive_send_source(
        name: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::ReceiveSendSource {
            base,
            name,
            metadata,
        }
    }

    /// Constructs a new SendReceiveSource node with the given attributes.
    pub fn new_send_receive_source(
        name: Arc<RholangNode>,
        inputs: RholangNodeVector,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::SendReceiveSource {
            base,
            name,
            inputs,
            metadata,
        }
    }

    /// Constructs a new Error node with the given attributes.
    pub fn new_error(
        children: RholangNodeVector,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Error {
            base,
            children,
            metadata,
        }
    }

    /// Constructs a new Disjunction node with the given attributes.
    pub fn new_disjunction(
        left: Arc<RholangNode>,
        right: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Disjunction {
            base,
            left,
            right,
            metadata,
        }
    }

    /// Constructs a new Conjunction node with the given attributes.
    pub fn new_conjunction(
        left: Arc<RholangNode>,
        right: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Conjunction {
            base,
            left,
            right,
            metadata,
        }
    }

    /// Constructs a new Negation node with the given attributes.
    pub fn new_negation(
        operand: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Negation {
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
        RholangNode::Unit { base, metadata }
    }
}

impl PartialEq for RholangNode {
    fn eq(&self, other: &Self) -> bool {
        RholangNode::node_cmp(self, other) == Ordering::Equal
    }
}

impl Eq for RholangNode {}

impl PartialOrd for RholangNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(RholangNode::node_cmp(self, other))
    }
}

impl Ord for RholangNode {
    fn cmp(&self, other: &Self) -> Ordering {
        RholangNode::node_cmp(self, other)
    }
}

// Implementation of SemanticNode trait for language-agnostic IR operations
impl super::semantic_node::SemanticNode for RholangNode {
    fn base(&self) -> &NodeBase {
        match self {
            RholangNode::Par { base, .. } => base,
            RholangNode::SendSync { base, .. } => base,
            RholangNode::Send { base, .. } => base,
            RholangNode::New { base, .. } => base,
            RholangNode::IfElse { base, .. } => base,
            RholangNode::Let { base, .. } => base,
            RholangNode::Bundle { base, .. } => base,
            RholangNode::Match { base, .. } => base,
            RholangNode::Choice { base, .. } => base,
            RholangNode::Contract { base, .. } => base,
            RholangNode::Input { base, .. } => base,
            RholangNode::Block { base, .. } => base,
            RholangNode::Parenthesized { base, .. } => base,
            RholangNode::BinOp { base, .. } => base,
            RholangNode::UnaryOp { base, .. } => base,
            RholangNode::Method { base, .. } => base,
            RholangNode::Eval { base, .. } => base,
            RholangNode::Quote { base, .. } => base,
            RholangNode::VarRef { base, .. } => base,
            RholangNode::BoolLiteral { base, .. } => base,
            RholangNode::LongLiteral { base, .. } => base,
            RholangNode::StringLiteral { base, .. } => base,
            RholangNode::UriLiteral { base, .. } => base,
            RholangNode::Nil { base, .. } => base,
            RholangNode::List { base, .. } => base,
            RholangNode::Set { base, .. } => base,
            RholangNode::Map { base, .. } => base,
            RholangNode::Tuple { base, .. } => base,
            RholangNode::Var { base, .. } => base,
            RholangNode::NameDecl { base, .. } => base,
            RholangNode::Decl { base, .. } => base,
            RholangNode::LinearBind { base, .. } => base,
            RholangNode::RepeatedBind { base, .. } => base,
            RholangNode::PeekBind { base, .. } => base,
            RholangNode::Comment { base, .. } => base,
            RholangNode::Wildcard { base, .. } => base,
            RholangNode::SimpleType { base, .. } => base,
            RholangNode::ReceiveSendSource { base, .. } => base,
            RholangNode::SendReceiveSource { base, .. } => base,
            RholangNode::Error { base, .. } => base,
            RholangNode::Disjunction { base, .. } => base,
            RholangNode::Conjunction { base, .. } => base,
            RholangNode::Negation { base, .. } => base,
            RholangNode::Unit { base, .. } => base,
        }
    }

    fn metadata(&self) -> Option<&Metadata> {
        match self {
            RholangNode::Par { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::SendSync { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Send { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::New { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::IfElse { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Let { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Bundle { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Match { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Choice { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Contract { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Input { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Block { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Parenthesized { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::BinOp { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::UnaryOp { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Method { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Eval { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Quote { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::VarRef { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::BoolLiteral { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::LongLiteral { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::StringLiteral { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::UriLiteral { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Nil { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::List { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Set { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Map { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Tuple { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Var { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::NameDecl { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Decl { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::LinearBind { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::RepeatedBind { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::PeekBind { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Comment { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Wildcard { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::SimpleType { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::ReceiveSendSource { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::SendReceiveSource { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Error { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Disjunction { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Conjunction { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Negation { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Unit { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
        }
    }

    fn metadata_mut(&mut self) -> Option<&mut Metadata> {
        // Metadata is currently Option<Arc<Metadata>> which is immutable
        // This would require refactoring to support mutable access
        // For now, return None to indicate unsupported
        None
    }

    fn semantic_category(&self) -> super::semantic_node::SemanticCategory {
        use super::semantic_node::SemanticCategory;
        match self {
            // Rholang-specific process calculus constructs
            RholangNode::Par { .. } => SemanticCategory::LanguageSpecific,
            RholangNode::SendSync { .. } | RholangNode::Send { .. } => SemanticCategory::LanguageSpecific,
            RholangNode::New { .. } => SemanticCategory::Binding,
            RholangNode::IfElse { .. } | RholangNode::Choice { .. } => SemanticCategory::Conditional,
            RholangNode::Let { .. } => SemanticCategory::Binding,
            RholangNode::Bundle { .. } => SemanticCategory::LanguageSpecific,
            RholangNode::Match { .. } => SemanticCategory::Match,
            RholangNode::Contract { .. } => SemanticCategory::Binding,
            RholangNode::Input { .. } => SemanticCategory::LanguageSpecific,
            RholangNode::Block { .. } | RholangNode::Parenthesized { .. } => SemanticCategory::Block,
            RholangNode::BinOp { .. } | RholangNode::UnaryOp { .. } | RholangNode::Method { .. } => SemanticCategory::Invocation,
            RholangNode::Eval { .. } => SemanticCategory::LanguageSpecific,
            RholangNode::Quote { .. } => SemanticCategory::LanguageSpecific,
            RholangNode::VarRef { .. } | RholangNode::Var { .. } => SemanticCategory::Variable,
            RholangNode::BoolLiteral { .. } | RholangNode::LongLiteral { .. }
            | RholangNode::StringLiteral { .. } | RholangNode::UriLiteral { .. } => SemanticCategory::Literal,
            RholangNode::Nil { .. } | RholangNode::Wildcard { .. } | RholangNode::Unit { .. } => SemanticCategory::Literal,
            RholangNode::List { .. } | RholangNode::Set { .. } | RholangNode::Map { .. } | RholangNode::Tuple { .. } => SemanticCategory::Collection,
            RholangNode::NameDecl { .. } | RholangNode::Decl { .. } => SemanticCategory::Binding,
            RholangNode::LinearBind { .. } | RholangNode::RepeatedBind { .. } | RholangNode::PeekBind { .. } => SemanticCategory::Binding,
            RholangNode::Comment { .. } => SemanticCategory::Unknown,
            RholangNode::SimpleType { .. } => SemanticCategory::Literal,
            RholangNode::ReceiveSendSource { .. } | RholangNode::SendReceiveSource { .. } => SemanticCategory::LanguageSpecific,
            RholangNode::Error { .. } => SemanticCategory::Unknown,
            RholangNode::Disjunction { .. } | RholangNode::Conjunction { .. } | RholangNode::Negation { .. } => SemanticCategory::Match,
        }
    }

    fn type_name(&self) -> &'static str {
        match self {
            RholangNode::Par { .. } => "Rholang::Par",
            RholangNode::SendSync { .. } => "Rholang::SendSync",
            RholangNode::Send { .. } => "Rholang::Send",
            RholangNode::New { .. } => "Rholang::New",
            RholangNode::IfElse { .. } => "Rholang::IfElse",
            RholangNode::Let { .. } => "Rholang::Let",
            RholangNode::Bundle { .. } => "Rholang::Bundle",
            RholangNode::Match { .. } => "Rholang::Match",
            RholangNode::Choice { .. } => "Rholang::Choice",
            RholangNode::Contract { .. } => "Rholang::Contract",
            RholangNode::Input { .. } => "Rholang::Input",
            RholangNode::Block { .. } => "Rholang::Block",
            RholangNode::Parenthesized { .. } => "Rholang::Parenthesized",
            RholangNode::BinOp { .. } => "Rholang::BinOp",
            RholangNode::UnaryOp { .. } => "Rholang::UnaryOp",
            RholangNode::Method { .. } => "Rholang::Method",
            RholangNode::Eval { .. } => "Rholang::Eval",
            RholangNode::Quote { .. } => "Rholang::Quote",
            RholangNode::VarRef { .. } => "Rholang::VarRef",
            RholangNode::Var { .. } => "Rholang::Var",
            RholangNode::BoolLiteral { .. } => "Rholang::BoolLiteral",
            RholangNode::LongLiteral { .. } => "Rholang::LongLiteral",
            RholangNode::StringLiteral { .. } => "Rholang::StringLiteral",
            RholangNode::UriLiteral { .. } => "Rholang::UriLiteral",
            RholangNode::Nil { .. } => "Rholang::Nil",
            RholangNode::Wildcard { .. } => "Rholang::Wildcard",
            RholangNode::Unit { .. } => "Rholang::Unit",
            RholangNode::List { .. } => "Rholang::List",
            RholangNode::Set { .. } => "Rholang::Set",
            RholangNode::Map { .. } => "Rholang::Map",
            RholangNode::Tuple { .. } => "Rholang::Tuple",
            RholangNode::NameDecl { .. } => "Rholang::NameDecl",
            RholangNode::Decl { .. } => "Rholang::Decl",
            RholangNode::LinearBind { .. } => "Rholang::LinearBind",
            RholangNode::RepeatedBind { .. } => "Rholang::RepeatedBind",
            RholangNode::PeekBind { .. } => "Rholang::PeekBind",
            RholangNode::Comment { .. } => "Rholang::Comment",
            RholangNode::SimpleType { .. } => "Rholang::SimpleType",
            RholangNode::ReceiveSendSource { .. } => "Rholang::ReceiveSendSource",
            RholangNode::SendReceiveSource { .. } => "Rholang::SendReceiveSource",
            RholangNode::Error { .. } => "Rholang::Error",
            RholangNode::Disjunction { .. } => "Rholang::Disjunction",
            RholangNode::Conjunction { .. } => "Rholang::Conjunction",
            RholangNode::Negation { .. } => "Rholang::Negation",
        }
    }

    fn children_count(&self) -> usize {
        match self {
            // N-ary nodes (variable children)
            RholangNode::Par { processes: Some(procs), .. } => procs.len(),
            // Binary nodes (2 children)
            RholangNode::Par { left: Some(left), right: Some(right), .. } => {
                let _ = (left, right);
                2
            }
            RholangNode::BinOp { left, right, .. } => {
                let _ = (left, right);
                2
            }
            RholangNode::Disjunction { left, right, .. } => {
                let _ = (left, right);
                2
            }
            RholangNode::Conjunction { left, right, .. } => {
                let _ = (left, right);
                2
            }

            // Unary nodes (1 child)
            RholangNode::UnaryOp { operand, .. } => {
                let _ = operand;
                1
            }
            RholangNode::Bundle { proc, .. } => {
                let _ = proc;
                1
            }
            RholangNode::Eval { name, .. } => {
                let _ = name;
                1
            }
            RholangNode::Quote { quotable, .. } => {
                let _ = quotable;
                1
            }
            RholangNode::Block { proc, .. } => {
                let _ = proc;
                1
            }
            RholangNode::Parenthesized { expr, .. } => {
                let _ = expr;
                1
            }
            RholangNode::Negation { operand, .. } => {
                let _ = operand;
                1
            }

            // Nodes with vector children
            RholangNode::SendSync { channel, inputs, cont, .. } => {
                let _ = (channel, cont);
                1 + inputs.len() + 1 // channel + inputs + cont
            }
            RholangNode::Send { channel, inputs, .. } => {
                let _ = channel;
                1 + inputs.len() // channel + inputs
            }
            RholangNode::New { decls, proc, .. } => {
                let _ = (decls, proc);
                decls.len() + 1 // decls + proc
            }
            RholangNode::Let { decls, proc, .. } => {
                let _ = (decls, proc);
                decls.len() + 1 // decls + proc
            }
            RholangNode::Match { expression, cases, .. } => {
                let _ = expression;
                1 + cases.len() * 2 // target + (pattern, body) for each case
            }
            RholangNode::Choice { branches, .. } => {
                branches.len() * 2 // (patterns, body) for each branch
            }
            RholangNode::Input { receipts, proc, .. } => {
                let _ = proc;
                receipts.len() + 1 // receipts + proc
            }
            RholangNode::Contract { name, formals, proc, .. } => {
                let _ = (name, proc);
                1 + formals.len() + 1 // name + formals + proc
            }
            RholangNode::Method { receiver, args, .. } => {
                let _ = receiver;
                1 + args.len() // receiver + args
            }

            // Conditional nodes
            RholangNode::IfElse { condition, consequence, alternative, .. } => {
                let _ = (condition, consequence);
                if alternative.is_some() { 3 } else { 2 }
            }

            // Collection nodes
            RholangNode::List { elements, remainder, .. } => {
                if remainder.is_some() {
                    elements.len() + 1
                } else {
                    elements.len()
                }
            }
            RholangNode::Set { elements, .. } => elements.len(),
            RholangNode::Map { pairs, .. } => pairs.len() * 2, // key + value for each pair
            RholangNode::Tuple { elements, .. } => elements.len(),

            // Leaf nodes and nodes we'll skip for now
            _ => 0,
        }
    }

    fn child_at(&self, index: usize) -> Option<&dyn super::semantic_node::SemanticNode> {
        match self {
            // N-ary nodes
            RholangNode::Par { processes: Some(procs), .. } => {
                procs.get(index).map(|p| &**p as &dyn super::semantic_node::SemanticNode)
            }
            // Binary nodes
            RholangNode::Par { left: Some(left), right: Some(right), .. } => match index {
                0 => Some(&**left),
                1 => Some(&**right),
                _ => None,
            },
            RholangNode::BinOp { left, right, .. } => match index {
                0 => Some(&**left),
                1 => Some(&**right),
                _ => None,
            },
            RholangNode::Disjunction { left, right, .. } => match index {
                0 => Some(&**left),
                1 => Some(&**right),
                _ => None,
            },
            RholangNode::Conjunction { left, right, .. } => match index {
                0 => Some(&**left),
                1 => Some(&**right),
                _ => None,
            },

            // Unary nodes
            RholangNode::UnaryOp { operand, .. } if index == 0 => Some(&**operand),
            RholangNode::Bundle { proc, .. } if index == 0 => Some(&**proc),
            RholangNode::Eval { name, .. } if index == 0 => Some(&**name),
            RholangNode::Quote { quotable, .. } if index == 0 => Some(&**quotable),
            RholangNode::Block { proc, .. } if index == 0 => Some(&**proc),
            RholangNode::Parenthesized { expr, .. } if index == 0 => Some(&**expr),
            RholangNode::Negation { operand, .. } if index == 0 => Some(&**operand),

            // Nodes with vector children
            RholangNode::SendSync { channel, inputs, cont, .. } => {
                if index == 0 {
                    Some(&**channel)
                } else if index <= inputs.len() {
                    Some(&**inputs.get(index - 1)?)
                } else if index == inputs.len() + 1 {
                    Some(&**cont)
                } else {
                    None
                }
            }
            RholangNode::Send { channel, inputs, .. } => {
                if index == 0 {
                    Some(&**channel)
                } else if index <= inputs.len() {
                    Some(&**inputs.get(index - 1)?)
                } else {
                    None
                }
            }
            RholangNode::New { decls, proc, .. } => {
                if index < decls.len() {
                    Some(&**decls.get(index)?)
                } else if index == decls.len() {
                    Some(&**proc)
                } else {
                    None
                }
            }
            RholangNode::Let { decls, proc, .. } => {
                if index < decls.len() {
                    decls.get(index).map(|d| &**d as &dyn super::semantic_node::SemanticNode)
                } else if index == decls.len() {
                    Some(&**proc)
                } else {
                    None
                }
            }
            RholangNode::Match { expression, cases, .. } => {
                if index == 0 {
                    Some(&**expression)
                } else {
                    let case_index = (index - 1) / 2;
                    if case_index < cases.len() {
                        let (pattern, body) = cases.get(case_index)?;
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
            RholangNode::Choice { branches, .. } => {
                let branch_index = index / 2;
                if branch_index < branches.len() {
                    let (patterns, body) = branches.get(branch_index)?;
                    if index % 2 == 0 {
                        // Return first pattern as representative
                        patterns.get(0).map(|p| &**p as &dyn super::semantic_node::SemanticNode)
                    } else {
                        Some(&**body)
                    }
                } else {
                    None
                }
            }
            RholangNode::Input { receipts, proc, .. } => {
                if index < receipts.len() {
                    // Access first receipt pattern as representative
                    receipts.get(index).and_then(|r| r.get(0).map(|p| &**p as &dyn super::semantic_node::SemanticNode))
                } else if index == receipts.len() {
                    Some(&**proc)
                } else {
                    None
                }
            }
            RholangNode::Contract { name, formals, proc, .. } => {
                if index == 0 {
                    Some(&**name)
                } else if index <= formals.len() {
                    Some(&**formals.get(index - 1)?)
                } else if index == formals.len() + 1 {
                    Some(&**proc)
                } else {
                    None
                }
            }
            RholangNode::Method { receiver, args, .. } => {
                if index == 0 {
                    Some(&**receiver)
                } else if index <= args.len() {
                    Some(&**args.get(index - 1)?)
                } else {
                    None
                }
            }

            // Conditional nodes
            RholangNode::IfElse { condition, consequence, alternative, .. } => match index {
                0 => Some(&**condition),
                1 => Some(&**consequence),
                2 => alternative.as_ref().map(|alt| &**alt as &dyn super::semantic_node::SemanticNode),
                _ => None,
            },

            // Collection nodes
            RholangNode::List { elements, remainder, .. } => {
                if index < elements.len() {
                    Some(&**elements.get(index)?)
                } else if index == elements.len() && remainder.is_some() {
                    remainder.as_ref().map(|r| &**r as &dyn super::semantic_node::SemanticNode)
                } else {
                    None
                }
            }
            RholangNode::Set { elements, .. } => {
                elements.get(index).map(|e| &**e as &dyn super::semantic_node::SemanticNode)
            }
            RholangNode::Map { pairs, .. } => {
                let pair_index = index / 2;
                if pair_index < pairs.len() {
                    let (key, value) = pairs.get(pair_index)?;
                    if index % 2 == 0 {
                        Some(&**key)
                    } else {
                        Some(&**value)
                    }
                } else {
                    None
                }
            }
            RholangNode::Tuple { elements, .. } => {
                elements.get(index).map(|e| &**e as &dyn super::semantic_node::SemanticNode)
            }

            // Leaf nodes and unhandled return None
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
    use quickcheck::{QuickCheck, TestResult};
    use test_utils::ir::generator::RholangProc;
    use crate::tree_sitter::{parse_code, parse_to_ir};

    #[test]
    fn test_position_computation() {
        let _ = crate::logging::init_logger(false, Some("warn"), false);
        let code = "ch!(\"msg\")\nNil";
        let tree = parse_code(code);
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree, &rope);
        let root = Arc::new(ir.clone());
        if let RholangNode::Par { left: Some(left), right: Some(right), .. } = &*ir {
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
        let _ = crate::logging::init_logger(false, Some("warn"), false);
        let code = r#"new x in { x!("msg") }"#;
        let tree = parse_code(code);
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree, &rope);
        let root = Arc::new(ir.clone());
        if let RholangNode::New { decls, proc, .. } = &*ir {
            let decl_start = decls[0].absolute_start(&root);
            assert_eq!(decl_start.row, 0);
            assert_eq!(decl_start.column, 4);
            assert_eq!(decl_start.byte, 4);
            if let RholangNode::Block { proc: inner, .. } = &**proc {
                if let RholangNode::Send { channel, .. } = &**inner {
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
        let _ = crate::logging::init_logger(false, Some("warn"), false);
        let code = "ch!(\n\"msg\"\n)";
        let tree = parse_code(code);
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree, &rope);
        let root = Arc::new(ir.clone());
        if let RholangNode::Send { inputs, .. } = &*ir {
            let input_start = inputs[0].absolute_start(&root);
            assert_eq!(input_start.row, 1);
            assert_eq!(input_start.column, 0);
        } else {
            panic!("Expected Send node");
        }
    }

    #[test]
    fn test_match_positioning() {
        let _ = crate::logging::init_logger(false, Some("warn"), false);
        let code = r#"match "target" { "pat" => Nil }"#;
        let tree = parse_code(code);
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree, &rope);
        let root = Arc::new(ir.clone());
        if let RholangNode::Match { expression, cases, .. } = &*ir {
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
        let metadata = Arc::new(data);
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
        let node = RholangNode::Nil {
            base,
            metadata: Some(metadata.clone()),
        };
        assert_eq!(
            node.metadata()
                .unwrap()
                .get("version")
                .unwrap()
                .downcast_ref::<usize>(),
            Some(&1)
        );
        assert_eq!(
            node.metadata()
                .unwrap()
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
        if let RholangNode::Par { left: Some(left), .. } = &*ir {
            if let RholangNode::Error { children, .. } = &**left {
                assert!(!children.is_empty(), "Error node should have children");
            }
        }
    }

    #[test]
    fn test_match_pat_simple() {
        let wild = Arc::new(RholangNode::new_wildcard(
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
        let var_pat = Arc::new(RholangNode::new_var(
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
        let var_con = Arc::new(RholangNode::new_var(
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
        let string_pat = Arc::new(RholangNode::new_string_literal(
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
        let string_con = Arc::new(RholangNode::new_string_literal(
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
        let string_con_diff = Arc::new(RholangNode::new_string_literal(
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
        let var_pat = Arc::new(RholangNode::new_var(
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
        let con1 = Arc::new(RholangNode::new_long_literal(
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
        let con2 = Arc::new(RholangNode::new_long_literal(
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
        let con_diff = Arc::new(RholangNode::new_long_literal(
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
        let channel = Arc::new(RholangNode::new_var(
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
        let inputs = Vector::new_with_ptr_kind().push_back(Arc::new(RholangNode::new_long_literal(
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
        let contract_name = Arc::new(RholangNode::new_var(
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
        let contract_formals = Vector::new_with_ptr_kind().push_back(Arc::new(RholangNode::new_var(
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
        let contract = Arc::new(RholangNode::new_contract(
            contract_name,
            contract_formals,
            None,
            Arc::new(RholangNode::new_nil(
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
            .push_back(Arc::new(RholangNode::LongLiteral {
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
            .push_back(Arc::new(RholangNode::LongLiteral {
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
        let pat = Arc::new(RholangNode::Set {
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
            .push_back(Arc::new(RholangNode::LongLiteral {
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
            .push_back(Arc::new(RholangNode::LongLiteral {
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
        let concrete = Arc::new(RholangNode::Set {
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
        assert!(crate::ir::rholang_node::match_pat(&pat, &concrete, &mut subst));
    }

    #[test]
    fn test_match_pat_map() {
        let p_pair1 = (
            Arc::new(RholangNode::StringLiteral {
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
            Arc::new(RholangNode::LongLiteral {
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
            Arc::new(RholangNode::StringLiteral {
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
            Arc::new(RholangNode::LongLiteral {
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
        let pat = Arc::new(RholangNode::Map {
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
            Arc::new(RholangNode::StringLiteral {
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
            Arc::new(RholangNode::LongLiteral {
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
            Arc::new(RholangNode::StringLiteral {
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
            Arc::new(RholangNode::LongLiteral {
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
        let concrete = Arc::new(RholangNode::Map {
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
        assert!(crate::ir::rholang_node::match_pat(&pat, &concrete, &mut subst));
    }

    #[test]
    fn test_match_pat_disjunction() {
        let p_left = Arc::new(RholangNode::LongLiteral {
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
        let p_right = Arc::new(RholangNode::LongLiteral {
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
        let pat = Arc::new(RholangNode::Disjunction {
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
        let c_left = Arc::new(RholangNode::LongLiteral {
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
        let c_right = Arc::new(RholangNode::LongLiteral {
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
        let concrete = Arc::new(RholangNode::Disjunction {
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
        assert!(crate::ir::rholang_node::match_pat(&pat, &concrete, &mut subst));
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
        QuickCheck::new().tests(50).max_tests(500).quickcheck(prop as fn(RholangProc, RholangProc) -> TestResult);
    }
}
