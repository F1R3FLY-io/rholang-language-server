
use super::node::{Node, BinOperator, SendType, BundleType, UnaryOperator, VarRefKind, CommentKind};
use std::sync::Arc;
use ropey::Rope;

/// Formats an IR node into a string representation, with optional indentation.
///
/// # Arguments
/// * `node` - The IR node to format.
/// * `indent` - Whether to apply indentation.
/// * `indent_size` - Optional size of each indentation level (defaults to 2 if not provided).
/// * `rope` - The Rope containing the source text.
/// * `root` - The root node for position calculations.
///
/// # Returns
/// A string representing the formatted node.
pub fn format_node(node: &Arc<Node>, indent: bool, indent_size: Option<usize>, rope: &Rope, root: &Arc<Node>) -> String {
    if indent {
        let size = indent_size.unwrap_or(2);
        format_node_helper(node, 0, size, rope, root)
    } else {
        format_node_helper(node, 0, 0, rope, root)
    }
}

/// Helper function to recursively format nodes with indentation.
///
/// # Arguments
/// * `node` - The IR node to format.
/// * `level` - Current indentation level.
/// * `indent_size` - Size of each indentation level (0 for no indentation).
/// * `rope` - The Rope containing the source text.
/// * `root` - The root node for position calculations.
///
/// # Returns
/// A string representing the formatted node with appropriate indentation.
fn format_node_helper(node: &Arc<Node>, level: usize, indent_size: usize, rope: &Rope, root: &Arc<Node>) -> String {
    let indent = if indent_size > 0 { " ".repeat(level * indent_size) } else { "".to_string() };
    match &**node {
        Node::Par { left, right, .. } => {
            let left_text = format_node_helper(left, level, indent_size, rope, root);
            let right_text = format_node_helper(right, level, indent_size, rope, root);
            format!("{} | {}", left_text, right_text)
        }
        Node::SendSync { channel, inputs, cont, .. } => {
            let inputs_str = inputs.iter().map(|i| format_node_helper(i, level, indent_size, rope, root)).collect::<Vec<_>>().join(", ");
            let cont_text = format_node_helper(cont, level, indent_size, rope, root);
            format!("{}!?({}; {})", format_node_helper(channel, level, indent_size, rope, root), inputs_str, cont_text)
        }
        Node::Send { channel, send_type, inputs, .. } => {
            let inputs_str = inputs.iter().map(|i| format_node_helper(i, level, indent_size, rope, root)).collect::<Vec<_>>().join(", ");
            let send_op = match send_type {
                SendType::Single => "!",
                SendType::Multiple => "!!",
            };
            format!("{}{}({})", format_node_helper(channel, level, indent_size, rope, root), send_op, inputs_str)
        }
        Node::New { decls, proc, .. } => {
            let decls_str = decls.iter().map(|d| format_node_helper(d, level, indent_size, rope, root)).collect::<Vec<_>>().join(", ");
            let proc_text = format_node_helper(proc, level + 1, indent_size, rope, root);
            let indented_proc = if indent_size > 0 {
                proc_text.lines().map(|line| format!("{}{}", " ".repeat((level + 1) * indent_size), line)).collect::<Vec<_>>().join("\n")
            } else {
                proc_text
            };
            format!("new {} in {{\n{}\n{}}}", decls_str, indented_proc, indent)
        }
        Node::IfElse { condition, consequence, alternative, .. } => {
            let cond_text = format_node_helper(condition, level, indent_size, rope, root);
            let then_text = format_node_helper(consequence, level + 1, indent_size, rope, root);
            let indented_then = if indent_size > 0 {
                then_text.lines().map(|line| format!("{}{}", " ".repeat((level + 1) * indent_size), line)).collect::<Vec<_>>().join("\n")
            } else {
                then_text
            };
            let else_str = if let Some(alt) = alternative {
                let alt_text = format_node_helper(alt, level + 1, indent_size, rope, root);
                let indented_alt = if indent_size > 0 {
                    alt_text.lines().map(|line| format!("{}{}", " ".repeat((level + 1) * indent_size), line)).collect::<Vec<_>>().join("\n")
                } else {
                    alt_text
                };
                format!(" else {{\n{}\n{}}}", indented_alt, indent)
            } else {
                "".to_string()
            };
            format!("if ({}) {{\n{}\n{}}}{}", cond_text, indented_then, indent, else_str)
        }
        Node::Let { decls, proc, .. } => {
            let decls_str = decls.iter().map(|d| format_node_helper(d, level, indent_size, rope, root)).collect::<Vec<_>>().join("; ");
            let proc_text = format_node_helper(proc, level + 1, indent_size, rope, root);
            let indented_proc = if indent_size > 0 {
                proc_text.lines().map(|line| format!("{}{}", " ".repeat((level + 1) * indent_size), line)).collect::<Vec<_>>().join("\n")
            } else {
                proc_text
            };
            format!("let {} in {{\n{}\n{}}}", decls_str, indented_proc, indent)
        }
        Node::Bundle { bundle_type, proc, .. } => {
            let prefix = match bundle_type {
                BundleType::Read => "bundle-",
                BundleType::Write => "bundle+",
                BundleType::Equiv => "bundle0",
                BundleType::ReadWrite => "bundle",
            };
            let proc_text = format_node_helper(proc, level + 1, indent_size, rope, root);
            let indented_proc = if indent_size > 0 {
                proc_text.lines().map(|line| format!("{}{}", " ".repeat((level + 1) * indent_size), line)).collect::<Vec<_>>().join("\n")
            } else {
                proc_text
            };
            format!("{} {{\n{}\n{}}}", prefix, indented_proc, indent)
        }
        Node::Match { expression, cases, .. } => {
            let expr_text = format_node_helper(expression, level, indent_size, rope, root);
            let cases_str = cases.iter().map(|(pat, proc)| {
                let pat_text = format_node_helper(pat, level, indent_size, rope, root);
                let proc_text = format_node_helper(proc, level + 1, indent_size, rope, root);
                let indented_proc = if indent_size > 0 {
                    proc_text.lines().map(|line| format!("{}{}", " ".repeat((level + 1) * indent_size), line)).collect::<Vec<_>>().join("\n")
                } else {
                    proc_text
                };
                format!("{} => {{\n{}\n{}}}", pat_text, indented_proc, indent)
            }).collect::<Vec<_>>().join("\n");
            format!("match {} {{\n{}\n{}}}", expr_text, cases_str, indent)
        }
        Node::Choice { branches, .. } => {
            let branches_str = branches.iter().map(|(inputs, proc)| {
                let inputs_str = inputs.iter().map(|i| format_node_helper(i, level, indent_size, rope, root)).collect::<Vec<_>>().join(" & ");
                let proc_text = format_node_helper(proc, level + 1, indent_size, rope, root);
                let indented_proc = if indent_size > 0 {
                    proc_text.lines().map(|line| format!("{}{}", " ".repeat((level + 1) * indent_size), line)).collect::<Vec<_>>().join("\n")
                } else {
                    proc_text
                };
                format!("{} => {{\n{}\n{}}}", inputs_str, indented_proc, indent)
            }).collect::<Vec<_>>().join("\n");
            format!("select {{\n{}\n{}}}", branches_str, indent)
        }
        Node::Contract { name, formals, formals_remainder, proc, .. } => {
            let formals_str = formals.iter().map(|f| format_node_helper(f, level, indent_size, rope, root)).collect::<Vec<_>>().join(", ");
            let remainder_str = formals_remainder.as_ref().map(|r| format!("...{}", format_node_helper(r, level, indent_size, rope, root))).unwrap_or_default();
            let formals_with_rem = if formals_remainder.is_some() { format!("{}{}", formals_str, if !formals_str.is_empty() { "," } else { "" }) } else { formals_str };
            let proc_text = format_node_helper(proc, level + 1, indent_size, rope, root);
            let indented_proc = if indent_size > 0 {
                proc_text.lines().map(|line| format!("{}{}", " ".repeat((level + 1) * indent_size), line)).collect::<Vec<_>>().join("\n")
            } else {
                proc_text
            };
            format!("contract {}({}{}) = {{\n{}\n{}}}", format_node_helper(name, level, indent_size, rope, root), formals_with_rem, remainder_str, indented_proc, indent)
        }
        Node::Input { receipts, proc, .. } => {
            let receipts_str = receipts.iter().map(|binds| binds.iter().map(|b| format_node_helper(b, level, indent_size, rope, root)).collect::<Vec<_>>().join(" & ")).collect::<Vec<_>>().join("; ");
            let proc_text = format_node_helper(proc, level + 1, indent_size, rope, root);
            let indented_proc = if indent_size > 0 {
                proc_text.lines().map(|line| format!("{}{}", " ".repeat((level + 1) * indent_size), line)).collect::<Vec<_>>().join("\n")
            } else {
                proc_text
            };
            format!("for ({}) {{\n{}\n{}}}", receipts_str, indented_proc, indent)
        }
        Node::Block { proc, .. } => {
            let proc_text = format_node_helper(proc, level + 1, indent_size, rope, root);
            let indented_proc = if indent_size > 0 {
                proc_text.lines().map(|line| format!("{}{}", " ".repeat((level + 1) * indent_size), line)).collect::<Vec<_>>().join("\n")
            } else {
                proc_text
            };
            format!("{{\n{}\n{}}}", indented_proc, indent)
        }
        Node::Parenthesized { expr, .. } => {
            let expr_text = format_node_helper(expr, level, indent_size, rope, root);
            format!("({})", expr_text)
        }
        Node::BinOp { op, left, right, .. } => {
            let left_text = format_node_helper(left, level, indent_size, rope, root);
            let right_text = format_node_helper(right, level, indent_size, rope, root);
            let op_str = match op {
                BinOperator::Or => "or",
                BinOperator::And => "and",
                BinOperator::Matches => "matches",
                BinOperator::Eq => "==",
                BinOperator::Neq => "!=",
                BinOperator::Lt => "<",
                BinOperator::Lte => "<=",
                BinOperator::Gt => ">",
                BinOperator::Gte => ">=",
                BinOperator::Concat => "++",
                BinOperator::Diff => "--",
                BinOperator::Add => "+",
                BinOperator::Sub => "-",
                BinOperator::Interpolation => "%%",
                BinOperator::Mult => "*",
                BinOperator::Div => "/",
                BinOperator::Mod => "%",
                BinOperator::Disjunction => "\\/",
                BinOperator::Conjunction => "/\\",
            };
            format!("({} {} {})", left_text, op_str, right_text)
        }
        Node::UnaryOp { op, operand, .. } => {
            let operand_text = format_node_helper(operand, level, indent_size, rope, root);
            match op {
                UnaryOperator::Not => format!("not {}", operand_text),
                UnaryOperator::Neg => format!("-{}", operand_text),
                UnaryOperator::Negation => format!("~{}", operand_text),
            }
        }
        Node::Method { receiver, name, args, .. } => {
            let args_str = args.iter().map(|a| format_node_helper(a, level, indent_size, rope, root)).collect::<Vec<_>>().join(", ");
            format!("{}.{}({})", format_node_helper(receiver, level, indent_size, rope, root), name, args_str)
        }
        Node::Eval { name, .. } => format!("*{}", format_node_helper(name, level, indent_size, rope, root)),
        Node::Quote { quotable, .. } => format!("@{}", format_node_helper(quotable, level, indent_size, rope, root)),
        Node::VarRef { kind, var, .. } => {
            let kind_str = match kind {
                VarRefKind::Bind => "=",
                VarRefKind::Unforgeable => "=*",
            };
            format!("{}{}", kind_str, format_node_helper(var, level, indent_size, rope, root))
        }
        Node::BoolLiteral { value, .. } => value.to_string(),
        Node::LongLiteral { value, .. } => value.to_string(),
        Node::StringLiteral { value, .. } => format!("\"{}\"", value),
        Node::UriLiteral { value, .. } => format!("`{}`", value),
        Node::Nil { .. } => "Nil".to_string(),
        Node::List { elements, remainder, .. } => {
            let elements_str = elements.iter().map(|e| format_node_helper(e, level, indent_size, rope, root)).collect::<Vec<_>>().join(", ");
            let remainder_str = remainder.as_ref().map(|r| format!("...{}", format_node_helper(r, level, indent_size, rope, root))).unwrap_or_default();
            format!("[{}{}]", elements_str, if remainder.is_some() { format!(",{}", remainder_str) } else { String::new() })
        }
        Node::Set { elements, remainder, .. } => {
            let elements_str = elements.iter().map(|e| format_node_helper(e, level, indent_size, rope, root)).collect::<Vec<_>>().join(", ");
            let remainder_str = remainder.as_ref().map(|r| format!("...{}", format_node_helper(r, level, indent_size, rope, root))).unwrap_or_default();
            format!("Set({}{})", elements_str, if remainder.is_some() { format!(",{}", remainder_str) } else { String::new() })
        }
        Node::Map { pairs, remainder, .. } => {
            let pairs_str = pairs.iter().map(|(k, v)| format!("{}: {}", format_node_helper(k, level, indent_size, rope, root), format_node_helper(v, level, indent_size, rope, root))).collect::<Vec<_>>().join(", ");
            let remainder_str = remainder.as_ref().map(|r| format!("...{}", format_node_helper(r, level, indent_size, rope, root))).unwrap_or_default();
            format!("{{{}{}}}", pairs_str, if remainder.is_some() { format!(",{}", remainder_str) } else { String::new() })
        }
        Node::Tuple { elements, .. } => {
            let elements_str = elements.iter().map(|e| format_node_helper(e, level, indent_size, rope, root)).collect::<Vec<_>>().join(", ");
            format!("({})", elements_str)
        }
        Node::Var { name, .. } => name.clone(),
        Node::NameDecl { var, uri, .. } => {
            if let Some(uri_node) = uri { format!("{}({})", format_node_helper(var, level, indent_size, rope, root), format_node_helper(uri_node, level, indent_size, rope, root)) } else { format_node_helper(var, level, indent_size, rope, root) }
        }
        Node::Decl { names, names_remainder, procs, .. } => {
            let names_str = names.iter().map(|n| format_node_helper(n, level, indent_size, rope, root)).collect::<Vec<_>>().join(", ");
            let remainder_str = names_remainder.as_ref().map(|r| format!("...{}", format_node_helper(r, level, indent_size, rope, root))).unwrap_or_default();
            let names_with_rem = if names_remainder.is_some() { format!("{}{}", names_str, if !names_str.is_empty() { "," } else { "" }) } else { names_str };
            let procs_str = procs.iter().map(|p| format_node_helper(p, level, indent_size, rope, root)).collect::<Vec<_>>().join(", ");
            format!("{}{} = {}", names_with_rem, remainder_str, procs_str)
        }
        Node::LinearBind { names, remainder, source, .. } => {
            let names_str = names.iter().map(|n| format_node_helper(n, level, indent_size, rope, root)).collect::<Vec<_>>().join(", ");
            let remainder_str = remainder.as_ref().map(|r| format!("...{}", format_node_helper(r, level, indent_size, rope, root))).unwrap_or_default();
            let names_with_rem = if remainder.is_some() { format!("{}{}", names_str, if !names_str.is_empty() { "," } else { "" }) } else { names_str };
            format!("{}{} <- {}", names_with_rem, remainder_str, format_node_helper(source, level, indent_size, rope, root))
        }
        Node::RepeatedBind { names, remainder, source, .. } => {
            let names_str = names.iter().map(|n| format_node_helper(n, level, indent_size, rope, root)).collect::<Vec<_>>().join(", ");
            let remainder_str = remainder.as_ref().map(|r| format!("...{}", format_node_helper(r, level, indent_size, rope, root))).unwrap_or_default();
            let names_with_rem = if remainder.is_some() { format!("{}{}", names_str, if !names_str.is_empty() { "," } else { "" }) } else { names_str };
            format!("{}{} <= {}", names_with_rem, remainder_str, format_node_helper(source, level, indent_size, rope, root))
        }
        Node::PeekBind { names, remainder, source, .. } => {
            let names_str = names.iter().map(|n| format_node_helper(n, level, indent_size, rope, root)).collect::<Vec<_>>().join(", ");
            let remainder_str = remainder.as_ref().map(|r| format!("...{}", format_node_helper(r, level, indent_size, rope, root))).unwrap_or_default();
            let names_with_rem = if remainder.is_some() { format!("{}{}", names_str, if !names_str.is_empty() { "," } else { "" }) } else { names_str };
            format!("{}{} <<- {}", names_with_rem, remainder_str, format_node_helper(source, level, indent_size, rope, root))
        }
        Node::Comment { kind, .. } => {
            let text = node.text(rope, root).to_string();
            match kind {
                CommentKind::Line => format!("//{}", text.trim_start_matches("//")),
                CommentKind::Block => format!("/*{}*/", text.trim_start_matches("/*").trim_end_matches("*/")),
            }
        }
        Node::Wildcard { .. } => "_".to_string(),
        Node::SimpleType { value, .. } => value.clone(),
        Node::ReceiveSendSource { name, .. } => format!("{}?!", format_node_helper(name, level, indent_size, rope, root)),
        Node::SendReceiveSource { name, inputs, .. } => {
            let inputs_str = inputs.iter().map(|i| format_node_helper(i, level, indent_size, rope, root)).collect::<Vec<_>>().join(", ");
            format!("{}!?({})", format_node_helper(name, level, indent_size, rope, root), inputs_str)
        }
        Node::Error { children, .. } => {
            let children_str = children
                .iter()
                .map(|child| format_node_helper(child, level, indent_size, rope, root))
                .collect::<Vec<_>>()
                .join("\n");
            format!("/* ERROR: \n{} */", children_str)
        }
        Node::Disjunction { left, right, .. } => {
            let left_text = format_node_helper(left, level, indent_size, rope, root);
            let right_text = format_node_helper(right, level, indent_size, rope, root);
            format!("{} \\/ {}", left_text, right_text)
        }
        Node::Conjunction { left, right, .. } => {
            let left_text = format_node_helper(left, level, indent_size, rope, root);
            let right_text = format_node_helper(right, level, indent_size, rope, root);
            format!("{} /\\ {}", left_text, right_text)
        }
        Node::Negation { operand, .. } => {
            let operand_text = format_node_helper(operand, level, indent_size, rope, root);
            format!("~{}", operand_text)
        }
        Node::Unit { .. } => format!("()"),
    }
}
