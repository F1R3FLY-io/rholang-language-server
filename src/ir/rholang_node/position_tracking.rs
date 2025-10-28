use std::collections::HashMap;
use std::sync::Arc;

use tracing::{debug, trace};

use super::node_types::*;
pub use super::node_types::{Position, RelativePosition};

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

    // Removed debug logging

    let start = Position {
        row: (prev_end.row as i32 + relative_start.delta_lines) as usize,
        column: if relative_start.delta_lines == 0 {
            (prev_end.column as i32 + relative_start.delta_columns) as usize
        } else {
            relative_start.delta_columns as usize
        },
        byte: prev_end.byte + relative_start.delta_bytes,
    };

    // Debug: Log ALL nodes up to byte 14850 to find where 2-byte offset is introduced
    if start.byte <= 14850 {
        let node_type = match &**node {
            RholangNode::Input { .. } => "Input".to_string(),
            RholangNode::LinearBind { .. } => "LinearBind".to_string(),
            RholangNode::Wildcard { .. } => "Wildcard".to_string(),
            RholangNode::Var { name, .. } => format!("Var({})", name),
            RholangNode::Send { .. } => "Send".to_string(),
            RholangNode::Par { .. } => "Par".to_string(),
            RholangNode::Block { .. } => "Block".to_string(),
            RholangNode::New { .. } => "New".to_string(),
            RholangNode::NameDecl { .. } => "NameDecl".to_string(),
            _ => format!("{:?}", node).chars().take(15).collect(),
        };
        debug!("POS_TRACK [{}]: prev_end={}, delta_bytes={}, COMPUTED start.byte={}",
               node_type, start.byte - base.delta_bytes(), base.delta_bytes(), start.byte);
    }

    // Use syntactic_length for reconstruction to include closing delimiters
    let end = compute_end_position(start, base.span_lines(), base.span_columns(), base.syntactic_length());

    // Hot path: Position computation runs during parsing for every node
    // Removed per-node debug logging to avoid excessive log volume
    // Use RUST_LOG=trace for detailed position tracking

    let mut current_prev = start;

    // Process children
    match &**node {
        RholangNode::Par { left: Some(left), right: Some(right), .. } => {
            // Debug: log Par processing around byte 14932
            if current_prev.byte >= 14900 && current_prev.byte <= 14950 {
                debug!("Par (left/right): processing left with current_prev.byte={}",
                       current_prev.byte);
            }
            current_prev = compute_positions_helper(left, current_prev, positions);
            if current_prev.byte >= 14900 && current_prev.byte <= 14950 {
                debug!("Par (left/right): processing right with current_prev.byte={}",
                       current_prev.byte);
            }
            current_prev = compute_positions_helper(right, current_prev, positions);
        }
        RholangNode::Par { processes: Some(procs), .. } => {
            for (i, proc) in procs.iter().enumerate() {
                // Debug: log Par processing around byte 14932
                if current_prev.byte >= 14900 && current_prev.byte <= 14950 {
                    let proc_type = match &**proc {
                        RholangNode::Send { .. } => "Send",
                        RholangNode::Block { .. } => "Block",
                        _ => "other",
                    };
                    debug!("Par (processes): processing child {} ({}) with current_prev.byte={}",
                           i, proc_type, current_prev.byte);
                }
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
            // Channel starts at the Send node's start position, not current_prev
            let channel_end = compute_positions_helper(channel, start, positions);
            let send_type_end = Position {
                row: (channel_end.row as i32 + send_type_delta.delta_lines) as usize,
                column: if send_type_delta.delta_lines == 0 {
                    (channel_end.column as i32 + send_type_delta.delta_columns) as usize
                } else {
                    send_type_delta.delta_columns as usize
                },
                byte: channel_end.byte + send_type_delta.delta_bytes,
            };
            if send_type_end.byte >= 14740 && send_type_end.byte <= 14750 {
                debug!("Send node: send_type_end.byte={}, send_type_delta.delta_bytes={}",
                       send_type_end.byte, send_type_delta.delta_bytes);
            }
            let mut temp_prev = send_type_end;
            for (i, input) in inputs.iter().enumerate() {
                if temp_prev.byte >= 14745 && temp_prev.byte <= 14800 {
                    debug!("Send node: processing input {} with temp_prev.byte={}",
                           i, temp_prev.byte);
                }
                let input_end = compute_positions_helper(input, temp_prev, positions);
                if input_end.byte >= 14790 && input_end.byte <= 14810 {
                    debug!("Send node: input {} ended at byte {}",
                           i, input_end.byte);
                }
                temp_prev = input_end;
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
            // Debug: Check Block's end position computation around byte 14850-14900
            if start.byte >= 14840 && start.byte <= 14910 {
                debug!("Block: start.byte={}, base.length()={}, computed end.byte={}",
                       start.byte, base.length(), end.byte);
                debug!("Block: processing proc child with current_prev.byte={}",
                       current_prev.byte);
            }
            current_prev = compute_positions_helper(proc, current_prev, positions);
            if start.byte >= 14840 && start.byte <= 14910 {
                debug!("Block: proc_end.byte={}, will return end.byte={}",
                       current_prev.byte, end.byte);
            }
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

    // Simplified position tracking: all nodes now encode correct lengths
    // No edge cases needed - the invariant node.end = node.start + node.length holds
    positions.insert(key, (start, end));

    end
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
