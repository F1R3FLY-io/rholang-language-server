use quickcheck::{Arbitrary, Gen};
use std::fmt;

#[derive(Clone, Debug)]
pub enum RholangProc {
    Nil,
    Var(String),
    Send {
        channel: String,
        inputs: Vec<RholangProc>,
    },
    SendSync {
        channel: String,
        inputs: Vec<RholangProc>,
        cont: Box<RholangProc>,
    },
    Par {
        left: Box<RholangProc>,
        right: Box<RholangProc>,
    },
    New {
        decls: Vec<String>,
        proc: Box<RholangProc>,
    },
    IfElse {
        condition: Box<RholangProc>,
        then: Box<RholangProc>,
        else_: Option<Box<RholangProc>>,
    },
    Let {
        bindings: Vec<(String, RholangProc)>,
        proc: Box<RholangProc>,
    },
    Bundle {
        is_read: bool,
        is_write: bool,
        proc: Box<RholangProc>,
    },
    Match {
        target: Box<RholangProc>,
        cases: Vec<(RholangProc, RholangProc)>,
    },
    Choice {
        branches: Vec<RholangProc>,
    },
    Contract {
        name: String,
        params: Vec<String>,
        body: Box<RholangProc>,
    },
    For {
        pattern: Vec<String>,
        channel: String,
        proc: Box<RholangProc>,
    },
    BinOp {
        op: BinOp,
        left: Box<RholangProc>,
        right: Box<RholangProc>,
    },
    UnaryOp {
        op: UnaryOp,
        operand: Box<RholangProc>,
    },
    StringLit(String),
    BoolLit(bool),
    IntLit(i64),
    ListLit(Vec<RholangProc>),
}

#[derive(Clone, Debug)]
pub enum BinOp {
    Add,
    Sub,
    Mult,
    Or,
    And,
}

impl fmt::Display for BinOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BinOp::Add => write!(f, "+"),
            BinOp::Sub => write!(f, "-"),
            BinOp::Mult => write!(f, "*"),
            BinOp::Or => write!(f, "or"),
            BinOp::And => write!(f, "and"),
        }
    }
}

#[derive(Clone, Debug)]
pub enum UnaryOp {
    Not,
    Neg,
}

impl fmt::Display for UnaryOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UnaryOp::Not => write!(f, "not"),
            UnaryOp::Neg => write!(f, "-"),
        }
    }
}

impl Arbitrary for BinOp {
    fn arbitrary(g: &mut Gen) -> Self {
        g.choose(&[BinOp::Add, BinOp::Sub, BinOp::Mult, BinOp::Or, BinOp::And])
            .unwrap()
            .clone()
    }
}

impl Arbitrary for UnaryOp {
    fn arbitrary(g: &mut Gen) -> Self {
        g.choose(&[UnaryOp::Not, UnaryOp::Neg]).unwrap().clone()
    }
}

macro_rules! gen_range {
    ($g:expr, $min:expr, $max:expr) => {
        $min + (u32::arbitrary($g) % ($max - $min + 1))
    };
    ($g:expr, $min:expr, $max:expr, $t:ty) => {
        $min + (<$t>::arbitrary($g) % ($max - $min + 1))
    };
}

fn gen_proc_expression(g: &mut Gen, depth: usize) -> RholangProc {
    let choices = [
        "var", "string", "bool", "int", "list", "binop", "unaryop", "nil",
    ];
    match *g.choose(&choices).unwrap() {
        "var" => RholangProc::Var(gen_var_name(g)),
        "string" => {
            let len = gen_range!(g, 0, 5);
            let s = (0..len)
                .map(|_| char::arbitrary(g))
                .filter(|c| !c.is_control())
                .collect();
            RholangProc::StringLit(s)
        }
        "bool" => RholangProc::BoolLit(bool::arbitrary(g)),
        "int" => RholangProc::IntLit(i64::arbitrary(g) % 1000),
        "list" => {
            let num_elements = gen_range!(g, 0, 2);
            let elements = (0..num_elements)
                .map(|_| gen_proc_expression(&mut Gen::new(depth / 2), depth / 2))
                .collect();
            RholangProc::ListLit(elements)
        }
        "binop" => RholangProc::BinOp {
            op: BinOp::arbitrary(g),
            left: Box::new(gen_proc_expression(&mut Gen::new(depth / 2), depth / 2)),
            right: Box::new(gen_proc_expression(&mut Gen::new(depth / 2), depth / 2)),
        },
        "unaryop" => RholangProc::UnaryOp {
            op: UnaryOp::arbitrary(g),
            operand: Box::new(gen_proc_expression(&mut Gen::new(depth / 2), depth / 2)),
        },
        "nil" => RholangProc::Nil,
        _ => unreachable!(),
    }
}

