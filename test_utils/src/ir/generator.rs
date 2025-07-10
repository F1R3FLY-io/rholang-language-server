//! Module for generating random Rholang code for property-based testing.
//!
//! This module defines the `RholangProc` enum which represents various Rholang constructs.
//! It provides functionality to generate random instances and convert them to Rholang code strings.
//! The generator aims to cover the full structure of the Rholang grammar as defined in grammar.js.
//! It includes support for remainders in name lists and collections where applicable.
//!
//! Generation functions use a depth parameter to limit recursion and prevent excessive tree depth,
//! which helps avoid stack overflows and improves performance in property-based tests.
//!
//! Reserved keywords are avoided in variable names to reduce parse errors during generation.

use quickcheck::{Arbitrary, Gen};
use std::fmt;

/// Represents a binary operator in Rholang expressions.
#[derive(Clone, Debug)]
pub enum BinOp {
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

/// Represents a unary operator in Rholang expressions.
#[derive(Clone, Debug)]
pub enum UnaryOp {
    Not,
    Neg,
    Negation,
}

/// Represents a process variable, either a named variable or a wildcard.
#[derive(Clone, Debug)]
pub enum ProcVar {
    Var(String),
    Wildcard,
}

/// Represents a name in Rholang, which can be a process variable or a quoted process.
#[derive(Clone, Debug)]
pub enum Name {
    ProcVar(ProcVar),
    Quote(Box<RholangProc>),
}

/// Represents a list of names, supporting optional remainder as per grammar.
#[derive(Clone, Debug)]
pub struct NameList {
    pub items: Vec<Name>,
    pub remainder: Option<ProcVar>,
}

/// Represents a source for binding in inputs.
#[derive(Clone, Debug)]
pub enum Source {
    Simple(Box<Name>),
    ReceiveSend(Box<Name>),
    SendReceive { name: Box<Name>, inputs: Vec<RholangProc> },
}

/// Represents a binding in receipts.
#[derive(Clone, Debug)]
pub enum Bind {
    Linear { names: NameList, source: Source },
    Repeated { names: NameList, name: Name },
    Peek { names: NameList, name: Name },
}

/// Represents a declaration in new constructs.
#[derive(Clone, Debug)]
pub enum NewDecl {
    Simple(String),
    WithUri(String, String),
}

/// Represents a declaration in let constructs.
#[derive(Clone, Debug)]
pub struct Decl {
    pub names: NameList,
    pub procs: Vec<RholangProc>,
}

/// Represents the type of bundle.
#[derive(Clone, Debug)]
pub enum BundleType {
    Read,
    Write,
    Equiv,
    ReadWrite,
}

/// Represents the type of send.
#[derive(Clone, Debug)]
pub enum SendType {
    Single,
    Multiple,
}

/// Represents the continuation for synchronous send.
#[derive(Clone, Debug)]
pub enum SyncCont {
    Empty,
    NonEmpty(Box<RholangProc>),
}

/// Represents a branch in choice.
#[derive(Clone, Debug)]
pub struct Branch {
    pub binds: Vec<LinearBind>,
    pub proc: RholangProc,
}

/// Represents a linear bind for branches.
#[derive(Clone, Debug)]
pub struct LinearBind {
    pub names: NameList,
    pub source: Source,
}

/// Represents a receipt as a conjunction of binds.
pub type Receipt = Vec<Bind>;

/// Represents a variable reference kind.
#[derive(Clone, Debug)]
pub enum VarRefKind {
    Bind,
    Unforgeable,
}

/// Enum representing various Rholang processes for property-based testing.
/// Covers the full grammar structure including expressions, processes, bindings, and collections.
#[derive(Clone, Debug)]
pub enum RholangProc {
    /// The nil process.
    Nil,
    /// A variable reference.
    Var(String),
    /// A wildcard pattern.
    Wildcard,
    /// A quoted process.
    Quote(Box<RholangProc>),
    /// Evaluation of a name.
    Eval(Box<Name>),
    /// Variable reference with kind.
    VarRef(VarRefKind, String),
    /// Disjunction of two processes.
    Disjunction(Box<RholangProc>, Box<RholangProc>),
    /// Conjunction of two processes.
    Conjunction(Box<RholangProc>, Box<RholangProc>),
    /// Negation of a process.
    Negation(Box<RholangProc>),
    /// A simple type.
    SimpleType(String),
    /// A block containing a process.
    Block(Box<RholangProc>),
    /// A tuple of processes.
    Tuple(Vec<RholangProc>),
    /// A list of processes with optional remainder.
    List(Vec<RholangProc>, Option<Box<RholangProc>>),
    /// A set of processes with optional remainder.
    Set(Vec<RholangProc>, Option<Box<RholangProc>>),
    /// A map of key-value pairs with optional remainder.
    Map(Vec<(RholangProc, RholangProc)>, Option<Box<RholangProc>>),
    /// Boolean literal.
    BoolLit(bool),
    /// Integer literal.
    IntLit(i64),
    /// String literal.
    StringLit(String),
    /// URI literal.
    UriLit(String),
    /// Binary operation.
    BinOp { op: BinOp, left: Box<RholangProc>, right: Box<RholangProc> },
    /// Unary operation.
    UnaryOp { op: UnaryOp, operand: Box<RholangProc> },
    /// Method call.
    Method { receiver: Box<RholangProc>, name: String, args: Vec<RholangProc> },
    /// Parallel composition.
    Par { left: Box<RholangProc>, right: Box<RholangProc> },
    /// Asynchronous send.
    Send { channel: Box<Name>, send_type: SendType, inputs: Vec<RholangProc> },
    /// Synchronous send.
    SendSync { channel: Box<Name>, inputs: Vec<RholangProc>, cont: SyncCont },
    /// New name declaration.
    New { decls: Vec<NewDecl>, proc: Box<RholangProc> },
    /// If-else conditional.
    IfElse { condition: Box<RholangProc>, consequence: Box<RholangProc>, alternative: Option<Box<RholangProc>> },
    /// Let binding.
    Let { is_conc: bool, decls: Vec<Decl>, proc: Box<RholangProc> },
    /// Bundle with access control.
    Bundle { bundle_type: BundleType, proc: Box<RholangProc> },
    /// Match expression.
    Match { expression: Box<RholangProc>, cases: Vec<(RholangProc, RholangProc)> },
    /// Choice (select) expression.
    Choice { branches: Vec<Branch> },
    /// Contract definition.
    Contract { name: Box<Name>, formals: NameList, proc: Box<RholangProc> },
    /// Input (for) comprehension.
    Input { receipts: Vec<Receipt>, proc: Box<RholangProc> },
    /// Parenthesized expression.
    Parenthesized(Box<RholangProc>),
}

/// Maximum recursion depth for generation to prevent excessive tree depth.
const MAX_DEPTH: usize = 10;

/// List of reserved keywords in Rholang to avoid in variable names.
const RESERVED_KEYWORDS: &[&str] = &[
    "if", "else", "new", "in", "match", "contract", "select", "for", "let",
    "bundle", "bundle+", "bundle-", "bundle0", "true", "false", "Nil",
    "or", "and", "not", "matches", "Set", "Bool", "Int", "String", "Uri", "ByteArray",
];

impl fmt::Display for BinOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BinOp::Or => write!(f, "or"),
            BinOp::And => write!(f, "and"),
            BinOp::Matches => write!(f, "matches"),
            BinOp::Eq => write!(f, "=="),
            BinOp::Neq => write!(f, "!="),
            BinOp::Lt => write!(f, "<"),
            BinOp::Lte => write!(f, "<="),
            BinOp::Gt => write!(f, ">"),
            BinOp::Gte => write!(f, ">="),
            BinOp::Concat => write!(f, "++"),
            BinOp::Diff => write!(f, "--"),
            BinOp::Add => write!(f, "+"),
            BinOp::Sub => write!(f, "-"),
            BinOp::Interpolation => write!(f, "%%"),
            BinOp::Mult => write!(f, "*"),
            BinOp::Div => write!(f, "/"),
            BinOp::Mod => write!(f, "%"),
            BinOp::Disjunction => write!(f, "\\/"),
            BinOp::Conjunction => write!(f, "/\\"),
        }
    }
}

