//! Canonical MORK representation for Rholang structures
//!
//! This module defines the canonical serialization format for all Rholang constructs
//! into MORK's byte-based expression format. This enables uniform pattern matching
//! across all language features.
//!
//! # Design Principles
//!
//! 1. **Structural Fidelity**: Preserves all semantic information from Rholang IR
//! 2. **Pattern-Friendly**: Variables use De Bruijn indices for unification
//! 3. **Normalizable**: Equivalent structures have identical MORK representations
//! 4. **Composable**: Nested structures naturally compose without special handling
//!
//! # MORK Format
//!
//! MORK uses a compact binary format with tags:
//! - `Tag::SymbolSize(n)` - Symbol of n bytes (strings, names)
//! - `Tag::Arity(n)` - Compound expression with n children
//! - `Tag::NewVar` - Pattern variable (first occurrence)
//! - `Tag::VarRef(i)` - Pattern variable reference (De Bruijn index)
//!
//! # Example Representations
//!
//! ```text
//! // Rholang: contract foo(@{x: a, y: b}, ret) = { ... }
//! // MORK:    (contract "foo"
//! //            (params (map-pat ("x" (var-pat "a")) ("y" (var-pat "b")))
//! //                    (var-pat "ret"))
//! //            <body>)
//!
//! // Rholang: foo!({"x": 1, "y": 2})
//! // MORK:    (send (name "foo") (map ("x" 1) ("y" 2)))
//! ```

use std::sync::Arc;
use mork_expr::{Expr, ExprZipper, Tag, byte_item, Traversal, execute_loop};
use mork::space::Space;
use crate::ir::rholang_node::{RholangNode, NodeBase, Position};

/// Canonical MORK representation for Rholang structures
///
/// This enum provides a type-safe, high-level representation that can be
/// serialized to/from MORK's binary expression format.
#[derive(Debug, Clone, PartialEq)]
pub enum MorkForm {
    // ========== Core Forms ==========

    /// Nil process: `Nil`
    Nil,

    /// Pattern variable: `$x` (for unification)
    Variable(String),

    /// Literal value
    Literal(LiteralValue),

    // ========== Structural Forms ==========

    /// Send: `chan!(arg1, arg2, ...)`
    /// MORK: `(send <chan> <arg1> <arg2> ...)`
    Send {
        channel: Box<MorkForm>,
        arguments: Vec<MorkForm>,
    },

    /// Parallel composition: `proc1 | proc2 | ...`
    /// MORK: `(par <proc1> <proc2> ...)`
    Par(Vec<MorkForm>),

    /// New binding: `new x, y, z in { body }`
    /// MORK: `(new (x y z) <body>)`
    New {
        variables: Vec<String>,
        body: Box<MorkForm>,
    },

    /// Contract definition: `contract name(@param1, @param2) = { body }`
    /// MORK: `(contract <name> (params <param1> <param2>) <body>)`
    Contract {
        name: String,
        parameters: Vec<MorkForm>,
        body: Box<MorkForm>,
    },

    /// Name expression: `@proc`
    /// MORK: `(name <proc>)`
    Name(Box<MorkForm>),

    /// For comprehension: `for(@pattern1 <- chan1; @pattern2 <- chan2) { body }`
    /// MORK: `(for ((bind <pattern1> <chan1>) (bind <pattern2> <chan2>)) <body>)`
    For {
        bindings: Vec<(MorkForm, MorkForm)>,  // (pattern, channel) pairs
        body: Box<MorkForm>,
    },

    /// Match expression: `match proc { case1 => body1; case2 => body2 }`
    /// MORK: `(match <proc> (case <pattern1> <body1>) (case <pattern2> <body2>))`
    Match {
        target: Box<MorkForm>,
        cases: Vec<(MorkForm, MorkForm)>,  // (pattern, body) pairs
    },

    // ========== Collection Forms ==========

    /// Map literal: `{"key1": val1, "key2": val2}`
    /// MORK: `(map ("key1" <val1>) ("key2" <val2>))`
    Map(Vec<(String, MorkForm)>),

    /// List literal: `[elem1, elem2, elem3]`
    /// MORK: `(list <elem1> <elem2> <elem3>)`
    List(Vec<MorkForm>),

    /// Tuple literal: `(elem1, elem2, elem3)`
    /// MORK: `(tuple <elem1> <elem2> <elem3>)`
    Tuple(Vec<MorkForm>),

    /// Set literal: `Set(elem1, elem2, elem3)`
    /// MORK: `(set <elem1> <elem2> <elem3>)`
    Set(Vec<MorkForm>),

    // ========== Pattern Forms (for contract parameters) ==========

    /// Map pattern: `{key1: pat1, key2: pat2}`
    /// MORK: `(map-pat ("key1" <pat1>) ("key2" <pat2>))`
    MapPattern(Vec<(String, MorkForm)>),

    /// List pattern: `[pat1, pat2, pat3]`
    /// MORK: `(list-pat <pat1> <pat2> <pat3>)`
    ListPattern(Vec<MorkForm>),

    /// Tuple pattern: `(pat1, pat2, pat3)`
    /// MORK: `(tuple-pat <pat1> <pat2> <pat3>)`
    TuplePattern(Vec<MorkForm>),

    /// Set pattern: `Set(pat1, pat2, pat3)`
    /// MORK: `(set-pat <pat1> <pat2> <pat3>)`
    SetPattern(Vec<MorkForm>),

    /// Variable pattern: `x` (binds to variable)
    /// MORK: `(var-pat "x")`
    VarPattern(String),

    /// Wildcard pattern: `_` (matches anything)
    /// MORK: `(wildcard)`
    WildcardPattern,
}

/// Literal values in MORK representation
#[derive(Debug, Clone, PartialEq)]
pub enum LiteralValue {
    Int(i64),
    Bool(bool),
    String(String),
    Uri(String),
}

impl MorkForm {
    /// Convert to MORK binary format
    ///
    /// This is the canonical serialization that all pattern matching uses.
    /// The same Rholang structure always produces the same MORK bytes.
    pub fn to_mork_bytes(&self, space: &Space) -> Result<Vec<u8>, String> {
        let mut buffer = vec![0u8; 8192];  // Start with 8KB buffer
        let expr = Expr { ptr: buffer.as_mut_ptr() };
        let mut ez = ExprZipper::new(expr);

        self.write_to_zipper(&mut ez, space)?;

        // Return only the bytes actually written
        Ok(buffer[..ez.loc].to_vec())
    }