fn gen_process(g: &mut Gen, depth: usize) -> RholangProc {
    if depth == 0 {
        return RholangProc::Nil;
    }
    let choices = [
        "nil", "send", "par", "new", "ifelse", "let", "bundle", "match",
        "choice", "contract", "for",
    ];
    match *g.choose(&choices).unwrap() {
        "nil" => RholangProc::Nil,
        "send" => RholangProc::Send {
            channel: gen_var_name(g),
            inputs: (0..gen_range!(g, 0, 1))
                .map(|_| gen_proc_expression(g, depth / 2))
                .collect(),
        },
        "par" => RholangProc::Par {
            left: Box::new(gen_process(g, depth / 2)),
            right: Box::new(gen_process(g, depth / 2)),
        },
        "new" => {
            let num_decls = gen_range!(g, 1, 2);
            let decls = (0..num_decls).map(|_| gen_var_name(g)).collect();
            RholangProc::New {
                decls,
                proc: Box::new(gen_process(g, depth / 2)),
            }
        },
        "ifelse" => RholangProc::IfElse {
            condition: Box::new(gen_proc_expression(g, depth / 2)),
            then: Box::new(gen_process(g, depth / 2)),
            else_: if bool::arbitrary(g) {
                Some(Box::new(gen_process(g, depth / 2)))
            } else {
                None
            },
        },
        "let" => {
            let num_bindings = gen_range!(g, 1, 2);
            let bindings = (0..num_bindings)
                .map(|_| (gen_var_name(g), gen_proc_expression(g, depth / 2)))
                .collect();
            RholangProc::Let {
                bindings,
                proc: Box::new(gen_process(g, depth / 2)),
            }
        },
        "bundle" => RholangProc::Bundle {
            is_read: bool::arbitrary(g),
            is_write: bool::arbitrary(g),
            proc: Box::new(gen_process(g, depth / 2)),
        },
        "match" => {
            let num_cases = gen_range!(g, 1, 2);
            let cases = (0..num_cases)
                .map(|_| (gen_pattern(g), gen_process(g, depth / 2)))
                .collect();
            RholangProc::Match {
                target: Box::new(gen_proc_expression(g, depth)),
                cases,
            }
        },
        "choice" => {
            let num_branches = gen_range!(g, 1, 2);
            let branches = (0..num_branches)
                .map(|_| {
                    let var = gen_var_name(g);
                    let channel = gen_var_name(g);
                    RholangProc::For {
                        pattern: vec![var],
                        channel,
                        proc: Box::new(gen_process(g, depth / 2)),
                    }
                })
                .collect();
            RholangProc::Choice { branches }
        },
        "contract" => {
            let name = gen_var_name(g);
            let num_params = gen_range!(g, 1, 2);
            let params = (0..num_params).map(|_| gen_var_name(g)).collect();
            RholangProc::Contract {
                name,
                params,
                body: Box::new(gen_process(g, depth / 2)),
            }
        },
        "for" => {
            let num_vars = gen_range!(g, 1, 2);
            let pattern = (0..num_vars).map(|_| gen_var_name(g)).collect();
            let channel = gen_var_name(g);
            RholangProc::For {
                pattern,
                channel,
                proc: Box::new(gen_process(g, depth / 2)),
            }
        },
        _ => unreachable!(),
    }
}

