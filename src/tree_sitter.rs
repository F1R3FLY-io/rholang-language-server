use std::any::Any;
use std::sync::Arc;
use std::collections::HashMap;

use tree_sitter::{InputEdit, Node as TSNode, Parser, Tree};

use tracing::{debug, trace, warn};

use rpds::Vector;

use archery::ArcK;

use ropey::Rope;

use crate::ir::rholang_node::{
    BinOperator, RholangBundleType, CommentKind, RholangNode, NodeBase, RholangSendType, UnaryOperator, RholangVarRefKind,
    Metadata, Position, RelativePosition
};

/// Safely slice a rope by byte range, returning empty string on invalid range
fn safe_byte_slice(rope: &Rope, start: usize, end: usize) -> String {
    if end > rope.len_bytes() || start > end {
        warn!("Invalid byte range {}-{} (rope len={})", start, end, rope.len_bytes());
        return String::new();
    }
    rope.byte_slice(start..end).to_string()
}

pub fn parse_code(code: &str) -> Tree {
    let mut parser = Parser::new();
    parser.set_language(&rholang_tree_sitter::LANGUAGE.into()).expect("Failed to set Tree-Sitter language");
    parser.parse(code, None).expect("Failed to parse Rholang code")
}

pub fn parse_to_ir(tree: &Tree, rope: &Rope) -> Arc<RholangNode> {
    debug!("Parsing Tree-Sitter tree into IR");
    if tree.root_node().has_error() {
        debug!("Parse tree contains errors");
    }
    let initial_prev_end = Position { row: 0, column: 0, byte: 0 };
    let (node, _) = convert_ts_node_to_ir(tree.root_node(), rope, initial_prev_end);
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
fn collect_named_descendants(node: TSNode, rope: &Rope, prev_end: Position) -> (Vector<Arc<RholangNode>, ArcK>, Position) {
    let mut nodes: Vector<Arc<RholangNode>, ArcK> = Vector::<Arc<RholangNode>, ArcK>::new_with_ptr_kind();
    let mut current_prev_end = prev_end;
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        let (child_node, child_end) = convert_ts_node_to_ir(child, rope, current_prev_end);
        nodes = nodes.push_back(child_node);
        current_prev_end = child_end;
    }
    (nodes, current_prev_end)
}