impl fmt::Display for UnaryOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UnaryOp::Not => write!(f, "not"),
            UnaryOp::Neg => write!(f, "-"),
            UnaryOp::Negation => write!(f, "~"),
        }
    }
}

/// Generates a random number in the range [min, max] inclusive.
fn gen_range(g: &mut Gen, min: u32, max: u32) -> u32 {
    min + (u32::arbitrary(g) % (max - min + 1))
}

/// Generates a random variable name that is not a reserved keyword.
fn gen_var_name(g: &mut Gen) -> String {
    let starters: Vec<char> = "abcdefghijklmnopqrstuvwxyz_".chars().collect();
    let continuers: Vec<char> = "abcdefghijklmnopqrstuvwxyz0123456789_".chars().collect(); // Removed ' to avoid potential parse issues
    loop {
        let len = gen_range(g, 1, 10);
        let mut name = String::new();
        name.push(*g.choose(&starters).unwrap());
        for _ in 1..len {
            name.push(*g.choose(&continuers).unwrap());
        }
        if !RESERVED_KEYWORDS.iter().any(|&kw| kw == name.as_str()) {
            return name;
        }
    }
}

/// Generates a random string literal content.
fn gen_string_content(g: &mut Gen) -> String {
    let len = gen_range(g, 0, 5);
    (0..len)
        .map(|_| {
            let mut c = char::arbitrary(g);
            while c.is_control() || c == '"' || c == '\\' {
                c = char::arbitrary(g);
            }
            c
        })
        .collect()
}