impl Arbitrary for RholangProc {
    fn arbitrary(g: &mut Gen) -> Self {
        let depth = g.size();
        if depth == 0 {
            return match *g.choose(&["nil", "var", "string", "bool", "int"]).unwrap() {
                "nil" => RholangProc::Nil,
                "var" => RholangProc::Var(gen_var_name(g)),
                "string" => RholangProc::StringLit(String::new()),
                "bool" => RholangProc::BoolLit(bool::arbitrary(g)),
                "int" => RholangProc::IntLit(i64::arbitrary(g) % 1000),
                _ => unreachable!(),
            };
        }

        let choices = [
            "nil", "var", "send", "par", "new", "ifelse", "let", "bundle", "match",
            "choice", "contract", "for", "binop", "unaryop", "string", "bool", "int", "list",
        ];
        match *g.choose(&choices).unwrap() {
            "nil" => RholangProc::Nil,
            "var" => RholangProc::Var(gen_var_name(g)),
            "send" => RholangProc::Send {
                channel: gen_var_name(g),
                inputs: (0..gen_range!(g, 0, 1)).map(|_| RholangProc::arbitrary(&mut Gen::new(depth / 2))).collect(),
            },
            "sendsync" => RholangProc::SendSync {
                channel: gen_var_name(g),
                inputs: (0..gen_range!(g, 0, 1)).map(|_| RholangProc::arbitrary(&mut Gen::new(depth / 2))).collect(),
                cont: Box::new(RholangProc::arbitrary(&mut Gen::new(depth / 2))),
            },
            "par" => RholangProc::Par {
                left: Box::new(gen_process(g, depth / 2)),
                right: Box::new(gen_process(g, depth / 2)),
            },
            "new" => {
                let num_decls = gen_range!(g, 1, 2);
                let decls = (0..num_decls).map(|_| gen_var_name(g)).collect();
                RholangProc::New {
                    decls,
                    proc: Box::new(gen_process(g, depth / 2)),
                }
            },
            "ifelse" => RholangProc::IfElse {
                condition: Box::new(gen_proc_expression(g, depth / 2)),
                then: Box::new(gen_process(g, depth / 2)),
                else_: if bool::arbitrary(g) {
                    Some(Box::new(gen_process(g, depth / 2)))
                } else {
                    None
                },
            },
            "let" => {
                let num_bindings = gen_range!(g, 1, 2);
                let bindings = (0..num_bindings)
                    .map(|_| (gen_var_name(g), gen_proc_expression(g, depth / 2)))
                    .collect();
                RholangProc::Let {
                    bindings,
                    proc: Box::new(gen_process(g, depth / 2)),
                }
            },
            "bundle" => RholangProc::Bundle {
                is_read: bool::arbitrary(g),
                is_write: bool::arbitrary(g),
                proc: Box::new(gen_process(g, depth / 2)),
            },
            "match" => {
                let num_cases = gen_range!(g, 1, 2);
                let cases = (0..num_cases)
                    .map(|_| (gen_pattern(g), gen_process(g, depth / 2)))
                    .collect();
                RholangProc::Match {
                    target: Box::new(gen_proc_expression(g, depth)),
                    cases,
                }
            },
            "choice" => {
                let num_branches = gen_range!(g, 1, 2);
                let branches = (0..num_branches)
                    .map(|_| {
                        let var = gen_var_name(g);
                        let channel = gen_var_name(g);
                        RholangProc::For {
                            pattern: vec![var],
                            channel,
                            proc: Box::new(gen_process(g, depth / 2)),
                        }
                    })
                    .collect();
                RholangProc::Choice { branches }
            },
            "contract" => {
                let name = gen_var_name(g);
                let num_params = gen_range!(g, 1, 2);
                let params = (0..num_params).map(|_| gen_var_name(g)).collect();
                RholangProc::Contract {
                    name,
                    params,
                    body: Box::new(gen_process(g, depth / 2)),
                }
            },
            "for" => {
                let num_vars = gen_range!(g, 1, 2);
                let pattern = (0..num_vars).map(|_| gen_var_name(g)).collect();
                let channel = gen_var_name(g);
                RholangProc::For {
                    pattern,
                    channel,
                    proc: Box::new(gen_process(g, depth / 2)),
                }
            },
            "binop" => RholangProc::BinOp {
                op: BinOp::arbitrary(g),
                left: Box::new(RholangProc::arbitrary(&mut Gen::new(depth / 2))),
                right: Box::new(RholangProc::arbitrary(&mut Gen::new(depth / 2))),
            },
            "unaryop" => RholangProc::UnaryOp {
                op: UnaryOp::arbitrary(g),
                operand: Box::new(RholangProc::arbitrary(&mut Gen::new(depth / 2))),
            },
            "string" => {
                let len = gen_range!(g, 0, 5);
                let s = (0..len)
                    .map(|_| char::arbitrary(g))
                    .filter(|c| !c.is_control())
                    .collect();
                RholangProc::StringLit(s)
            },
            "bool" => RholangProc::BoolLit(bool::arbitrary(g)),
            "int" => RholangProc::IntLit(i64::arbitrary(g) % 1000),
            "list" => {
                let num_elements = gen_range!(g, 0, 2);
                let elements = (0..num_elements)
                    .map(|_| RholangProc::arbitrary(&mut Gen::new(depth / 2)))
                    .collect();
                RholangProc::ListLit(elements)
            },
            _ => unreachable!(),
        }
    }
}

fn gen_pattern(g: &mut Gen) -> RholangProc {
    match *g.choose(&["var", "bool", "int", "string"]).unwrap() {
        "var" => RholangProc::Var(gen_var_name(g)),
        "bool" => RholangProc::BoolLit(bool::arbitrary(g)),
        "int" => RholangProc::IntLit(i64::arbitrary(g) % 1000),
        "string" => {
            let len = gen_range!(g, 0, 5);
            let s = (0..len)
                .map(|_| char::arbitrary(g))
                .filter(|c| !c.is_control())
                .collect();
            RholangProc::StringLit(s)
        }
        _ => unreachable!(),
    }
}