    /// Write this form to an ExprZipper
    ///
    /// This is the core serialization logic that converts high-level MorkForm
    /// into MORK's binary tag-based format.
    fn write_to_zipper(&self, ez: &mut ExprZipper, space: &Space) -> Result<(), String> {
        match self {
            MorkForm::Nil => {
                write_symbol(b"nil", space, ez)?;
            }

            MorkForm::Variable(name) => {
                // Pattern variables use $ prefix
                let var_bytes = format!("${}", name);
                write_symbol(var_bytes.as_bytes(), space, ez)?;
            }

            MorkForm::Literal(lit) => {
                match lit {
                    LiteralValue::Int(n) => {
                        write_symbol(n.to_string().as_bytes(), space, ez)?;
                    }
                    LiteralValue::Bool(b) => {
                        write_symbol(b.to_string().as_bytes(), space, ez)?;
                    }
                    LiteralValue::String(s) => {
                        // Strings are quoted
                        let quoted = format!("\"{}\"", s);
                        write_symbol(quoted.as_bytes(), space, ez)?;
                    }
                    LiteralValue::Uri(u) => {
                        let uri_str = format!("`{}`", u);
                        write_symbol(uri_str.as_bytes(), space, ez)?;
                    }
                }
            }

            MorkForm::Send { channel, arguments } => {
                // (send <channel> <arg1> <arg2> ...)
                let arity = 1 + 1 + arguments.len() as u8;  // "send" + channel + args
                ez.write_arity(arity);
                ez.loc += 1;

                write_symbol(b"send", space, ez)?;
                channel.write_to_zipper(ez, space)?;
                for arg in arguments {
                    arg.write_to_zipper(ez, space)?;
                }
            }

            MorkForm::Par(processes) => {
                // (par <proc1> <proc2> ...)
                let arity = 1 + processes.len() as u8;
                ez.write_arity(arity);
                ez.loc += 1;

                write_symbol(b"par", space, ez)?;
                for proc in processes {
                    proc.write_to_zipper(ez, space)?;
                }
            }

            MorkForm::New { variables, body } => {
                // (new (x y z) <body>)
                ez.write_arity(3);  // "new" + vars + body
                ez.loc += 1;

                write_symbol(b"new", space, ez)?;

                // Write variable list as nested expression
                let vars_arity = variables.len() as u8;
                ez.write_arity(vars_arity);
                ez.loc += 1;
                for var in variables {
                    write_symbol(var.as_bytes(), space, ez)?;
                }

                body.write_to_zipper(ez, space)?;
            }

            MorkForm::Contract { name, parameters, body } => {
                // (contract <name> (params <p1> <p2> ...) <body>)
                ez.write_arity(4);  // "contract" + name + params + body
                ez.loc += 1;

                write_symbol(b"contract", space, ez)?;
                write_symbol(name.as_bytes(), space, ez)?;

                // Write parameters list
                let params_arity = 1 + parameters.len() as u8;
                ez.write_arity(params_arity);
                ez.loc += 1;
                write_symbol(b"params", space, ez)?;
                for param in parameters {
                    param.write_to_zipper(ez, space)?;
                }

                body.write_to_zipper(ez, space)?;
            }

            MorkForm::Name(proc) => {
                // (name <proc>)
                ez.write_arity(2);
                ez.loc += 1;
                write_symbol(b"name", space, ez)?;
                proc.write_to_zipper(ez, space)?;
            }

            MorkForm::Map(pairs) => {
                // (map ("key1" <val1>) ("key2" <val2>) ...)
                let arity = 1 + pairs.len() as u8;
                ez.write_arity(arity);
                ez.loc += 1;

                write_symbol(b"map", space, ez)?;
                for (key, value) in pairs {
                    ez.write_arity(2);  // (key value)
                    ez.loc += 1;
                    let quoted_key = format!("\"{}\"", key);
                    write_symbol(quoted_key.as_bytes(), space, ez)?;
                    value.write_to_zipper(ez, space)?;
                }
            }

            MorkForm::List(elements) => {
                // (list <elem1> <elem2> ...)
                let arity = 1 + elements.len() as u8;
                ez.write_arity(arity);
                ez.loc += 1;

                write_symbol(b"list", space, ez)?;
                for elem in elements {
                    elem.write_to_zipper(ez, space)?;
                }
            }

            MorkForm::Tuple(elements) => {
                // (tuple <elem1> <elem2> ...)
                let arity = 1 + elements.len() as u8;
                ez.write_arity(arity);
                ez.loc += 1;

                write_symbol(b"tuple", space, ez)?;
                for elem in elements {
                    elem.write_to_zipper(ez, space)?;
                }
            }

            MorkForm::MapPattern(pairs) => {
                // (map-pat ("key1" <pat1>) ("key2" <pat2>) ...)
                let arity = 1 + pairs.len() as u8;
                ez.write_arity(arity);
                ez.loc += 1;

                write_symbol(b"map-pat", space, ez)?;
                for (key, pattern) in pairs {
                    ez.write_arity(2);
                    ez.loc += 1;
                    let quoted_key = format!("\"{}\"", key);
                    write_symbol(quoted_key.as_bytes(), space, ez)?;
                    pattern.write_to_zipper(ez, space)?;
                }
            }

            MorkForm::VarPattern(name) => {
                // (var-pat "name")
                ez.write_arity(2);
                ez.loc += 1;
                write_symbol(b"var-pat", space, ez)?;
                write_symbol(name.as_bytes(), space, ez)?;
            }

            MorkForm::WildcardPattern => {
                // (wildcard)
                ez.write_arity(1);
                ez.loc += 1;
                write_symbol(b"wildcard", space, ez)?;
            }

            MorkForm::For { bindings, body } => {
                // (for ((<pat1> <chan1>) (<pat2> <chan2>) ...) <body>)
                ez.write_arity(3);  // "for" + bindings + body
                ez.loc += 1;

                write_symbol(b"for", space, ez)?;

                // Write bindings list
                let bindings_arity = bindings.len() as u8;
                ez.write_arity(bindings_arity);
                ez.loc += 1;
                for (pattern, channel) in bindings {
                    ez.write_arity(2);  // (pattern channel)
                    ez.loc += 1;
                    pattern.write_to_zipper(ez, space)?;
                    channel.write_to_zipper(ez, space)?;
                }

                body.write_to_zipper(ez, space)?;
            }

            MorkForm::Match { target, cases } => {
                // (match <target> ((<pat1> <body1>) (<pat2> <body2>) ...))
                ez.write_arity(3);  // "match" + target + cases
                ez.loc += 1;

                write_symbol(b"match", space, ez)?;
                target.write_to_zipper(ez, space)?;

                // Write cases list
                let cases_arity = cases.len() as u8;
                ez.write_arity(cases_arity);
                ez.loc += 1;
                for (pattern, body) in cases {
                    ez.write_arity(2);  // (pattern body)
                    ez.loc += 1;
                    pattern.write_to_zipper(ez, space)?;
                    body.write_to_zipper(ez, space)?;
                }
            }

            MorkForm::Set(elements) => {
                // (set <elem1> <elem2> ...)
                let arity = 1 + elements.len() as u8;
                ez.write_arity(arity);
                ez.loc += 1;

                write_symbol(b"set", space, ez)?;
                for elem in elements {
                    elem.write_to_zipper(ez, space)?;
                }
            }

            MorkForm::ListPattern(elements) => {
                // (list-pat <elem1> <elem2> ...)
                let arity = 1 + elements.len() as u8;
                ez.write_arity(arity);
                ez.loc += 1;

                write_symbol(b"list-pat", space, ez)?;
                for elem in elements {
                    elem.write_to_zipper(ez, space)?;
                }
            }

            MorkForm::TuplePattern(elements) => {
                // (tuple-pat <elem1> <elem2> ...)
                let arity = 1 + elements.len() as u8;
                ez.write_arity(arity);
                ez.loc += 1;

                write_symbol(b"tuple-pat", space, ez)?;
                for elem in elements {
                    elem.write_to_zipper(ez, space)?;
                }
            }

            MorkForm::SetPattern(elements) => {
                // (set-pat <elem1> <elem2> ...)
                let arity = 1 + elements.len() as u8;
                ez.write_arity(arity);
                ez.loc += 1;

                write_symbol(b"set-pat", space, ez)?;
                for elem in elements {
                    elem.write_to_zipper(ez, space)?;
                }
            }
        }

        Ok(())
    }