/// Generates a random URI literal content.
fn gen_uri_content(g: &mut Gen) -> String {
    let len = gen_range(g, 0, 10);
    (0..len)
        .map(|_| {
            let mut c = char::arbitrary(g);
            while c.is_control() || c == '`' {
                c = char::arbitrary(g);
            }
            c
        })
        .collect()
}

/// Generates a random literal(bool, int, string, uri).
fn gen_literal(g: &mut Gen) -> RholangProc {
    const CHOICES: &[&str] = &["bool", "int", "string", "uri"];
    match *g.choose(CHOICES).unwrap() {
        "bool" => RholangProc::BoolLit(bool::arbitrary(g)),
        "int" => RholangProc::IntLit(i64::arbitrary(g) % 1000),
        "string" => RholangProc::StringLit(gen_string_content(g)),
        "uri" => RholangProc::UriLit(gen_uri_content(g)),
        _ => unreachable!(),
    }
}

/// Generates a random simple type (Bool, Int, String, Uri, ByteArray).
fn gen_simple_type(g: &mut Gen) -> String {
    const CHOICES: &[&str] = &["Bool", "Int", "String", "Uri", "ByteArray"];
    g.choose(CHOICES).unwrap().to_string()
}

/// Generates a random process variable (named or wildcard).
fn gen_proc_var(g: &mut Gen) -> ProcVar {
    if bool::arbitrary(g) && gen_range(g, 0, 3) == 0 {
        ProcVar::Wildcard
    } else {
        ProcVar::Var(gen_var_name(g))
    }
}

/// Generates a random name list with optional remainder.
fn gen_name_list(g: &mut Gen, depth: usize) -> NameList {
    let num_items = gen_range(g, 1, 2);
    let has_remainder = bool::arbitrary(g) && gen_range(g, 0, 5) == 0; // Rarely add remainder
    NameList {
        items: (0..num_items).map(|_| gen_name(g, depth)).collect(),
        remainder: if has_remainder { Some(gen_proc_var(g)) } else { None },
    }
}

/// Generates a random name (proc_var or quote).
fn gen_name(g: &mut Gen, depth: usize) -> Name {
    const CHOICES: &[&str] = &["procvar", "quote"];
    match *g.choose(CHOICES).unwrap() {
        "procvar" => Name::ProcVar(gen_proc_var(g)),
        "quote" => Name::Quote(Box::new(gen_quotable(g, depth))),
        _ => unreachable!(),
    }
}

/// Generates a random source (simple, receive_send, send_receive).
fn gen_source(g: &mut Gen, depth: usize) -> Source {
    const CHOICES: &[&str] = &["simple", "receive_send", "send_receive"];
    match *g.choose(CHOICES).unwrap() {
        "simple" => Source::Simple(Box::new(gen_name(g, depth))),
        "receive_send" => Source::ReceiveSend(Box::new(gen_name(g, depth))),
        "send_receive" => Source::SendReceive {
            name: Box::new(gen_name(g, depth)),
            inputs: (0..gen_range(g, 0, 2)).map(|_| gen_expr(g, depth)).collect(),
        },
        _ => unreachable!(),
    }
}

/// Generates a random bind (linear, repeated, peek).
fn gen_bind(g: &mut Gen, depth: usize) -> Bind {
    let names = gen_name_list(g, depth);
    const CHOICES: &[&str] = &["linear", "repeated", "peek"];
    match *g.choose(CHOICES).unwrap() {
        "linear" => Bind::Linear { names, source: gen_source(g, depth) },
        "repeated" => Bind::Repeated { names, name: gen_name(g, depth) },
        "peek" => Bind::Peek { names, name: gen_name(g, depth) },
        _ => unreachable!(),
    }
}