/// Collects patterns from a names node, separating elements and optional remainder.
fn collect_patterns(node: TSNode, rope: &Rope, prev_end: Position) -> (Vector<Arc<RholangNode>, ArcK>, Option<Arc<RholangNode>>, Position) {
    let mut elements: Vector<Arc<RholangNode>, ArcK> = Vector::<Arc<RholangNode>, ArcK>::new_with_ptr_kind();
    let mut remainder: Option<Arc<RholangNode>> = None;
    let mut current_prev_end = prev_end;
    let mut cursor = node.walk();
    let mut is_remainder = false;
    let mut is_quote = false;
    let mut quote_delta: Option<RelativePosition> = None;
    let mut quote_start_byte: Option<usize> = None;
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
            let absolute_start = Position {
                row: child.start_position().row,
                column: child.start_position().column,
                byte: child.start_byte(),
            };
            let delta_lines = absolute_start.row as i32 - current_prev_end.row as i32;
            let delta_columns = if delta_lines == 0 {
                absolute_start.column as i32 - current_prev_end.column as i32
            } else {
                absolute_start.column as i32
            };
            let delta_bytes = absolute_start.byte - current_prev_end.byte;
            quote_delta = Some(RelativePosition {
                delta_lines,
                delta_columns,
                delta_bytes,
            });
            quote_start_byte = Some(absolute_start.byte);
            current_prev_end = Position {
                row: child.end_position().row,
                column: child.end_position().column,
                byte: child.end_byte(),
            };
            continue;
        } else if child.is_named() {
            let (mut child_node, child_end) = convert_ts_node_to_ir(child, rope, current_prev_end);
            if let RholangNode::Var { ref name, .. } = *child_node && name.is_empty() {
                trace!("Skipped empty variable name at {:?}", current_prev_end);
                continue;
            }
            if is_quote {
                let q_delta = quote_delta.take().expect("Quote delta not set");
                let q_start_byte = quote_start_byte.take().expect("Quote start byte not set");
                let length = child.end_byte() - q_start_byte;
                let span_lines = child.end_position().row - child.start_position().row;
                let span_columns = if span_lines == 0 {
                    child.end_position().column - child.start_position().column + 1 // +1 for '@'
                } else {
                    child.end_position().column
                };
                let quote_base = NodeBase::new(
                    q_delta,
                    length,
                    span_lines,
                    span_columns,
                );
                child_node = Arc::new(RholangNode::Quote {
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
            current_prev_end = Position {
                row: child.end_position().row,
                column: child.end_position().column,
                byte: child.end_byte(),
            };
        }
    }
    (elements, remainder, current_prev_end)
}

/// Collects linear binds for Choice nodes, maintaining position continuity.
fn collect_linear_binds(branch_node: TSNode, rope: &Rope, prev_end: Position) -> (Vector<Arc<RholangNode>, ArcK>, Position) {
    let mut linear_binds: Vector<Arc<RholangNode>, ArcK> = Vector::<Arc<RholangNode>, ArcK>::new_with_ptr_kind();
    let mut current_prev_end = prev_end;
    let mut cursor = branch_node.walk();
    for child in branch_node.children(&mut cursor) {
        if child.kind() == "linear_bind" {
            let (bind_node, bind_end) = convert_ts_node_to_ir(child, rope, current_prev_end);
            linear_binds = linear_binds.push_back(bind_node);
            current_prev_end = bind_end;
        } else if child.kind() == "=>" {
            break;
        }
    }
    (linear_binds, current_prev_end)
}

/// Optimized comment detection using kind_id for O(1) comparison
#[inline(always)]
fn is_comment(kind_id: u16) -> bool {
    // Get the kind IDs for comment nodes
    // These are compile-time constants after the first call
    static LINE_COMMENT_KIND: std::sync::OnceLock<u16> = std::sync::OnceLock::new();
    static BLOCK_COMMENT_KIND: std::sync::OnceLock<u16> = std::sync::OnceLock::new();

    let language: tree_sitter::Language = rholang_tree_sitter::LANGUAGE.into();
    let line_comment_kind = *LINE_COMMENT_KIND.get_or_init(|| {
        language.id_for_node_kind("line_comment", true)
    });
    let block_comment_kind = *BLOCK_COMMENT_KIND.get_or_init(|| {
        language.id_for_node_kind("block_comment", true)
    });

    kind_id == line_comment_kind || kind_id == block_comment_kind
}

/// Converts Tree-Sitter nodes to IR nodes with accurate relative positions.
fn convert_ts_node_to_ir(ts_node: TSNode, rope: &Rope, prev_end: Position) -> (Arc<RholangNode>, Position) {
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
    // The delta_bytes must include bytes for whitespace and newlines to maintain accurate byte offsets.
    let delta_bytes = absolute_start.byte - prev_end.byte;
    debug!("absolute_start.byte = {}", absolute_start.byte);
    debug!("absolute_end.byte = {}", absolute_end.byte);
    debug!("delta_bytes = {}", delta_bytes);
    let relative_start = RelativePosition {
        delta_lines,
        delta_columns,
        delta_bytes,
    };
    trace!(
        "RholangNode '{}': prev_end={:?}, start={:?}, end={:?}, delta=({}, {}, {})",
        ts_node.kind(), prev_end, absolute_start, absolute_end, delta_lines, delta_columns, delta_bytes
    );
    let length = absolute_end.byte - absolute_start.byte;
    let span_lines = absolute_end.row - absolute_start.row;
    let span_columns = if span_lines == 0 {
        absolute_end.column - absolute_start.column
    } else {
        absolute_end.column
    };
    let base = NodeBase::new(
        relative_start,
        length,
        span_lines,
        span_columns,
    );
    let mut data = HashMap::new();
    data.insert("version".to_string(), Arc::new(0usize) as Arc<dyn Any + Send + Sync>);
    let metadata = Some(Arc::new(data));

    match ts_node.kind() {
        "source_file" => {
            let mut current_prev_end = absolute_start;
            let mut all_nodes = Vec::new();

            // Comments are named nodes in extras, so filter them efficiently
            for child in ts_node.named_children(&mut ts_node.walk()) {
                // Skip comments - they're in extras and don't belong in the IR
                let kind_id = child.kind_id();
                if is_comment(kind_id) {
                    continue;
                }
                debug!("Before converting child '{}': current_prev_end = {:?}", child.kind(), current_prev_end);
                let (node, end) = convert_ts_node_to_ir(child, rope, current_prev_end);
                debug!("After converting child '{}': end = {:?}", child.kind(), end);
                all_nodes.push(node);
                current_prev_end = end;
            }

            let result = if all_nodes.len() == 1 {
                all_nodes[0].clone()
            } else if all_nodes.is_empty() {
                Arc::new(RholangNode::Nil { base: base.clone(), metadata: metadata.clone() })
            } else {
                // When reducing multiple top-level nodes into nested Par, each Par should have
                // zero relative position (starts where parent starts) and zero span.
                // The actual span is determined by children during position computation.
                let par_base = NodeBase::new(
                    RelativePosition {
                        delta_lines: 0,
                        delta_columns: 0,
                        delta_bytes: 0,
                    },
                    0,  // length
                    0,  // span_lines
                    0,  // span_columns
                );

                all_nodes.into_iter().reduce(|left, right| {
                    Arc::new(RholangNode::Par {
                        base: par_base.clone(),
                        left,
                        right,
                        metadata: metadata.clone(),
                    })
                }).expect("Expected at least one child for source_file reduction")
            };
            (result, absolute_end)
        }
        "collection" => {
            let child = ts_node.named_child(0).expect("Collection node must have a named child");
            convert_ts_node_to_ir(child, rope, prev_end)
        }
        "par" => {
            let left_ts = ts_node.child(0).expect("Par node must have a left child");
            let (left, left_end) = convert_ts_node_to_ir(left_ts, rope, absolute_start);
            let right_ts = ts_node.child(2).expect("Par node must have a right child"); // After '|'
            let (right, right_end) = convert_ts_node_to_ir(right_ts, rope, left_end);
            let node = Arc::new(RholangNode::Par { base, left, right, metadata });
            (node, right_end)
        }
        "send_sync" => {
            let channel_ts = ts_node.child_by_field_name("channel").expect("SendSync node must have a channel");
            let (channel, channel_end) = convert_ts_node_to_ir(channel_ts, rope, absolute_start);
            let inputs_ts = ts_node.child_by_field_name("inputs").expect("SendSync node must have inputs");
            let mut current_prev_end = channel_end;
            let inputs = inputs_ts.named_children(&mut inputs_ts.walk())
                .map(|child| {
                    let (node, end) = convert_ts_node_to_ir(child, rope, current_prev_end);
                    current_prev_end = end;
                    node
                })
                .collect::<Vector<_, ArcK>>();
            let cont_ts = ts_node.child_by_field_name("cont").expect("SendSync node must have a continuation");
            let (cont, cont_end) = convert_ts_node_to_ir(cont_ts, rope, current_prev_end);
            let node = Arc::new(RholangNode::SendSync { base, channel, inputs, cont, metadata });
            (node, cont_end)
        }
        "non_empty_cont" => {
            let proc_ts = ts_node.named_child(0).expect("NonEmptyCont node must have a process");
            convert_ts_node_to_ir(proc_ts, rope, prev_end)
        }
        "empty_cont" => {
            let node = Arc::new(RholangNode::Nil { base, metadata });
            (node, absolute_end)
        }
        "sync_send_cont" => {
            if ts_node.named_child_count() == 0 {
                let node = Arc::new(RholangNode::Nil { base, metadata });
                (node, absolute_end)
            } else {
                let proc_ts = ts_node.named_child(0).expect("SyncSendCont node must have a process");
                convert_ts_node_to_ir(proc_ts, rope, absolute_start)
            }
        }
        "send" => {
            let channel_ts = ts_node.child_by_field_name("channel").expect("Send node must have a channel");
            let (channel, channel_end) = convert_ts_node_to_ir(channel_ts, rope, absolute_start);
            let send_type_ts = ts_node.child_by_field_name("send_type").expect("Send node must have a send_type");
            let send_type_abs_end = Position {
                row: send_type_ts.end_position().row,
                column: send_type_ts.end_position().column,
                byte: send_type_ts.end_byte(),
            };
            let send_type_delta_lines = send_type_abs_end.row as i32 - channel_end.row as i32;
            let send_type_delta_columns = if send_type_delta_lines == 0 {
                send_type_abs_end.column as i32 - channel_end.column as i32
            } else {
                send_type_abs_end.column as i32
            };
            let send_type_delta_bytes = send_type_abs_end.byte - channel_end.byte;
            let send_type_delta = RelativePosition {
                delta_lines: send_type_delta_lines,
                delta_columns: send_type_delta_columns,
                delta_bytes: send_type_delta_bytes,
            };
            let inputs_ts = ts_node.child_by_field_name("inputs").expect("Send node must have inputs");
            let mut current_prev_end = send_type_abs_end;
            let inputs = inputs_ts.named_children(&mut inputs_ts.walk())
                .map(|child| {
                    let (node, end) = convert_ts_node_to_ir(child, rope, current_prev_end);
                    current_prev_end = end;
                    node
                })
                .collect::<Vector<_, ArcK>>();
            let send_type = match send_type_ts.kind() {
                "send_single" => RholangSendType::Single,
                "send_multiple" => RholangSendType::Multiple,
                kind => {
                    warn!("Unknown send_type: {}", kind);
                    RholangSendType::Single
                }
            };
            let node = Arc::new(RholangNode::Send {
                base,
                channel,
                send_type,
                send_type_delta,
                inputs,
                metadata,
            });
            (node, absolute_end)
        }
        "new" => {
            let decls_ts = ts_node.child_by_field_name("decls").expect("New node must have decls");
            let (decls, decls_end) = collect_named_descendants(decls_ts, rope, absolute_start);
            let proc_ts = ts_node.child_by_field_name("proc").expect("New node must have a process");
            let (proc, proc_end) = convert_ts_node_to_ir(proc_ts, rope, decls_end);
            let node = Arc::new(RholangNode::New { base, decls, proc, metadata });
            (node, proc_end)
        }
        "ifElse" => {
            let condition_ts = ts_node.named_child(0).expect("IfElse node must have a condition");
            let (condition, cond_end) = convert_ts_node_to_ir(condition_ts, rope, absolute_start);
            let consequence_ts = ts_node.named_child(1).expect("IfElse node must have a consequence");
            let (consequence, cons_end) = convert_ts_node_to_ir(consequence_ts, rope, cond_end);
            let alternative = if ts_node.named_child_count() > 2 {
                let alt_ts = ts_node.named_child(2).expect("IfElse node alternative child missing");
                let (alt, alt_end) = convert_ts_node_to_ir(alt_ts, rope, cons_end);
                Some((alt, alt_end))
            } else {
                None
            };
            let node = Arc::new(RholangNode::IfElse {
                base,
                condition,
                consequence,
                alternative: alternative.as_ref().map(|(alt, _)| alt.clone()),
                metadata,
            });
            (node, alternative.map_or(cons_end, |(_, end)| end))
        }
        "let" => {
            let decls_ts = ts_node.child_by_field_name("decls").expect("Let node must have decls");
            let (decls, decls_end) = collect_named_descendants(decls_ts, rope, absolute_start);
            let proc_ts = ts_node.child_by_field_name("proc").expect("Let node must have a process");
            let (proc, proc_end) = convert_ts_node_to_ir(proc_ts, rope, decls_end);
            let node = Arc::new(RholangNode::Let { base, decls, proc, metadata });
            (node, proc_end)
        }
        "bundle" => {
            let bundle_type_ts = ts_node.child_by_field_name("bundle_type").expect("Bundle node must have a bundle_type");
            let bundle_type_end = Position {
                row: bundle_type_ts.end_position().row,
                column: bundle_type_ts.end_position().column,
                byte: bundle_type_ts.end_byte(),
            };
            let bundle_type = match bundle_type_ts.kind() {
                "bundle_read" => RholangBundleType::Read,
                "bundle_write" => RholangBundleType::Write,
                "bundle_equiv" => RholangBundleType::Equiv,
                "bundle_read_write" => RholangBundleType::ReadWrite,
                kind => {
                    warn!("Unknown bundle type: {}", kind);
                    RholangBundleType::ReadWrite
                }
            };
            let proc_ts = ts_node.child_by_field_name("proc").expect("Bundle node must have a process");
            let (proc, proc_end) = convert_ts_node_to_ir(proc_ts, rope, bundle_type_end);
            let node = Arc::new(RholangNode::Bundle { base, bundle_type, proc, metadata });
            (node, proc_end)
        }
        "match" => {
            let expression_ts = ts_node.child_by_field_name("expression").expect("Match node must have an expression");
            let (expression, expr_end) = convert_ts_node_to_ir(expression_ts, rope, absolute_start);
            let cases_ts = ts_node.child_by_field_name("cases").expect("Match node must have cases");
            let mut current_prev_end = expr_end;
            let cases = cases_ts.named_children(&mut cases_ts.walk())
                .filter(|n| n.kind() == "case")
                .map(|case_node| {
                    let pattern_ts = case_node.child_by_field_name("pattern").expect("Case node must have a pattern");
                    let (pattern, pat_end) = convert_ts_node_to_ir(pattern_ts, rope, current_prev_end);
                    let proc_ts = case_node.child_by_field_name("proc").expect("Case node must have a process");
                    let (proc, proc_end) = convert_ts_node_to_ir(proc_ts, rope, pat_end);
                    current_prev_end = proc_end;
                    (pattern, proc)
                })
                .collect::<Vector<_, ArcK>>();
            let node = Arc::new(RholangNode::Match { base, expression, cases, metadata });
            (node, current_prev_end)
        }
        "choice" => {
            let branches_ts = ts_node.child_by_field_name("branches").expect("Choice node must have branches");
            let mut current_prev_end = absolute_start;
            let branches = branches_ts.named_children(&mut branches_ts.walk())
                .filter(|n| n.kind() == "branch")
                .map(|branch_node| {
                    let (inputs, inputs_end) = collect_linear_binds(branch_node, rope, current_prev_end);
                    let proc_ts = branch_node.child_by_field_name("proc").expect("Branch node must have a process");
                    let (proc, proc_end) = convert_ts_node_to_ir(proc_ts, rope, inputs_end);
                    current_prev_end = proc_end;
                    (inputs, proc)
                })
                .collect::<Vector<_, ArcK>>();
            let node = Arc::new(RholangNode::Choice { base, branches, metadata });
            (node, current_prev_end)
        }
        "contract" => {
            let name_ts = ts_node.child_by_field_name("name").expect("Contract node must have a name");
            let (name, name_end) = convert_ts_node_to_ir(name_ts, rope, absolute_start);
            let formals_ts_opt = ts_node.child_by_field_name("formals");
            let (formals, formals_remainder, formals_end) = if let Some(formals_ts) = formals_ts_opt {
                collect_patterns(formals_ts, rope, name_end)
            } else {
                (Vector::new_with_ptr_kind(), None, name_end)
            };
            let proc_ts = ts_node.child_by_field_name("proc").expect("Contract node must have a process");
            let (proc, proc_end) = convert_ts_node_to_ir(proc_ts, rope, formals_end);
            let node = Arc::new(RholangNode::Contract { base, name, formals, formals_remainder, proc, metadata });
            (node, proc_end)
        }
        "input" => {
            let receipts_ts = ts_node.child_by_field_name("receipts").expect("Input node must have receipts");
            let mut current_prev_end = absolute_start;
            let receipts = receipts_ts.named_children(&mut receipts_ts.walk())
                .map(|receipt_node| {
                    let (binds, binds_end) = collect_named_descendants(receipt_node, rope, current_prev_end);
                    current_prev_end = binds_end;
                    binds
                })
                .collect::<Vector<_, ArcK>>();
            let proc_ts = ts_node.child_by_field_name("proc").expect("Input node must have a process");
            let (proc, proc_end) = convert_ts_node_to_ir(proc_ts, rope, current_prev_end);
            let node = Arc::new(RholangNode::Input { base, receipts, proc, metadata });
            (node, proc_end)
        }
        "block" => {
            let proc_ts = ts_node.child(1).expect("Block node must have a process"); // After '{'
            let (proc, _proc_end) = convert_ts_node_to_ir(proc_ts, rope, absolute_start);
            let node = Arc::new(RholangNode::Block { base, proc, metadata });
            (node, absolute_end)  // Block includes '{' and '}', so use absolute_end
        }
        "_parenthesized" => {
            let expr_ts = ts_node.named_child(0).expect("Parenthesized node must have an expression");
            let (expr, _expr_end) = convert_ts_node_to_ir(expr_ts, rope, absolute_start);
            let node = Arc::new(RholangNode::Parenthesized { base, expr, metadata });
            (node, absolute_end)  // Parenthesized includes '(' and ')', so use absolute_end
        }
        "_name_remainder" => {
            let cont_ts = ts_node.child_by_field_name("cont").expect("NameRemainder node must have a continuation");
            let (cont, cont_end) = convert_ts_node_to_ir(cont_ts, rope, absolute_start);
            let node = Arc::new(RholangNode::Quote { base, quotable: cont, metadata });
            (node, cont_end)
        }
        "or" => binary_op(ts_node, rope, base, BinOperator::Or, absolute_start),
        "and" => binary_op(ts_node, rope, base, BinOperator::And, absolute_start),
        "matches" => binary_op(ts_node, rope, base, BinOperator::Matches, absolute_start),
        "eq" => binary_op(ts_node, rope, base, BinOperator::Eq, absolute_start),
        "neq" => binary_op(ts_node, rope, base, BinOperator::Neq, absolute_start),
        "lt" => binary_op(ts_node, rope, base, BinOperator::Lt, absolute_start),
        "lte" => binary_op(ts_node, rope, base, BinOperator::Lte, absolute_start),
        "gt" => binary_op(ts_node, rope, base, BinOperator::Gt, absolute_start),
        "gte" => binary_op(ts_node, rope, base, BinOperator::Gte, absolute_start),
        "concat" => binary_op(ts_node, rope, base, BinOperator::Concat, absolute_start),
        "diff" => binary_op(ts_node, rope, base, BinOperator::Diff, absolute_start),
        "add" => binary_op(ts_node, rope, base, BinOperator::Add, absolute_start),
        "sub" => binary_op(ts_node, rope, base, BinOperator::Sub, absolute_start),
        "interpolation" => binary_op(ts_node, rope, base, BinOperator::Interpolation, absolute_start),
        "mult" => binary_op(ts_node, rope, base, BinOperator::Mult, absolute_start),
        "div" => binary_op(ts_node, rope, base, BinOperator::Div, absolute_start),
        "mod" => binary_op(ts_node, rope, base, BinOperator::Mod, absolute_start),
        "not" => unary_op(ts_node, rope, base, UnaryOperator::Not, absolute_start),
        "neg" => unary_op(ts_node, rope, base, UnaryOperator::Neg, absolute_start),
        "method" => {
            let receiver_ts = ts_node.child_by_field_name("receiver").expect("Method node must have a receiver");
            let (receiver, _receiver_end) = convert_ts_node_to_ir(receiver_ts, rope, absolute_start);
            let name_ts = ts_node.child_by_field_name("name").expect("Method node must have a name");
            let name = rope.byte_slice(name_ts.start_byte()..name_ts.end_byte()).to_string();
            let name_end = Position {
                row: name_ts.end_position().row,
                column: name_ts.end_position().column,
                byte: name_ts.end_byte(),
            };
            let args_ts = ts_node.child_by_field_name("args").expect("Method node must have args");
            let mut current_prev_end = name_end;
            let args = args_ts.named_children(&mut args_ts.walk())
                .map(|child| {
                    let (node, end) = convert_ts_node_to_ir(child, rope, current_prev_end);
                    current_prev_end = end;
                    node
                })
                .collect::<Vector<_, ArcK>>();
            let node = Arc::new(RholangNode::Method { base, receiver, name, args, metadata });
            (node, absolute_end)
        }
        "eval" => {
            let name_ts = ts_node.child(1).expect("Eval node must have a name");
            let (name, name_end) = convert_ts_node_to_ir(name_ts, rope, absolute_start);
            let node = Arc::new(RholangNode::Eval { base, name, metadata });
            (node, name_end)
        }
        "quote" => {
            let quotable_ts = ts_node.child(1).expect("Quote node must have a quotable");
            let (quotable, quotable_end) = convert_ts_node_to_ir(quotable_ts, rope, absolute_start);
            let node = Arc::new(RholangNode::Quote { base, quotable, metadata });
            (node, quotable_end)
        }
        "var_ref" => {
            let kind_ts = ts_node.child_by_field_name("kind").expect("VarRef node must have a kind");
            let kind_text = safe_byte_slice(rope, kind_ts.start_byte(), kind_ts.end_byte());
            let kind = match kind_text.as_str() {
                "=" => RholangVarRefKind::Bind,
                "=*" => RholangVarRefKind::Unforgeable,
                kind => {
                    warn!("Unknown var_ref kind text: {:?}", kind);
                    RholangVarRefKind::Bind
                },
            };
            let var_ts = ts_node.child_by_field_name("var").expect("VarRef node must have a var");
            let (var, var_end) = convert_ts_node_to_ir(var_ts, rope, absolute_start);
            let node = Arc::new(RholangNode::VarRef { base, kind, var, metadata });
            (node, var_end)
        }
        "disjunction" => binary_op(ts_node, rope, base, BinOperator::Disjunction, absolute_start),
        "conjunction" => binary_op(ts_node, rope, base, BinOperator::Conjunction, absolute_start),
        "negation" => unary_op(ts_node, rope, base, UnaryOperator::Negation, absolute_start),
        "_ground_expression" => {
            let child = ts_node.named_child(0).expect("GroundExpression node must have a child");
            convert_ts_node_to_ir(child, rope, prev_end)
        }
        "bool_literal" => {
            let slice_str = safe_byte_slice(rope, ts_node.start_byte(), ts_node.end_byte());
            let value = slice_str == "true";
            let node = Arc::new(RholangNode::BoolLiteral { base, value, metadata });
            (node, absolute_end)
        }
        "long_literal" => {
            // Check bounds to avoid panic
            let start_byte = ts_node.start_byte();
            let end_byte = ts_node.end_byte();
            if end_byte > rope.len_bytes() || start_byte > end_byte {
                warn!("Invalid byte range for long_literal at {}-{} (rope len={})", start_byte, end_byte, rope.len_bytes());
                let node = Arc::new(RholangNode::Error {
                    base,
                    children: Vector::new_with_ptr_kind(),
                    metadata,
                });
                return (node, absolute_end);
            }

            let slice_str = safe_byte_slice(rope, start_byte, end_byte);
            // Validate that the string contains only valid integer characters
            let is_valid = slice_str.chars().all(|c| c.is_ascii_digit() || c == '-');
            if !is_valid {
                warn!("Invalid long literal '{}' at byte {}: contains non-numeric characters", slice_str, absolute_start.byte);
                let node = Arc::new(RholangNode::Error {
                    base,
                    children: Vector::new_with_ptr_kind(),
                    metadata,
                });
                return (node, absolute_end);
            }
            let value = slice_str.parse::<i64>().unwrap_or_else(|_| {
                warn!("Failed to parse long literal '{}' at byte {}", slice_str, absolute_start.byte);
                0 // Return 0 instead of panicking
            });
            let node = Arc::new(RholangNode::LongLiteral { base, value, metadata });
            (node, absolute_end)
        }
        "string_literal" => {
            let inner_start = ts_node.start_byte() + 1;
            let inner_end = ts_node.end_byte() - 1;
            let value = if inner_end > inner_start {
                let inner_slice = rope.byte_slice(inner_start..inner_end);
                let inner_str = inner_slice.to_string();
                inner_str.replace("\\\"", "\"").replace("\\\\", "\\")
            } else {
                debug!("Invalid string literal at byte {}", absolute_start.byte);
                String::new()
            };
            let node = Arc::new(RholangNode::StringLiteral { base, value, metadata });
            (node, absolute_end)
        }
        "uri_literal" => {
            let inner_start = ts_node.start_byte() + 1;
            let inner_end = ts_node.end_byte() - 1;
            let value = if inner_end > inner_start {
                let inner_slice = rope.byte_slice(inner_start..inner_end);
                inner_slice.to_string()
            } else {
                warn!("Invalid URI literal at byte {}", absolute_start.byte);
                String::new()
            };
            let node = Arc::new(RholangNode::UriLiteral { base, value, metadata });
            (node, absolute_end)
        }
        "nil" => {
            let node = Arc::new(RholangNode::Nil { base, metadata });
            (node, absolute_end)
        }
        "list" => {
            let mut current_prev_end = absolute_start;
            let mut cursor = ts_node.walk();
            let elements = ts_node.named_children(&mut cursor)
                .filter(|n| n.kind() != "_proc_remainder" && n.is_named())
                .map(|child| {
                    let (node, end) = convert_ts_node_to_ir(child, rope, current_prev_end);
                    current_prev_end = end;
                    node
                })
                .collect::<Vector<_, ArcK>>();
            let remainder = ts_node.children(&mut cursor)
                .find(|n| n.kind() == "_proc_remainder")
                .map(|rem| {
                    let rem_ts = rem.child_by_field_name("remainder").expect("ProcRemainder node must have a remainder");
                    let (rem_node, rem_end) = convert_ts_node_to_ir(rem_ts, rope, current_prev_end);
                    current_prev_end = rem_end;
                    rem_node
                });
            let node = Arc::new(RholangNode::List { base, elements, remainder, metadata });
            (node, absolute_end)
        }
        "set" => {
            let mut current_prev_end = absolute_start;
            let elements = ts_node.named_children(&mut ts_node.walk())
                .filter(|n| n.kind() != "_proc_remainder")
                .map(|child| {
                    let (node, end) = convert_ts_node_to_ir(child, rope, current_prev_end);
                    current_prev_end = end;
                    node
                })
                .collect::<Vector<_, ArcK>>();
            let remainder = ts_node.children(&mut ts_node.walk())
                .find(|n| n.kind() == "_proc_remainder")
                .map(|rem| {
                    let rem_ts = rem.child_by_field_name("remainder").expect("ProcRemainder node must have a remainder");
                    let (rem_node, rem_end) = convert_ts_node_to_ir(rem_ts, rope, current_prev_end);
                    current_prev_end = rem_end;
                    rem_node
                });
            let node = Arc::new(RholangNode::Set { base, elements, remainder, metadata });
            (node, absolute_end)
        }
        "map" => {
            let mut current_prev_end = absolute_start;
            let pairs = ts_node.named_children(&mut ts_node.walk())
                .filter(|n| n.kind() == "key_value_pair")
                .map(|pair| {
                    let key_ts = pair.child_by_field_name("key").expect("KeyValuePair node must have a key");
                    let (key, key_end) = convert_ts_node_to_ir(key_ts, rope, current_prev_end);
                    let value_ts = pair.child_by_field_name("value").expect("KeyValuePair node must have a value");
                    let (value, value_end) = convert_ts_node_to_ir(value_ts, rope, key_end);
                    current_prev_end = value_end;
                    (key, value)
                })
                .collect::<Vector<_, ArcK>>();
            let remainder = ts_node.children(&mut ts_node.walk())
                .find(|n| n.kind() == "_proc_remainder")
                .map(|rem| {
                    let rem_ts = rem.child_by_field_name("remainder").expect("ProcRemainder node must have a remainder");
                    let (rem_node, rem_end) = convert_ts_node_to_ir(rem_ts, rope, current_prev_end);
                    current_prev_end = rem_end;
                    rem_node
                });
            let node = Arc::new(RholangNode::Map { base, pairs, remainder, metadata });
            (node, absolute_end)
        }
        "tuple" => {
            let mut current_prev_end = absolute_start;
            let elements = ts_node.named_children(&mut ts_node.walk())
                .map(|child| {
                    let (node, end) = convert_ts_node_to_ir(child, rope, current_prev_end);
                    current_prev_end = end;
                    node
                })
                .collect::<Vector<_, ArcK>>();
            let node = Arc::new(RholangNode::Tuple { base, elements, metadata });
            (node, absolute_end)
        }
        "var" => {
            let name = safe_byte_slice(rope, ts_node.start_byte(), ts_node.end_byte());
            let node = Arc::new(RholangNode::Var { base, name, metadata });
            (node, absolute_end)
        }
        "name_decl" => {
            let var_ts = ts_node.named_child(0).expect("NameDecl node must have a variable");
            let (var, var_end) = convert_ts_node_to_ir(var_ts, rope, absolute_start);
            let uri = ts_node.child_by_field_name("uri")
                .map(|uri_ts| {
                    let (uri_node, uri_end) = convert_ts_node_to_ir(uri_ts, rope, var_end);
                    (uri_node, uri_end)
                });
            let node = Arc::new(RholangNode::NameDecl { base, var, uri: uri.as_ref().map(|(u, _)| u.clone()), metadata });
            (node, uri.map_or(var_end, |(_, end)| end))
        }
        "decl" => {
            let names_ts = ts_node.child_by_field_name("names").expect("Decl node must have names");
            let (names, names_remainder, names_end) = collect_patterns(names_ts, rope, absolute_start);
            let procs_ts = ts_node.child_by_field_name("procs").expect("Decl node must have procs");
            let (procs, procs_end) = collect_named_descendants(procs_ts, rope, names_end);
            let node = Arc::new(RholangNode::Decl { base, names, names_remainder, procs, metadata });
            (node, procs_end)
        }
        "linear_bind" => {
            let names_ts_opt = ts_node.child_by_field_name("names");
            let (mut names, remainder, names_end) = if let Some(names_ts) = names_ts_opt {
                collect_patterns(names_ts, rope, absolute_start)
            } else {
                (Vector::new_with_ptr_kind(), None, absolute_start)
            };
            if names.is_empty() && remainder.is_none() {
                let wildcard_base = NodeBase::new(
                    RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 },
                    1,
                    0,
                    1,
                );
                names = names.push_back(Arc::new(RholangNode::Wildcard { base: wildcard_base, metadata: None }));
            }
            let input_ts = ts_node.child_by_field_name("input").expect("LinearBind node must have an input");
            let (source, source_end) = convert_ts_node_to_ir(input_ts, rope, names_end);
            let node = Arc::new(RholangNode::LinearBind { base, names, remainder, source, metadata });
            (node, source_end)
        }
        "repeated_bind" => {
            let names_ts_opt = ts_node.child_by_field_name("names");
            let (mut names, remainder, names_end) = if let Some(names_ts) = names_ts_opt {
                collect_patterns(names_ts, rope, absolute_start)
            } else {
                (Vector::new_with_ptr_kind(), None, absolute_start)
            };
            if names.is_empty() && remainder.is_none() {
                let wildcard_base = NodeBase::new(
                    RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 },
                    1,
                    0,
                    1,
                );
                names = names.push_back(Arc::new(RholangNode::Wildcard { base: wildcard_base, metadata: None }));
            }
            let input_ts = ts_node.child_by_field_name("input").expect("RepeatedBind node must have an input");
            let (source, source_end) = convert_ts_node_to_ir(input_ts, rope, names_end);
            let node = Arc::new(RholangNode::RepeatedBind { base, names, remainder, source, metadata });
            (node, source_end)
        }
        "peek_bind" => {
            let names_ts_opt = ts_node.child_by_field_name("names");
            let (mut names, remainder, names_end) = if let Some(names_ts) = names_ts_opt {
                collect_patterns(names_ts, rope, absolute_start)
            } else {
                (Vector::new_with_ptr_kind(), None, absolute_start)
            };
            if names.is_empty() && remainder.is_none() {
                let wildcard_base = NodeBase::new(
                    RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 },
                    1,
                    0,
                    1,
                );
                names = names.push_back(Arc::new(RholangNode::Wildcard { base: wildcard_base, metadata: None }));
            }
            let input_ts = ts_node.child_by_field_name("input").expect("PeekBind node must have an input");
            let (source, source_end) = convert_ts_node_to_ir(input_ts, rope, names_end);
            let node = Arc::new(RholangNode::PeekBind { base, names, remainder, source, metadata });
            (node, source_end)
        }
        "simple_source" => {
            let child = ts_node.named_child(0).expect("SimpleSource node must have a child");
            convert_ts_node_to_ir(child, rope, prev_end)
        }
        "receive_send_source" => {
            let name_ts = ts_node.named_child(0).expect("ReceiveSendSource node must have a name");
            let (name, name_end) = convert_ts_node_to_ir(name_ts, rope, absolute_start);
            let node = Arc::new(RholangNode::ReceiveSendSource { base, name, metadata });
            (node, name_end)
        }
        "send_receive_source" => {
            let name_ts = ts_node.named_child(0).expect("SendReceiveSource node must have a name");
            let (name, name_end) = convert_ts_node_to_ir(name_ts, rope, absolute_start);
            let inputs_ts = ts_node.child_by_field_name("inputs").expect("SendReceiveSource node must have inputs");
            let mut current_prev_end = name_end;
            let inputs = inputs_ts.named_children(&mut inputs_ts.walk())
                .map(|child| {
                    let (node, end) = convert_ts_node_to_ir(child, rope, current_prev_end);
                    current_prev_end = end;
                    node
                })
                .collect::<Vector<_, ArcK>>();
            let node = Arc::new(RholangNode::SendReceiveSource { base, name, inputs, metadata });
            (node, absolute_end)
        }
        "wildcard" => {
            let node = Arc::new(RholangNode::Wildcard { base, metadata });
            (node, absolute_end)
        }
        "simple_type" => {
            let value = safe_byte_slice(rope, ts_node.start_byte(), ts_node.end_byte());
            let node = Arc::new(RholangNode::SimpleType { base, value, metadata });
            (node, absolute_end)
        }
        "line_comment" => {
            let node = Arc::new(RholangNode::Comment { base, kind: CommentKind::Line, metadata });
            (node, absolute_end)
        }
        "block_comment" => {
            let node = Arc::new(RholangNode::Comment { base, kind: CommentKind::Block, metadata });
            (node, absolute_end)
        }
        "unit" => {
            let node = Arc::new(RholangNode::Unit { base, metadata });
            (node, absolute_end)
        }
        "ERROR" => {
            debug!("Encountered ERROR node at {}:{}", absolute_start.row, absolute_start.column);
            let (children, _) = collect_named_descendants(ts_node, rope, absolute_start);
            let node = Arc::new(RholangNode::Error { base, children, metadata });
            (node, absolute_end)
        }
        _ => {
            if ts_node.is_named() {
                warn!("Unhandled node type '{}' at byte {}", ts_node.kind(), absolute_start.byte);
                let node = Arc::new(RholangNode::Error {
                    base,
                    children: Vector::<Arc<RholangNode>, ArcK>::new_with_ptr_kind(),
                    metadata,
                });
                (node, absolute_end)
            } else {
                let node = Arc::new(RholangNode::Nil { base, metadata });
                (node, absolute_end)
            }
        }
    }
}

