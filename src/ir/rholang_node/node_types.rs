use std::sync::Arc;

use rpds::Vector;
use archery::ArcK;



pub use super::super::semantic_node::{Metadata, NodeBase, Position};

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
///
/// Phase B-3: Added Serialize/Deserialize for persistent cache support.
/// Metadata fields are skipped (contain dyn Any trait objects).
/// Arc-wrapped fields use custom serde helpers from src/serde_helpers.rs.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[allow(dead_code)]
pub enum RholangNode {
    /// Parallel composition of processes.
    /// Supports both binary (left/right) and n-ary (processes) forms for gradual migration.
    Par {
        base: NodeBase,
        // Legacy binary form (deprecated, will be removed after migration)
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_option_arc",
            deserialize_with = "crate::serde_helpers::deserialize_option_arc"
        )]
        left: Option<Arc<RholangNode>>,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_option_arc",
            deserialize_with = "crate::serde_helpers::deserialize_option_arc"
        )]
        right: Option<Arc<RholangNode>>,
        // New n-ary form (preferred)
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_option_rpds_arc_vec",
            deserialize_with = "crate::serde_helpers::deserialize_option_rpds_arc_vec"
        )]
        processes: Option<RholangNodeVector>,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Synchronous send with a continuation process.
    SendSync {
        base: NodeBase,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_arc",
            deserialize_with = "crate::serde_helpers::deserialize_arc"
        )]
        channel: Arc<RholangNode>,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_rpds_arc_vec",
            deserialize_with = "crate::serde_helpers::deserialize_rpds_arc_vec"
        )]
        inputs: RholangNodeVector,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_arc",
            deserialize_with = "crate::serde_helpers::deserialize_arc"
        )]
        cont: Arc<RholangNode>,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Asynchronous send operation on a channel.
    Send {
        base: NodeBase,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_arc",
            deserialize_with = "crate::serde_helpers::deserialize_arc"
        )]
        channel: Arc<RholangNode>,
        send_type: RholangSendType,
        send_type_pos: Position,  // Absolute position of the send type (! or !!)
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_rpds_arc_vec",
            deserialize_with = "crate::serde_helpers::deserialize_rpds_arc_vec"
        )]
        inputs: RholangNodeVector,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Declaration of new names with a scoped process
    New {
        base: NodeBase,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_rpds_arc_vec",
            deserialize_with = "crate::serde_helpers::deserialize_rpds_arc_vec"
        )]
        decls: RholangNodeVector,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_arc",
            deserialize_with = "crate::serde_helpers::deserialize_arc"
        )]
        proc: Arc<RholangNode>,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Conditional branching with optional else clause.
    IfElse {
        base: NodeBase,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_arc",
            deserialize_with = "crate::serde_helpers::deserialize_arc"
        )]
        condition: Arc<RholangNode>,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_arc",
            deserialize_with = "crate::serde_helpers::deserialize_arc"
        )]
        consequence: Arc<RholangNode>,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_option_arc",
            deserialize_with = "crate::serde_helpers::deserialize_option_arc"
        )]
        alternative: Option<Arc<RholangNode>>,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Variable binding with a subsequent process.
    Let {
        base: NodeBase,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_rpds_arc_vec",
            deserialize_with = "crate::serde_helpers::deserialize_rpds_arc_vec"
        )]
        decls: RholangNodeVector,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_arc",
            deserialize_with = "crate::serde_helpers::deserialize_arc"
        )]
        proc: Arc<RholangNode>,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Access-controlled process with a bundle type.
    Bundle {
        base: NodeBase,
        bundle_type: RholangBundleType,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_arc",
            deserialize_with = "crate::serde_helpers::deserialize_arc"
        )]
        proc: Arc<RholangNode>,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Pattern matching construct with cases.
    Match {
        base: NodeBase,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_arc",
            deserialize_with = "crate::serde_helpers::deserialize_arc"
        )]
        expression: Arc<RholangNode>,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_rpds_arc_tuple_vec",
            deserialize_with = "crate::serde_helpers::deserialize_rpds_arc_tuple_vec"
        )]
        cases: RholangNodePairVector,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Non-deterministic choice among branches.
    Choice {
        base: NodeBase,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_rpds_branch_vec",
            deserialize_with = "crate::serde_helpers::deserialize_rpds_branch_vec"
        )]
        branches: RholangBranchVector,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Contract definition with name, parameters, and body.
    Contract {
        base: NodeBase,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_arc",
            deserialize_with = "crate::serde_helpers::deserialize_arc"
        )]
        name: Arc<RholangNode>,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_rpds_arc_vec",
            deserialize_with = "crate::serde_helpers::deserialize_rpds_arc_vec"
        )]
        formals: RholangNodeVector,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_option_arc",
            deserialize_with = "crate::serde_helpers::deserialize_option_arc"
        )]
        formals_remainder: Option<Arc<RholangNode>>,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_arc",
            deserialize_with = "crate::serde_helpers::deserialize_arc"
        )]
        proc: Arc<RholangNode>,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Input binding from channels with a process.
    Input {
        base: NodeBase,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_rpds_nested_arc_vec",
            deserialize_with = "crate::serde_helpers::deserialize_rpds_nested_arc_vec"
        )]
        receipts: RholangReceiptVector,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_arc",
            deserialize_with = "crate::serde_helpers::deserialize_arc"
        )]
        proc: Arc<RholangNode>,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Block of a single process (e.g., { P }).
    Block {
        base: NodeBase,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_arc",
            deserialize_with = "crate::serde_helpers::deserialize_arc"
        )]
        proc: Arc<RholangNode>,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Parenthesized expression (e.g., (P)).
    Parenthesized {
        base: NodeBase,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_arc",
            deserialize_with = "crate::serde_helpers::deserialize_arc"
        )]
        expr: Arc<RholangNode>,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Binary operation (e.g., P + Q).
    BinOp {
        base: NodeBase,
        op: BinOperator,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_arc",
            deserialize_with = "crate::serde_helpers::deserialize_arc"
        )]
        left: Arc<RholangNode>,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_arc",
            deserialize_with = "crate::serde_helpers::deserialize_arc"
        )]
        right: Arc<RholangNode>,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Unary operation (e.g., -P or not P).
    UnaryOp {
        base: NodeBase,
        op: UnaryOperator,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_arc",
            deserialize_with = "crate::serde_helpers::deserialize_arc"
        )]
        operand: Arc<RholangNode>,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Method call on a receiver (e.g., obj.method(args)).
    Method {
        base: NodeBase,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_arc",
            deserialize_with = "crate::serde_helpers::deserialize_arc"
        )]
        receiver: Arc<RholangNode>,
        name: String,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_rpds_arc_vec",
            deserialize_with = "crate::serde_helpers::deserialize_rpds_arc_vec"
        )]
        args: RholangNodeVector,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Evaluation of a name (e.g., *name).
    Eval {
        base: NodeBase,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_arc",
            deserialize_with = "crate::serde_helpers::deserialize_arc"
        )]
        name: Arc<RholangNode>,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Quotation of a process (e.g., @P).
    Quote {
        base: NodeBase,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_arc",
            deserialize_with = "crate::serde_helpers::deserialize_arc"
        )]
        quotable: Arc<RholangNode>,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Variable reference with assignment kind.
    VarRef {
        base: NodeBase,
        kind: RholangVarRefKind,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_arc",
            deserialize_with = "crate::serde_helpers::deserialize_arc"
        )]
        var: Arc<RholangNode>,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Boolean literal (e.g., true or false).
    BoolLiteral {
        base: NodeBase,
        value: bool,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Integer literal (e.g., 42).
    LongLiteral {
        base: NodeBase,
        value: i64,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// String literal (e.g., "hello").
    StringLiteral {
        base: NodeBase,
        value: String,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// URI literal (e.g., `` http://example.com ``).
    UriLiteral {
        base: NodeBase,
        value: String,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Empty process (e.g., Nil).
    Nil {
        base: NodeBase,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// List collection (e.g., [1, 2, 3]).
    List {
        base: NodeBase,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_rpds_arc_vec",
            deserialize_with = "crate::serde_helpers::deserialize_rpds_arc_vec"
        )]
        elements: RholangNodeVector,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_option_arc",
            deserialize_with = "crate::serde_helpers::deserialize_option_arc"
        )]
        remainder: Option<Arc<RholangNode>>,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Set collection (e.g., Set(1, 2, 3)).
    Set {
        base: NodeBase,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_rpds_arc_vec",
            deserialize_with = "crate::serde_helpers::deserialize_rpds_arc_vec"
        )]
        elements: RholangNodeVector,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_option_arc",
            deserialize_with = "crate::serde_helpers::deserialize_option_arc"
        )]
        remainder: Option<Arc<RholangNode>>,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Map collection (e.g., {k: v}).
    Map {
        base: NodeBase,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_rpds_arc_tuple_vec",
            deserialize_with = "crate::serde_helpers::deserialize_rpds_arc_tuple_vec"
        )]
        pairs: RholangNodePairVector,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_option_arc",
            deserialize_with = "crate::serde_helpers::deserialize_option_arc"
        )]
        remainder: Option<Arc<RholangNode>>,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Pathmap collection (e.g., {| proc1, proc2 |}).
    Pathmap {
        base: NodeBase,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_rpds_arc_vec",
            deserialize_with = "crate::serde_helpers::deserialize_rpds_arc_vec"
        )]
        elements: RholangNodeVector,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_option_arc",
            deserialize_with = "crate::serde_helpers::deserialize_option_arc"
        )]
        remainder: Option<Arc<RholangNode>>,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Tuple collection (e.g., (1, 2)).
    Tuple {
        base: NodeBase,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_rpds_arc_vec",
            deserialize_with = "crate::serde_helpers::deserialize_rpds_arc_vec"
        )]
        elements: RholangNodeVector,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Variable identifier (e.g., x).
    Var {
        base: NodeBase,
        name: String,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Name declaration in a new construct (e.g., x or x(uri)).
    NameDecl {
        base: NodeBase,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_arc",
            deserialize_with = "crate::serde_helpers::deserialize_arc"
        )]
        var: Arc<RholangNode>,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_option_arc",
            deserialize_with = "crate::serde_helpers::deserialize_option_arc"
        )]
        uri: Option<Arc<RholangNode>>,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Declaration in a let statement (e.g., x = P).
    Decl {
        base: NodeBase,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_rpds_arc_vec",
            deserialize_with = "crate::serde_helpers::deserialize_rpds_arc_vec"
        )]
        names: RholangNodeVector,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_option_arc",
            deserialize_with = "crate::serde_helpers::deserialize_option_arc"
        )]
        names_remainder: Option<Arc<RholangNode>>,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_rpds_arc_vec",
            deserialize_with = "crate::serde_helpers::deserialize_rpds_arc_vec"
        )]
        procs: RholangNodeVector,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Linear binding in a for (e.g., x <- ch).
    LinearBind {
        base: NodeBase,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_rpds_arc_vec",
            deserialize_with = "crate::serde_helpers::deserialize_rpds_arc_vec"
        )]
        names: RholangNodeVector,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_option_arc",
            deserialize_with = "crate::serde_helpers::deserialize_option_arc"
        )]
        remainder: Option<Arc<RholangNode>>,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_arc",
            deserialize_with = "crate::serde_helpers::deserialize_arc"
        )]
        source: Arc<RholangNode>,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Repeated binding in a for (e.g., x <= ch).
    RepeatedBind {
        base: NodeBase,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_rpds_arc_vec",
            deserialize_with = "crate::serde_helpers::deserialize_rpds_arc_vec"
        )]
        names: RholangNodeVector,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_option_arc",
            deserialize_with = "crate::serde_helpers::deserialize_option_arc"
        )]
        remainder: Option<Arc<RholangNode>>,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_arc",
            deserialize_with = "crate::serde_helpers::deserialize_arc"
        )]
        source: Arc<RholangNode>,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Peek binding in a for (e.g., x <<- ch).
    PeekBind {
        base: NodeBase,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_rpds_arc_vec",
            deserialize_with = "crate::serde_helpers::deserialize_rpds_arc_vec"
        )]
        names: RholangNodeVector,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_option_arc",
            deserialize_with = "crate::serde_helpers::deserialize_option_arc"
        )]
        remainder: Option<Arc<RholangNode>>,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_arc",
            deserialize_with = "crate::serde_helpers::deserialize_arc"
        )]
        source: Arc<RholangNode>,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Comment in the source code (e.g., // text or /* text */).
    Comment {
        base: NodeBase,
        kind: CommentKind,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Wildcard pattern (e.g., _).
    Wildcard {
        base: NodeBase,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Simple type annotation (e.g., Bool).
    SimpleType {
        base: NodeBase,
        value: String,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Receive-send source (e.g., ch?!).
    ReceiveSendSource {
        base: NodeBase,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_arc",
            deserialize_with = "crate::serde_helpers::deserialize_arc"
        )]
        name: Arc<RholangNode>,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Send-receive source (e.g., ch!?(args)).
    SendReceiveSource {
        base: NodeBase,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_arc",
            deserialize_with = "crate::serde_helpers::deserialize_arc"
        )]
        name: Arc<RholangNode>,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_rpds_arc_vec",
            deserialize_with = "crate::serde_helpers::deserialize_rpds_arc_vec"
        )]
        inputs: RholangNodeVector,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Represents a syntax error in the source code with its erroneous subtree.
    Error {
        base: NodeBase,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_rpds_arc_vec",
            deserialize_with = "crate::serde_helpers::deserialize_rpds_arc_vec"
        )]
        children: RholangNodeVector,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Pattern disjunction (e.g., P | Q in patterns).
    Disjunction {
        base: NodeBase,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_arc",
            deserialize_with = "crate::serde_helpers::deserialize_arc"
        )]
        left: Arc<RholangNode>,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_arc",
            deserialize_with = "crate::serde_helpers::deserialize_arc"
        )]
        right: Arc<RholangNode>,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Pattern conjunction (e.g., P & Q in patterns).
    Conjunction {
        base: NodeBase,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_arc",
            deserialize_with = "crate::serde_helpers::deserialize_arc"
        )]
        left: Arc<RholangNode>,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_arc",
            deserialize_with = "crate::serde_helpers::deserialize_arc"
        )]
        right: Arc<RholangNode>,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Pattern negation (e.g., ~P in patterns).
    Negation {
        base: NodeBase,
        #[serde(
            serialize_with = "crate::serde_helpers::serialize_arc",
            deserialize_with = "crate::serde_helpers::deserialize_arc"
        )]
        operand: Arc<RholangNode>,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
    /// Unit value (e.g., ()).
    Unit {
        base: NodeBase,
        #[serde(skip)]
        metadata: Option<Arc<Metadata>>,
    },
}

