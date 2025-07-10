use std::any::Any;
use std::sync::Arc;
use std::collections::HashMap;
use tree_sitter::{InputEdit, Node as TSNode, Parser, Tree};
use tracing::{debug, trace, warn};
use rpds::Vector;
use archery::ArcK;

use crate::ir::node::{
    BinOperator, BundleType, CommentKind, Node, NodeBase, SendType, UnaryOperator, VarRefKind,
    Metadata, Position, RelativePosition
};

pub fn parse_code(code: &str) -> Tree {
    let mut parser = Parser::new();
    parser.set_language(&rholang_tree_sitter::LANGUAGE.into()).expect("Failed to set Tree-Sitter language");
    parser.parse(code, None).expect("Failed to parse Rholang code")
}

pub fn parse_to_ir<'a>(tree: &'a Tree, source_code: &'a str) -> Arc<Node<'a>> {
    debug!("Parsing Tree-Sitter tree into IR for source: {}", source_code);
    if tree.root_node().has_error() {
        warn!("Parse tree contains errors for source: {}", source_code);
    }
    let initial_prev_end = Position { row: 0, column: 0, byte: 0 };
    let (node, _) = convert_ts_node_to_ir(tree.root_node(), source_code, initial_prev_end);
    node
}

/// Updates the syntax tree incrementally based on text changes.
pub fn update_tree(tree: &Tree, new_text: &str, start_byte: usize, old_end_byte: usize, new_length: usize) -> Tree {
    let mut parser = Parser::new();
    parser.set_language(&rholang_tree_sitter::LANGUAGE.into()).expect("Failed to set Tree-Sitter language");
    let edit = InputEdit {
        start_byte,
        old_end_byte,
        new_end_byte: start_byte + new_length,
        start_position: tree.root_node().start_position(),
        old_end_position: tree.root_node().end_position(),
        new_end_position: tree.root_node().end_position(),
    };
    let mut new_tree = tree.clone();
    new_tree.edit(&edit);
    parser.parse(new_text, Some(&new_tree)).unwrap_or_else(|| {
        warn!("Incremental parse failed, performing full parse");
        parse_code(new_text)
    })
}

/// Collects named descendant nodes, updating prev_end sequentially.
fn collect_named_descendants<'a>(node: TSNode<'a>, source_code: &'a str, prev_end: Position) -> (Vector<Arc<Node<'a>>, ArcK>, Position) {
    let mut nodes: Vector<Arc<Node<'_>>, ArcK> = Vector::<Arc<Node<'_>>, ArcK>::new_with_ptr_kind();
    let mut current_prev_end = prev_end;
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        let (child_node, child_end) = convert_ts_node_to_ir(child, source_code, current_prev_end);
        nodes = nodes.push_back(child_node);
        current_prev_end = child_end;
    }
    (nodes, current_prev_end)
}

/// Collects patterns from a names node, separating elements and optional remainder.
fn collect_patterns<'a>(node: TSNode<'a>, source_code: &'a str, prev_end: Position) -> (Vector<Arc<Node<'a>>, ArcK>, Option<Arc<Node<'a>>>, Position) {
    let mut elements: Vector<Arc<Node<'_>>, ArcK> = Vector::<Arc<Node<'_>>, ArcK>::new_with_ptr_kind();
    let mut remainder: Option<Arc<Node<'_>>> = None;
    let mut current_prev_end = prev_end;
    let mut cursor = node.walk();
    let mut is_remainder = false;
    let mut is_quote = false;
    for child in node.children(&mut cursor) {
        let child_kind = child.kind();
        trace!("Pattern child: '{}' at start={:?}, end={:?}", child_kind, child.start_position(), child.end_position());
        if child_kind == "," {
            continue;
        } else if child_kind == "..." {
            is_remainder = true;
            continue;
        } else if child_kind == "@" {
            is_quote = true;
            current_prev_end = Position {
                row: child.end_position().row,
                column: child.end_position().column,
                byte: child.end_byte(),
            };
            continue;
        } else if child.is_named() {
            let (mut child_node, child_end) = convert_ts_node_to_ir(child, source_code, current_prev_end);
            if is_quote {
                let quote_base = NodeBase::new(
                    Some(child),
                    RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 },
                    child.end_byte() - child.start_byte() + 1,
                    Some("@".to_string() + child_node.text().as_str()),
                );
                child_node = Arc::new(Node::Quote {
                    base: quote_base,
                    quotable: child_node,
                    metadata: None,
                });
                is_quote = false;
            }
            if is_remainder {
                remainder = Some(child_node);
                is_remainder = false;
            } else {
                elements = elements.push_back(child_node);
            }
            current_prev_end = child_end;
        } else {
            warn!("Unhandled child in patterns: {}", child_kind);
        }
    }
    (elements, remainder, current_prev_end)
}