fn binary_op(ts_node: TSNode, rope: &Rope, base: NodeBase, op: BinOperator, prev_end: Position) -> (Arc<RholangNode>, Position) {
    let left_ts = ts_node.child(0).expect("BinaryOp node must have a left operand");
    let (left, left_end) = convert_ts_node_to_ir(left_ts, rope, prev_end);
    let right_ts = ts_node.child(2).expect("BinaryOp node must have a right operand");
    let (right, right_end) = convert_ts_node_to_ir(right_ts, rope, left_end);
    let mut data = HashMap::new();
    data.insert("version".to_string(), Arc::new(0usize) as Arc<dyn Any + Send + Sync>);
    let metadata = Some(Arc::new(data));
    let node = Arc::new(RholangNode::BinOp { base, op, left, right, metadata });
    (node, right_end)
}

fn unary_op(ts_node: TSNode, rope: &Rope, base: NodeBase, op: UnaryOperator, prev_end: Position) -> (Arc<RholangNode>, Position) {
    let operand_ts = ts_node.child(1).expect("UnaryOp node must have an operand");
    let (operand, operand_end) = convert_ts_node_to_ir(operand_ts, rope, prev_end);
    let mut data = HashMap::new();
    data.insert("version".to_string(), Arc::new(0usize) as Arc<dyn Any + Send + Sync>);
    let metadata = Some(Arc::new(data));
    let node = Arc::new(RholangNode::UnaryOp { base, op, operand, metadata });
    (node, operand_end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use quickcheck::{QuickCheck, TestResult};
    use test_utils::ir::generator::RholangProc;

    #[test]
    fn test_parse_send() {
        let code = r#"ch!("msg")"#;
        let tree = parse_code(code);
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree, &rope);
        let root = ir.clone();
        match &*ir {
            RholangNode::Send { channel, send_type, inputs, .. } => {
                assert_eq!(channel.text(&rope, &root).to_string(), "ch");
                assert_eq!(*send_type, RholangSendType::Single);
                assert_eq!(inputs.len(), 1);
                assert_eq!(inputs[0].text(&rope, &root).to_string(), r#""msg""#);
                let start = ir.absolute_start(&root);
                assert_eq!(start.row, 0);
                assert_eq!(start.column, 0);
            }
            _ => panic!("Expected a Send node"),
        }
    }

    #[test]
    fn test_parse_par_position() {
        let _ = crate::logging::init_logger(false, Some("warn"), false);
        let code = r#"Nil | Nil"#;
        let tree = parse_code(code);
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree, &rope);
        let root = ir.clone();
        match &*ir {
            RholangNode::Par { left, right, .. } => {
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
        let _ = crate::logging::init_logger(false, Some("warn"), false);
        let code = r#"new x in { x!() }"#;
        let tree = parse_code(code);
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree, &rope);
        let root = ir.clone();
        match &*ir {
            RholangNode::New { decls, proc, .. } => {
                let decl_start = decls[0].absolute_start(&root);
                assert_eq!(decl_start.row, 0);
                assert_eq!(decl_start.column, 4);
                match &**proc {
                    RholangNode::Block { proc: inner, .. } => {
                        match &**inner {
                            RholangNode::Send { channel, .. } => {
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
            let rope = Rope::from_str(&code);
            let ir = parse_to_ir(&tree, &rope);
            let root = ir.clone();
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
        // Note: _parenthesized is a hidden/transparent node in the Tree-Sitter grammar,
        // so Tree-Sitter doesn't report it in the parse tree. It's purely for precedence.
        // The parentheses are syntax-level only and don't create an IR node.
        let _ = crate::logging::init_logger(false, Some("warn"), false);
        let code = r#"(Nil)"#;
        let tree = parse_code(code);
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree, &rope);

        // Tree-Sitter gives us the Nil directly, not a _parenthesized wrapper
        match &*ir {
            RholangNode::Nil { .. } => {
                // Verify position is correct (should skip the opening paren)
                let root = std::sync::Arc::new(ir.clone());
                let start = ir.absolute_start(&root);
                assert_eq!(start.column, 1, "Nil should start at column 1 (after '(')");
            }
            _ => panic!("Expected Nil node (parentheses are transparent in grammar)"),
        }
    }

    #[test]
    fn test_parse_name_remainder() {
        let _ = crate::logging::init_logger(false, Some("warn"), false);
        let code = r#"for(x, ...@y <- ch) { Nil }"#;
        let tree = parse_code(code);
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree, &rope);
        match &*ir {
            RholangNode::Input { receipts, .. } => {
                let bind = &receipts[0][0];
                match &**bind {
                    RholangNode::LinearBind { names, remainder, source, .. } => {
                        assert_eq!(names.len(), 1);
                        assert_eq!(names[0].text(&rope, &ir).to_string(), "x");
                        assert!(remainder.is_some());
                        let rem = remainder.as_ref().unwrap();
                        match &**rem {
                            RholangNode::Quote { quotable, .. } => {
                                assert_eq!(quotable.text(&rope, &ir).to_string(), "y");
                            }
                            _ => panic!("Expected Quote for remainder"),
                        }
                        assert_eq!(source.text(&rope, &ir).to_string(), "ch");
                    }
                    _ => panic!("Expected LinearBind"),
                }
            }
            _ => panic!("Expected Input node"),
        }
    }

    #[test]
    fn test_parse_sync_send_empty_cont() {
        let _ = crate::logging::init_logger(false, Some("warn"), false);
        let code = r#"ch!?("msg")."#;
        let tree = parse_code(code);
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree, &rope);
        match &*ir {
            RholangNode::SendSync { cont, .. } => {
                match &**cont {
                    RholangNode::Nil { .. } => {}
                    _ => panic!("Expected Nil for empty continuation"),
                }
            }
            _ => panic!("Expected SendSync node"),
        }
    }

    #[test]
    fn test_parse_sync_send_non_empty_cont() {
        let _ = crate::logging::init_logger(false, Some("warn"), false);
        let code = r#"ch!?("msg"); Nil"#;
        let tree = parse_code(code);
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree, &rope);
        match &*ir {
            RholangNode::SendSync { cont, .. } => {
                match &**cont {
                    RholangNode::Nil { .. } => {}
                    _ => panic!("Expected Nil for continuation"),
                }
            }
            _ => panic!("Expected SendSync node"),
        }
    }

    #[test]
    fn test_parse_invalid_long_literal() {
        let _ = crate::logging::init_logger(false, Some("warn"), false);
        let code = r#"\u{509ae}"#; // Invalid integer literal
        let tree = parse_code(code);

        // Tree-Sitter should report this as having errors
        assert!(tree.root_node().has_error(), "Tree should have errors for invalid input");

        let rope = Rope::from_str(code);
        let _ir = parse_to_ir(&tree, &rope);

        // The IR may be a Par node containing error fragments, or a single Error node.
        // Either way, Tree-Sitter detected the error, which is what matters.
        // We verify the tree has errors above, and that parsing doesn't panic.
    }

    #[test]
    fn test_parse_valid_long_literal() {
        let _ = crate::logging::init_logger(false, Some("warn"), false);
        let code = r#"123"#;
        let tree = parse_code(code);
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree, &rope);
        match &*ir {
            RholangNode::LongLiteral { value, .. } => {
                assert_eq!(*value, 123);
            }
            _ => panic!("Expected LongLiteral node"),
        }
    }

    #[test]
    fn test_parse_string_literal_with_escapes() {
        let _ = crate::logging::init_logger(false, Some("warn"), false);
        let code = r#""hello \"world\"""#;
        let tree = parse_code(code);
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree, &rope);
        match &*ir {
            RholangNode::StringLiteral { value, .. } => {
                assert_eq!(value, "hello \"world\"");
            }
            _ => panic!("Expected StringLiteral node"),
        }
    }

    #[test]
    fn test_parse_invalid_string_literal() {
        let _ = crate::logging::init_logger(false, Some("warn"), false);
        let code = r#""""#; // Empty quotes
        let tree = parse_code(code);
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree, &rope);
        match &*ir {
            RholangNode::StringLiteral { value, .. } => {
                assert_eq!(value, "");
            }
            _ => panic!("Expected StringLiteral node"),
        }
    }

    #[test]
    fn test_parse_uri_literal() {
        let _ = crate::logging::init_logger(false, Some("warn"), false);
        let code = r#"`http://example.com`"#;
        let tree = parse_code(code);
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree, &rope);
        match &*ir {
            RholangNode::UriLiteral { value, .. } => {
                assert_eq!(value, "http://example.com");
            }
            _ => panic!("Expected UriLiteral node"),
        }
    }

    #[test]
    fn test_debug_block_positions() {
        use std::sync::Arc;
        let code = "if (true) { Nil } else { Nil }";
        let tree = parse_code(code);
        let rope = Rope::from_str(code);

        // First, let's see what Tree-Sitter reports
        let root_node = tree.root_node();
        println!("\n=== Tree-Sitter Raw RholangNode Positions ===");
        let ifelse_node = root_node.named_child(0).unwrap();
        println!("ifElse node: kind='{}' start={} end={}", ifelse_node.kind(), ifelse_node.start_byte(), ifelse_node.end_byte());

        for i in 0..ifelse_node.named_child_count() {
            if let Some(child) = ifelse_node.named_child(i) {
                println!("  child {}: kind='{}' start={} end={}", i, child.kind(), child.start_byte(), child.end_byte());

                if child.kind() == "block" {
                    println!("    Block children:");
                    for j in 0..child.child_count() {
                        if let Some(block_child) = child.child(j) {
                            println!("      child {}: kind='{}' start={} end={} named={}",
                                j, block_child.kind(), block_child.start_byte(), block_child.end_byte(), block_child.is_named());
                        }
                    }
                }
            }
        }

        let ir = parse_to_ir(&tree, &rope);
        let root = Arc::new(ir.clone());

        println!("\n=== IR RholangNode Positions ===");
        match &*ir {
            RholangNode::IfElse { alternative, .. } => {
                if let Some(alt) = alternative {
                    match &**alt {
                        RholangNode::Block { base, proc, .. } => {
                            println!("Block IR base: delta_bytes={}, length={}", base.relative_start().delta_bytes, base.length());

                            let alt_start = alt.absolute_start(&root);
                            let alt_end = alt.absolute_end(&root);
                            println!("Alternative block: start_byte={}, end_byte={}", alt_start.byte, alt_end.byte);
                            println!("Alternative block: start_col={}, end_col={}", alt_start.column, alt_end.column);

                            match &**proc {
                                RholangNode::Nil { base: proc_base, .. } => {
                                    println!("Proc IR base: delta_bytes={}, length={}", proc_base.relative_start().delta_bytes, proc_base.length());

                                    let proc_start = proc.absolute_start(&root);
                                    let proc_end = proc.absolute_end(&root);
                                    println!("Proc in alt: start_byte={}, end_byte={}", proc_start.byte, proc_end.byte);
                                    println!("Proc in alt: start_col={}, end_col={}", proc_start.column, proc_end.column);

                                    // Expected: alt at 23, proc at 25
                                    assert_eq!(alt_start.column, 23, "Alternative block should start at column 23");
                                    assert_eq!(proc_start.column, 25, "Proc should start at column 25");
                                }
                                _ => panic!("Expected Nil"),
                            }
                        }
                        _ => panic!("Expected Block"),
                    }
                }
            }
            _ => panic!("Expected IfElse"),
        }
    }

    #[test]
    fn test_tree_sitter_extras_access() {
        // Demonstrates that Tree-Sitter already parses comments as "extras"
        // and we can access them via the cursor API
        let code = r#"// This is a comment
Nil"#;
        let tree = parse_code(code);
        let root = tree.root_node();

        println!("\n=== Using named_children (skips extras) ===");
        for child in root.named_children(&mut root.walk()) {
            println!("  kind='{}' is_extra={}", child.kind(), child.is_extra());
        }

        println!("\n=== Using cursor to iterate ALL children (including extras) ===");
        let mut cursor = root.walk();
        if cursor.goto_first_child() {
            loop {
                let node = cursor.node();
                println!("  kind='{}' is_extra={} is_named={} text='{}'",
                    node.kind(), node.is_extra(), node.is_named(),
                    node.utf8_text(code.as_bytes()).unwrap_or(""));

                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }

        println!("\n=== Checking all children by index (child_count) ===");
        println!("root.child_count() = {}", root.child_count());
        for i in 0..root.child_count() {
            if let Some(child) = root.child(i) {
                println!("  [{}] kind='{}' is_extra={} is_named={} text='{}'",
                    i, child.kind(), child.is_extra(), child.is_named(),
                    child.utf8_text(code.as_bytes()).unwrap_or(""));
            }
        }

        // Try to find comment in all children by index
        let mut found_comment = false;
        for i in 0..root.child_count() {
            if let Some(child) = root.child(i) {
                if child.kind() == "_line_comment" || child.kind() == "line_comment" {
                    found_comment = true;
                    println!("\n✓ Found comment at index {}: '{}'", i, child.utf8_text(code.as_bytes()).unwrap());
                }
            }
        }

        if !found_comment {
            println!("\n✗ Comment NOT found in tree structure - gap-based detection is necessary");
        }
    }

}