/// Generates a random receipt (conjunction of 1-2 binds).
fn gen_receipt(g: &mut Gen, depth: usize) -> Receipt {
    let num_binds = gen_range(g, 1, 2);
    (0..num_binds).map(|_| gen_bind(g, depth)).collect()
}

/// Generates a random linear bind for choice branches.
fn gen_linear_bind(g: &mut Gen, depth: usize) -> LinearBind {
    LinearBind {
        names: gen_name_list(g, depth),
        source: gen_source(g, depth),
    }
}

/// Generates a random new declaration (simple or with URI).
fn gen_new_decl(g: &mut Gen, _depth: usize) -> NewDecl {
    let var = gen_var_name(g);
    if bool::arbitrary(g) {
        NewDecl::WithUri(var, gen_uri_content(g))
    } else {
        NewDecl::Simple(var)
    }
}

/// Generates a random let declaration.
fn gen_decl(g: &mut Gen, depth: usize) -> Decl {
    let num_procs = gen_range(g, 1, 2);
    Decl {
        names: gen_name_list(g, depth),
        procs: (0..num_procs).map(|_| gen_proc(g, depth)).collect(),
    }
}

/// Generates a random collection (list, tuple, set, map) with optional remainder.
fn gen_collection(g: &mut Gen, depth: usize) -> RholangProc {
    const CHOICES: &[&str] = &["list", "tuple", "set", "map"];
    let has_remainder = bool::arbitrary(g) && depth > 0 && gen_range(g, 0, 5) == 0; // Rarely add remainder
    let remainder = if has_remainder {
        match gen_proc_var(g) {
            ProcVar::Var(s) => Some(Box::new(RholangProc::Var(s))),
            ProcVar::Wildcard => Some(Box::new(RholangProc::Wildcard)),
        }
    } else {
        None
    };
    let num_elements = gen_range(g, 0, 2);
    match *g.choose(CHOICES).unwrap() {
        "list" => RholangProc::List(
            (0..num_elements).map(|_| gen_expr(g, depth)).collect(),
            remainder,
        ),
        "tuple" => RholangProc::Tuple(
            (0..gen_range(g, 1, 3)).map(|_| gen_expr(g, depth)).collect(),
        ),
        "set" => RholangProc::Set(
            (0..num_elements).map(|_| gen_expr(g, depth)).collect(),
            remainder,
        ),
        "map" => RholangProc::Map(
            (0..num_elements).map(|_| (gen_expr(g, depth), gen_expr(g, depth))).collect(),
            remainder,
        ),
        _ => unreachable!(),
    }
}

/// Generates a random quotable process (var_ref, eval, disjunction, conjunction, negation, ground).
fn gen_quotable(g: &mut Gen, depth: usize) -> RholangProc {
    if depth == 0 {
        return gen_ground(g, 0);
    }
    const CHOICES: &[&str] = &["var_ref", "eval", "disjunction", "conjunction", "negation", "ground"];
    match *g.choose(CHOICES).unwrap() {
        "var_ref" => RholangProc::VarRef(
            if bool::arbitrary(g) { VarRefKind::Bind } else { VarRefKind::Unforgeable },
            gen_var_name(g),
        ),
        "eval" => RholangProc::Eval(Box::new(gen_name(g, depth - 1))),
        "disjunction" => RholangProc::Disjunction(Box::new(gen_quotable(g, depth - 1)), Box::new(gen_quotable(g, depth - 1))),
        "conjunction" => RholangProc::Conjunction(Box::new(gen_quotable(g, depth - 1)), Box::new(gen_quotable(g, depth - 1))),
        "negation" => RholangProc::Negation(Box::new(gen_quotable(g, depth - 1))),
        "ground" => gen_ground(g, depth - 1),
        _ => unreachable!(),
    }
}

/// Generates a random ground expression (block, literal, nil, collection, proc_var, simple_type).
fn gen_ground(g: &mut Gen, depth: usize) -> RholangProc {
    const CHOICES: &[&str] = &["block", "literal", "nil", "collection", "proc_var", "simple_type"];
    match *g.choose(CHOICES).unwrap() {
        "block" => RholangProc::Block(Box::new(gen_proc(g, depth))),
        "literal" => gen_literal(g),
        "nil" => RholangProc::Nil,
        "collection" => gen_collection(g, depth),
        "proc_var" => match gen_proc_var(g) {
            ProcVar::Var(s) => RholangProc::Var(s),
            ProcVar::Wildcard => RholangProc::Wildcard,
        },
        "simple_type" => RholangProc::SimpleType(gen_simple_type(g)),
        _ => unreachable!(),
    }
}