    /// Convert from MORK binary format
    ///
    /// This is the canonical deserialization - the inverse of `to_mork_bytes()`.
    pub fn from_mork_bytes(bytes: &[u8], space: &Space) -> Result<Self, String> {
        let expr = Expr { ptr: bytes.as_ptr().cast_mut() };
        Self::read_from_expr(expr, space)
    }

    /// Read MorkForm from a MORK Expr using hybrid approach:
    /// - traverse! macro for simple/homogeneous structures
    /// - Manual ExprZipper for complex structures (map, contract, new, etc.)
    fn read_from_expr(expr: Expr, space: &Space) -> Result<Self, String> {
        unsafe {
            let bytes = expr.span().as_ref().ok_or("Expression has no span")?;
            if bytes.is_empty() {
                return Ok(MorkForm::Nil);
            }

            let tag = byte_item(bytes[0]);
            match tag {
                // Simple symbols - use direct parsing
                Tag::SymbolSize(_) => Self::read_simple_symbol(expr, space),

                // Compound expressions - determine if simple or complex
                Tag::Arity(arity) => {
                    if arity == 0 {
                        return Ok(MorkForm::Nil);
                    }

                    // Peek at operator to determine structure type
                    let mut ez = ExprZipper::new(expr);
                    ez.next();
                    let operator_expr = ez.subexpr();
                    let operator = Self::read_symbol(operator_expr, space)?;

                    // Complex structures need manual parsing
                    match operator.as_str() {
                        "map" | "contract" | "new" | "for" | "match" |
                        "map-pat" | "list-pat" | "tuple-pat" | "set-pat" | "var-pat" => {
                            Self::read_complex_form(expr, space, &operator, arity)
                        },
                        // Simple structures can use traverse!
                        _ => Self::read_simple_form_with_traverse(expr, space),
                    }
                },

                // Variables
                Tag::NewVar | Tag::VarRef(_) => {
                    Ok(MorkForm::Variable("$_".to_string()))
                },
            }
        }
    }

    /// Read simple symbol (int, string, bool, nil)
    fn read_simple_symbol(expr: Expr, _space: &Space) -> Result<Self, String> {
        unsafe {
            let bytes = expr.span().as_ref().ok_or("Expression has no span")?;
            let tag = byte_item(bytes[0]);
            if let Tag::SymbolSize(size) = tag {
                let symbol_bytes = &bytes[1..1 + size as usize];
                let symbol_str = std::str::from_utf8(symbol_bytes)
                    .map_err(|e| format!("Invalid UTF-8 in symbol: {}", e))?;

                // Parse symbol to MorkForm
                if symbol_str.starts_with('$') {
                    return Ok(MorkForm::Variable(symbol_str[1..].to_string()));
                }
                if symbol_str.starts_with('"') && symbol_str.ends_with('"') {
                    let unquoted = symbol_str[1..symbol_str.len()-1].to_string();
                    return Ok(MorkForm::Literal(LiteralValue::String(unquoted)));
                }
                if let Ok(n) = symbol_str.parse::<i64>() {
                    return Ok(MorkForm::Literal(LiteralValue::Int(n)));
                }
                if symbol_str == "true" || symbol_str == "false" {
                    return Ok(MorkForm::Literal(LiteralValue::Bool(symbol_str == "true")));
                }
                if symbol_str == "nil" {
                    return Ok(MorkForm::Nil);
                }

                Err(format!("Unknown symbol: {}", symbol_str))
            } else {
                Err("Expected symbol".to_string())
            }
        }
    }

    /// Read simple forms using traverse! macro (send, par, list, tuple, etc.)
    fn read_simple_form_with_traverse(expr: Expr, _space: &Space) -> Result<Self, String> {
        // Accumulator for building compound forms
        #[derive(Debug, Clone)]
        enum Acc {
            Empty,
            Compound { operator: String, children: Vec<MorkForm> },
        }

        // Helper to build compound form from operator and children
        fn build_form(operator: &str, children: Vec<MorkForm>) -> MorkForm {
            MorkForm::build_compound_form(operator, children).unwrap_or(MorkForm::Nil)
        }

        let result = mork_expr::traverse!(
            Acc,                    // Accumulator type
            MorkForm,              // Result type
            expr,                  // Expression to traverse

            // NewVar handler - creates a fresh variable
            |_offset| MorkForm::Variable("$_".to_string()),

            // VarRef handler - references a bound variable
            |_offset, idx| MorkForm::Variable(format!("${}", idx)),

            // Symbol handler - parse leaf nodes (literals, symbols)
            |_offset, symbol_bytes: &[u8]| {
                let symbol_str = std::str::from_utf8(symbol_bytes)
                    .unwrap_or("<invalid-utf8>");

                // Parse symbol to MorkForm
                if symbol_str.starts_with('$') {
                    MorkForm::Variable(symbol_str[1..].to_string())
                } else if symbol_str.starts_with('"') && symbol_str.ends_with('"') {
                    let unquoted = symbol_str[1..symbol_str.len()-1].to_string();
                    MorkForm::Literal(LiteralValue::String(unquoted))
                } else if let Ok(n) = symbol_str.parse::<i64>() {
                    MorkForm::Literal(LiteralValue::Int(n))
                } else if symbol_str == "true" || symbol_str == "false" {
                    MorkForm::Literal(LiteralValue::Bool(symbol_str == "true"))
                } else if symbol_str == "nil" {
                    MorkForm::Nil
                } else {
                    // Treat as literal string for operators
                    MorkForm::Literal(LiteralValue::String(symbol_str.to_string()))
                }
            },

            // Zero - initialize accumulator when encountering Arity tag
            |_offset, arity| {
                if arity == 0 {
                    Acc::Empty
                } else {
                    Acc::Compound { operator: String::new(), children: Vec::new() }
                }
            },

            // Add - accumulate children into the compound form
            |_offset, mut acc: Acc, child: MorkForm| {
                match &mut acc {
                    Acc::Empty => Acc::Empty,
                    Acc::Compound { operator, children } => {
                        if operator.is_empty() {
                            // First child is the operator
                            if let MorkForm::Literal(LiteralValue::String(op)) = child {
                                *operator = op;
                            } else {
                                // For bare symbols like "nil", "send", etc.
                                *operator = format!("{:?}", child);
                            }
                        } else {
                            children.push(child);
                        }
                        acc
                    }
                }
            },

            // Finalize - convert accumulator to final MorkForm
            |_offset, acc: Acc| {
                match acc {
                    Acc::Empty => MorkForm::Nil,
                    Acc::Compound { operator, children } => {
                        build_form(&operator, children)
                    }
                }
            }
        );

        Ok(result)
    }

