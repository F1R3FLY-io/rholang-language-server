use std::any::Any;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use rpds::Vector;
use archery::ArcK;


use crate::ir::rholang_node::{
    BinOperator, RholangBundleType, CommentKind, RholangNode, NodeBase, Metadata, RholangSendType, UnaryOperator,
    RholangVarRefKind, Position, RelativePosition,
};
use crate::ir::visitor::Visitor;

use super::json_formatters::format_json_string;

/// A visitor that constructs a JSON-like string representation of the IR tree.
/// Configurable for compact or pretty-printed output.
pub struct PrettyPrinter {
    /// If true, formats output with indentation and alignment.
    pub(super) pretty_print: bool,

    /// The accumulating string result.
    result: RefCell<String>,

    /// Tracks the current column position for alignment.
    pub(super) current_column: RefCell<usize>,

    /// Stack of alignment column positions for nested structures.
    alignment_columns: RefCell<Vec<usize>>,

    /// Indicates if the next field is the first in its map.
    is_first_field: RefCell<bool>,

    /// Maps node IDs to their absolute positions in the source code.
    positions: HashMap<usize, (Position, Position)>,
}

impl PrettyPrinter {
    /// Creates a new pretty printer instance.
    ///
    /// # Arguments
    /// * pretty_print - Enables indentation and alignment if true.
    /// * positions - Precomputed node positions for accurate metadata.
    pub fn new(pretty_print: bool, positions: HashMap<usize, (Position, Position)>) -> Self {
        PrettyPrinter {
            pretty_print,
            result: RefCell::new(String::new()),
            current_column: RefCell::new(0),
            alignment_columns: RefCell::new(Vec::new()),
            is_first_field: RefCell::new(true),
            positions,
        }
    }

    /// Adds common base fields (position, length, text) to the current map.
    fn add_base_fields(&self, node: &Arc<RholangNode>) {
        let key = &**node as *const RholangNode as usize;
        let (start, end) = self.positions.get(&key).unwrap();
        // Compute length from positions instead of base to handle structural nodes
        // with zero span (like Par nodes created during reduction)
        let length = end.byte - start.byte;
        self.add_field("start_line", |p| p.append(&start.row.to_string()));
        self.add_field("start_column", |p| p.append(&start.column.to_string()));
        self.add_field("end_line", |p| p.append(&end.row.to_string()));
        self.add_field("end_column", |p| p.append(&end.column.to_string()));
        self.add_field("position", |p| p.append(&start.byte.to_string()));
        self.add_field("length", |p| p.append(&length.to_string()));
    }

    /// Adds metadata to the output, respecting pretty_print for indentation and newlines.
    fn add_metadata(&self, metadata: &Option<Arc<Metadata>>) {
        if let Some(meta) = metadata {
            self.add_field("metadata", |p| {
                if meta.is_empty() {
                    p.append("{}");
                    return;
                }
                let mut sorted: Vec<_> = meta.iter().collect();
                sorted.sort_by_key(|&(k, _)| k);
                if p.pretty_print {
                    p.append("{");
                    let align_col = *p.current_column.borrow();
                    for (i, (key, value)) in sorted.iter().enumerate() {
                        if i > 0 {
                            p.append("\n");
                            p.append(&" ".repeat(align_col));
                        }
                        p.append(":");
                        p.append(key);
                        p.append(" ");
                        format_json_string(p, value);
                    }
                    p.append("}");
                } else {
                    p.append("{");
                    for (i, (key, value)) in sorted.iter().enumerate() {
                        if i > 0 {
                            p.append(",");
                        }
                        p.append(":");
                        p.append(key);
                        p.append(" ");
                        format_json_string(p, value);
                    }
                    p.append("}");
                }
            });
        }
    }

    /// Escapes a string for JSON compatibility.
    pub(super) fn escape_json_string(&self, s: &str) {
        self.append_char('"');
        for c in s.chars() {
            match c {
                '"' => self.append("\\\""),
                '\\' => self.append("\\\\"),
                '\n' => self.append("\\n"),
                '\r' => self.append("\\r"),
                '\t' => self.append("\\t"),
                _ if c.is_control() => self.append(&format!("\\u{:04x}", c as u32)),
                _ => self.append_char(c),
            }
        }
        self.append_char('"');
    }

    fn update_column(&self, c: char) {
        let mut current_column = self.current_column.borrow_mut();
        if c == '\n' {
            *current_column = 0;
        } else {
            *current_column += 1;
        }
    }

    /// Appends a character to the result, updating the current column position.
    fn append_char(&self, c: char) {
        let mut result = self.result.borrow_mut();
        result.push(c);
        self.update_column(c);
    }

    /// Appends a string to the result, updating the current column position.
    pub(super) fn append(&self, s: &str) {
        let mut result = self.result.borrow_mut();
        result.push_str(s);
        for c in s.chars() {
            self.update_column(c);
        }
    }

    /// Starts a new map structure in the output.
    fn start_map(&self) {
        self.append("{");
        if self.pretty_print {
            let current_col = *self.current_column.borrow();
            self.alignment_columns.borrow_mut().push(current_col);
        }
        *self.is_first_field.borrow_mut() = true;
    }

    /// Ends the current map structure in the output.
    fn end_map(&self) {
        self.append("}");
        if self.pretty_print {
            self.alignment_columns.borrow_mut().pop();
        }
    }

    /// Adds a key-value pair to the current map, handling alignment if pretty-printing.
    ///
    /// # Arguments
    /// * key - The field name.
    /// * value - A closure that appends the field value.
    fn add_field<F>(&self, key: &str, value: F)
    where
        F: FnOnce(&Self),
    {
        let is_first = *self.is_first_field.borrow();
        if self.pretty_print {
            {
                let alignment = *self.alignment_columns.borrow().last().unwrap_or(&0);
                let current_col = *self.current_column.borrow();
                if !is_first && current_col != alignment {
                    self.append("\n");
                    self.append(&" ".repeat(alignment));
                }
                self.append(&format!(":{} ", key));
            }
            value(self);
            *self.is_first_field.borrow_mut() = false;
        } else {
            {
                if is_first {
                    self.append(&format!(":{} ", key));
                } else {
                    self.append(&format!(",:{} ", key));
                }
            }
            value(self);
            *self.is_first_field.borrow_mut() = false;
        }
    }