/// Collects linear binds for Choice nodes, maintaining position continuity.
fn collect_linear_binds<'a>(branch_node: TSNode<'a>, source_code: &'a str, prev_end: Position) -> (Vector<Arc<Node<'a>>, ArcK>, Position) {
    let mut linear_binds: Vector<Arc<Node<'_>>, ArcK> = Vector::<Arc<Node<'_>>, ArcK>::new_with_ptr_kind();
    let mut current_prev_end = prev_end;
    let mut cursor = branch_node.walk();
    for child in branch_node.children(&mut cursor) {
        if child.kind() == "linear_bind" {
            let (bind_node, bind_end) = convert_ts_node_to_ir(child, source_code, current_prev_end);
            linear_binds = linear_binds.push_back(bind_node);
            current_prev_end = bind_end;
        } else if child.kind() == "=>" {
            break;
        }
    }
    (linear_binds, current_prev_end)
}

/// Converts Tree-Sitter nodes to IR nodes with accurate relative positions.
fn convert_ts_node_to_ir<'a>(ts_node: TSNode<'a>, source_code: &'a str, prev_end: Position) -> (Arc<Node<'a>>, Position) {
    let absolute_start = Position {
        row: ts_node.start_position().row,
        column: ts_node.start_position().column,
        byte: ts_node.start_byte(),
    };
    let absolute_end = Position {
        row: ts_node.end_position().row,
        column: ts_node.end_position().column,
        byte: ts_node.end_byte(),
    };

    let delta_lines = absolute_start.row as i32 - prev_end.row as i32;
    let delta_columns = if delta_lines == 0 {
        absolute_start.column as i32 - prev_end.column as i32
    } else {
        absolute_start.column as i32
    };
    let delta_bytes = absolute_start.byte - prev_end.byte;

    let relative_start = RelativePosition {
        delta_lines,
        delta_columns,
        delta_bytes,
    };
    trace!(
        "Node '{}': prev_end={:?}, start={:?}, end={:?}, delta=({}, {}, {})",
        ts_node.kind(), prev_end, absolute_start, absolute_end, delta_lines, delta_columns, delta_bytes
    );

    let base = NodeBase::new(
        Some(ts_node),
        relative_start,
        absolute_end.byte - absolute_start.byte,
        Some(ts_node.utf8_text(source_code.as_bytes()).unwrap_or("").to_string()),
    );
    let mut data = HashMap::new();
    data.insert("version".to_string(), Arc::new(0_usize) as Arc<dyn Any + Send + Sync>);
    let metadata = Some(Arc::new(Metadata { data }));

    match ts_node.kind() {
        "source_file" => {
            let mut current_prev_end = absolute_start;
            let children = ts_node.named_children(&mut ts_node.walk())
                .map(|child| {
                    let (node, end) = convert_ts_node_to_ir(child, source_code, current_prev_end);
                    current_prev_end = end;
                    node
                })
                .collect::<Vec<_>>();
            trace!("Source file children: {:?}", children.iter().map(|n| n.text()).collect::<Vec<_>>());
            let mut result = if children.len() == 1 {
                children[0].clone()
            } else if children.is_empty() {
                Arc::new(Node::Nil { base: base.clone(), metadata: metadata.clone() })
            } else {
                children.clone().into_iter().reduce(|left, right| {
                    Arc::new(Node::Par {
                        base: base.clone(),
                        left,
                        right,
                        metadata: metadata.clone(),
                    })
                }).unwrap_or(Arc::new(Node::Nil { base: base.clone(), metadata: metadata.clone() }))
            };
            let trimmed_source = source_code.trim();
            if children.len() == 1 && trimmed_source.starts_with('(') && trimmed_source.ends_with(')') {
                result = Arc::new(Node::Parenthesized {
                    base,
                    expr: result,
                    metadata,
                });
            }
            (result, absolute_end)
        }
        "collection" => {
            let child = ts_node.named_child(0).expect("Collection node must have a named child");
            convert_ts_node_to_ir(child, source_code, prev_end)
        }
        "par" => {
            let left_ts = ts_node.child(0).unwrap();
            let (left, left_end) = convert_ts_node_to_ir(left_ts, source_code, absolute_start);
            let right_ts = ts_node.child(2).unwrap(); // After '|'
            let (right, right_end) = convert_ts_node_to_ir(right_ts, source_code, left_end);
            let node = Arc::new(Node::Par { base, left, right, metadata });
            (node, right_end)
        }
        "send_sync" => {
            let channel_ts = ts_node.child_by_field_name("channel").unwrap();
            let (channel, _channel_end) = convert_ts_node_to_ir(channel_ts, source_code, absolute_start);
            let inputs_ts = ts_node.child_by_field_name("inputs").unwrap();
            let mut current_prev_end = absolute_start; // Reset to start for inputs
            let inputs = inputs_ts.named_children(&mut inputs_ts.walk())
                .map(|child| {
                    let (node, end) = convert_ts_node_to_ir(child, source_code, current_prev_end);
                    current_prev_end = end;
                    node
                })
                .collect::<Vector<_, ArcK>>();
            let cont_ts = ts_node.child_by_field_name("cont").unwrap();
            let (cont, cont_end) = convert_ts_node_to_ir(cont_ts, source_code, current_prev_end);
            let node = Arc::new(Node::SendSync { base, channel, inputs, cont, metadata });
            (node, cont_end)
        }
        "non_empty_cont" => {
            let proc_ts = ts_node.named_child(0).unwrap();
            convert_ts_node_to_ir(proc_ts, source_code, prev_end)
        }
        "empty_cont" => {
            let node = Arc::new(Node::Nil { base, metadata });
            (node, absolute_end)
        }
        "sync_send_cont" => {
            if ts_node.named_child_count() == 0 {
                let node = Arc::new(Node::Nil { base, metadata });
                (node, absolute_end)
            } else {
                let proc_ts = ts_node.named_child(0).unwrap();
                convert_ts_node_to_ir(proc_ts, source_code, absolute_start)
            }
        }
        "send" => {
            let channel_ts = ts_node.child_by_field_name("channel").unwrap();
            let (channel, _channel_end) = convert_ts_node_to_ir(channel_ts, source_code, absolute_start);
            let send_type_ts = ts_node.child_by_field_name("send_type").unwrap();
            let send_type_end = Position {
                row: send_type_ts.end_position().row,
                column: send_type_ts.end_position().column,
                byte: send_type_ts.end_byte(),
            };
            let inputs_ts = ts_node.child_by_field_name("inputs").unwrap();
            let mut current_prev_end = send_type_end;
            let inputs = inputs_ts.named_children(&mut inputs_ts.walk())
                .map(|child| {
                    let (node, end) = convert_ts_node_to_ir(child, source_code, current_prev_end);
                    current_prev_end = end;
                    node
                })
                .collect::<Vector<_, ArcK>>();
            let send_type = match send_type_ts.kind() {
                "send_single" => SendType::Single,
                "send_multiple" => SendType::Multiple,
                _ => { warn!("Unknown send_type: {}", send_type_ts.kind()); SendType::Single }
            };
            let node = Arc::new(Node::Send {
                base,
                channel,
                send_type,
                send_type_end,
                inputs,
                metadata,
            });
            (node, absolute_end)
        }
        "new" => {
            let decls_ts = ts_node.child_by_field_name("decls").unwrap();
            let (decls, decls_end) = collect_named_descendants(decls_ts, source_code, absolute_start);
            let proc_ts = ts_node.child_by_field_name("proc").unwrap();
            let (proc, proc_end) = convert_ts_node_to_ir(proc_ts, source_code, decls_end);
            let node = Arc::new(Node::New { base, decls, proc, metadata });
            (node, proc_end)
        }
        "ifElse" => {
            let condition_ts = ts_node.named_child(0).unwrap();
            let (condition, cond_end) = convert_ts_node_to_ir(condition_ts, source_code, absolute_start);
            let consequence_ts = ts_node.named_child(1).unwrap();
            let (consequence, cons_end) = convert_ts_node_to_ir(consequence_ts, source_code, cond_end);
            let alternative = if ts_node.named_child_count() > 2 {
                let alt_ts = ts_node.named_child(2).unwrap();
                let (alt, alt_end) = convert_ts_node_to_ir(alt_ts, source_code, cons_end);
                Some((alt, alt_end))
            } else {
                None
            };
            let node = Arc::new(Node::IfElse {
                base,
                condition,
                consequence,
                alternative: alternative.as_ref().map(|(alt, _)| alt.clone()),
                metadata,
            });
            (node, alternative.map_or(cons_end, |(_, end)| end))
        }
        "let" => {
            let decls_ts = ts_node.child_by_field_name("decls").unwrap();
            let (decls, decls_end) = collect_named_descendants(decls_ts, source_code, absolute_start);
            let proc_ts = ts_node.child_by_field_name("proc").unwrap();
            let (proc, proc_end) = convert_ts_node_to_ir(proc_ts, source_code, decls_end);
            let node = Arc::new(Node::Let { base, decls, proc, metadata });
            (node, proc_end)
        }
        "bundle" => {
            let bundle_type_ts = ts_node.child_by_field_name("bundle_type").unwrap();
            let bundle_type_end = Position {
                row: bundle_type_ts.end_position().row,
                column: bundle_type_ts.end_position().column,
                byte: bundle_type_ts.end_byte(),
            };
            let bundle_type = match bundle_type_ts.kind() {
                "bundle_read" => BundleType::Read,
                "bundle_write" => BundleType::Write,
                "bundle_equiv" => BundleType::Equiv,
                "bundle_read_write" => BundleType::ReadWrite,
                _ => unreachable!("Unknown bundle type: {}", bundle_type_ts.kind()),
            };
            let proc_ts = ts_node.child_by_field_name("proc").unwrap();
            let (proc, proc_end) = convert_ts_node_to_ir(proc_ts, source_code, bundle_type_end);
            let node = Arc::new(Node::Bundle { base, bundle_type, proc, metadata });
            (node, proc_end)
        }
        "match" => {
            let expression_ts = ts_node.child_by_field_name("expression").unwrap();
            let (expression, expr_end) = convert_ts_node_to_ir(expression_ts, source_code, absolute_start);
            let cases_ts = ts_node.child_by_field_name("cases").unwrap();
            let mut current_prev_end = expr_end;
            let cases = cases_ts.named_children(&mut cases_ts.walk())
                .filter(|n| n.kind() == "case")
                .map(|case_node| {
                    let pattern_ts = case_node.child_by_field_name("pattern").unwrap();
                    let (pattern, pat_end) = convert_ts_node_to_ir(pattern_ts, source_code, current_prev_end);
                    let proc_ts = case_node.child_by_field_name("proc").unwrap();
                    let (proc, proc_end) = convert_ts_node_to_ir(proc_ts, source_code, pat_end);
                    current_prev_end = proc_end;
                    (pattern, proc)
                })
                .collect::<Vector<_, ArcK>>();
            let node = Arc::new(Node::Match { base, expression, cases, metadata });
            (node, current_prev_end)
        }
        "choice" => {
            let branches_ts = ts_node.child_by_field_name("branches").unwrap();
            let mut current_prev_end = absolute_start;
            let branches = branches_ts.named_children(&mut branches_ts.walk())
                .filter(|n| n.kind() == "branch")
                .map(|branch_node| {
                    let (inputs, inputs_end) = collect_linear_binds(branch_node, source_code, current_prev_end);
                    let proc_ts = branch_node.child_by_field_name("proc").unwrap();
                    let (proc, proc_end) = convert_ts_node_to_ir(proc_ts, source_code, inputs_end);
                    current_prev_end = proc_end;
                    (inputs, proc)
                })
                .collect::<Vector<_, ArcK>>();
            let node = Arc::new(Node::Choice { base, branches, metadata });
            (node, current_prev_end)
        }
        "contract" => {
            let name_ts = ts_node.child_by_field_name("name").unwrap();
            let (name, name_end) = convert_ts_node_to_ir(name_ts, source_code, absolute_start);
            let formals_ts = ts_node.child_by_field_name("formals").unwrap();
            let (formals, formals_remainder, formals_end) = collect_patterns(formals_ts, source_code, name_end);
            let proc_ts = ts_node.child_by_field_name("proc").unwrap();
            let (proc, proc_end) = convert_ts_node_to_ir(proc_ts, source_code, formals_end);
            let node = Arc::new(Node::Contract { base, name, formals, formals_remainder, proc, metadata });
            (node, proc_end)
        }
        "input" => {
            let receipts_ts = ts_node.child_by_field_name("receipts").unwrap();
            let mut current_prev_end = absolute_start;
            let receipts = receipts_ts.named_children(&mut receipts_ts.walk())
                .map(|receipt_node| {
                    let (binds, binds_end) = collect_named_descendants(receipt_node, source_code, current_prev_end);
                    current_prev_end = binds_end;
                    binds
                })
                .collect::<Vector<_, ArcK>>();
            let proc_ts = ts_node.child_by_field_name("proc").unwrap();
            let (proc, proc_end) = convert_ts_node_to_ir(proc_ts, source_code, current_prev_end);
            let node = Arc::new(Node::Input { base, receipts, proc, metadata });
            (node, proc_end)
        }
        "block" => {
            let proc_ts = ts_node.child(1).unwrap(); // After '{'
            let (proc, proc_end) = convert_ts_node_to_ir(proc_ts, source_code, absolute_start);
            let node = Arc::new(Node::Block { base, proc, metadata });
            (node, proc_end)
        }
        "_parenthesized" => {
            let expr_ts = ts_node.named_child(0).unwrap();
            let (expr, _expr_end) = convert_ts_node_to_ir(expr_ts, source_code, absolute_start);
            let node = Arc::new(Node::Parenthesized { base, expr, metadata });
            (node, absolute_end)
        }
        "_name_remainder" => {
            let cont_ts = ts_node.child_by_field_name("cont").unwrap();
            let (cont, cont_end) = convert_ts_node_to_ir(cont_ts, source_code, absolute_start);
            let node = Arc::new(Node::Quote { base, quotable: cont, metadata });
            (node, cont_end)
        }
        "or" => binary_op(ts_node, source_code, base, BinOperator::Or, absolute_start),
        "and" => binary_op(ts_node, source_code, base, BinOperator::And, absolute_start),
        "matches" => binary_op(ts_node, source_code, base, BinOperator::Matches, absolute_start),
        "eq" => binary_op(ts_node, source_code, base, BinOperator::Eq, absolute_start),
        "neq" => binary_op(ts_node, source_code, base, BinOperator::Neq, absolute_start),
        "lt" => binary_op(ts_node, source_code, base, BinOperator::Lt, absolute_start),
        "lte" => binary_op(ts_node, source_code, base, BinOperator::Lte, absolute_start),
        "gt" => binary_op(ts_node, source_code, base, BinOperator::Gt, absolute_start),
        "gte" => binary_op(ts_node, source_code, base, BinOperator::Gte, absolute_start),
        "concat" => binary_op(ts_node, source_code, base, BinOperator::Concat, absolute_start),
        "diff" => binary_op(ts_node, source_code, base, BinOperator::Diff, absolute_start),
        "add" => binary_op(ts_node, source_code, base, BinOperator::Add, absolute_start),
        "sub" => binary_op(ts_node, source_code, base, BinOperator::Sub, absolute_start),
        "interpolation" => binary_op(ts_node, source_code, base, BinOperator::Interpolation, absolute_start),
        "mult" => binary_op(ts_node, source_code, base, BinOperator::Mult, absolute_start),
        "div" => binary_op(ts_node, source_code, base, BinOperator::Div, absolute_start),
        "mod" => binary_op(ts_node, source_code, base, BinOperator::Mod, absolute_start),
        "not" => unary_op(ts_node, source_code, base, UnaryOperator::Not, absolute_start),
        "neg" => unary_op(ts_node, source_code, base, UnaryOperator::Neg, absolute_start),
        "method" => {
            let receiver_ts = ts_node.child_by_field_name("receiver").unwrap();
            let (receiver, receiver_end) = convert_ts_node_to_ir(receiver_ts, source_code, absolute_start);
            let name = ts_node.child_by_field_name("name").unwrap().utf8_text(source_code.as_bytes()).unwrap().to_string();
            let args_ts = ts_node.child_by_field_name("args").unwrap();
            let mut current_prev_end = receiver_end;
            let args = args_ts.named_children(&mut args_ts.walk())
                .map(|child| {
                    let (node, end) = convert_ts_node_to_ir(child, source_code, current_prev_end);
                    current_prev_end = end;
                    node
                })
                .collect::<Vector<_, ArcK>>();
            let node = Arc::new(Node::Method { base, receiver, name, args, metadata });
            (node, absolute_end)
        }
        "eval" => {
            let name_ts = ts_node.child(1).unwrap();
            let (name, name_end) = convert_ts_node_to_ir(name_ts, source_code, absolute_start);
            let node = Arc::new(Node::Eval { base, name, metadata });
            (node, name_end)
        }
        "quote" => {
            let quotable_ts = ts_node.child(1).unwrap();
            let (quotable, quotable_end) = convert_ts_node_to_ir(quotable_ts, source_code, absolute_start);
            let node = Arc::new(Node::Quote { base, quotable, metadata });
            (node, quotable_end)
        }
        "var_ref" => {
            let kind_ts = ts_node.child_by_field_name("kind").unwrap();
            let kind_text = kind_ts.utf8_text(source_code.as_bytes()).unwrap().to_string();
            let kind = match kind_text.as_str() {
                "=" => VarRefKind::Bind,
                "=*" => VarRefKind::Unforgeable,
                _ => panic!("Unknown var_ref kind text: {}", kind_text),
            };
            let var_ts = ts_node.child_by_field_name("var").unwrap();
            let (var, var_end) = convert_ts_node_to_ir(var_ts, source_code, absolute_start);
            let node = Arc::new(Node::VarRef { base, kind, var, metadata });
            (node, var_end)
        }
        "disjunction" => binary_op(ts_node, source_code, base, BinOperator::Disjunction, absolute_start),
        "conjunction" => binary_op(ts_node, source_code, base, BinOperator::Conjunction, absolute_start),
        "negation" => unary_op(ts_node, source_code, base, UnaryOperator::Negation, absolute_start),
        "bool_literal" => {
            let value = ts_node.utf8_text(source_code.as_bytes()).unwrap() == "true";
            let node = Arc::new(Node::BoolLiteral { base, value, metadata });
            (node, absolute_end)
        }
        "long_literal" => {
            let value = ts_node.utf8_text(source_code.as_bytes()).unwrap().parse::<i64>().unwrap();
            let node = Arc::new(Node::LongLiteral { base, value, metadata });
            (node, absolute_end)
        }
        "string_literal" => {
            let value = ts_node.utf8_text(source_code.as_bytes()).unwrap().trim_matches('"').to_string();
            let node = Arc::new(Node::StringLiteral { base, value, metadata });
            (node, absolute_end)
        }
        "uri_literal" => {
            let value = ts_node.utf8_text(source_code.as_bytes()).unwrap().trim_matches('`').to_string();
            let node = Arc::new(Node::UriLiteral { base, value, metadata });
            (node, absolute_end)
        }
        "nil" => {
            let node = Arc::new(Node::Nil { base, metadata });
            (node, absolute_end)
        }
        "list" => {
            let mut current_prev_end = absolute_start;
            let mut cursor = ts_node.walk();
            let elements = ts_node.named_children(&mut cursor)
                .filter(|n| n.kind() != "_proc_remainder" && n.is_named())
                .map(|child| {
                    let (node, end) = convert_ts_node_to_ir(child, source_code, current_prev_end);
                    current_prev_end = end;
                    node
                })
                .collect::<Vector<_, ArcK>>();
            let remainder = ts_node.children(&mut cursor)
                .find(|n| n.kind() == "_proc_remainder")
                .map(|rem| {
                    let rem_ts = rem.child_by_field_name("remainder").unwrap();
                    let (rem_node, rem_end) = convert_ts_node_to_ir(rem_ts, source_code, current_prev_end);
                    current_prev_end = rem_end;
                    rem_node
                });
            let node = Arc::new(Node::List { base, elements, remainder, metadata });
            (node, absolute_end)
        }
        "set" => {
            let mut current_prev_end = absolute_start;
            let elements = ts_node.named_children(&mut ts_node.walk())
                .filter(|n| n.kind() != "_proc_remainder")
                .map(|child| {
                    let (node, end) = convert_ts_node_to_ir(child, source_code, current_prev_end);
                    current_prev_end = end;
                    node
                })
                .collect::<Vector<_, ArcK>>();
            let remainder = ts_node.children(&mut ts_node.walk())
                .find(|n| n.kind() == "_proc_remainder")
                .map(|rem| {
                    let rem_ts = rem.child_by_field_name("remainder").unwrap();
                    let (rem_node, rem_end) = convert_ts_node_to_ir(rem_ts, source_code, current_prev_end);
                    current_prev_end = rem_end;
                    rem_node
                });
            let node = Arc::new(Node::Set { base, elements, remainder, metadata });
            (node, absolute_end)
        }
        "map" => {
            let mut current_prev_end = absolute_start;
            let pairs = ts_node.named_children(&mut ts_node.walk())
                .filter(|n| n.kind() == "key_value_pair")
                .map(|pair| {
                    let key_ts = pair.child_by_field_name("key").unwrap();
                    let (key, key_end) = convert_ts_node_to_ir(key_ts, source_code, current_prev_end);
                    let value_ts = pair.child_by_field_name("value").unwrap();
                    let (value, value_end) = convert_ts_node_to_ir(value_ts, source_code, key_end);
                    current_prev_end = value_end;
                    (key, value)
                })
                .collect::<Vector<_, ArcK>>();
            let remainder = ts_node.children(&mut ts_node.walk())
                .find(|n| n.kind() == "_proc_remainder")
                .map(|rem| {
                    let rem_ts = rem.child_by_field_name("remainder").unwrap();
                    let (rem_node, rem_end) = convert_ts_node_to_ir(rem_ts, source_code, current_prev_end);
                    current_prev_end = rem_end;
                    rem_node
                });
            let node = Arc::new(Node::Map { base, pairs, remainder, metadata });
            (node, absolute_end)
        }
        "tuple" => {
            let mut current_prev_end = absolute_start;
            let elements = ts_node.named_children(&mut ts_node.walk())
                .map(|child| {
                    let (node, end) = convert_ts_node_to_ir(child, source_code, current_prev_end);
                    current_prev_end = end;
                    node
                })
                .collect::<Vector<_, ArcK>>();
            let node = Arc::new(Node::Tuple { base, elements, metadata });
            (node, absolute_end)
        }
        "var" => {
            let name = ts_node.utf8_text(source_code.as_bytes()).unwrap().to_string();
            let node = Arc::new(Node::Var { base, name, metadata });
            (node, absolute_end)
        }
        "name_decl" => {
            let var_ts = ts_node.named_child(0).unwrap();
            let (var, var_end) = convert_ts_node_to_ir(var_ts, source_code, absolute_start);
            let uri = ts_node.child_by_field_name("uri")
                .map(|uri_ts| {
                    let (uri_node, uri_end) = convert_ts_node_to_ir(uri_ts, source_code, var_end);
                    (uri_node, uri_end)
                });
            let node = Arc::new(Node::NameDecl { base, var, uri: uri.as_ref().map(|(u, _)| u.clone()), metadata });
            (node, uri.map_or(var_end, |(_, end)| end))
        }
        "decl" => {
            let names_ts = ts_node.child_by_field_name("names").unwrap();
            let (names, names_remainder, names_end) = collect_patterns(names_ts, source_code, absolute_start);
            let procs_ts = ts_node.child_by_field_name("procs").unwrap();
            let (procs, procs_end) = collect_named_descendants(procs_ts, source_code, names_end);
            let node = Arc::new(Node::Decl { base, names, names_remainder, procs, metadata });
            (node, procs_end)
        }
        "linear_bind" => {
            let names_ts = ts_node.child_by_field_name("names").unwrap();
            let (names, remainder, names_end) = collect_patterns(names_ts, source_code, absolute_start);
            let input_ts = ts_node.child_by_field_name("input").unwrap();
            let (source, source_end) = convert_ts_node_to_ir(input_ts, source_code, names_end);
            let node = Arc::new(Node::LinearBind { base, names, remainder, source, metadata });
            (node, source_end)
        }
        "repeated_bind" => {
            let names_ts = ts_node.child_by_field_name("names").unwrap();
            let (names, remainder, names_end) = collect_patterns(names_ts, source_code, absolute_start);
            let input_ts = ts_node.child_by_field_name("input").unwrap();
            let (source, source_end) = convert_ts_node_to_ir(input_ts, source_code, names_end);
            let node = Arc::new(Node::RepeatedBind { base, names, remainder, source, metadata });
            (node, source_end)
        }
        "peek_bind" => {
            let names_ts = ts_node.child_by_field_name("names").unwrap();
            let (names, remainder, names_end) = collect_patterns(names_ts, source_code, absolute_start);
            let input_ts = ts_node.child_by_field_name("input").unwrap();
            let (source, source_end) = convert_ts_node_to_ir(input_ts, source_code, names_end);
            let node = Arc::new(Node::PeekBind { base, names, remainder, source, metadata });
            (node, source_end)
        }
        "simple_source" => {
            let child = ts_node.named_child(0).unwrap();
            convert_ts_node_to_ir(child, source_code, absolute_start)
        }
        "receive_send_source" => {
            let name_ts = ts_node.named_child(0).unwrap();
            let (name, name_end) = convert_ts_node_to_ir(name_ts, source_code, absolute_start);
            let node = Arc::new(Node::ReceiveSendSource { base, name, metadata });
            (node, name_end)
        }
        "send_receive_source" => {
            let name_ts = ts_node.named_child(0).unwrap();
            let (name, name_end) = convert_ts_node_to_ir(name_ts, source_code, absolute_start);
            let inputs_ts = ts_node.child_by_field_name("inputs").unwrap();
            let mut current_prev_end = name_end;
            let inputs = inputs_ts.named_children(&mut inputs_ts.walk())
                .map(|child| {
                    let (node, end) = convert_ts_node_to_ir(child, source_code, current_prev_end);
                    current_prev_end = end;
                    node
                })
                .collect::<Vector<_, ArcK>>();
            let node = Arc::new(Node::SendReceiveSource { base, name, inputs, metadata });
            (node, absolute_end)
        }
        "wildcard" => {
            let node = Arc::new(Node::Wildcard { base, metadata });
            (node, absolute_end)
        }
        "simple_type" => {
            let value = ts_node.utf8_text(source_code.as_bytes()).unwrap().to_string();
            let node = Arc::new(Node::SimpleType { base, value, metadata });
            (node, absolute_end)
        }
        "line_comment" => {
            let node = Arc::new(Node::Comment { base, kind: CommentKind::Line, metadata });
            (node, absolute_end)
        }
        "block_comment" => {
            let node = Arc::new(Node::Comment { base, kind: CommentKind::Block, metadata });
            (node, absolute_end)
        }
        "ERROR" => {
            warn!("Encountered ERROR node at {}:{}", absolute_start.row, absolute_start.column);
            let (children, _) = collect_named_descendants(ts_node, source_code, absolute_start);
            let node = Arc::new(Node::Error { base, children, metadata });
            (node, absolute_end)
        }
        _ => {
            if ts_node.is_named() {
                warn!("Unhandled node type '{}'", ts_node.kind());
                let node = Arc::new(Node::Error {
                    base,
                    children: Vector::<Arc<Node<'_>>, ArcK>::new_with_ptr_kind(),
                    metadata,
                });
                (node, absolute_end)
            } else {
                let node = Arc::new(Node::Nil { base, metadata });
                (node, absolute_end)
            }
        }
    }
}