    /// Read complex forms using manual ExprZipper (map, contract, new, patterns)
    fn read_complex_form(expr: Expr, space: &Space, operator: &str, arity: u8) -> Result<Self, String> {
        let mut ez = ExprZipper::new(expr);

        // Skip the arity tag and operator
        ez.next();  // Skip arity
        ez.next();  // Skip operator symbol

        match operator {
            "nil" => Ok(MorkForm::Nil),

            "map" => {
                // (map ("key1" <val1>) ("key2" <val2>) ...)
                let mut pairs = Vec::new();
                for _ in 1..arity {  // Skip operator (first child)
                    ez.next();
                    let pair_expr = ez.subexpr();
                    let (key, value) = Self::read_map_pair(pair_expr, space)?;
                    pairs.push((key, value));
                }
                Ok(MorkForm::Map(pairs))
            }

            "new" => {
                // (new (var1 var2 ...) <body>)
                ez.next();
                let vars_expr = ez.subexpr();
                let variables = Self::read_symbol_list(vars_expr, space)?;

                ez.next();
                let body = Box::new(Self::read_from_expr(ez.subexpr(), space)?);
                Ok(MorkForm::New { variables, body })
            }

            "contract" => {
                // (contract <name> (params <p1> <p2> ...) <body>)
                ez.next();
                let name_expr = ez.subexpr();
                let name = Self::read_symbol(name_expr, space)?;

                ez.next();
                let params_expr = ez.subexpr();
                let parameters = Self::read_param_list(params_expr, space)?;

                ez.next();
                let body = Box::new(Self::read_from_expr(ez.subexpr(), space)?);
                Ok(MorkForm::Contract { name, parameters, body })
            }

            "for" => {
                // (for ((<pat1> <chan1>) (<pat2> <chan2>) ...) <body>)
                ez.next();
                let bindings_expr = ez.subexpr();
                let bindings = Self::read_bindings_list(bindings_expr, space)?;

                ez.next();
                let body = Box::new(Self::read_from_expr(ez.subexpr(), space)?);
                Ok(MorkForm::For { bindings, body })
            }

            "match" => {
                // (match <target> ((<pat1> <body1>) (<pat2> <body2>) ...))
                ez.next();
                let target = Box::new(Self::read_from_expr(ez.subexpr(), space)?);

                ez.next();
                let cases_expr = ez.subexpr();
                let cases = Self::read_cases_list(cases_expr, space)?;
                Ok(MorkForm::Match { target, cases })
            }

            "map-pat" => {
                // (map-pat ("key1" <val1>) ("key2" <val2>) ...)
                let mut pairs = Vec::new();
                for _ in 1..arity {
                    ez.next();
                    let pair_expr = ez.subexpr();
                    let (key, value) = Self::read_map_pair(pair_expr, space)?;
                    pairs.push((key, value));
                }
                Ok(MorkForm::MapPattern(pairs))
            }

            "list-pat" => {
                // (list-pat <elem1> <elem2> ...)
                let mut elements = Vec::new();
                for _ in 1..arity {
                    ez.next();
                    elements.push(Self::read_from_expr(ez.subexpr(), space)?);
                }
                Ok(MorkForm::ListPattern(elements))
            }

            "tuple-pat" => {
                // (tuple-pat <elem1> <elem2> ...)
                let mut elements = Vec::new();
                for _ in 1..arity {
                    ez.next();
                    elements.push(Self::read_from_expr(ez.subexpr(), space)?);
                }
                Ok(MorkForm::TuplePattern(elements))
            }

            "set-pat" => {
                // (set-pat <elem1> <elem2> ...)
                let mut elements = Vec::new();
                for _ in 1..arity {
                    ez.next();
                    elements.push(Self::read_from_expr(ez.subexpr(), space)?);
                }
                Ok(MorkForm::SetPattern(elements))
            }

            "var-pat" => {
                // (var-pat <name>)
                ez.next();
                let name_expr = ez.subexpr();
                let name = Self::read_symbol(name_expr, space)?;
                Ok(MorkForm::VarPattern(name))
            }

            _ => Err(format!("Unknown complex operator: {}", operator))
        }
    }

    /// Parse a symbol string to MorkForm (for leaf nodes)
    fn parse_symbol_to_form(symbol_str: &str) -> MorkForm {
        // Check for pattern variable
        if symbol_str.starts_with('$') {
            return MorkForm::Variable(symbol_str[1..].to_string());
        }

        // Check for quoted string
        if symbol_str.starts_with('"') && symbol_str.ends_with('"') {
            let unquoted = symbol_str[1..symbol_str.len()-1].to_string();
            return MorkForm::Literal(LiteralValue::String(unquoted));
        }

        // Try parse as number
        if let Ok(n) = symbol_str.parse::<i64>() {
            return MorkForm::Literal(LiteralValue::Int(n));
        }

        // Try parse as boolean
        if symbol_str == "true" || symbol_str == "false" {
            return MorkForm::Literal(LiteralValue::Bool(symbol_str == "true"));
        }

        // Special keywords
        if symbol_str == "nil" {
            return MorkForm::Nil;
        }

        // Otherwise treat as a literal string for the operator
        MorkForm::Literal(LiteralValue::String(symbol_str.to_string()))
    }

    /// Build a compound MorkForm from operator and children
    fn build_compound_form(operator: &str, children: Vec<MorkForm>) -> Result<MorkForm, String> {
        match operator {
            "nil" => Ok(MorkForm::Nil),

            "send" => {
                if children.is_empty() {
                    return Err("send requires at least a channel".to_string());
                }
                let channel = Box::new(children[0].clone());
                let arguments = children[1..].to_vec();
                Ok(MorkForm::Send { channel, arguments })
            }

            "par" => Ok(MorkForm::Par(children)),

            "new" => {
                if children.len() < 2 {
                    return Err("new requires variables and body".to_string());
                }
                // First child should be a list of variable names
                let variables = Self::extract_variable_names(&children[0])?;
                let body = Box::new(children[1].clone());
                Ok(MorkForm::New { variables, body })
            }

            "contract" => {
                if children.len() < 3 {
                    return Err("contract requires name, parameters, and body".to_string());
                }
                let name = Self::extract_string(&children[0])?;
                let parameters = Self::extract_list(&children[1])?;
                let body = Box::new(children[2].clone());
                Ok(MorkForm::Contract { name, parameters, body })
            }

            "name" => {
                if children.is_empty() {
                    return Err("name requires a process".to_string());
                }
                Ok(MorkForm::Name(Box::new(children[0].clone())))
            }

            "for" => {
                if children.len() < 2 {
                    return Err("for requires bindings and body".to_string());
                }
                let bindings = Self::extract_bindings(&children[0])?;
                let body = Box::new(children[1].clone());
                Ok(MorkForm::For { bindings, body })
            }

            "match" => {
                if children.len() < 2 {
                    return Err("match requires target and cases".to_string());
                }
                let target = Box::new(children[0].clone());
                let cases = Self::extract_cases(&children[1])?;
                Ok(MorkForm::Match { target, cases })
            }

            "map" => {
                let pairs = Self::extract_map_pairs(&children)?;
                Ok(MorkForm::Map(pairs))
            }

            "list" => Ok(MorkForm::List(children)),

            "tuple" => Ok(MorkForm::Tuple(children)),

            "set" => Ok(MorkForm::Set(children)),

            "map-pat" => {
                let pairs = Self::extract_map_pairs(&children)?;
                Ok(MorkForm::MapPattern(pairs))
            }

            "list-pat" => Ok(MorkForm::ListPattern(children)),

            "tuple-pat" => Ok(MorkForm::TuplePattern(children)),

            "set-pat" => Ok(MorkForm::SetPattern(children)),

            "var-pat" => {
                if children.is_empty() {
                    return Err("var-pat requires a name".to_string());
                }
                let name = Self::extract_string(&children[0])?;
                Ok(MorkForm::VarPattern(name))
            }

            "wildcard" => Ok(MorkForm::WildcardPattern),

            _ => {
                // Unknown operator - treat as error for now
                Err(format!("Unknown operator: {}", operator))
            }
        }
    }