    /// Formats a vector of nodes as an array, with alignment if pretty-printing.
    fn visit_vector(&self, items: &Vector<Arc<RholangNode>, ArcK>) {
        if items.is_empty() {
            self.append("[]");
            return;
        }
        if self.pretty_print {
            self.append("[");
            let vector_start_column = *self.current_column.borrow();
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    self.append("\n");
                    self.append(&" ".repeat(vector_start_column));
                }
                self.visit_node(item);
            }
            self.append("]");
        } else {
            self.append("[");
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    self.append(",");
                }
                self.visit_node(item);
            }
            self.append("]");
        }
    }

    /// Retrieves the final formatted string result.
    pub fn get_result(&self) -> String {
        self.result.borrow().clone()
    }

    /// Returns a reference to the positions map.
    pub fn positions(&self) -> &HashMap<usize, (Position, Position)> {
        &self.positions
    }

    /// Formats a vector of key-value pairs as an array of maps.
    fn format_pairs(&self, pairs: &Vector<(Arc<RholangNode>, Arc<RholangNode>), ArcK>, key_name: &str, value_name: &str) {
        self.append("[");
        for (i, (key, value)) in pairs.iter().enumerate() {
            if i > 0 {
                self.append(",");
                if self.pretty_print {
                    self.append("\n");
                    let alignment = *self.alignment_columns.borrow().last().unwrap_or(&0);
                    self.append(&" ".repeat(alignment));
                }
            }
            self.start_map();
            self.add_field(key_name, |p| {
                p.visit_node(key);
            });
            self.add_field(value_name, |p| {
                p.visit_node(value);
            });
            self.end_map();
        }
        self.append("]");
    }
}

