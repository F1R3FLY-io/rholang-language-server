use std::sync::Arc;

use rpds::Vector;
use archery::ArcK;



pub use super::super::semantic_node::{Metadata, NodeBase, Position, RelativePosition};

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
