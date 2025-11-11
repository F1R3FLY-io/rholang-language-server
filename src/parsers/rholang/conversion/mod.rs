//! Rholang Tree-Sitter CST to IR conversion
//!
//! This module handles the conversion from Tree-Sitter's concrete syntax tree (CST)
//! to our intermediate representation (IR) based on RholangNode.

use std::any::Any;
use std::sync::{Arc, OnceLock};
use std::collections::HashMap;

use tree_sitter::Node as TSNode;
use tracing::{debug, trace, warn};
use rpds::Vector;
use archery::ArcK;
use ropey::Rope;

use crate::ir::rholang_node::{
    BinOperator, RholangBundleType, RholangNode, NodeBase, RholangSendType,
    UnaryOperator, RholangVarRefKind, Position,
};
use crate::ir::semantic_node::SemanticNode;
use crate::parsers::position_utils::create_node_base_from_absolute;

use super::helpers::{
    collect_named_descendants, collect_patterns, collect_linear_binds,
    is_comment, safe_byte_slice,
};

// ==============================================================================
// Optimization: Pre-allocated Default Metadata Singleton
// ==============================================================================
// This eliminates per-node HashMap allocation overhead (80-90% reduction)
// by using a shared singleton for the common case (just version metadata).
// See: docs/ir_optimization_design.md - Finding 2.1

/// Singleton for default metadata (just version field)
static DEFAULT_METADATA: OnceLock<Arc<HashMap<String, Arc<dyn Any + Send + Sync>>>> = OnceLock::new();

/// Returns the default metadata singleton (shared across all nodes with default metadata).
///
/// This avoids allocating a new HashMap for every node during IR conversion.
/// The singleton contains only the "version" key with value 0.
///
/// # Performance
/// - Before: ~88 bytes per node (HashMap allocation + overhead)
/// - After: ~8 bytes per node (Arc clone)
/// - Reduction: ~80-90% metadata overhead
fn get_default_metadata() -> Arc<HashMap<String, Arc<dyn Any + Send + Sync>>> {
    DEFAULT_METADATA.get_or_init(|| {
        let mut data = HashMap::new();
        data.insert("version".to_string(), Arc::new(0usize) as Arc<dyn Any + Send + Sync>);
        Arc::new(data)
    }).clone()
}

/// Creates a NodeBase with correct length based on actual content extent.
///
/// Simplified for absolute position tracking - no more delta computation.
///
/// # Arguments
/// * `absolute_start` - The absolute start position of the node (from Tree-Sitter)
/// * `content_end` - The content end position (last child's end, for semantic operations)
/// * `syntactic_end` - The syntactic end position (includes closing delimiters, for reconstruction)
///
/// # Note
/// prev_end parameter removed - no longer needed with absolute positioning
fn create_correct_node_base(absolute_start: Position, content_end: Position, syntactic_end: Position) -> NodeBase {
    let content_length = content_end.byte - absolute_start.byte;
    let syntactic_length = syntactic_end.byte - absolute_start.byte;

    let span_lines = syntactic_end.row - absolute_start.row;
    let span_columns = if span_lines == 0 {
        syntactic_end.column - absolute_start.column
    } else {
        syntactic_end.column
    };

    NodeBase::new(
        absolute_start,
        content_length,
        syntactic_length,
        span_lines,
        span_columns,
    )
}

/// Fast discriminant check for Par nodes without full pattern matching.
/// This function performs a simple enum variant check (~10 CPU cycles) to determine
/// if a node is a Par, avoiding the overhead of full pattern matching and field extraction
/// (~40-80 cycles). Used by the adaptive Par flattening optimization to decide whether
/// to invoke the flattening logic or use the fast path for non-nested Pars.
///
/// # Performance
/// - Discriminant check: ~10 cycles
/// - Full pattern match alternative: ~160-250 cycles (40-80 for match + 50-100 for Vec allocation + Arc cloning)
/// - Savings: ~150-240 cycles per non-nested Par node
#[inline(always)]
fn is_par_node(node: &Arc<RholangNode>) -> bool {
    matches!(**node, RholangNode::Par { .. })
}