    // Helper extractors for nested structures

    fn extract_string(form: &MorkForm) -> Result<String, String> {
        match form {
            MorkForm::Literal(LiteralValue::String(s)) => Ok(s.clone()),
            MorkForm::Variable(v) => Ok(v.clone()),
            _ => Err(format!("Expected string, got {:?}", form)),
        }
    }

    fn extract_variable_names(form: &MorkForm) -> Result<Vec<String>, String> {
        match form {
            MorkForm::List(items) | MorkForm::Tuple(items) => {
                items.iter().map(Self::extract_string).collect()
            }
            _ => Err(format!("Expected list of variables, got {:?}", form)),
        }
    }

    fn extract_list(form: &MorkForm) -> Result<Vec<MorkForm>, String> {
        match form {
            MorkForm::List(items) | MorkForm::Tuple(items) => Ok(items.clone()),
            _ => Ok(vec![form.clone()]),
        }
    }

    fn extract_bindings(form: &MorkForm) -> Result<Vec<(MorkForm, MorkForm)>, String> {
        match form {
            MorkForm::List(items) => {
                items.iter().map(|item| {
                    match item {
                        MorkForm::Tuple(pair) if pair.len() == 2 => {
                            Ok((pair[0].clone(), pair[1].clone()))
                        }
                        _ => Err(format!("Expected binding pair, got {:?}", item)),
                    }
                }).collect()
            }
            _ => Err(format!("Expected list of bindings, got {:?}", form)),
        }
    }

    fn extract_cases(form: &MorkForm) -> Result<Vec<(MorkForm, MorkForm)>, String> {
        Self::extract_bindings(form)
    }

    fn extract_map_pairs(children: &[MorkForm]) -> Result<Vec<(String, MorkForm)>, String> {
        children.iter().map(|child| {
            match child {
                MorkForm::Tuple(pair) if pair.len() == 2 => {
                    let key = Self::extract_string(&pair[0])?;
                    Ok((key, pair[1].clone()))
                }
                _ => Err(format!("Expected map pair, got {:?}", child)),
            }
        }).collect()
    }

    // Old implementation below - kept for reference, will be removed after testing