fn binary_op<'a>(ts_node: TSNode<'a>, source_code: &'a str, base: NodeBase<'a>, op: BinOperator, prev_end: Position) -> (Arc<Node<'a>>, Position) {
    let left_ts = ts_node.child(0).unwrap();
    let (left, left_end) = convert_ts_node_to_ir(left_ts, source_code, prev_end);
    let right_ts = ts_node.child(2).unwrap();
    let (right, right_end) = convert_ts_node_to_ir(right_ts, source_code, left_end);
    let mut data = HashMap::new();
    data.insert("version".to_string(), Arc::new(0_usize) as Arc<dyn Any + Send + Sync>);
    let metadata = Some(Arc::new(Metadata { data }));
    let node = Arc::new(Node::BinOp { base, op, left, right, metadata });
    (node, right_end)
}

fn unary_op<'a>(ts_node: TSNode<'a>, source_code: &'a str, base: NodeBase<'a>, op: UnaryOperator, prev_end: Position) -> (Arc<Node<'a>>, Position) {
    let operand_ts = ts_node.child(1).unwrap();
    let (operand, operand_end) = convert_ts_node_to_ir(operand_ts, source_code, prev_end);
    let mut data = HashMap::new();
    data.insert("version".to_string(), Arc::new(0_usize) as Arc<dyn Any + Send + Sync>);
    let metadata = Some(Arc::new(Metadata { data }));
    let node = Arc::new(Node::UnaryOp { base, op, operand, metadata });
    (node, operand_end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use quickcheck::{QuickCheck, TestResult};
    use test_utils::ir::generator::RholangProc;

    #[test]
    fn test_parse_send() {
        let code = "ch!(\"msg\")";
        let tree = parse_code(code);
        let ir = parse_to_ir(&tree, code);
        let root = Arc::new(ir.clone());
        match &*ir {
            Node::Send { channel, send_type, inputs, .. } => {
                assert_eq!(channel.text(), "ch");
                assert_eq!(*send_type, SendType::Single);
                assert_eq!(inputs.len(), 1);
                assert_eq!(inputs[0].text(), "\"msg\"");
                let start = ir.absolute_start(&root);
                assert_eq!(start.row, 0);
                assert_eq!(start.column, 0);
            }
            _ => panic!("Expected a Send node"),
        }
    }

    #[test]
    fn test_parse_par_position() {
        let _ = crate::logging::init_logger(false, Some("debug"));
        let code = "Nil | Nil";
        let tree = parse_code(code);
        let ir = parse_to_ir(&tree, code);
        let root = Arc::new(ir.clone());
        match &*ir {
            Node::Par { left, right, .. } => {
                let left_start = left.absolute_start(&root);
                assert_eq!(left_start.row, 0);
                assert_eq!(left_start.column, 0);
                let right_start = right.absolute_start(&root);
                assert_eq!(right_start.row, 0);
                assert_eq!(right_start.column, 6);
            }
            _ => panic!("Expected a Par node"),
        }
    }

    #[test]
    fn test_parse_new_nested() {
        let _ = crate::logging::init_logger(false, Some("debug"));
        let code = "new x in { x!() }";
        let tree = parse_code(code);
        let ir = parse_to_ir(&tree, code);
        let root = Arc::new(ir.clone());
        match &*ir {
            Node::New { decls, proc, .. } => {
                let decl_start = decls[0].absolute_start(&root);
                assert_eq!(decl_start.row, 0);
                assert_eq!(decl_start.column, 4);
                match &**proc {
                    Node::Block { proc: inner, .. } => {
                        match &**inner {
                            Node::Send { channel, .. } => {
                                let chan_start = channel.absolute_start(&root);
                                assert_eq!(chan_start.row, 0);
                                assert_eq!(chan_start.column, 11);
                            }
                            _ => panic!("Expected Send node"),
                        }
                    }
                    _ => panic!("Expected Block node"),
                }
            }
            _ => panic!("Expected New node"),
        }
    }

    #[test]
    fn test_position_consistency() {
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
    fn test_parse_parenthesized() {
        crate::logging::init_logger(false, Some("trace")).expect("Failed to initialize logger");
        let code = "(Nil)";
        let tree = parse_code(code);
        let ir = parse_to_ir(&tree, code);
        match &*ir {
            Node::Parenthesized { expr, .. } => {
                match &**expr {
                    Node::Nil { .. } => {}
                    _ => panic!("Expected Nil inside Parenthesized"),
                }
            }
            _ => panic!("Expected Parenthesized node"),
        }
    }

    #[test]
    fn test_parse_name_remainder() {
        crate::logging::init_logger(false, Some("trace")).expect("Failed to initialize logger");
        let code = "for(x, ...@y <- ch) { Nil }";
        let tree = parse_code(code);
        let ir = parse_to_ir(&tree, code);
        match &*ir {
            Node::Input { receipts, .. } => {
                let bind = &receipts[0][0];
                match &**bind {
                    Node::LinearBind { names, remainder, source, .. } => {
                        assert_eq!(names.len(), 1);
                        assert_eq!(names[0].text(), "x");
                        assert!(remainder.is_some());
                        let rem = remainder.as_ref().unwrap();
                        match &**rem {
                            Node::Quote { quotable, .. } => {
                                assert_eq!(quotable.text(), "y");
                            }
                            _ => panic!("Expected Quote for remainder"),
                        }
                        assert_eq!(source.text(), "ch");
                    }
                    _ => panic!("Expected LinearBind"),
                }
            }
            _ => panic!("Expected Input node"),
        }
    }

    #[test]
    fn test_parse_sync_send_empty_cont() {
        crate::logging::init_logger(false, Some("trace")).expect("Failed to initialize logger");
        let code = "ch!?(\"msg\").";
        let tree = parse_code(code);
        let ir = parse_to_ir(&tree, code);
        match &*ir {
            Node::SendSync { cont, .. } => {
                match &**cont {
                    Node::Nil { .. } => {}
                    _ => panic!("Expected Nil for empty continuation"),
                }
            }
            _ => panic!("Expected SendSync node"),
        }
    }

    #[test]
    fn test_parse_sync_send_non_empty_cont() {
        crate::logging::init_logger(false, Some("trace")).expect("Failed to initialize logger");
        let code = "ch!?(\"msg\"); Nil";
        let tree = parse_code(code);
        let ir = parse_to_ir(&tree, code);
        match &*ir {
            Node::SendSync { cont, .. } => {
                match &**cont {
                    Node::Nil { .. } => {}
                    _ => panic!("Expected Nil for continuation"),
                }
            }
            _ => panic!("Expected SendSync node"),
        }
    }
}