impl Visitor for PrettyPrinter {
    fn visit_par(&self, node: &Arc<RholangNode>, _base: &NodeBase, left: &Arc<RholangNode>, right: &Arc<RholangNode>, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"par\""));
        self.add_base_fields(node);
        self.add_field("left", |p| {
            p.visit_node(left);
        });
        self.add_field("right", |p| {
            p.visit_node(right);
        });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_send_sync(&self, node: &Arc<RholangNode>, _base: &NodeBase, channel: &Arc<RholangNode>, inputs: &Vector<Arc<RholangNode>, ArcK>, cont: &Arc<RholangNode>, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"sendsync\""));
        self.add_base_fields(node);
        self.add_field("channel", |p| {
            p.visit_node(channel);
        });
        self.add_field("inputs", |p| p.visit_vector(inputs));
        self.add_field("cont", |p| {
            p.visit_node(cont);
        });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_send(&self, node: &Arc<RholangNode>, _base: &NodeBase, channel: &Arc<RholangNode>, send_type: &RholangSendType, _send_type_delta: &RelativePosition, inputs: &Vector<Arc<RholangNode>, ArcK>, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"send\""));
        self.add_base_fields(node);
        self.add_field("channel", |p| {
            p.visit_node(channel);
        });
        self.add_field("send_type", |p| p.append(&format!("\"{:?}\"", send_type)));
        self.add_field("inputs", |p| p.visit_vector(inputs));
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_new(&self, node: &Arc<RholangNode>, _base: &NodeBase, decls: &Vector<Arc<RholangNode>, ArcK>, proc: &Arc<RholangNode>, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"new\""));
        self.add_base_fields(node);
        self.add_field("decls", |p| p.visit_vector(decls));
        self.add_field("proc", |p| {
            p.visit_node(proc);
        });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_ifelse(&self, node: &Arc<RholangNode>, _base: &NodeBase, condition: &Arc<RholangNode>, consequence: &Arc<RholangNode>, alternative: &Option<Arc<RholangNode>>, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"ifelse\""));
        self.add_base_fields(node);
        self.add_field("condition", |p| {
            p.visit_node(condition);
        });
        self.add_field("consequence", |p| {
            p.visit_node(consequence);
        });
        if let Some(alt) = alternative {
            self.add_field("alternative", |p| {
                p.visit_node(alt);
            });
        }
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_let(&self, node: &Arc<RholangNode>, _base: &NodeBase, decls: &Vector<Arc<RholangNode>, ArcK>, proc: &Arc<RholangNode>, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"let\""));
        self.add_base_fields(node);
        self.add_field("decls", |p| p.visit_vector(decls));
        self.add_field("proc", |p| {
            p.visit_node(proc);
        });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_bundle(&self, node: &Arc<RholangNode>, _base: &NodeBase, bundle_type: &RholangBundleType, proc: &Arc<RholangNode>, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"bundle\""));
        self.add_base_fields(node);
        self.add_field("bundle_type", |p| p.append(&format!("\"{:?}\"", bundle_type)));
        self.add_field("proc", |p| {
            p.visit_node(proc);
        });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_match(&self, node: &Arc<RholangNode>, _base: &NodeBase, expression: &Arc<RholangNode>, cases: &Vector<(Arc<RholangNode>, Arc<RholangNode>), ArcK>, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"match\""));
        self.add_base_fields(node);
        self.add_field("expression", |p| {
            p.visit_node(expression);
        });
        self.add_field("cases", |p| p.format_pairs(cases, "pattern", "proc"));
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_choice(&self, node: &Arc<RholangNode>, _base: &NodeBase, branches: &Vector<(Vector<Arc<RholangNode>, ArcK>, Arc<RholangNode>), ArcK>, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"choice\""));
        self.add_base_fields(node);
        self.add_field("branches", |p| {
            p.append("[");
            for (i, (inputs, proc)) in branches.iter().enumerate() {
                if i > 0 {
                    p.append(",");
                    if p.pretty_print {
                        p.append("\n");
                        let alignment = *p.alignment_columns.borrow().last().unwrap_or(&0);
                        p.append(&" ".repeat(alignment));
                    }
                }
                p.start_map();
                p.add_field("inputs", |p| p.visit_vector(inputs));
                p.add_field("proc", |p| {
                    p.visit_node(proc);
                });
                p.end_map();
            }
            p.append("]");
        });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_contract(&self, node: &Arc<RholangNode>, _base: &NodeBase, name: &Arc<RholangNode>, formals: &Vector<Arc<RholangNode>, ArcK>, formals_remainder: &Option<Arc<RholangNode>>, proc: &Arc<RholangNode>, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"contract\""));
        self.add_base_fields(node);
        self.add_field("name", |p| {
            p.visit_node(name);
        });
        self.add_field("formals", |p| p.visit_vector(formals));
        if let Some(rem) = formals_remainder {
            self.add_field("formals_remainder", |p| {
                p.visit_node(rem);
            });
        }
        self.add_field("proc", |p| {
            p.visit_node(proc);
        });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_input(&self, node: &Arc<RholangNode>, _base: &NodeBase, receipts: &Vector<Vector<Arc<RholangNode>, ArcK>, ArcK>, proc: &Arc<RholangNode>, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"input\""));
        self.add_base_fields(node);
        self.add_field("receipts", |p| {
            p.append("[");
            for (i, receipt) in receipts.iter().enumerate() {
                if i > 0 {
                    p.append(",");
                    if p.pretty_print {
                        p.append("\n");
                        let alignment = *p.alignment_columns.borrow().last().unwrap_or(&0);
                        p.append(&" ".repeat(alignment));
                    }
                }
                p.visit_vector(receipt);
            }
            p.append("]");
        });
        self.add_field("proc", |p| {
            p.visit_node(proc);
        });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_block(&self, node: &Arc<RholangNode>, _base: &NodeBase, proc: &Arc<RholangNode>, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"block\""));
        self.add_base_fields(node);
        self.add_field("proc", |p| {
            p.visit_node(proc);
        });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_parenthesized(&self, node: &Arc<RholangNode>, _base: &NodeBase, expr: &Arc<RholangNode>, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"parenthesized\""));
        self.add_base_fields(node);
        self.add_field("expr", |p| {
            p.visit_node(expr);
        });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_binop(&self, node: &Arc<RholangNode>, _base: &NodeBase, op: BinOperator, left: &Arc<RholangNode>, right: &Arc<RholangNode>, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"binop\""));
        self.add_base_fields(node);
        self.add_field("op", |p| p.append(&format!("\"{:?}\"", op)));
        self.add_field("left", |p| {
            p.visit_node(left);
        });
        self.add_field("right", |p| {
            p.visit_node(right);
        });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_unaryop(&self, node: &Arc<RholangNode>, _base: &NodeBase, op: UnaryOperator, operand: &Arc<RholangNode>, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"unaryop\""));
        self.add_base_fields(node);
        self.add_field("op", |p| p.append(&format!("\"{:?}\"", op)));
        self.add_field("operand", |p| {
            p.visit_node(operand);
        });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_method(&self, node: &Arc<RholangNode>, _base: &NodeBase, receiver: &Arc<RholangNode>, name: &String, args: &Vector<Arc<RholangNode>, ArcK>, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"method\""));
        self.add_base_fields(node);
        self.add_field("receiver", |p| {
            p.visit_node(receiver);
        });
        self.add_field("name", |p| p.escape_json_string(name));
        self.add_field("args", |p| p.visit_vector(args));
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_eval(&self, node: &Arc<RholangNode>, _base: &NodeBase, name: &Arc<RholangNode>, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"eval\""));
        self.add_base_fields(node);
        self.add_field("name", |p| {
            p.visit_node(name);
        });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_quote(&self, node: &Arc<RholangNode>, _base: &NodeBase, quotable: &Arc<RholangNode>, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"quote\""));
        self.add_base_fields(node);
        self.add_field("quotable", |p| {
            p.visit_node(quotable);
        });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_varref(&self, node: &Arc<RholangNode>, _base: &NodeBase, kind: RholangVarRefKind, var: &Arc<RholangNode>, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"varref\""));
        self.add_base_fields(node);
        self.add_field("kind", |p| p.append(&format!("\"{:?}\"", kind)));
        self.add_field("var", |p| {
            p.visit_node(var);
        });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_bool_literal(&self, node: &Arc<RholangNode>, _base: &NodeBase, _value: bool, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"bool\""));
        self.add_base_fields(node);
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_long_literal(&self, node: &Arc<RholangNode>, _base: &NodeBase, _value: i64, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"long\""));
        self.add_base_fields(node);
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_string_literal(&self, node: &Arc<RholangNode>, _base: &NodeBase, _value: &String, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"string\""));
        self.add_base_fields(node);
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_uri_literal(&self, node: &Arc<RholangNode>, _base: &NodeBase, _value: &String, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"uri\""));
        self.add_base_fields(node);
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_nil(&self, node: &Arc<RholangNode>, _base: &NodeBase, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"nil\""));
        self.add_base_fields(node);
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_list(&self, node: &Arc<RholangNode>, _base: &NodeBase, elements: &Vector<Arc<RholangNode>, ArcK>, remainder: &Option<Arc<RholangNode>>, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"list\""));
        self.add_base_fields(node);
        self.add_field("elements", |p| p.visit_vector(elements));
        if let Some(rem) = remainder {
            self.add_field("remainder", |p| {
                p.visit_node(rem);
            });
        }
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_set(&self, node: &Arc<RholangNode>, _base: &NodeBase, elements: &Vector<Arc<RholangNode>, ArcK>, remainder: &Option<Arc<RholangNode>>, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"set\""));
        self.add_base_fields(node);
        self.add_field("elements", |p| p.visit_vector(elements));
        if let Some(rem) = remainder {
            self.add_field("remainder", |p| {
                p.visit_node(rem);
            });
        }
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_map(&self, node: &Arc<RholangNode>, _base: &NodeBase, pairs: &Vector<(Arc<RholangNode>, Arc<RholangNode>), ArcK>, remainder: &Option<Arc<RholangNode>>, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"map\""));
        self.add_base_fields(node);
        self.add_field("pairs", |p| p.format_pairs(pairs, "key", "value"));
        if let Some(rem) = remainder {
            self.add_field("remainder", |p| {
                p.visit_node(rem);
            });
        }
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_tuple(&self, node: &Arc<RholangNode>, _base: &NodeBase, elements: &Vector<Arc<RholangNode>, ArcK>, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"tuple\""));
        self.add_base_fields(node);
        self.add_field("elements", |p| p.visit_vector(elements));
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_var(&self, node: &Arc<RholangNode>, _base: &NodeBase, name: &String, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"var\""));
        self.add_base_fields(node);
        self.add_field("name", |p| p.escape_json_string(name));
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_name_decl(&self, node: &Arc<RholangNode>, _base: &NodeBase, var: &Arc<RholangNode>, uri: &Option<Arc<RholangNode>>, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"name_decl\""));
        self.add_base_fields(node);
        self.add_field("var", |p| {
            p.visit_node(var);
        });
        if let Some(u) = uri {
            self.add_field("uri", |p| {
                p.visit_node(u);
            });
        }
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_decl(&self, node: &Arc<RholangNode>, _base: &NodeBase, names: &Vector<Arc<RholangNode>, ArcK>, names_remainder: &Option<Arc<RholangNode>>, procs: &Vector<Arc<RholangNode>, ArcK>, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"decl\""));
        self.add_base_fields(node);
        self.add_field("names", |p| p.visit_vector(names));
        if let Some(rem) = names_remainder {
            self.add_field("names_remainder", |p| {
                p.visit_node(rem);
            });
        }
        self.add_field("procs", |p| p.visit_vector(procs));
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_linear_bind(&self, node: &Arc<RholangNode>, _base: &NodeBase, names: &Vector<Arc<RholangNode>, ArcK>, remainder: &Option<Arc<RholangNode>>, source: &Arc<RholangNode>, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"linear_bind\""));
        self.add_base_fields(node);
        self.add_field("names", |p| p.visit_vector(names));
        if let Some(rem) = remainder {
            self.add_field("remainder", |p| {
                p.visit_node(rem);
            });
        }
        self.add_field("source", |p| {
            p.visit_node(source);
        });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_repeated_bind(&self, node: &Arc<RholangNode>, _base: &NodeBase, names: &Vector<Arc<RholangNode>, ArcK>, remainder: &Option<Arc<RholangNode>>, source: &Arc<RholangNode>, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"repeated_bind\""));
        self.add_base_fields(node);
        self.add_field("names", |p| p.visit_vector(names));
        if let Some(rem) = remainder {
            self.add_field("remainder", |p| {
                p.visit_node(rem);
            });
        }
        self.add_field("source", |p| {
            p.visit_node(source);
        });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_peek_bind(&self, node: &Arc<RholangNode>, _base: &NodeBase, names: &Vector<Arc<RholangNode>, ArcK>, remainder: &Option<Arc<RholangNode>>, source: &Arc<RholangNode>, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"peek_bind\""));
        self.add_base_fields(node);
        self.add_field("names", |p| p.visit_vector(names));
        if let Some(rem) = remainder {
            self.add_field("remainder", |p| {
                p.visit_node(rem);
            });
        }
        self.add_field("source", |p| {
            p.visit_node(source);
        });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_comment(&self, node: &Arc<RholangNode>, _base: &NodeBase, kind: &CommentKind, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"comment\""));
        self.add_base_fields(node);
        self.add_field("kind", |p| p.append(&format!("\"{:?}\"", kind)));
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_wildcard(&self, node: &Arc<RholangNode>, _base: &NodeBase, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"wildcard\""));
        self.add_base_fields(node);
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_simple_type(&self, node: &Arc<RholangNode>, _base: &NodeBase, value: &String, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"simple_type\""));
        self.add_base_fields(node);
        self.add_field("value", |p| p.escape_json_string(value));
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_receive_send_source(&self, node: &Arc<RholangNode>, _base: &NodeBase, name: &Arc<RholangNode>, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"receive_send_source\""));
        self.add_base_fields(node);
        self.add_field("name", |p| {
            p.visit_node(name);
        });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_send_receive_source(&self, node: &Arc<RholangNode>, _base: &NodeBase, name: &Arc<RholangNode>, inputs: &Vector<Arc<RholangNode>, ArcK>, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"send_receive_source\""));
        self.add_base_fields(node);
        self.add_field("name", |p| {
            p.visit_node(name);
        });
        self.add_field("inputs", |p| p.visit_vector(inputs));
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_error(&self, node: &Arc<RholangNode>, _base: &NodeBase, children: &Vector<Arc<RholangNode>, ArcK>, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"error\""));
        self.add_base_fields(node);
        self.add_field("children", |p| p.visit_vector(children));
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_disjunction(&self, node: &Arc<RholangNode>, _base: &NodeBase, left: &Arc<RholangNode>, right: &Arc<RholangNode>, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"disjunction\""));
        self.add_base_fields(node);
        self.add_field("left", |p| {
            p.visit_node(left);
        });
        self.add_field("right", |p| {
            p.visit_node(right);
        });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_conjunction(&self, node: &Arc<RholangNode>, _base: &NodeBase, left: &Arc<RholangNode>, right: &Arc<RholangNode>, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"conjunction\""));
        self.add_base_fields(node);
        self.add_field("left", |p| {
            p.visit_node(left);
        });
        self.add_field("right", |p| {
            p.visit_node(right);
        });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_negation(&self, node: &Arc<RholangNode>, _base: &NodeBase, operand: &Arc<RholangNode>, metadata: &Option<Arc<Metadata>>) -> Arc<RholangNode> {
        self.start_map();
        self.add_field("type", |p| p.append("\"negation\""));
        self.add_base_fields(node);
        self.add_field("operand", |p| {
            p.visit_node(operand);
        });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use crate::ir::rholang_node::{Metadata, RholangNode, NodeBase, RelativePosition};
    use crate::ir::transforms::pretty_printer::format;
    use std::sync::Arc;
    use ropey::Rope;

    #[test]
    fn test_pretty_printer_aligned() {
        let _ = crate::logging::init_logger(false, Some("warn"), false);
        let rholang_code = r#"true|42"#;
        let tree = crate::tree_sitter::parse_code(rholang_code);
        let rope = Rope::from_str(rholang_code);
        let ir = crate::tree_sitter::parse_to_ir(&tree, &rope);
        let actual = format(&ir, true, &rope).expect("Failed to format tree");
        let expected = indoc! {r#"
            {:type "par"
             :start_line 0
             :start_column 0
             :end_line 0
             :end_column 7
             :position 0
             :length 7
             :left {:type "bool"
                    :start_line 0
                    :start_column 0
                    :end_line 0
                    :end_column 4
                    :position 0
                    :length 4
                    :metadata {:version 0}}
             :right {:type "long"
                     :start_line 0
                     :start_column 5
                     :end_line 0
                     :end_column 7
                     :position 5
                     :length 2
                     :metadata {:version 0}}
             :metadata {:version 0}}"#}.trim();
        println!("{}", actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_pretty_printer_unaligned() {
        let _ = crate::logging::init_logger(false, Some("warn"), false);
        let rholang_code = r#"true|42"#;
        let tree = crate::tree_sitter::parse_code(rholang_code);
        let rope = Rope::from_str(rholang_code);
        let ir = crate::tree_sitter::parse_to_ir(&tree, &rope);
        let actual = format(&ir, false, &rope).expect("Failed to format tree");
        let expected = r#"{:type "par",:start_line 0,:start_column 0,:end_line 0,:end_column 7,:position 0,:length 7,:left {:type "bool",:start_line 0,:start_column 0,:end_line 0,:end_column 4,:position 0,:length 4,:metadata {:version 0}},:right {:type "long",:start_line 0,:start_column 5,:end_line 0,:end_column 7,:position 5,:length 2,:metadata {:version 0}},:metadata {:version 0}}"#;
        println!("{}", actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_pretty_print_send() {
        let _ = crate::logging::init_logger(false, Some("warn"), false);
        let rholang_code = r#"ch!("msg")"#;
        let tree = crate::tree_sitter::parse_code(rholang_code);
        let rope = Rope::from_str(rholang_code);
        let ir = crate::tree_sitter::parse_to_ir(&tree, &rope);
        let actual = format(&ir, true, &rope).expect("Failed to format tree");
        let expected = indoc! {r#"
            {:type "send"
             :start_line 0
             :start_column 0
             :end_line 0
             :end_column 10
             :position 0
             :length 10
             :channel {:type "var"
                       :start_line 0
                       :start_column 0
                       :end_line 0
                       :end_column 2
                       :position 0
                       :length 2
                       :name "ch"
                       :metadata {:version 0}}
             :send_type "Single"
             :inputs [{:type "string"
                       :start_line 0
                       :start_column 4
                       :end_line 0
                       :end_column 9
                       :position 4
                       :length 5
                       :metadata {:version 0}}]
             :metadata {:version 0}}"#}.trim();
        println!("{}", actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_pretty_print_special_chars() {
        let _ = crate::logging::init_logger(false, Some("warn"), false);
        let rholang_code = r#"ch!("Hello\nWorld")"#;
        let tree = crate::tree_sitter::parse_code(rholang_code);
        let rope = Rope::from_str(rholang_code);
        let ir = crate::tree_sitter::parse_to_ir(&tree, &rope);
        let actual = format(&ir, true, &rope).expect("Failed to format tree");
        let expected = indoc! {r#"
            {:type "send"
             :start_line 0
             :start_column 0
             :end_line 0
             :end_column 19
             :position 0
             :length 19
             :channel {:type "var"
                       :start_line 0
                       :start_column 0
                       :end_line 0
                       :end_column 2
                       :position 0
                       :length 2
                       :name "ch"
                       :metadata {:version 0}}
             :send_type "Single"
             :inputs [{:type "string"
                       :start_line 0
                       :start_column 4
                       :end_line 0
                       :end_column 18
                       :position 4
                       :length 14
                       :metadata {:version 0}}]
             :metadata {:version 0}}"#}.trim();
        println!("{}", actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_pretty_print_decl() {
        let _ = crate::logging::init_logger(false, Some("warn"), false);
        let rholang_code = r#"let x = "hello" in { Nil }"#;
        let tree = crate::tree_sitter::parse_code(rholang_code);
        let rope = Rope::from_str(rholang_code);
        let ir = crate::tree_sitter::parse_to_ir(&tree, &rope);
        let actual = format(&ir, true, &rope).expect("Failed to format tree");
        let expected = indoc! {r#"
            {:type "let"
             :start_line 0
             :start_column 0
             :end_line 0
             :end_column 26
             :position 0
             :length 26
             :decls [{:type "decl"
                      :start_line 0
                      :start_column 4
                      :end_line 0
                      :end_column 15
                      :position 4
                      :length 11
                      :names [{:type "var"
                               :start_line 0
                               :start_column 4
                               :end_line 0
                               :end_column 5
                               :position 4
                               :length 1
                               :name "x"
                               :metadata {:version 0}}]
                      :procs [{:type "string"
                               :start_line 0
                               :start_column 8
                               :end_line 0
                               :end_column 15
                               :position 8
                               :length 7
                               :metadata {:version 0}}]
                      :metadata {:version 0}}]
             :proc {:type "block"
                    :start_line 0
                    :start_column 19
                    :end_line 0
                    :end_column 26
                    :position 19
                    :length 7
                    :proc {:type "nil"
                           :start_line 0
                           :start_column 21
                           :end_line 0
                           :end_column 24
                           :position 21
                           :length 3
                           :metadata {:version 0}}
                    :metadata {:version 0}}
             :metadata {:version 0}}"#}.trim();
        println!("{}", actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_pretty_print_new() {
        let _ = crate::logging::init_logger(false, Some("warn"), false);
        let rholang_code = r#"new x in { x!("hello") }"#;
        let tree = crate::tree_sitter::parse_code(rholang_code);
        let rope = Rope::from_str(rholang_code);
        let ir = crate::tree_sitter::parse_to_ir(&tree, &rope);
        let actual = format(&ir, true, &rope).expect("Failed to format tree");
        let expected = indoc! {r#"
            {:type "new"
             :start_line 0
             :start_column 0
             :end_line 0
             :end_column 24
             :position 0
             :length 24
             :decls [{:type "name_decl"
                      :start_line 0
                      :start_column 4
                      :end_line 0
                      :end_column 5
                      :position 4
                      :length 1
                      :var {:type "var"
                            :start_line 0
                            :start_column 4
                            :end_line 0
                            :end_column 5
                            :position 4
                            :length 1
                            :name "x"
                            :metadata {:version 0}}
                      :metadata {:version 0}}]
             :proc {:type "block"
                    :start_line 0
                    :start_column 9
                    :end_line 0
                    :end_column 24
                    :position 9
                    :length 15
                    :proc {:type "send"
                           :start_line 0
                           :start_column 11
                           :end_line 0
                           :end_column 22
                           :position 11
                           :length 11
                           :channel {:type "var"
                                     :start_line 0
                                     :start_column 11
                                     :end_line 0
                                     :end_column 12
                                     :position 11
                                     :length 1
                                     :name "x"
                                     :metadata {:version 0}}
                           :send_type "Single"
                           :inputs [{:type "string"
                                     :start_line 0
                                     :start_column 14
                                     :end_line 0
                                     :end_column 21
                                     :position 14
                                     :length 7
                                     :metadata {:version 0}}]
                           :metadata {:version 0}}
                    :metadata {:version 0}}
             :metadata {:version 0}}"#}.trim();
        println!("{}", actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_pretty_print_ifelse() {
        let _ = crate::logging::init_logger(false, Some("warn"), false);
        let rholang_code = r#"if (true) { Nil } else { Nil }"#;
        let tree = crate::tree_sitter::parse_code(rholang_code);
        let rope = Rope::from_str(rholang_code);
        let ir = crate::tree_sitter::parse_to_ir(&tree, &rope);
        let actual = format(&ir, true, &rope).expect("Failed to format tree");
        let expected = indoc! {r#"
            {:type "ifelse"
             :start_line 0
             :start_column 0
             :end_line 0
             :end_column 30
             :position 0
             :length 30
             :condition {:type "bool"
                         :start_line 0
                         :start_column 4
                         :end_line 0
                         :end_column 8
                         :position 4
                         :length 4
                         :metadata {:version 0}}
             :consequence {:type "block"
                           :start_line 0
                           :start_column 10
                           :end_line 0
                           :end_column 17
                           :position 10
                           :length 7
                           :proc {:type "nil"
                                  :start_line 0
                                  :start_column 12
                                  :end_line 0
                                  :end_column 15
                                  :position 12
                                  :length 3
                                  :metadata {:version 0}}
                           :metadata {:version 0}}
             :alternative {:type "block"
                           :start_line 0
                           :start_column 23
                           :end_line 0
                           :end_column 30
                           :position 23
                           :length 7
                           :proc {:type "nil"
                                  :start_line 0
                                  :start_column 25
                                  :end_line 0
                                  :end_column 28
                                  :position 25
                                  :length 3
                                  :metadata {:version 0}}
                           :metadata {:version 0}}
             :metadata {:version 0}}"#}.trim();
        println!("{}", actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_pretty_print_match() {
        let _ = crate::logging::init_logger(false, Some("warn"), false);
        let rholang_code = r#"match "hello" { "hello" => Nil }"#;
        let tree = crate::tree_sitter::parse_code(rholang_code);
        let rope = Rope::from_str(rholang_code);
        let ir = crate::tree_sitter::parse_to_ir(&tree, &rope);
        let actual = format(&ir, true, &rope).expect("Failed to format tree");
        let expected = indoc! {r#"
            {:type "match"
             :start_line 0
             :start_column 0
             :end_line 0
             :end_column 32
             :position 0
             :length 32
             :expression {:type "string"
                          :start_line 0
                          :start_column 6
                          :end_line 0
                          :end_column 13
                          :position 6
                          :length 7
                          :metadata {:version 0}}
             :cases [{:pattern {:type "string"
                                :start_line 0
                                :start_column 16
                                :end_line 0
                                :end_column 23
                                :position 16
                                :length 7
                                :metadata {:version 0}}
                      :proc {:type "nil"
                             :start_line 0
                             :start_column 27
                             :end_line 0
                             :end_column 30
                             :position 27
                             :length 3
                             :metadata {:version 0}}}]
             :metadata {:version 0}}"#}.trim();
        println!("{}", actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_pretty_print_contract() {
        let _ = crate::logging::init_logger(false, Some("warn"), false);
        let rholang_code = r#"contract myContract(param) = { Nil }"#;
        let tree = crate::tree_sitter::parse_code(rholang_code);
        let rope = Rope::from_str(rholang_code);
        let ir = crate::tree_sitter::parse_to_ir(&tree, &rope);
        let actual = format(&ir, true, &rope).expect("Failed to format tree");
        let expected = indoc! {r#"
            {:type "contract"
             :start_line 0
             :start_column 0
             :end_line 0
             :end_column 36
             :position 0
             :length 36
             :name {:type "var"
                    :start_line 0
                    :start_column 9
                    :end_line 0
                    :end_column 19
                    :position 9
                    :length 10
                    :name "myContract"
                    :metadata {:version 0}}
             :formals [{:type "var"
                        :start_line 0
                        :start_column 20
                        :end_line 0
                        :end_column 25
                        :position 20
                        :length 5
                        :name "param"
                        :metadata {:version 0}}]
             :proc {:type "block"
                    :start_line 0
                    :start_column 29
                    :end_line 0
                    :end_column 36
                    :position 29
                    :length 7
                    :proc {:type "nil"
                           :start_line 0
                           :start_column 31
                           :end_line 0
                           :end_column 34
                           :position 31
                           :length 3
                           :metadata {:version 0}}
                    :metadata {:version 0}}
             :metadata {:version 0}}"#}.trim();
        println!("{}", actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_pretty_print_input() {
        let _ = crate::logging::init_logger(false, Some("warn"), false);
        let rholang_code = r#"for (x <- ch) { Nil }"#;
        let tree = crate::tree_sitter::parse_code(rholang_code);
        let rope = Rope::from_str(rholang_code);
        let ir = crate::tree_sitter::parse_to_ir(&tree, &rope);
        let actual = format(&ir, true, &rope).expect("Failed to format tree");
        let expected = indoc! {r#"
            {:type "input"
             :start_line 0
             :start_column 0
             :end_line 0
             :end_column 21
             :position 0
             :length 21
             :receipts [[{:type "linear_bind"
                          :start_line 0
                          :start_column 5
                          :end_line 0
                          :end_column 12
                          :position 5
                          :length 7
                          :names [{:type "var"
                                   :start_line 0
                                   :start_column 5
                                   :end_line 0
                                   :end_column 6
                                   :position 5
                                   :length 1
                                   :name "x"
                                   :metadata {:version 0}}]
                          :source {:type "var"
                                   :start_line 0
                                   :start_column 10
                                   :end_line 0
                                   :end_column 12
                                   :position 10
                                   :length 2
                                   :name "ch"
                                   :metadata {:version 0}}
                          :metadata {:version 0}}]]
             :proc {:type "block"
                    :start_line 0
                    :start_column 14
                    :end_line 0
                    :end_column 21
                    :position 14
                    :length 7
                    :proc {:type "nil"
                           :start_line 0
                           :start_column 16
                           :end_line 0
                           :end_column 19
                           :position 16
                           :length 3
                           :metadata {:version 0}}
                    :metadata {:version 0}}
             :metadata {:version 0}}"#}.trim();
        println!("{}", actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_pretty_print_binop() {
        let _ = crate::logging::init_logger(false, Some("warn"), false);
        let rholang_code = r#"1 + 2"#;
        let tree = crate::tree_sitter::parse_code(rholang_code);
        let rope = Rope::from_str(rholang_code);
        let ir = crate::tree_sitter::parse_to_ir(&tree, &rope);
        let actual = format(&ir, true, &rope).expect("Failed to format tree");
        let expected = indoc! {r#"
            {:type "binop"
             :start_line 0
             :start_column 0
             :end_line 0
             :end_column 5
             :position 0
             :length 5
             :op "Add"
             :left {:type "long"
                    :start_line 0
                    :start_column 0
                    :end_line 0
                    :end_column 1
                    :position 0
                    :length 1
                    :metadata {:version 0}}
             :right {:type "long"
                     :start_line 0
                     :start_column 4
                     :end_line 0
                     :end_column 5
                     :position 4
                     :length 1
                     :metadata {:version 0}}
             :metadata {:version 0}}"#}.trim();
        println!("{}", actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_pretty_print_list() {
        let _ = crate::logging::init_logger(false, Some("warn"), false);
        let rholang_code = r#"[1, 2, 3]"#;
        let tree = crate::tree_sitter::parse_code(rholang_code);
        let rope = Rope::from_str(rholang_code);
        let ir = crate::tree_sitter::parse_to_ir(&tree, &rope);
        let actual = format(&ir, true, &rope).expect("Failed to format tree");
        let expected = indoc! {r#"
            {:type "list"
             :start_line 0
             :start_column 0
             :end_line 0
             :end_column 9
             :position 0
             :length 9
             :elements [{:type "long"
                         :start_line 0
                         :start_column 1
                         :end_line 0
                         :end_column 2
                         :position 1
                         :length 1
                         :metadata {:version 0}}
                        {:type "long"
                         :start_line 0
                         :start_column 4
                         :end_line 0
                         :end_column 5
                         :position 4
                         :length 1
                         :metadata {:version 0}}
                        {:type "long"
                         :start_line 0
                         :start_column 7
                         :end_line 0
                         :end_column 8
                         :position 7
                         :length 1
                         :metadata {:version 0}}]
             :metadata {:version 0}}"#}.trim();
        println!("{}", actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_pretty_print_comment() {
        let _ = crate::logging::init_logger(false, Some("warn"), false);
        let rholang_code = r#"// This is a comment
Nil"#;
        let tree = crate::tree_sitter::parse_code(rholang_code);
        let rope = Rope::from_str(rholang_code);
        let ir = crate::tree_sitter::parse_to_ir(&tree, &rope);

        let actual = format(&ir, true, &rope).expect("Failed to format tree");
        // Comments are in extras and filtered out of the IR, so only Nil remains
        let expected = indoc! {r#"
            {:type "nil"
             :start_line 1
             :start_column 0
             :end_line 1
             :end_column 3
             :position 21
             :length 3
             :metadata {:version 0}}"#}.trim();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_pretty_print_match_fixed() {
        let _ = crate::logging::init_logger(false, Some("warn"), false);
        let code = r#"match "target" { "pat" => Nil }"#;
        let tree = crate::tree_sitter::parse_code(code);
        let rope = Rope::from_str(code);
        let ir = crate::tree_sitter::parse_to_ir(&tree, &rope);
        let actual = format(&ir, true, &rope).unwrap();
        let expected = indoc! {r#"
            {:type "match"
             :start_line 0
             :start_column 0
             :end_line 0
             :end_column 31
             :position 0
             :length 31
             :expression {:type "string"
                          :start_line 0
                          :start_column 6
                          :end_line 0
                          :end_column 14
                          :position 6
                          :length 8
                          :metadata {:version 0}}
             :cases [{:pattern {:type "string"
                                :start_line 0
                                :start_column 17
                                :end_line 0
                                :end_column 22
                                :position 17
                                :length 5
                                :metadata {:version 0}}
                      :proc {:type "nil"
                             :start_line 0
                             :start_column 26
                             :end_line 0
                             :end_column 29
                             :position 26
                             :length 3
                             :metadata {:version 0}}}]
             :metadata {:version 0}}"#}.trim();
        println!("{}", actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_pretty_print_input_fixed() {
        let _ = crate::logging::init_logger(false, Some("warn"), false);
        let code = r#"for (x <- ch) { Nil }"#;
        let tree = crate::tree_sitter::parse_code(code);
        let rope = Rope::from_str(code);
        let ir = crate::tree_sitter::parse_to_ir(&tree, &rope);
        let actual = format(&ir, true, &rope).unwrap();
        let expected = indoc! {r#"
            {:type "input"
             :start_line 0
             :start_column 0
             :end_line 0
             :end_column 21
             :position 0
             :length 21
             :receipts [[{:type "linear_bind"
                          :start_line 0
                          :start_column 5
                          :end_line 0
                          :end_column 12
                          :position 5
                          :length 7
                          :names [{:type "var"
                                   :start_line 0
                                   :start_column 5
                                   :end_line 0
                                   :end_column 6
                                   :position 5
                                   :length 1
                                   :name "x"
                                   :metadata {:version 0}}]
                          :source {:type "var"
                                   :start_line 0
                                   :start_column 10
                                   :end_line 0
                                   :end_column 12
                                   :position 10
                                   :length 2
                                   :name "ch"
                                   :metadata {:version 0}}
                          :metadata {:version 0}}]]
             :proc {:type "block"
                    :start_line 0
                    :start_column 14
                    :end_line 0
                    :end_column 21
                    :position 14
                    :length 7
                    :proc {:type "nil"
                           :start_line 0
                           :start_column 16
                           :end_line 0
                           :end_column 19
                           :position 16
                           :length 3
                           :metadata {:version 0}}
                    :metadata {:version 0}}
             :metadata {:version 0}}"#}.trim();
        println!("{}", actual);
        assert_eq!(actual, expected);
    }

    /// Creates a Nil node with default metadata containing a version field.
    fn create_nil_node() -> Arc<RholangNode> {
        let base = NodeBase::new(RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 }, 3, 0, 3);
        let mut data = HashMap::new();
        data.insert("version".to_string(), Arc::new(0_usize) as Arc<dyn Any + Send + Sync>);
        let metadata = Some(Arc::new(data));
        Arc::new(RholangNode::Nil { base, metadata })
    }

    /// Adds a key-value pair to the node's metadata, returning a new node.
    fn add_metadata<T: 'static + Send + Sync>(node: Arc<RholangNode>, key: &str, value: T) -> Arc<RholangNode> {
        let mut data = node.metadata().unwrap().as_ref().clone();
        data.insert(key.to_string(), Arc::new(value) as Arc<dyn Any + Send + Sync>);
        node.with_metadata(Some(Arc::new(data)))
    }

    /// Helper to assert that the formatted output contains the expected key-value pair.
    fn assert_contains_key_value(formatted: &str, key: &str, value: &str) {
        let pattern = format!(":{} {}", key, value);
        assert!(
            formatted.contains(&pattern),
            "Expected '{}' to contain '{}', but got '{}'",
            formatted,
            pattern,
            formatted
        );
    }

    #[test]
    fn test_format_bool_metadata() {
        let node = create_nil_node();
        let mut nested_map = HashMap::new();
        nested_map.insert("subkey".to_string(), 42_i32);
        let node_with_nested = add_metadata(node, "nested", nested_map);
        let rope = Rope::from_str("Nil");
        let actual = format(&node_with_nested, true, &rope).unwrap();
        let expected = indoc! {r#"
            {:type "nil"
             :start_line 0
             :start_column 0
             :end_line 0
             :end_column 3
             :position 0
             :length 3
             :metadata {:nested {:subkey 42}
                        :version 0}}
        "#}.trim();
        println!("{}", actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_format_i32_metadata() {
        let node = create_nil_node();
        let node_with_int = add_metadata(node, "int", 42_i32);
        let rope = Rope::from_str("Nil");
        let formatted = format(&node_with_int, false, &rope).unwrap();
        assert_contains_key_value(&formatted, "int", "42");
    }

    #[test]
    fn test_format_i8_metadata() {
        let node = create_nil_node();
        let node_with_int = add_metadata(node, "int8", -8_i8);
        let rope = Rope::from_str("Nil");
        let formatted = format(&node_with_int, false, &rope).unwrap();
        assert_contains_key_value(&formatted, "int8", "-8");
    }

    #[test]
    fn test_format_i16_metadata() {
        let node = create_nil_node();
        let node_with_int = add_metadata(node, "int16", 16_i16);
        let rope = Rope::from_str("Nil");
        let formatted = format(&node_with_int, false, &rope).unwrap();
        assert_contains_key_value(&formatted, "int16", "16");
    }

    #[test]
    fn test_format_i64_metadata() {
        let node = create_nil_node();
        let node_with_int = add_metadata(node, "int64", -64_i64);
        let rope = Rope::from_str("Nil");
        let formatted = format(&node_with_int, false, &rope).unwrap();
        assert_contains_key_value(&formatted, "int64", "-64");
    }

    #[test]
    fn test_format_i128_metadata() {
        let node = create_nil_node();
        let node_with_int = add_metadata(node, "int128", 128_i128);
        let rope = Rope::from_str("Nil");
        let formatted = format(&node_with_int, false, &rope).unwrap();
        assert_contains_key_value(&formatted, "int128", "128");
    }

    #[test]
    fn test_format_isize_metadata() {
        let node = create_nil_node();
        let node_with_int = add_metadata(node, "isize", -100_isize);
        let rope = Rope::from_str("Nil");
        let formatted = format(&node_with_int, false, &rope).unwrap();
        assert_contains_key_value(&formatted, "isize", "-100");
    }

    #[test]
    fn test_format_u8_metadata() {
        let node = create_nil_node();
        let node_with_uint = add_metadata(node, "uint8", 8_u8);
        let rope = Rope::from_str("Nil");
        let formatted = format(&node_with_uint, false, &rope).unwrap();
        assert_contains_key_value(&formatted, "uint8", "8");
    }

    #[test]
    fn test_format_u16_metadata() {
        let node = create_nil_node();
        let node_with_uint = add_metadata(node, "uint16", 16_u16);
        let rope = Rope::from_str("Nil");
        let formatted = format(&node_with_uint, false, &rope).unwrap();
        assert_contains_key_value(&formatted, "uint16", "16");
    }

    #[test]
    fn test_format_u32_metadata() {
        let node = create_nil_node();
        let node_with_uint = add_metadata(node, "uint32", 32_u32);
        let rope = Rope::from_str("Nil");
        let formatted = format(&node_with_uint, false, &rope).unwrap();
        assert_contains_key_value(&formatted, "uint32", "32");
    }

    #[test]
    fn test_format_u64_metadata() {
        let node = create_nil_node();
        let node_with_uint = add_metadata(node, "uint64", 64_u64);
        let rope = Rope::from_str("Nil");
        let formatted = format(&node_with_uint, false, &rope).unwrap();
        assert_contains_key_value(&formatted, "uint64", "64");
    }

    #[test]
    fn test_format_u128_metadata() {
        let node = create_nil_node();
        let node_with_uint = add_metadata(node, "uint128", 128_u128);
        let rope = Rope::from_str("Nil");
        let formatted = format(&node_with_uint, false, &rope).unwrap();
        assert_contains_key_value(&formatted, "uint128", "128");
    }

    #[test]
    fn test_format_usize_metadata() {
        let node = create_nil_node();
        let node_with_uint = add_metadata(node, "usize", 100_usize);
        let rope = Rope::from_str("Nil");
        let formatted = format(&node_with_uint, false, &rope).unwrap();
        assert_contains_key_value(&formatted, "usize", "100");
    }

    #[test]
    fn test_format_f32_metadata() {
        let node = create_nil_node();
        let node_with_float = add_metadata(node, "float32", 3.14_f32);
        let rope = Rope::from_str("Nil");
        let formatted = format(&node_with_float, false, &rope).unwrap();
        assert_contains_key_value(&formatted, "float32", "3.14");
    }

    #[test]
    fn test_format_f64_metadata() {
        let node = create_nil_node();
        let node_with_float = add_metadata(node, "float64", 2.718_f64);
        let rope = Rope::from_str("Nil");
        let formatted = format(&node_with_float, false, &rope).unwrap();
        assert_contains_key_value(&formatted, "float64", "2.718");
    }

    #[test]
    fn test_format_char_metadata() {
        let node = create_nil_node();
        let node_with_char = add_metadata(node, "char", 'a');
        let rope = Rope::from_str("Nil");
        let formatted = format(&node_with_char, false, &rope).unwrap();
        assert_contains_key_value(&formatted, "char", "\"a\"");
    }

    #[test]
    fn test_format_string_metadata() {
        let node = create_nil_node();
        let node_with_str = add_metadata(node, "str", "hello".to_string());
        let rope = Rope::from_str("Nil");
        let formatted = format(&node_with_str, false, &rope).unwrap();
        assert_contains_key_value(&formatted, "str", "\"hello\"");
    }

    #[test]
    fn test_format_string_with_special_chars() {
        let node = create_nil_node();
        let node_with_str = add_metadata(node, "str", "hello\nworld".to_string());
        let rope = Rope::from_str("Nil");
        let formatted = format(&node_with_str, false, &rope).unwrap();
        assert_contains_key_value(&formatted, "str", "\"hello\\nworld\"");
    }

    #[test]
    fn test_format_vec_metadata() {
        let node = create_nil_node();
        let vec_data = vec![1_i32, 2, 3];
        let node_with_vec = add_metadata(node, "vec", vec_data);
        let rope = Rope::from_str("Nil");
        let actual = format(&node_with_vec, true, &rope).unwrap();
        let expected = indoc! {r#"
            {:type "nil"
             :start_line 0
             :start_column 0
             :end_line 0
             :end_column 3
             :position 0
             :length 3
             :metadata {:vec [1
                              2
                              3]
                        :version 0}}
        "#}.trim();
        println!("{}", actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_metadata_empty_map() {
        let node = create_nil_node();
        let mut data = HashMap::new();
        data.insert("empty".to_string(), Arc::new(HashMap::<String, i32>::new()) as Arc<dyn Any + Send + Sync>);
        let node_with_empty = node.with_metadata(Some(Arc::new(data)));
        let rope = Rope::from_str("Nil");
        let formatted = format(&node_with_empty, true, &rope).unwrap();
        let expected = indoc! {r#"
            {:type "nil"
             :start_line 0
             :start_column 0
             :end_line 0
             :end_column 3
             :position 0
             :length 3
             :metadata {:empty {}}}
        "#}.trim();
        assert_eq!(formatted, expected);
    }

    #[test]
    fn test_metadata_map_pretty() {
        let node = create_nil_node();
        let mut nested_map = HashMap::new();
        nested_map.insert("subkey".to_string(), 42_i32);
        let node_with_nested = add_metadata(node, "nested", nested_map);
        let rope = Rope::from_str("Nil");
        let formatted = format(&node_with_nested, true, &rope).unwrap();
        let expected = indoc! {r#"
            {:type "nil"
             :start_line 0
             :start_column 0
             :end_line 0
             :end_column 3
             :position 0
             :length 3
             :metadata {:nested {:subkey 42}
                        :version 0}}
        "#}.trim();
        assert_eq!(formatted, expected);
    }

    #[test]
    fn test_metadata_map_compact() {
        let node = create_nil_node();
        let mut nested_map = HashMap::new();
        nested_map.insert("subkey".to_string(), 42_i32);
        let node_with_nested = add_metadata(node, "nested", nested_map);
        let rope = Rope::from_str("Nil");
        let formatted = format(&node_with_nested, false, &rope).unwrap();
        let expected = r#"{:type "nil",:start_line 0,:start_column 0,:end_line 0,:end_column 3,:position 0,:length 3,:metadata {:nested {:subkey 42},:version 0}}"#;
        assert_eq!(formatted, expected);
    }

    #[test]
    fn test_metadata_vector_pretty() {
        let node = create_nil_node();
        let vec_data = vec![1_i32, 2, 3];
        let node_with_vec = add_metadata(node, "vec", vec_data);
        let rope = Rope::from_str("Nil");
        let formatted = format(&node_with_vec, true, &rope).unwrap();
        let expected = indoc! {r#"
            {:type "nil"
             :start_line 0
             :start_column 0
             :end_line 0
             :end_column 3
             :position 0
             :length 3
             :metadata {:vec [1
                              2
                              3]
                        :version 0}}
        "#}.trim();
        assert_eq!(formatted, expected);
    }

    #[test]
    fn test_metadata_vector_compact() {
        let node = create_nil_node();
        let vec_data = vec![1_i32, 2, 3];
        let node_with_vec = add_metadata(node, "vec", vec_data);
        let rope = Rope::from_str("Nil");
        let formatted = format(&node_with_vec, false, &rope).unwrap();
        let expected = r#"{:type "nil",:start_line 0,:start_column 0,:end_line 0,:end_column 3,:position 0,:length 3,:metadata {:vec [1 2 3],:version 0}}"#;
        assert_eq!(formatted, expected);
    }

    #[test]
    fn test_metadata_empty_vector() {
        let node = create_nil_node();
        let vec_data: Vec<i32> = vec![];
        let node_with_vec = add_metadata(node, "vec", vec_data);
        let rope = Rope::from_str("Nil");
        let formatted = format(&node_with_vec, true, &rope).unwrap();
        let expected = indoc! {r#"
            {:type "nil"
             :start_line 0
             :start_column 0
             :end_line 0
             :end_column 3
             :position 0
             :length 3
             :metadata {:vec []
                        :version 0}}
        "#}.trim();
        assert_eq!(formatted, expected);
    }

    #[test]
    fn test_metadata_with_set_pretty() {
        let node = create_nil_node();
        let mut set_data = HashSet::new();
        set_data.insert(1);
        let node_with_set = add_metadata(node, "set", set_data);
        let rope = Rope::from_str("Nil");
        let actual = format(&node_with_set, true, &rope).unwrap();
        let expected = indoc! {r#"
            {:type "nil"
             :start_line 0
             :start_column 0
             :end_line 0
             :end_column 3
             :position 0
             :length 3
             :metadata {:set #{1}
                        :version 0}}
        "#}.trim();
        print!("{}", actual);
        assert_eq!(actual, expected);
    }
}