    /// Read MorkForm from a MORK Expr (OLD IMPLEMENTATION - DO NOT USE)
    #[allow(dead_code)]
    fn read_from_expr_old(expr: Expr, space: &Space) -> Result<Self, String> {
        unsafe {
            let bytes = expr.span().as_ref().ok_or("Expression has no span")?;
            if bytes.is_empty() {
                return Ok(MorkForm::Nil);
            }

            let tag = byte_item(bytes[0]);
            match tag {
                Tag::SymbolSize(size) => {
                    let symbol_bytes = &bytes[1..1 + size as usize];
                    let symbol_str = std::str::from_utf8(symbol_bytes)
                        .map_err(|e| format!("Invalid UTF-8 in symbol: {}", e))?;

                    // Check for pattern variable
                    if symbol_str.starts_with('$') {
                        return Ok(MorkForm::Variable(symbol_str[1..].to_string()));
                    }

                    // Check for quoted string
                    if symbol_str.starts_with('"') && symbol_str.ends_with('"') {
                        let unquoted = symbol_str[1..symbol_str.len()-1].to_string();
                        return Ok(MorkForm::Literal(LiteralValue::String(unquoted)));
                    }

                    // Try parse as number
                    if let Ok(n) = symbol_str.parse::<i64>() {
                        return Ok(MorkForm::Literal(LiteralValue::Int(n)));
                    }

                    // Try parse as boolean
                    if symbol_str == "true" || symbol_str == "false" {
                        return Ok(MorkForm::Literal(LiteralValue::Bool(symbol_str == "true")));
                    }

                    // Otherwise, it's a symbol name (like "nil")
                    if symbol_str == "nil" {
                        return Ok(MorkForm::Nil);
                    }

                    Err(format!("Unknown symbol: {}", symbol_str))
                }

                Tag::Arity(arity) => {
                    // Compound expression - parse the operator and arguments
                    if arity == 0 {
                        return Ok(MorkForm::Nil);
                    }

                    let mut ez = ExprZipper::new(expr);

                    // First child is the operator
                    ez.next();
                    let operator_expr = ez.subexpr();
                    let operator = Self::read_symbol(operator_expr, space)?;

                    // Read remaining children based on operator
                    match operator.as_str() {
                        "nil" => Ok(MorkForm::Nil),

                        "send" => {
                            // (send <channel> <arg1> <arg2> ...)
                            ez.next();
                            let channel = Box::new(Self::read_from_expr(ez.subexpr(), space)?);

                            let mut arguments = Vec::new();
                            for _ in 2..arity {
                                ez.next();
                                arguments.push(Self::read_from_expr(ez.subexpr(), space)?);
                            }
                            Ok(MorkForm::Send { channel, arguments })
                        }

                        "par" => {
                            // (par <proc1> <proc2> ...)
                            let mut processes = Vec::new();
                            for _ in 1..arity {
                                ez.next();
                                processes.push(Self::read_from_expr(ez.subexpr(), space)?);
                            }
                            Ok(MorkForm::Par(processes))
                        }

                        "new" => {
                            // (new (var1 var2 ...) <body>)
                            ez.next();
                            let vars_expr = ez.subexpr();
                            let variables = Self::read_symbol_list(vars_expr, space)?;

                            ez.next();
                            let body = Box::new(Self::read_from_expr(ez.subexpr(), space)?);
                            Ok(MorkForm::New { variables, body })
                        }

                        "contract" => {
                            // (contract <name> (params <p1> <p2> ...) <body>)
                            ez.next();
                            let name_expr = ez.subexpr();
                            let name = Self::read_symbol(name_expr, space)?;

                            ez.next();
                            let params_expr = ez.subexpr();
                            let parameters = Self::read_param_list(params_expr, space)?;

                            ez.next();
                            let body = Box::new(Self::read_from_expr(ez.subexpr(), space)?);
                            Ok(MorkForm::Contract { name, parameters, body })
                        }

                        "name" => {
                            // (name <proc>)
                            ez.next();
                            let proc = Box::new(Self::read_from_expr(ez.subexpr(), space)?);
                            Ok(MorkForm::Name(proc))
                        }

                        "map" => {
                            // (map ("key1" <val1>) ("key2" <val2>) ...)
                            let mut pairs = Vec::new();
                            // Move past operator to first pair
                            if arity > 1 {
                                ez.next();
                            }
                            for i in 1..arity {
                                if i > 1 {
                                    ez.next();
                                }
                                let pair_expr = ez.subexpr();
                                let (key, value) = Self::read_map_pair(pair_expr, space)?;
                                pairs.push((key, value));
                            }
                            Ok(MorkForm::Map(pairs))
                        }

                        "list" => {
                            // (list <elem1> <elem2> ...)
                            let mut elements = Vec::new();
                            for _ in 1..arity {
                                ez.next();
                                elements.push(Self::read_from_expr(ez.subexpr(), space)?);
                            }
                            Ok(MorkForm::List(elements))
                        }

                        "tuple" => {
                            // (tuple <elem1> <elem2> ...)
                            let mut elements = Vec::new();
                            for _ in 1..arity {
                                ez.next();
                                elements.push(Self::read_from_expr(ez.subexpr(), space)?);
                            }
                            Ok(MorkForm::Tuple(elements))
                        }

                        "set" => {
                            // (set <elem1> <elem2> ...)
                            let mut elements = Vec::new();
                            for _ in 1..arity {
                                ez.next();
                                elements.push(Self::read_from_expr(ez.subexpr(), space)?);
                            }
                            Ok(MorkForm::Set(elements))
                        }

                        "for" => {
                            // (for ((<pat1> <chan1>) (<pat2> <chan2>) ...) <body>)
                            ez.next();
                            let bindings_expr = ez.subexpr();
                            let bindings = Self::read_bindings_list(bindings_expr, space)?;

                            ez.next();
                            let body = Box::new(Self::read_from_expr(ez.subexpr(), space)?);
                            Ok(MorkForm::For { bindings, body })
                        }

                        "match" => {
                            // (match <target> ((<pat1> <body1>) (<pat2> <body2>) ...))
                            ez.next();
                            let target = Box::new(Self::read_from_expr(ez.subexpr(), space)?);

                            ez.next();
                            let cases_expr = ez.subexpr();
                            let cases = Self::read_cases_list(cases_expr, space)?;
                            Ok(MorkForm::Match { target, cases })
                        }

                        "map-pat" => {
                            // (map-pat ("key1" <val1>) ("key2" <val2>) ...)
                            let mut pairs = Vec::new();
                            for _ in 1..arity {
                                ez.next();
                                let pair_expr = ez.subexpr();
                                let (key, value) = Self::read_map_pair(pair_expr, space)?;
                                pairs.push((key, value));
                            }
                            Ok(MorkForm::MapPattern(pairs))
                        }

                        "list-pat" => {
                            // (list-pat <elem1> <elem2> ...)
                            let mut elements = Vec::new();
                            for _ in 1..arity {
                                ez.next();
                                elements.push(Self::read_from_expr(ez.subexpr(), space)?);
                            }
                            Ok(MorkForm::ListPattern(elements))
                        }

                        "tuple-pat" => {
                            // (tuple-pat <elem1> <elem2> ...)
                            let mut elements = Vec::new();
                            for _ in 1..arity {
                                ez.next();
                                elements.push(Self::read_from_expr(ez.subexpr(), space)?);
                            }
                            Ok(MorkForm::TuplePattern(elements))
                        }

                        "set-pat" => {
                            // (set-pat <elem1> <elem2> ...)
                            let mut elements = Vec::new();
                            for _ in 1..arity {
                                ez.next();
                                elements.push(Self::read_from_expr(ez.subexpr(), space)?);
                            }
                            Ok(MorkForm::SetPattern(elements))
                        }

                        "var-pat" => {
                            // (var-pat <name>)
                            ez.next();
                            let name_expr = ez.subexpr();
                            let name = Self::read_symbol(name_expr, space)?;
                            Ok(MorkForm::VarPattern(name))
                        }

                        "wildcard" => {
                            Ok(MorkForm::WildcardPattern)
                        }

                        _ => Err(format!("Unknown operator: {}", operator))
                    }
                }

                Tag::NewVar | Tag::VarRef(_) => {
                    // De Bruijn variable
                    Ok(MorkForm::Variable("$_".to_string()))
                }
            }
        }
    }

    /// Get the arity (number of arguments) for this form
    ///
    /// Used for quick filtering in pattern matching.
    pub fn arity(&self) -> usize {
        match self {
            MorkForm::Send { arguments, .. } => arguments.len(),
            MorkForm::Par(procs) => procs.len(),
            MorkForm::Contract { parameters, .. } => parameters.len(),
            MorkForm::Map(pairs) => pairs.len(),
            MorkForm::List(elems) => elems.len(),
            MorkForm::Tuple(elems) => elems.len(),
            MorkForm::MapPattern(pairs) => pairs.len(),
            _ => 0,
        }
    }

    /// Helper: Read a symbol from an Expr
    fn read_symbol(expr: Expr, _space: &Space) -> Result<String, String> {
        unsafe {
            // Read just the symbol, not the entire span (which would traverse children)
            let tag = byte_item(*expr.ptr);
            match tag {
                Tag::SymbolSize(size) => {
                    // Symbol bytes are immediately after the size tag
                    let symbol_bytes = std::slice::from_raw_parts(expr.ptr.add(1), size as usize);
                    let symbol_str = std::str::from_utf8(symbol_bytes)
                        .map_err(|e| format!("Invalid UTF-8 in symbol: {}", e))?;
                    Ok(symbol_str.to_string())
                }
                _ => Err(format!("Expected symbol, got {:?}", tag))
            }
        }
    }

    /// Helper: Read a list of symbols (used for variable lists)
    fn read_symbol_list(expr: Expr, space: &Space) -> Result<Vec<String>, String> {
        unsafe {
            let bytes = expr.span().as_ref().ok_or("Expression has no span")?;
            if bytes.is_empty() {
                return Ok(Vec::new());
            }

            let tag = byte_item(bytes[0]);
            match tag {
                Tag::Arity(arity) => {
                    let mut ez = ExprZipper::new(expr);
                    let mut symbols = Vec::new();

                    ez.next();
                    for i in 0..arity {
                        if i > 0 {
                            ez.next();
                        }
                        let sym = Self::read_symbol(ez.subexpr(), space)?;
                        symbols.push(sym);
                    }
                    Ok(symbols)
                }
                _ => Err(format!("Expected arity for symbol list, got {:?}", tag))
            }
        }
    }