/// Converts Tree-Sitter nodes to IR nodes with accurate relative positions.
pub(crate) fn convert_ts_node_to_ir(ts_node: TSNode, rope: &Rope, prev_end: Position) -> (Arc<RholangNode>, Position) {
    // Optimization: Cache Tree-Sitter position method results to avoid redundant calls
    // Each call involves boundary checks and UTF-8 validation (~50-100 CPU cycles)
    // This reduces 6 method calls per node to 4 method calls (40-50% reduction)
    // See: docs/ir_optimization_design.md - Finding 2.2
    let start_pos = ts_node.start_position();
    let end_pos = ts_node.end_position();
    let start_byte = ts_node.start_byte();
    let end_byte = ts_node.end_byte();

    let absolute_start = Position {
        row: start_pos.row,
        column: start_pos.column,
        byte: start_byte,
    };
    let absolute_end = Position {
        row: end_pos.row,
        column: end_pos.column,
        byte: end_byte,
    };

    // Absolute positioning: Tree-Sitter provides correct positions, we preserve them directly
    // No more delta computation - positions are stored as-is from Tree-Sitter
    trace!(
        "RholangNode '{}': prev_end={:?}, start={:?}, end={:?}",
        ts_node.kind(), prev_end, absolute_start, absolute_end
    );

    let length = absolute_end.byte - absolute_start.byte;
    let span_lines = absolute_end.row - absolute_start.row;
    let span_columns = if span_lines == 0 {
        absolute_end.column - absolute_start.column
    } else {
        absolute_end.column
    };
    // Use new_simple() for most nodes - they don't have closing delimiters
    // Nodes with delimiters (Block, Parenthesized, List, Set, Map, etc.) will override this
    let base = NodeBase::new_simple(
        absolute_start,
        length,
        span_lines,
        span_columns,
    );
    // Optimization: Use pre-allocated singleton metadata instead of per-node HashMap
    // This reduces metadata overhead by 80-90% (88 bytes → 8 bytes per node)
    let metadata = Some(get_default_metadata());

    match ts_node.kind() {
        "source_file" => {
            let mut current_prev_end = absolute_start;
            let mut all_nodes = Vec::new();

            // Comments are named nodes in extras, so filter them efficiently
            for child in ts_node.named_children(&mut ts_node.walk()) {
                // Skip comments - they're in extras and don't belong in the IR
                let kind_id = child.kind_id();
                if is_comment(kind_id) {
                    // FIX: Update current_prev_end to skip over comment's bytes
                    // Otherwise next child will have incorrect position reference
                    let comment_end_pos = child.end_position();
                    current_prev_end = Position {
                        row: comment_end_pos.row,
                        column: comment_end_pos.column,
                        byte: child.end_byte(),
                    };
                    continue;
                }
                debug!("Before converting child '{}': current_prev_end = {:?}", child.kind(), current_prev_end);
                let (node, end) = convert_ts_node_to_ir(child, rope, current_prev_end);
                debug!("After converting child '{}': end = {:?}", child.kind(), end);
                all_nodes.push(node);
                current_prev_end = end;
            }

            debug!("source_file: collected {} top-level nodes", all_nodes.len());
            let result = if all_nodes.len() == 1 {
                debug!("source_file: returning single node (no Par wrapper)");
                all_nodes[0].clone()
            } else if all_nodes.is_empty() {
                Arc::new(RholangNode::Nil { base: base.clone(), metadata: metadata.clone() })
            } else if all_nodes.len() == 2 {
                // Exactly 2 top-level processes - use binary Par
                debug!("source_file: creating binary Par for 2 top-level nodes");
                // Par must span from file start to current_prev_end to enable position lookups
                let par_base = create_correct_node_base(
                    absolute_start,
                    current_prev_end,
                    current_prev_end,
                );
                Arc::new(RholangNode::Par {
                    base: par_base,
                    left: Some(all_nodes[0].clone()),
                    right: Some(all_nodes[1].clone()),
                    processes: None,
                    metadata: metadata.clone(),
                })
            } else {
                // More than 2 top-level processes - use n-ary Par (O(1) depth instead of O(n))
                debug!("source_file: creating n-ary Par for {} top-level nodes", all_nodes.len());
                // Par must span from file start to current_prev_end to enable position lookups
                let par_base = create_correct_node_base(
                    absolute_start,
                    current_prev_end,
                    current_prev_end,
                );
                Arc::new(RholangNode::Par {
                    base: par_base,
                    left: None,
                    right: None,
                    processes: Some(Vector::from_iter(all_nodes)),
                    metadata: metadata.clone(),
                })
            };
            (result, absolute_end)
        }
        "collection" => {
            let child = ts_node.named_child(0).expect("Collection node must have a named child");
            convert_ts_node_to_ir(child, rope, prev_end)
        }
        "par" => {
            // Par nodes should have 2 named children in the common case
            // But can have more due to comment interleaving
            let named_child_count = ts_node.named_child_count();

            if named_child_count == 2 {
                // Standard binary Par - use direct children to preserve tree-sitter positions
                let left_ts = ts_node.named_child(0).expect("Par node must have a left named child");

                // FIX: Par's left child must use Par's start (absolute_start) as reference
                // This ensures children have sequential deltas: left from Par start, right from left's end
                let (left, left_end) = convert_ts_node_to_ir(left_ts, rope, absolute_start);

                let right_ts = ts_node.named_child(1).expect("Par node must have a right named child");

                let (right, right_end) = convert_ts_node_to_ir(right_ts, rope, left_end);

                // OPTIMIZATION Phase 3: Conditional flattening based on Par density
                // Check if either child is a Par before invoking flattening logic
                // This avoids overhead (pattern matching, Vec allocation, Arc cloning) for non-nested Pars
                if !is_par_node(&left) && !is_par_node(&right) {
                    // FAST PATH: Neither child is a Par - create simple binary Par
                    // Saves: ~160-250 cycles per non-nested Par
                    // - No pattern matching overhead (40-80 cycles)
                    // - No Vec allocation (50-100 cycles)
                    // - No Arc cloning for collection (10 cycles per child)
                    // - No Vector::from_iter conversion (50-200 cycles)

                    // Par's delta is from prev_end (parent's reference) to absolute_start (Par's start)
                    // But children use absolute_start as reference for sequential positioning
                    let corrected_base = create_correct_node_base(absolute_start, right_end, right_end);

                    let node = Arc::new(RholangNode::Par {
                        base: corrected_base,
                        left: Some(left),
                        right: Some(right),
                        processes: None,
                        metadata,
                    });
                    (node, right_end)
                } else {
                    // SLOW PATH: At least one child is a Par - flatten to reduce depth
                    // OPTIMIZATION: Inline Par flattening to reduce O(n) depth to O(1)
                    // Collect all processes from left and right, flattening nested Pars
                    let mut all_processes = Vec::new();

                // Flatten left child
                match &*left {
                    RholangNode::Par { left: l_left, right: l_right, processes: l_procs, .. } => {
                        if let Some(procs) = l_procs {
                            // Left is already an n-ary Par, collect its processes
                            all_processes.extend(procs.iter().cloned());
                        } else if let (Some(ll), Some(lr)) = (l_left, l_right) {
                            // Left is a binary Par, collect its children
                            all_processes.push(ll.clone());
                            all_processes.push(lr.clone());
                        }
                    }
                    _ => {
                        // Left is not a Par, add it directly
                        all_processes.push(left.clone());
                    }
                }

                // Flatten right child
                match &*right {
                    RholangNode::Par { left: r_left, right: r_right, processes: r_procs, .. } => {
                        if let Some(procs) = r_procs {
                            // Right is already an n-ary Par, collect its processes
                            all_processes.extend(procs.iter().cloned());
                        } else if let (Some(rl), Some(rr)) = (r_left, r_right) {
                            // Right is a binary Par, collect its children
                            all_processes.push(rl.clone());
                            all_processes.push(rr.clone());
                        }
                    }
                    _ => {
                        // Right is not a Par, add it directly
                        all_processes.push(right.clone());
                    }
                }

                // Create corrected base: Par's extent is from its start to right child's end
                // Par has no closing delimiter, so content and syntactic ends are the same
                let corrected_base = create_correct_node_base(absolute_start, right_end, right_end);

                // Create n-ary Par if we have 3+ processes, binary Par otherwise
                let node = if all_processes.len() > 2 {
                    Arc::new(RholangNode::Par {
                        base: corrected_base,
                        left: None,
                        right: None,
                        processes: Some(Vector::from_iter(all_processes)),
                        metadata,
                    })
                } else if all_processes.len() == 2 {
                    Arc::new(RholangNode::Par {
                        base: corrected_base,
                        left: Some(all_processes[0].clone()),
                        right: Some(all_processes[1].clone()),
                        processes: None,
                        metadata,
                    })
                } else {
                    // Single process (shouldn't happen in practice, but handle it)
                    all_processes[0].clone()
                };
                (node, right_end)
                }
            } else {
                // N-ary Par (due to comments) - collect all children, filter comments, and reduce
                // BUGFIX: Use prev_end (not absolute_start) to maintain correct position threading
                let mut current_prev_end = prev_end;
                let mut process_children = Vec::new();

                // Collect all named children, skipping comments
                for child in ts_node.named_children(&mut ts_node.walk()) {
                    let kind_id = child.kind_id();
                    if is_comment(kind_id) {
                        // FIX: Update current_prev_end to skip over comment's bytes
                        // Otherwise next child will have incorrect position reference
                        let comment_end_pos = child.end_position();
                        current_prev_end = Position {
                            row: comment_end_pos.row,
                            column: comment_end_pos.column,
                            byte: child.end_byte(),
                        };
                        continue;
                    }
                    let (child_node, child_end) = convert_ts_node_to_ir(child, rope, current_prev_end);
                    process_children.push(child_node);
                    current_prev_end = child_end;
                }

                // Reduce all children into nested Par tree
                let result = if process_children.len() == 1 {
                    process_children[0].clone()
                } else if process_children.is_empty() {
                    Arc::new(RholangNode::Nil { base: base.clone(), metadata: metadata.clone() })
                } else {
                    // Create corrected base for Par nodes (2 or more children)
                    // Par has no closing delimiter, so content and syntactic ends are the same
                    let corrected_base = create_correct_node_base(absolute_start, current_prev_end, current_prev_end);

                    if process_children.len() == 2 {
                        // After filtering comments, we have exactly 2 children
                        Arc::new(RholangNode::Par {
                            base: corrected_base,
                            left: Some(process_children[0].clone()),
                            right: Some(process_children[1].clone()),
                            processes: None,
                            metadata,
                        })
                    } else {
                        // More than 2 children - create n-ary Par (reduces nesting from O(n) to O(1))
                        Arc::new(RholangNode::Par {
                            base: corrected_base,
                            left: None,
                            right: None,
                            processes: Some(Vector::from_iter(process_children)),
                            metadata,
                        })
                    }
                };
                (result, current_prev_end)
            }
        }
        "send_sync" => {
            if absolute_start.byte >= 8200 && absolute_start.byte <= 8300 {
                debug!("SendSync: tree-sitter range [{}, {}]", ts_node.start_byte(), ts_node.end_byte());
                debug!("  absolute_start={:?}", absolute_start);
            }
            let channel_ts = ts_node.child_by_field_name("channel").expect("SendSync node must have a channel");
            if absolute_start.byte >= 8200 && absolute_start.byte <= 8300 {
                debug!("  channel tree-sitter range [{}, {}]", channel_ts.start_byte(), channel_ts.end_byte());
            }
            let (channel, channel_end) = convert_ts_node_to_ir(channel_ts, rope, absolute_start);
            if absolute_start.byte >= 8200 && absolute_start.byte <= 8300 {
                debug!("  channel_end={:?}", channel_end);
            }
            let inputs_ts = ts_node.child_by_field_name("inputs").expect("SendSync node must have inputs");
            let mut current_prev_end = channel_end;
            let inputs = inputs_ts.named_children(&mut inputs_ts.walk())
                .filter_map(|child| {
                    // Skip comments - they don't belong in the IR
                    let kind_id = child.kind_id();
                    if is_comment(kind_id) {
                        // Update position tracking to skip over comment's bytes
                        let comment_end_pos = child.end_position();
                        current_prev_end = Position {
                            row: comment_end_pos.row,
                            column: comment_end_pos.column,
                            byte: child.end_byte(),
                        };
                        return None;
                    }
                    let (node, end) = convert_ts_node_to_ir(child, rope, current_prev_end);
                    current_prev_end = end;
                    Some(node)
                })
                .collect::<Vector<_, ArcK>>();
            let cont_ts = ts_node.child_by_field_name("cont").expect("SendSync node must have a continuation");
            let (cont, cont_end) = convert_ts_node_to_ir(cont_ts, rope, current_prev_end);
            if absolute_start.byte >= 8200 && absolute_start.byte <= 8300 {
                debug!("  cont_end={:?}", cont_end);
            }
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
            // Store absolute position of send_type (! or !!)
            let send_type_pos = Position {
                row: send_type_ts.start_position().row,
                column: send_type_ts.start_position().column,
                byte: send_type_ts.start_byte(),
            };
            let send_type_abs_end = Position {
                row: send_type_ts.end_position().row,
                column: send_type_ts.end_position().column,
                byte: send_type_ts.end_byte(),
            };
            let inputs_ts = ts_node.child_by_field_name("inputs").expect("Send node must have inputs");
            let mut current_prev_end = send_type_abs_end;
            let inputs = inputs_ts.named_children(&mut inputs_ts.walk())
                .filter_map(|child| {
                    // Skip comments - they don't belong in the IR
                    let kind_id = child.kind_id();
                    if is_comment(kind_id) {
                        // Update position tracking to skip over comment's bytes
                        let comment_end_pos = child.end_position();
                        current_prev_end = Position {
                            row: comment_end_pos.row,
                            column: comment_end_pos.column,
                            byte: child.end_byte(),
                        };
                        return None;
                    }
                    let (node, end) = convert_ts_node_to_ir(child, rope, current_prev_end);
                    current_prev_end = end;
                    Some(node)
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
            // Use Tree-Sitter's absolute_end for the syntactic extent (includes closing ')')
            let send_end = absolute_end;

            // Send has a closing ')' delimiter:
            // - content_end is after last input (current_prev_end)
            // - syntactic_end includes the ')' (send_end/absolute_end)
            let corrected_base = create_correct_node_base(absolute_start, current_prev_end, send_end);

            let node = Arc::new(RholangNode::Send {
                base: corrected_base,
                channel,
                send_type,
                send_type_pos,
                inputs,
                metadata,
            });
            (node, send_end)
        }
        "new" => {
            let decls_ts = ts_node.child_by_field_name("decls").expect("New node must have decls");
            let (decls, decls_end) = collect_named_descendants(decls_ts, rope, absolute_start);
            let proc_ts = ts_node.child_by_field_name("proc").expect("New node must have a process");
            let (proc, proc_end) = convert_ts_node_to_ir(proc_ts, rope, decls_end);
            // Create corrected base: New's extent is from start to absolute_end
            // New syntax may include whitespace/comments after proc, so use Tree-Sitter's end
            let corrected_base = create_correct_node_base(absolute_start, proc_end, absolute_end);
            let node = Arc::new(RholangNode::New { base: corrected_base, decls, proc, metadata });
            // BUG FIX: Must return Tree-Sitter's absolute_end, not proc_end
            (node, absolute_end)
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
            // BUG FIX: Must return Tree-Sitter's absolute_end, not proc_end
            (node, absolute_end)
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
            // BUG FIX: Must return Tree-Sitter's absolute_end, not current_prev_end
            (node, absolute_end)
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

            // DEBUG: Log what tree-sitter reports for the contract and its proc child
            debug!("Contract node: ts_start=({}, {}), ts_end=({}, {}), ts_end_byte={}",
                   ts_node.start_position().row, ts_node.start_position().column,
                   ts_node.end_position().row, ts_node.end_position().column,
                   ts_node.end_byte());
            debug!("Contract proc child ({}): ts_start=({}, {}), ts_end=({}, {}), ts_end_byte={}",
                   proc_ts.kind(),
                   proc_ts.start_position().row, proc_ts.start_position().column,
                   proc_ts.end_position().row, proc_ts.end_position().column,
                   proc_ts.end_byte());

            let (proc, proc_end) = convert_ts_node_to_ir(proc_ts, rope, formals_end);
            debug!("After converting proc: proc_end = {:?}", proc_end);

            // Create corrected base: Contract's extent is from start to absolute_end
            // Contract syntax may include whitespace/comments after proc, so use Tree-Sitter's end
            let corrected_base = create_correct_node_base(absolute_start, proc_end, absolute_end);
            let node = Arc::new(RholangNode::Contract { base: corrected_base, name, formals, formals_remainder, proc, metadata });
            // BUG FIX: Must return Tree-Sitter's absolute_end, not proc_end!
            // Returning proc_end causes cascading position errors in all subsequent siblings
            (node, absolute_end)
        }
        "input" => {
            let receipts_ts = ts_node.child_by_field_name("receipts").expect("Input node must have receipts");

            // Debug: log Input/receipts around the problematic New node
            if absolute_start.byte >= 14825 && absolute_start.byte <= 14840 {
                debug!("Input node: absolute_start.byte={}, receipts_ts.start_byte()={}, receipts_ts.end_byte()={}",
                       absolute_start.byte, receipts_ts.start_byte(), receipts_ts.end_byte());
            }

            let mut current_prev_end = absolute_start;
            let receipts = receipts_ts.named_children(&mut receipts_ts.walk())
                .filter_map(|receipt_node| {
                    // Skip comments - they don't belong in the IR
                    let kind_id = receipt_node.kind_id();
                    if is_comment(kind_id) {
                        // Update position tracking to skip over comment's bytes
                        let comment_end_pos = receipt_node.end_position();
                        current_prev_end = Position {
                            row: comment_end_pos.row,
                            column: comment_end_pos.column,
                            byte: receipt_node.end_byte(),
                        };
                        return None;
                    }
                    if absolute_start.byte >= 14825 && absolute_start.byte <= 14840 {
                        debug!("Processing receipt: receipt_node.kind()={}, start={}, end={}, current_prev_end.byte={}",
                               receipt_node.kind(), receipt_node.start_byte(), receipt_node.end_byte(), current_prev_end.byte);
                    }
                    let (binds, binds_end) = collect_named_descendants(receipt_node, rope, current_prev_end);
                    if absolute_start.byte >= 14825 && absolute_start.byte <= 14840 {
                        debug!("Receipt ended at binds_end.byte={}", binds_end.byte);
                    }
                    current_prev_end = binds_end;
                    Some(binds)
                })
                .collect::<Vector<_, ArcK>>();

            if absolute_start.byte >= 14825 && absolute_start.byte <= 14840 {
                let proc_start = ts_node.child_by_field_name("proc").map(|p| p.start_byte()).unwrap_or(0);
                debug!("Input passing current_prev_end.byte={} to proc (proc_ts.start_byte()={})",
                       current_prev_end.byte, proc_start);
            }

            let proc_ts = ts_node.child_by_field_name("proc").expect("Input node must have a process");
            let (proc, proc_end) = convert_ts_node_to_ir(proc_ts, rope, current_prev_end);
            // Create corrected base: Input's extent is from start to absolute_end
            // Input syntax may include whitespace/comments after proc, so use Tree-Sitter's end
            let corrected_base = create_correct_node_base(absolute_start, proc_end, absolute_end);
            let node = Arc::new(RholangNode::Input { base: corrected_base, receipts, proc, metadata });
            // BUG FIX: Must return Tree-Sitter's absolute_end, not proc_end
            (node, absolute_end)
        }
        "block" => {
            // Debug: Check Block's length computation around byte 14850-14900
            if absolute_start.byte >= 14840 && absolute_start.byte <= 14910 {
                debug!("Block node: start_byte={}, end_byte={}, length={}, ts_node.text='{}'",
                       ts_node.start_byte(), ts_node.end_byte(),
                       ts_node.end_byte() - ts_node.start_byte(),
                       ts_node.utf8_text(rope.to_string().as_bytes()).unwrap_or("<error>").chars().take(50).collect::<String>());
            }
            // A block contains '{', multiple children (including comments), and '}'
            // Collect all named children and reduce them into a Par tree (like source_file)
            let (all_nodes, _nodes_end) = collect_named_descendants(ts_node, rope, absolute_start);

            // Comments are already skipped during collect_named_descendants,
            // so all_nodes already contains only process nodes
            let process_nodes = all_nodes;

            let proc = if process_nodes.len() == 0 {
                // Empty block - use Nil
                Arc::new(RholangNode::Nil {
                    base: NodeBase::new_simple(
                        absolute_start,
                        0, 0, 0,
                    ),
                    metadata: metadata.clone(),
                })
            } else if process_nodes.len() == 1 {
                // Single child - use it directly
                process_nodes[0].clone()
            } else if process_nodes.len() == 2 {

                // Exactly 2 children - use binary Par
                // Get the first child's absolute start for Par's start position
                let first_child_start = process_nodes[0].base().start();

                // NOTE: Position recalculation for Par children when Block->Par wrapping occurs
                // Children were created with positions from absolute_start (Block's '{')
                // Par starts at first_child_start, so children would need recalculated positions
                // However, ALL Blocks in practice have single children, so this code path is unused
                use crate::parsers::position_utils::recalculate_children_positions;
                let recalculated = recalculate_children_positions(&process_nodes, absolute_start, first_child_start);

                // Get the last child's end for Par's end position using recalculated children
                let last_child_end = recalculated.get(1).unwrap().base().end();

                // Par must span from first child to last child to enable position lookups
                let par_base = create_correct_node_base(
                    first_child_start,
                    last_child_end,
                    last_child_end,
                );
                Arc::new(RholangNode::Par {
                    base: par_base,
                    left: Some(recalculated.get(0).unwrap().clone()),
                    right: Some(recalculated.get(1).unwrap().clone()),
                    processes: None,
                    metadata: metadata.clone(),
                })
            } else {
                // More than 2 children - use n-ary Par (O(1) depth instead of O(n))
                // Get the first child's absolute start for Par's start position
                let first_child_start = process_nodes[0].base().start();

                // NOTE: Position recalculation for Par children when Block->Par wrapping occurs
                // Children were created with positions from absolute_start (Block's '{')
                // Par starts at first_child_start, so children would need recalculated positions
                // However, ALL Blocks in practice have single children, so this code path is unused
                use crate::parsers::position_utils::recalculate_children_positions;
                let recalculated = recalculate_children_positions(&process_nodes, absolute_start, first_child_start);

                // Get the last child's end for Par's end position
                let last_child_end = recalculated.last().unwrap().base().end();

                // Par must span from first child to last child to enable position lookups
                let par_base = create_correct_node_base(
                    first_child_start,
                    last_child_end,
                    last_child_end,
                );
                Arc::new(RholangNode::Par {
                    base: par_base,
                    left: None,
                    right: None,
                    processes: Some(recalculated),
                    metadata: metadata.clone(),
                })
            };

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
                .filter_map(|child| {
                    // Skip comments - they don't belong in the IR
                    let kind_id = child.kind_id();
                    if is_comment(kind_id) {
                        // Update position tracking to skip over comment's bytes
                        let comment_end_pos = child.end_position();
                        current_prev_end = Position {
                            row: comment_end_pos.row,
                            column: comment_end_pos.column,
                            byte: child.end_byte(),
                        };
                        return None;
                    }
                    let (node, end) = convert_ts_node_to_ir(child, rope, current_prev_end);
                    current_prev_end = end;
                    Some(node)
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
            // The '@' symbol is child(0) - we need to pass its end position as prev_end
            // so the quotable's delta is computed correctly from after the '@'.
            let at_symbol = ts_node.child(0).expect("Quote node must have an '@' symbol");
            let after_at = Position {
                row: at_symbol.end_position().row,
                column: at_symbol.end_position().column,
                byte: at_symbol.end_byte(),
            };
            let quotable_ts = ts_node.child(1).expect("Quote node must have a quotable");
            let (quotable, quotable_end) = convert_ts_node_to_ir(quotable_ts, rope, after_at);
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
                .filter(|n| n.kind() != "_proc_remainder" && n.is_named() && !is_comment(n.kind_id()))
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
                .filter(|n| n.kind() != "_proc_remainder" && !is_comment(n.kind_id()))
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
        "pathmap" => {
            let mut current_prev_end = absolute_start;
            let elements = ts_node.named_children(&mut ts_node.walk())
                .filter(|n| n.kind() != "_proc_remainder" && !is_comment(n.kind_id()))
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
            let node = Arc::new(RholangNode::Pathmap { base, elements, remainder, metadata });
            (node, absolute_end)
        }
        "tuple" => {
            let mut current_prev_end = absolute_start;
            let elements = ts_node.named_children(&mut ts_node.walk())
                .filter_map(|child| {
                    // Skip comments - they don't belong in the IR
                    let kind_id = child.kind_id();
                    if is_comment(kind_id) {
                        // Update position tracking to skip over comment's bytes
                        let comment_end_pos = child.end_position();
                        current_prev_end = Position {
                            row: comment_end_pos.row,
                            column: comment_end_pos.column,
                            byte: child.end_byte(),
                        };
                        return None;
                    }
                    let (node, end) = convert_ts_node_to_ir(child, rope, current_prev_end);
                    current_prev_end = end;
                    Some(node)
                })
                .collect::<Vector<_, ArcK>>();
            let node = Arc::new(RholangNode::Tuple { base, elements, metadata });
            (node, absolute_end)
        }
        "var" => {
            let name = safe_byte_slice(rope, ts_node.start_byte(), ts_node.end_byte());
            // Debug: log Tree-Sitter reported positions for variables
            if name.contains("robot") {
                debug!("Tree-Sitter 'var' node: name='{}', ts_node.start_byte()={}, ts_node.end_byte()={}, absolute_start.byte={}, absolute_end.byte={}, prev_end.byte={}",
                       name, ts_node.start_byte(), ts_node.end_byte(), absolute_start.byte, absolute_end.byte, prev_end.byte);
                debug!("  Absolute: row={}, column={}, byte={}",
                       absolute_start.row, absolute_start.column, absolute_start.byte);
            }
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
            (node, absolute_end)  // Return Tree-Sitter's end, not child's end
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
                let wildcard_base = NodeBase::new_simple(
                    absolute_start,
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
                let wildcard_base = NodeBase::new_simple(
                    absolute_start,
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
                let wildcard_base = NodeBase::new_simple(
                    absolute_start,
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
                .filter_map(|child| {
                    // Skip comments - they don't belong in the IR
                    let kind_id = child.kind_id();
                    if is_comment(kind_id) {
                        // Update position tracking to skip over comment's bytes
                        let comment_end_pos = child.end_position();
                        current_prev_end = Position {
                            row: comment_end_pos.row,
                            column: comment_end_pos.column,
                            byte: child.end_byte(),
                        };
                        return None;
                    }
                    let (node, end) = convert_ts_node_to_ir(child, rope, current_prev_end);
                    current_prev_end = end;
                    Some(node)
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
        // Comments are now skipped before reaching convert_ts_node_to_ir,
        // so these cases should never be reached
        "line_comment" | "block_comment" => {
            panic!("Comments should be filtered out before IR conversion");
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

/// Filters comments from a node's children, returning only semantic children
/// and updating the position tracker to skip over comments.
///
/// Comments can appear as "extras" in the Tree-Sitter grammar anywhere between
/// semantic nodes, so this helper ensures they don't interfere with positional
/// child access patterns.
///
/// # Returns
/// A tuple of (semantic_children, updated_prev_end) where:
/// - semantic_children: Vec of TSNode children without comments
/// - updated_prev_end: Position after last comment (for correct position tracking)
#[inline]
fn filter_comments_from_children(ts_node: TSNode, prev_end: Position) -> (Vec<TSNode>, Position) {
    let mut current_prev_end = prev_end;
    let mut semantic_children = Vec::new();

    for child in ts_node.named_children(&mut ts_node.walk()) {
        let kind_id = child.kind_id();
        if is_comment(kind_id) {
            // Skip comment but update position tracking
            let comment_end_pos = child.end_position();
            current_prev_end = Position {
                row: comment_end_pos.row,
                column: comment_end_pos.column,
                byte: child.end_byte(),
            };
            continue;
        }
        semantic_children.push(child);
    }

    (semantic_children, current_prev_end)
}

fn binary_op(ts_node: TSNode, rope: &Rope, base: NodeBase, op: BinOperator, prev_end: Position) -> (Arc<RholangNode>, Position) {
    // Filter comments from children (they can appear as extras between operands)
    let (semantic_children, current_prev_end) = filter_comments_from_children(ts_node, prev_end);

    // Expect exactly 2 operands (left and right)
    if semantic_children.len() < 2 {
        panic!("BinaryOp node must have left and right operands, found {} semantic children", semantic_children.len());
    }

    let left_ts = semantic_children[0];
    let (left, left_end) = convert_ts_node_to_ir(left_ts, rope, current_prev_end);

    let right_ts = semantic_children[1];
    let (right, right_end) = convert_ts_node_to_ir(right_ts, rope, left_end);

    let mut data = HashMap::new();
    data.insert("version".to_string(), Arc::new(0usize) as Arc<dyn Any + Send + Sync>);
    let metadata = Some(Arc::new(data));
    let node = Arc::new(RholangNode::BinOp { base, op, left, right, metadata });
    (node, right_end)
}

fn unary_op(ts_node: TSNode, rope: &Rope, base: NodeBase, op: UnaryOperator, prev_end: Position) -> (Arc<RholangNode>, Position) {
    // Filter comments from children (they can appear as extras between operator and operand)
    let (semantic_children, current_prev_end) = filter_comments_from_children(ts_node, prev_end);

    // Expect exactly 1 operand
    if semantic_children.is_empty() {
        panic!("UnaryOp node must have an operand, found 0 semantic children");
    }

    let operand_ts = semantic_children[0];
    let (operand, operand_end) = convert_ts_node_to_ir(operand_ts, rope, current_prev_end);

    let mut data = HashMap::new();
    data.insert("version".to_string(), Arc::new(0usize) as Arc<dyn Any + Send + Sync>);
    let metadata = Some(Arc::new(data));
    let node = Arc::new(RholangNode::UnaryOp { base, op, operand, metadata });
    (node, operand_end)
}