/// Generates a random expression with limited depth.
fn gen_expr(g: &mut Gen, depth: usize) -> RholangProc {
    if depth == 0 {
        return gen_ground(g, 0);
    }
    const CHOICES: &[&str] = &[
        "ground", "parenthesized", "add", "sub", "mult", "div", "mod", "or", "and", "concat", "diff",
        "interpolation", "eq", "neq", "lt", "lte", "gt", "gte", "matches", "not", "neg", "method",
        "quote", "var_ref", "disjunction", "conjunction", "negation", "eval",
    ];
    match *g.choose(CHOICES).unwrap() {
        "ground" => gen_ground(g, depth - 1),
        "parenthesized" => RholangProc::Parenthesized(Box::new(gen_expr(g, depth - 1))),
        op_str => {
            if let Some(op) = match op_str {
                "add" => Some(BinOp::Add),
                "sub" => Some(BinOp::Sub),
                "mult" => Some(BinOp::Mult),
                "div" => Some(BinOp::Div),
                "mod" => Some(BinOp::Mod),
                "or" => Some(BinOp::Or),
                "and" => Some(BinOp::And),
                "concat" => Some(BinOp::Concat),
                "diff" => Some(BinOp::Diff),
                "interpolation" => Some(BinOp::Interpolation),
                "eq" => Some(BinOp::Eq),
                "neq" => Some(BinOp::Neq),
                "lt" => Some(BinOp::Lt),
                "lte" => Some(BinOp::Lte),
                "gt" => Some(BinOp::Gt),
                "gte" => Some(BinOp::Gte),
                "matches" => Some(BinOp::Matches),
                _ => None,
            } {
                RholangProc::BinOp {
                    op,
                    left: Box::new(gen_expr(g, depth - 1)),
                    right: Box::new(gen_expr(g, depth - 1)),
                }
            } else if op_str == "not" {
                RholangProc::UnaryOp { op: UnaryOp::Not, operand: Box::new(gen_expr(g, depth - 1)) }
            } else if op_str == "neg" {
                RholangProc::UnaryOp { op: UnaryOp::Neg, operand: Box::new(gen_expr(g, depth - 1)) }
            } else if op_str == "method" {
                RholangProc::Method {
                    receiver: Box::new(gen_expr(g, depth - 1)),
                    name: gen_var_name(g),
                    args: (0..gen_range(g, 0, 2)).map(|_| gen_expr(g, depth - 1)).collect(),
                }
            } else if op_str == "quote" {
                RholangProc::Quote(Box::new(gen_quotable(g, depth - 1)))
            } else if op_str == "var_ref" {
                RholangProc::VarRef(
                    if bool::arbitrary(g) { VarRefKind::Bind } else { VarRefKind::Unforgeable },
                    gen_var_name(g),
                )
            } else if op_str == "disjunction" {
                RholangProc::BinOp { op: BinOp::Disjunction, left: Box::new(gen_expr(g, depth - 1)), right: Box::new(gen_expr(g, depth - 1)) }
            } else if op_str == "conjunction" {
                RholangProc::BinOp { op: BinOp::Conjunction, left: Box::new(gen_expr(g, depth - 1)), right: Box::new(gen_expr(g, depth - 1)) }
            } else if op_str == "negation" {
                RholangProc::UnaryOp { op: UnaryOp::Negation, operand: Box::new(gen_expr(g, depth - 1)) }
            } else if op_str == "eval" {
                RholangProc::Eval(Box::new(gen_name(g, depth - 1)))
            } else {
                unreachable!()
            }
        }
    }
}