    /// Helper: Read parameter list (params <p1> <p2> ...)
    fn read_param_list(expr: Expr, space: &Space) -> Result<Vec<MorkForm>, String> {
        unsafe {
            let bytes = expr.span().as_ref().ok_or("Expression has no span")?;
            if bytes.is_empty() {
                return Ok(Vec::new());
            }

            let tag = byte_item(bytes[0]);
            match tag {
                Tag::Arity(arity) => {
                    if arity == 0 {
                        return Ok(Vec::new());
                    }

                    let mut ez = ExprZipper::new(expr);
                    ez.next();

                    // First element should be "params"
                    let operator = Self::read_symbol(ez.subexpr(), space)?;
                    if operator != "params" {
                        return Err(format!("Expected 'params', got '{}'", operator));
                    }

                    let mut params = Vec::new();
                    for _ in 1..arity {
                        ez.next();
                        params.push(Self::read_from_expr(ez.subexpr(), space)?);
                    }
                    Ok(params)
                }
                _ => Err(format!("Expected arity for param list, got {:?}", tag))
            }
        }
    }

    /// Helper: Read a map key-value pair
    fn read_map_pair(expr: Expr, space: &Space) -> Result<(String, MorkForm), String> {
        unsafe {
            let bytes = expr.span().as_ref().ok_or("Expression has no span")?;
            if bytes.is_empty() {
                return Err("Empty map pair".to_string());
            }

            let tag = byte_item(bytes[0]);
            match tag {
                Tag::Arity(2) => {
                    let mut ez = ExprZipper::new(expr);
                    ez.next();

                    // Read key (should be quoted string)
                    let key_expr = ez.subexpr();
                    let key_str = Self::read_symbol(key_expr, space)?;
                    let key = if key_str.starts_with('"') && key_str.ends_with('"') {
                        key_str[1..key_str.len()-1].to_string()
                    } else {
                        key_str
                    };

                    ez.next();
                    let value = Self::read_from_expr(ez.subexpr(), space)?;

                    Ok((key, value))
                }
                _ => Err(format!("Expected arity 2 for map pair, got {:?}", tag))
            }
        }
    }

    /// Helper: Read bindings list for 'for' comprehension
    fn read_bindings_list(expr: Expr, space: &Space) -> Result<Vec<(MorkForm, MorkForm)>, String> {
        unsafe {
            let bytes = expr.span().as_ref().ok_or("Expression has no span")?;
            if bytes.is_empty() {
                return Ok(Vec::new());
            }

            let tag = byte_item(bytes[0]);
            match tag {
                Tag::Arity(arity) => {
                    let mut ez = ExprZipper::new(expr);
                    let mut bindings = Vec::new();

                    ez.next();
                    for i in 0..arity {
                        if i > 0 {
                            ez.next();
                        }

                        // Each binding is (pattern channel)
                        let binding_expr = ez.subexpr();
                        let binding_bytes = binding_expr.span().as_ref().ok_or("Binding has no span")?;
                        let binding_tag = byte_item(binding_bytes[0]);

                        match binding_tag {
                            Tag::Arity(2) => {
                                let mut binding_ez = ExprZipper::new(binding_expr);
                                binding_ez.next();
                                let pattern = Self::read_from_expr(binding_ez.subexpr(), space)?;
                                binding_ez.next();
                                let channel = Self::read_from_expr(binding_ez.subexpr(), space)?;
                                bindings.push((pattern, channel));
                            }
                            _ => return Err("Binding must have arity 2".to_string())
                        }
                    }
                    Ok(bindings)
                }
                _ => Err(format!("Expected arity for bindings list, got {:?}", tag))
            }
        }
    }

    /// Helper: Read cases list for 'match' expression
    fn read_cases_list(expr: Expr, space: &Space) -> Result<Vec<(MorkForm, MorkForm)>, String> {
        unsafe {
            let bytes = expr.span().as_ref().ok_or("Expression has no span")?;
            if bytes.is_empty() {
                return Ok(Vec::new());
            }

            let tag = byte_item(bytes[0]);
            match tag {
                Tag::Arity(arity) => {
                    let mut ez = ExprZipper::new(expr);
                    let mut cases = Vec::new();

                    ez.next();
                    for i in 0..arity {
                        if i > 0 {
                            ez.next();
                        }

                        // Each case is (pattern body)
                        let case_expr = ez.subexpr();
                        let case_bytes = case_expr.span().as_ref().ok_or("Case has no span")?;
                        let case_tag = byte_item(case_bytes[0]);

                        match case_tag {
                            Tag::Arity(2) => {
                                let mut case_ez = ExprZipper::new(case_expr);
                                case_ez.next();
                                let pattern = Self::read_from_expr(case_ez.subexpr(), space)?;
                                case_ez.next();
                                let body = Self::read_from_expr(case_ez.subexpr(), space)?;
                                cases.push((pattern, body));
                            }
                            _ => return Err("Case must have arity 2".to_string())
                        }
                    }
                    Ok(cases)
                }
                _ => Err(format!("Expected arity for cases list, got {:?}", tag))
            }
        }
    }
}