/// Phase B-3: Added Serialize/Deserialize for persistent cache
#[derive(Clone, PartialEq, Debug, Hash, serde::Serialize, serde::Deserialize)]
pub enum RholangBundleType {
    Read,
    Write,
    Equiv,
    ReadWrite,
}

/// Phase B-3: Added Serialize/Deserialize for persistent cache
#[derive(Clone, PartialEq, Debug, Hash, serde::Serialize, serde::Deserialize)]
pub enum RholangSendType {
    Single,
    Multiple,
}

/// Phase B-3: Added Serialize/Deserialize for persistent cache
#[derive(Clone, PartialEq, Debug, Hash, serde::Serialize, serde::Deserialize)]
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

/// Phase B-3: Added Serialize/Deserialize for persistent cache
#[derive(Clone, PartialEq, Debug, Hash, serde::Serialize, serde::Deserialize)]
pub enum UnaryOperator {
    Not,
    Neg,
    Negation,
}

/// Phase B-3: Added Serialize/Deserialize for persistent cache
#[derive(Clone, PartialEq, Debug, Hash, Eq, Ord, PartialOrd, serde::Serialize, serde::Deserialize)]
pub enum RholangVarRefKind {
    Bind,
    Unforgeable,
}

/// Phase B-3: Added Serialize/Deserialize for persistent cache
#[derive(Clone, PartialEq, Debug, Hash, serde::Serialize, serde::Deserialize)]
pub enum CommentKind {
    Line,
    Block,
}