/// Generates a random process with limited depth.
fn gen_proc(g: &mut Gen, depth: usize) -> RholangProc {
    let depth = depth.min(MAX_DEPTH);
    if depth == 0 {
        return gen_expr(g, 0);
    }
    const CHOICES: &[&str] = &[
        "par", "send_sync", "new", "ifelse", "let", "bundle", "match", "choice", "contract", "input", "send", "expr",
    ];
    match *g.choose(CHOICES).unwrap() {
        "par" => RholangProc::Par {
            left: Box::new(gen_proc(g, depth - 1)),
            right: Box::new(gen_proc(g, depth - 1)),
        },
        "send_sync" => RholangProc::SendSync {
            channel: Box::new(gen_name(g, depth - 1)),
            inputs: (0..gen_range(g, 0, 2)).map(|_| gen_expr(g, depth - 1)).collect(),
            cont: if bool::arbitrary(g) { SyncCont::Empty } else { SyncCont::NonEmpty(Box::new(gen_proc(g, depth - 1))) },
        },
        "new" => RholangProc::New {
            decls: (0..gen_range(g, 1, 2)).map(|_| gen_new_decl(g, depth - 1)).collect(),
            proc: Box::new(gen_proc(g, depth - 1)),
        },
        "ifelse" => RholangProc::IfElse {
            condition: Box::new(gen_expr(g, depth - 1)),
            consequence: Box::new(gen_proc(g, depth - 1)),
            alternative: if bool::arbitrary(g) { Some(Box::new(gen_proc(g, depth - 1))) } else { None },
        },
        "let" => RholangProc::Let {
            is_conc: bool::arbitrary(g),
            decls: (0..gen_range(g, 1, 2)).map(|_| gen_decl(g, depth - 1)).collect(),
            proc: Box::new(gen_proc(g, depth - 1)),
        },
        "bundle" => RholangProc::Bundle {
            bundle_type: match gen_range(g, 0, 3) {
                0 => BundleType::Read,
                1 => BundleType::Write,
                2 => BundleType::Equiv,
                _ => BundleType::ReadWrite,
            },
            proc: Box::new(gen_proc(g, depth - 1)),
        },
        "match" => RholangProc::Match {
            expression: Box::new(gen_expr(g, depth - 1)),
            cases: (0..gen_range(g, 1, 2)).map(|_| (gen_expr(g, depth - 1), gen_proc(g, depth - 1))).collect(),
        },
        "choice" => RholangProc::Choice {
            branches: (0..gen_range(g, 1, 2)).map(|_| Branch {
                binds: (0..gen_range(g, 1, 2)).map(|_| gen_linear_bind(g, depth - 1)).collect(),
                proc: gen_proc(g, depth - 1),
            }).collect(),
        },
        "contract" => RholangProc::Contract {
            name: Box::new(gen_name(g, depth - 1)),
            formals: gen_name_list(g, depth - 1),
            proc: Box::new(gen_proc(g, depth - 1)),
        },
        "input" => RholangProc::Input {
            receipts: (0..gen_range(g, 1, 2)).map(|_| gen_receipt(g, depth - 1)).collect(),
            proc: Box::new(gen_proc(g, depth - 1)),
        },
        "send" => RholangProc::Send {
            channel: Box::new(gen_name(g, depth - 1)),
            send_type: if bool::arbitrary(g) { SendType::Single } else { SendType::Multiple },
            inputs: (0..gen_range(g, 0, 2)).map(|_| gen_expr(g, depth - 1)).collect(),
        },
        "expr" => gen_expr(g, depth - 1),
        _ => unreachable!(),
    }
}

impl Arbitrary for BinOp {
    fn arbitrary(g: &mut Gen) -> Self {
        const CHOICES: &[BinOp] = &[
            BinOp::Or, BinOp::And, BinOp::Matches, BinOp::Eq, BinOp::Neq, BinOp::Lt, BinOp::Lte, BinOp::Gt, BinOp::Gte,
            BinOp::Concat, BinOp::Diff, BinOp::Add, BinOp::Sub, BinOp::Interpolation, BinOp::Mult, BinOp::Div, BinOp::Mod,
            BinOp::Disjunction, BinOp::Conjunction,
        ];
        g.choose(CHOICES).unwrap().clone()
    }
}

impl Arbitrary for UnaryOp {
    fn arbitrary(g: &mut Gen) -> Self {
        const CHOICES: &[UnaryOp] = &[UnaryOp::Not, UnaryOp::Neg, UnaryOp::Negation];
        g.choose(CHOICES).unwrap().clone()
    }
}

impl Arbitrary for RholangProc {
    fn arbitrary(g: &mut Gen) -> Self {
        gen_proc(g, g.size().min(MAX_DEPTH))
    }
}

/// Escapes special characters in string literals for Rholang.
fn escape_rholang_string(s: &str) -> String {
    let mut escaped = String::new();
    for c in s.chars() {
        match c {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            _ => escaped.push(c),
        }
    }
    escaped
}