const RESERVED_KEYWORDS: &[&str] = &[
    "if", "else", "new", "in", "match", "contract", "select", "for", "let",
    "bundle", "bundle+", "bundle-", "bundle0", "true", "false", "Nil",
    "or", "and", "not", "matches",
];

fn gen_var_name(g: &mut Gen) -> String {
    let alphabet = "abcdefghijklmnopqrstuvwxyz";
    let len = gen_range!(g, 1, 5);
    loop {
        let name: String = (0..len)
            .map(|_| *g.choose(alphabet.chars().collect::<Vec<_>>().as_slice()).unwrap())
            .collect();
        if !RESERVED_KEYWORDS.contains(&name.as_str()) {
            return name;
        }
    }
}

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

impl RholangProc {
    pub fn to_code(&self) -> String {
        match self {
            RholangProc::Nil => "Nil".to_string(),
            RholangProc::Var(var) => var.clone(),
            RholangProc::Send { channel, inputs } => {
                let inputs_str = inputs.iter().map(|i| i.to_code()).collect::<Vec<_>>().join(", ");
                format!("{}!({})", channel, inputs_str)
            }
            RholangProc::SendSync { channel, inputs, cont } => {
                let inputs_str = inputs.iter().map(|i| i.to_code()).collect::<Vec<_>>().join(", ");
                format!("{}!?({}); {}", channel, inputs_str, cont.to_code())
            }
            RholangProc::Par { left, right } => {
                format!("{} | {}", left.to_code(), right.to_code())
            }
            RholangProc::New { decls, proc } => {
                let decls_str = decls.join(", ");
                format!("new {} in {{{}}}", decls_str, proc.to_code())
            },
            RholangProc::IfElse { condition, then, else_ } => {
                let mut code = format!(
                    "if ({}) {{{}}}",
                    condition.to_code(),
                    then.to_code()
                );
                if let Some(else_proc) = else_ {
                    code.push_str(&format!(" else {{{}}}", else_proc.to_code()));
                }
                code
            }
            RholangProc::Let { bindings, proc } => {
                let bindings_str = bindings
                    .iter()
                    .map(|(var, val)| format!("{} = {}", var, val.to_code()))
                    .collect::<Vec<_>>()
                    .join("; ");
                format!("let {} in {{{}}}", bindings_str, proc.to_code())
            }
            RholangProc::Bundle { is_read, is_write, proc } => {
                let prefix = match (*is_read, *is_write) {
                    (true, false) => "bundle-",
                    (false, true) => "bundle+",
                    (true, true) => "bundle",
                    (false, false) => "bundle0",
                };
                format!("{} {{{}}}", prefix, proc.to_code())
            }
            RholangProc::Match { target, cases } => {
                let cases_str = cases
                    .iter()
                    .map(|(pat, proc)| format!("{} => {{{}}}", pat.to_code(), proc.to_code()))
                    .collect::<Vec<_>>()
                    .join(" ");
                format!("match {} {{{}}}", target.to_code(), cases_str)
            }
            RholangProc::Choice { branches } => {
                let branches_str = branches
                    .iter()
                    .map(|b| {
                        if let RholangProc::For { pattern, channel, proc } = b {
                            format!("{} <- {} => {{{}}}", pattern.join(", "), channel, proc.to_code())
                        } else {
                            "/* invalid branch */".to_string()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                format!("select {{{}}}", branches_str)
            }
            RholangProc::Contract { name, params, body } => {
                let params_str = params.join(", ");
                format!("contract {}({}) = {{{}}}", name, params_str, body.to_code())
            }
            RholangProc::For { pattern, channel, proc } => {
                let pattern_str = pattern.join(", ");
                format!("for ({} <- {}) {{{}}}", pattern_str, channel, proc.to_code())
            }
            RholangProc::BinOp { op, left, right } => {
                format!("({} {} {})", left.to_code(), op, right.to_code())
            }
            RholangProc::UnaryOp { op, operand } => {
                format!("{} {}", op, operand.to_code())
            }
            RholangProc::StringLit(s) => format!("\"{}\"", escape_rholang_string(s)),
            RholangProc::BoolLit(b) => b.to_string(),
            RholangProc::IntLit(i) => i.to_string(),
            RholangProc::ListLit(elements) => {
                let elements_str = elements
                    .iter()
                    .map(|e| e.to_code())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("[{}]", elements_str)
            }
        }
    }
}