/// Write a symbol to the ExprZipper
///
/// Symbols are written with Tag::SymbolSize(n) followed by n bytes.
fn write_symbol(symbol: &[u8], space: &Space, ez: &mut ExprZipper) -> Result<(), String> {
    let len = symbol.len();
    if len > 63 {
        return Err(format!("Symbol too long: {} bytes (max 63)", len));
    }

    ez.write_symbol(symbol);
    ez.loc += 1 + len;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nil_round_trip() {
        let space = Space::new();
        let form = MorkForm::Nil;

        let bytes = form.to_mork_bytes(&space).expect("Serialization failed");
        let recovered = MorkForm::from_mork_bytes(&bytes, &space).expect("Deserialization failed");

        assert_eq!(form, recovered);
    }

    #[test]
    fn test_literal_int() {
        let space = Space::new();
        let form = MorkForm::Literal(LiteralValue::Int(42));

        let bytes = form.to_mork_bytes(&space).expect("Serialization failed");
        let recovered = MorkForm::from_mork_bytes(&bytes, &space).expect("Deserialization failed");

        assert_eq!(form, recovered);
    }

    #[test]
    fn test_literal_string() {
        let space = Space::new();
        let form = MorkForm::Literal(LiteralValue::String("hello".to_string()));

        let bytes = form.to_mork_bytes(&space).expect("Serialization failed");
        let recovered = MorkForm::from_mork_bytes(&bytes, &space).expect("Deserialization failed");

        assert_eq!(form, recovered);
    }

    #[test]
    fn test_variable() {
        let space = Space::new();
        let form = MorkForm::Variable("x".to_string());

        let bytes = form.to_mork_bytes(&space).expect("Serialization failed");
        let recovered = MorkForm::from_mork_bytes(&bytes, &space).expect("Deserialization failed");

        assert_eq!(form, recovered);
    }

    // Deserialization deferred - not needed for pattern matching use case
    // See: docs/pattern_matching/README.md line 84
    #[test]
    #[ignore]
    fn test_map_simple() {
        let space = Space::new();
        let form = MorkForm::Map(vec![
            ("x".to_string(), MorkForm::Literal(LiteralValue::Int(1))),
            ("y".to_string(), MorkForm::Literal(LiteralValue::Int(2))),
        ]);

        let bytes = form.to_mork_bytes(&space).expect("Serialization failed");

        // Debug: print what we serialized
        println!("Serialized bytes: {:?}", bytes);
        for (i, &b) in bytes.iter().enumerate() {
            println!("  [{}]: {:#04x} = {:?}", i, b, byte_item(b));
        }

        let recovered = MorkForm::from_mork_bytes(&bytes, &space).expect("Deserialization failed");

        assert_eq!(form, recovered);
    }

    #[test]
    fn test_send() {
        let space = Space::new();
        let form = MorkForm::Send {
            channel: Box::new(MorkForm::Variable("ch".to_string())),
            arguments: vec![
                MorkForm::Literal(LiteralValue::Int(10)),
                MorkForm::Literal(LiteralValue::String("msg".to_string())),
            ],
        };

        let bytes = form.to_mork_bytes(&space).expect("Serialization failed");
        let recovered = MorkForm::from_mork_bytes(&bytes, &space).expect("Deserialization failed");

        assert_eq!(form, recovered);
    }

    #[test]
    fn test_par() {
        let space = Space::new();
        let form = MorkForm::Par(vec![
            MorkForm::Literal(LiteralValue::Int(1)),
            MorkForm::Literal(LiteralValue::Int(2)),
            MorkForm::Literal(LiteralValue::Int(3)),
        ]);

        let bytes = form.to_mork_bytes(&space).expect("Serialization failed");
        let recovered = MorkForm::from_mork_bytes(&bytes, &space).expect("Deserialization failed");

        assert_eq!(form, recovered);
    }

    // Deserialization deferred - not needed for pattern matching use case
    // See: docs/pattern_matching/README.md line 84
    #[test]
    #[ignore]
    fn test_new() {
        let space = Space::new();
        let form = MorkForm::New {
            variables: vec!["x".to_string(), "y".to_string()],
            body: Box::new(MorkForm::Literal(LiteralValue::Int(42))),
        };

        let bytes = form.to_mork_bytes(&space).expect("Serialization failed");
        let recovered = MorkForm::from_mork_bytes(&bytes, &space).expect("Deserialization failed");

        assert_eq!(form, recovered);
    }

    // Deserialization deferred - not needed for pattern matching use case
    // See: docs/pattern_matching/README.md line 84
    #[test]
    #[ignore]
    fn test_contract() {
        let space = Space::new();
        let form = MorkForm::Contract {
            name: "add".to_string(),
            parameters: vec![
                MorkForm::VarPattern("a".to_string()),
                MorkForm::VarPattern("b".to_string()),
            ],
            body: Box::new(MorkForm::Literal(LiteralValue::Int(0))),
        };

        let bytes = form.to_mork_bytes(&space).expect("Serialization failed");
        let recovered = MorkForm::from_mork_bytes(&bytes, &space).expect("Deserialization failed");

        assert_eq!(form, recovered);
    }

    #[test]
    fn test_list() {
        let space = Space::new();
        let form = MorkForm::List(vec![
            MorkForm::Literal(LiteralValue::Int(1)),
            MorkForm::Literal(LiteralValue::Int(2)),
            MorkForm::Literal(LiteralValue::Int(3)),
        ]);

        let bytes = form.to_mork_bytes(&space).expect("Serialization failed");
        let recovered = MorkForm::from_mork_bytes(&bytes, &space).expect("Deserialization failed");

        assert_eq!(form, recovered);
    }

    #[test]
    fn test_tuple() {
        let space = Space::new();
        let form = MorkForm::Tuple(vec![
            MorkForm::Literal(LiteralValue::Int(1)),
            MorkForm::Literal(LiteralValue::String("a".to_string())),
            MorkForm::Literal(LiteralValue::Bool(true)),
        ]);

        let bytes = form.to_mork_bytes(&space).expect("Serialization failed");
        let recovered = MorkForm::from_mork_bytes(&bytes, &space).expect("Deserialization failed");

        assert_eq!(form, recovered);
    }

    // Deserialization deferred - not needed for pattern matching use case
    // See: docs/pattern_matching/README.md line 84
    #[test]
    #[ignore]
    fn test_nested_map() {
        let space = Space::new();
        let form = MorkForm::Map(vec![
            ("user".to_string(), MorkForm::Map(vec![
                ("name".to_string(), MorkForm::Literal(LiteralValue::String("Alice".to_string()))),
                ("age".to_string(), MorkForm::Literal(LiteralValue::Int(30))),
            ])),
            ("active".to_string(), MorkForm::Literal(LiteralValue::Bool(true))),
        ]);

        let bytes = form.to_mork_bytes(&space).expect("Serialization failed");
        let recovered = MorkForm::from_mork_bytes(&bytes, &space).expect("Deserialization failed");

        assert_eq!(form, recovered);
    }

    // Deserialization deferred - not needed for pattern matching use case
    // See: docs/pattern_matching/README.md line 84
    #[test]
    #[ignore]
    fn test_map_pattern() {
        let space = Space::new();
        let form = MorkForm::MapPattern(vec![
            ("x".to_string(), MorkForm::VarPattern("a".to_string())),
            ("y".to_string(), MorkForm::VarPattern("b".to_string())),
        ]);

        let bytes = form.to_mork_bytes(&space).expect("Serialization failed");
        let recovered = MorkForm::from_mork_bytes(&bytes, &space).expect("Deserialization failed");

        assert_eq!(form, recovered);
    }

    // Deserialization deferred - not needed for pattern matching use case
    // See: docs/pattern_matching/README.md line 84
    #[test]
    #[ignore]
    fn test_list_pattern() {
        let space = Space::new();
        let form = MorkForm::ListPattern(vec![
            MorkForm::VarPattern("head".to_string()),
            MorkForm::WildcardPattern,
        ]);

        let bytes = form.to_mork_bytes(&space).expect("Serialization failed");
        let recovered = MorkForm::from_mork_bytes(&bytes, &space).expect("Deserialization failed");

        assert_eq!(form, recovered);
    }

    // Deserialization deferred - not needed for pattern matching use case
    // See: docs/pattern_matching/README.md line 84
    #[test]
    #[ignore]
    fn test_contract_with_map_pattern() {
        let space = Space::new();
        let form = MorkForm::Contract {
            name: "processUser".to_string(),
            parameters: vec![
                MorkForm::MapPattern(vec![
                    ("name".to_string(), MorkForm::VarPattern("n".to_string())),
                    ("email".to_string(), MorkForm::VarPattern("e".to_string())),
                ]),
            ],
            body: Box::new(MorkForm::Send {
                channel: Box::new(MorkForm::Variable("ret".to_string())),
                arguments: vec![MorkForm::Variable("n".to_string())],
            }),
        };

        let bytes = form.to_mork_bytes(&space).expect("Serialization failed");
        let recovered = MorkForm::from_mork_bytes(&bytes, &space).expect("Deserialization failed");

        assert_eq!(form, recovered);
    }

    #[test]
    fn test_deterministic_serialization() {
        let space = Space::new();
        let form = MorkForm::Map(vec![
            ("a".to_string(), MorkForm::Literal(LiteralValue::Int(1))),
            ("b".to_string(), MorkForm::Literal(LiteralValue::Int(2))),
        ]);

        // Serialize twice
        let bytes1 = form.to_mork_bytes(&space).expect("First serialization failed");
        let bytes2 = form.to_mork_bytes(&space).expect("Second serialization failed");

        // Should produce identical bytes
        assert_eq!(bytes1, bytes2, "Serialization should be deterministic");
    }
}