impl ProcVar {
    pub fn to_code(&self) -> String {
        match self {
            ProcVar::Var(s) => s.clone(),
            ProcVar::Wildcard => "_".to_string(),
        }
    }
}

impl Name {
    pub fn to_code(&self) -> String {
        match self {
            Name::ProcVar(pv) => pv.to_code(),
            Name::Quote(q) => format!("@{}", q.to_code()),
        }
    }
}

impl NameList {
    pub fn to_code(&self) -> String {
        let mut code = self.items.iter().map(|n| n.to_code()).collect::<Vec<_>>().join(", ");
        if let Some(rem) = &self.remainder {
            if !code.is_empty() {
                code.push_str(", ");
            }
            code.push_str(&format!("... @{}", rem.to_code()));
        }
        code
    }
}

impl Source {
    pub fn to_code(&self) -> String {
        match self {
            Source::Simple(n) => n.to_code(),
            Source::ReceiveSend(n) => format!("{}?!", n.to_code()),
            Source::SendReceive { name, inputs } => {
                let inputs_str = inputs.iter().map(|i| i.to_code()).collect::<Vec<_>>().join(", ");
                format!("{}!?({})", name.to_code(), inputs_str)
            },
        }
    }
}

impl Bind {
    pub fn to_code(&self) -> String {
        match self {
            Bind::Linear { names, source } => {
                format!("{} <- {}", names.to_code(), source.to_code())
            },
            Bind::Repeated { names, name } => {
                format!("{} <= {}", names.to_code(), name.to_code())
            },
            Bind::Peek { names, name } => {
                format!("{} <<- {}", names.to_code(), name.to_code())
            },
        }
    }
}

impl NewDecl {
    pub fn to_code(&self) -> String {
        match self {
            NewDecl::Simple(s) => s.clone(),
            NewDecl::WithUri(s, u) => format!("{}(`{}`)", s, u),
        }
    }
}

impl BundleType {
    pub fn to_code(&self) -> String {
        match self {
            BundleType::Read => "bundle-".to_string(),
            BundleType::Write => "bundle+".to_string(),
            BundleType::Equiv => "bundle0".to_string(),
            BundleType::ReadWrite => "bundle".to_string(),
        }
    }
}

impl SendType {
    pub fn to_code(&self) -> String {
        match self {
            SendType::Single => "!".to_string(),
            SendType::Multiple => "!!".to_string(),
        }
    }
}

impl SyncCont {
    pub fn to_code(&self) -> String {
        match self {
            SyncCont::Empty => ".".to_string(),
            SyncCont::NonEmpty(p) => format!("; {}", p.to_code()),
        }
    }
}

impl Branch {
    pub fn to_code(&self) -> String {
        let binds_str = self.binds.iter().map(|b| {
            format!("{} <- {}", b.names.to_code(), b.source.to_code())
        }).collect::<Vec<_>>().join(" & ");
        format!("{} => {}", binds_str, self.proc.to_code())
    }
}

impl Decl {
    pub fn to_code(&self) -> String {
        let names_str = self.names.to_code();
        let procs_str = self.procs.iter().map(|p| p.to_code()).collect::<Vec<_>>().join(", ");
        format!("{} = {}", names_str, procs_str)
    }
}

impl RholangProc {
    /// Converts the process to a Rholang code string.
    pub fn to_code(&self) -> String {
        match self {
            RholangProc::Nil => "Nil".to_string(),
            RholangProc::Var(var) => var.clone(),
            RholangProc::Wildcard => "_".to_string(),
            RholangProc::Quote(q) => format!("@{}", q.to_code()),
            RholangProc::Eval(n) => format!("*{}", n.to_code()),
            RholangProc::VarRef(kind, var) => format!("{}{}", if matches!(kind, VarRefKind::Bind) { "=" } else { "=*" }, var),
            RholangProc::Disjunction(l, r) => format!("{} \\/ {}", l.to_code(), r.to_code()),
            RholangProc::Conjunction(l, r) => format!("{} /\\ {}", l.to_code(), r.to_code()),
            RholangProc::Negation(o) => format!("~{}", o.to_code()),
            RholangProc::SimpleType(t) => t.clone(),
            RholangProc::Block(p) => format!("{{{}}}", p.to_code()),
            RholangProc::Tuple(elements) => {
                let els = elements.iter().map(|e| e.to_code()).collect::<Vec<_>>().join(", ");
                format!("({})", els)
            },
            RholangProc::List(elements, remainder) => {
                let els = elements.iter().map(|e| e.to_code()).collect::<Vec<_>>().join(", ");
                let rem = remainder.as_ref().map(|r| format!(" ... {}", r.to_code())).unwrap_or_default();
                format!("[{}{}]", els, rem)
            },
            RholangProc::Set(elements, remainder) => {
                let els = elements.iter().map(|e| e.to_code()).collect::<Vec<_>>().join(", ");
                let rem = remainder.as_ref().map(|r| format!(" ... {}", r.to_code())).unwrap_or_default();
                format!("Set({}{})", els, rem)
            },
            RholangProc::Map(pairs, remainder) => {
                let prs = pairs.iter().map(|(k, v)| format!("{}: {}", k.to_code(), v.to_code())).collect::<Vec<_>>().join(", ");
                let rem = remainder.as_ref().map(|r| format!(" ... {}", r.to_code())).unwrap_or_default();
                format!("{{{}{}}}", prs, rem)
            },
            RholangProc::BoolLit(b) => b.to_string(),
            RholangProc::IntLit(i) => i.to_string(),
            RholangProc::StringLit(s) => format!("\"{}\"", escape_rholang_string(s)),
            RholangProc::UriLit(u) => format!("`{}`", u),
            RholangProc::BinOp { op, left, right } => format!("{} {} {}", left.to_code(), op, right.to_code()),
            RholangProc::UnaryOp { op, operand } => format!("{} {}", op, operand.to_code()),
            RholangProc::Method { receiver, name, args } => {
                let args_str = args.iter().map(|a| a.to_code()).collect::<Vec<_>>().join(", ");
                format!("{}.{}({})", receiver.to_code(), name, args_str)
            },
            RholangProc::Par { left, right } => format!("{} | {}", left.to_code(), right.to_code()),
            RholangProc::Send { channel, send_type, inputs } => {
                let inputs_str = inputs.iter().map(|i| i.to_code()).collect::<Vec<_>>().join(", ");
                format!("{}{}({})", channel.to_code(), send_type.to_code(), inputs_str)
            },
            RholangProc::SendSync { channel, inputs, cont } => {
                let inputs_str = inputs.iter().map(|i| i.to_code()).collect::<Vec<_>>().join(", ");
                format!("{}!?({}){}", channel.to_code(), inputs_str, cont.to_code())
            },
            RholangProc::New { decls, proc } => {
                let decls_str = decls.iter().map(|d| d.to_code()).collect::<Vec<_>>().join(", ");
                format!("new {} in {}", decls_str, proc.to_code())
            },
            RholangProc::IfElse { condition, consequence, alternative } => {
                let mut code = format!("if ({}) {}", condition.to_code(), consequence.to_code());
                if let Some(alt) = alternative {
                    code += &format!(" else {}", alt.to_code());
                }
                code
            },
            RholangProc::Let { is_conc, decls, proc } => {
                let sep = if *is_conc { " & " } else { "; " };
                let decls_str = decls.iter().map(|d| d.to_code()).collect::<Vec<_>>().join(sep);
                format!("let {} in {}", decls_str, proc.to_code())
            },
            RholangProc::Bundle { bundle_type, proc } => format!("{}{}", bundle_type.to_code(), proc.to_code()),
            RholangProc::Match { expression, cases } => {
                let cases_str = cases.iter().map(|(pat, pr)| format!(" {} => {}", pat.to_code(), pr.to_code())).collect::<Vec<_>>().join("");
                format!("match {} {{{}}}", expression.to_code(), cases_str)
            },
            RholangProc::Choice { branches } => {
                let branches_str = branches.iter().map(|b| b.to_code()).collect::<Vec<_>>().join("");
                format!("select {{{}}}", branches_str)
            },
            RholangProc::Contract { name, formals, proc } => {
                let formals_str = formals.to_code();
                format!("contract {}({}) = {}", name.to_code(), formals_str, proc.to_code())
            },
            RholangProc::Input { receipts, proc } => {
                let receipts_str = receipts.iter().map(|r| r.iter().map(|b| b.to_code()).collect::<Vec<_>>().join(" & ")).collect::<Vec<_>>().join("; ");
                format!("for ({}) {}", receipts_str, proc.to_code())
            },
            RholangProc::Parenthesized(p) => format!("({})", p.to_code()),
        }
    }
}
