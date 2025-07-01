use std::collections::HashMap;
use std::cell::RefCell;
use std::sync::Arc;
use tracing::{debug, trace};
use super::super::node::{
    BinOperator, BundleType, CommentKind, Node, NodeBase, Metadata, SendType, UnaryOperator,
    VarRefKind, Position, compute_absolute_positions,
};
use super::super::visitor::Visitor;
use rpds::Vector;

/// Formats the Rholang IR tree into a JSON-like string representation.
/// This module provides a visitor that serializes the IR tree, supporting both
/// compact and pretty-printed output with alignment and indentation options.
pub fn format<'a>(tree: &Arc<Node<'a>>, pretty_print: bool) -> Result<String, String> {
    tree.validate()?;
    let positions = compute_absolute_positions(tree);
    let printer = PrettyPrinter::new(pretty_print, &positions);
    printer.visit_node(tree);
    let result = printer.get_result();
    trace!("Formatted IR tree (pretty_print={}): {}", pretty_print, result);
    let (start, _) = positions.get(&tree.base().ts_node().map_or(0, |n| n.id())).unwrap();
    debug!("Formatted IR at {}:{} (length={})", start.row, start.column, result.len());
    Ok(result)
}

/// A visitor that constructs a JSON-like string representation of the IR tree.
/// Supports compact or pretty-printed output based on configuration.
pub struct PrettyPrinter<'a> {
    /// If true, formats output with indentation and alignment.
    pretty_print: bool,
    /// The accumulating string result.
    result: RefCell<String>,
    /// Tracks the current column position for alignment.
    current_column: RefCell<usize>,
    /// Stack of alignment column positions for nested structures.
    alignment_columns: RefCell<Vec<usize>>,
    /// Indicates if the next field is the first in its map.
    is_first_field: RefCell<bool>,
    /// Maps node IDs to their absolute positions in the source code.
    positions: &'a HashMap<usize, (Position, Position)>,
}

impl<'a> PrettyPrinter<'a> {
    /// Creates a new pretty printer instance.
    ///
    /// # Arguments
    /// * `pretty_print` - Enables indentation and alignment if true.
    /// * `positions` - Precomputed node positions for accurate metadata.
    pub fn new(pretty_print: bool, positions: &'a HashMap<usize, (Position, Position)>) -> Self {
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
    fn add_base_fields(&self, node: &Arc<Node<'a>>) {
        let key = node.base().ts_node().map_or(0, |n| n.id());
        let (start, end) = self.positions.get(&key).unwrap();
        let base = node.base();
        self.add_field("start_line", |p| p.append(&start.row.to_string()));
        self.add_field("start_column", |p| p.append(&start.column.to_string()));
        self.add_field("end_line", |p| p.append(&end.row.to_string()));
        self.add_field("end_column", |p| p.append(&end.column.to_string()));
        self.add_field("position", |p| p.append(&start.byte.to_string()));
        self.add_field("length", |p| p.append(&base.length().to_string()));
        if let Some(text) = base.text() {
            self.add_field("text", |p| p.append(&p.escape_json_string(text)));
        }
    }

    /// Adds metadata fields to the current map if metadata exists.
    fn add_metadata(&self, metadata: &Option<Arc<Metadata>>) {
        if let Some(meta) = metadata {
            self.add_field("metadata", |p| {
                p.start_map();
                p.add_field("version", |p| p.append(&meta.version.to_string()));
                p.end_map();
            });
        }
    }

    /// Escapes a string for JSON compatibility.
    fn escape_json_string(&self, s: &str) -> String {
        let mut escaped = String::new();
        escaped.push('"');
        for c in s.chars() {
            match c {
                '"' => escaped.push_str("\\\""),
                '\\' => escaped.push_str("\\\\"),
                '\n' => escaped.push_str("\\n"),
                '\r' => escaped.push_str("\\r"),
                '\t' => escaped.push_str("\\t"),
                _ if c.is_control() => escaped.push_str(&format!("\\u{:04x}", c as u32)),
                _ => escaped.push(c),
            }
        }
        escaped.push('"');
        escaped
    }

    /// Appends a string to the result, updating the current column position.
    fn append(&self, s: &str) {
        let mut result = self.result.borrow_mut();
        let mut current_column = self.current_column.borrow_mut();
        result.push_str(s);
        for c in s.chars() {
            if c == '\n' {
                *current_column = 0;
            } else {
                *current_column += 1;
            }
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
    /// * `key` - The field name.
    /// * `value` - A closure that appends the field value.
    fn add_field<F>(&self, key: &str, value: F) where F: FnOnce(&Self) {
        let is_first = *self.is_first_field.borrow();
        if self.pretty_print {
            {
                let alignment = *self.alignment_columns.borrow().last().unwrap_or(&0);
                let current_col = *self.current_column.borrow();
                if !is_first && current_col != alignment {
                    self.append("\n");
                    self.append(&" ".repeat(alignment));
                }
                self.append(&format!(":{key} "));
            }
            value(self);
            *self.is_first_field.borrow_mut() = false;
        } else {
            {
                if is_first {
                    self.append(&format!(":{key} "));
                } else {
                    self.append(&format!(",:{key} "));
                }
            }
            value(self);
            *self.is_first_field.borrow_mut() = false;
        }
    }

    /// Formats a vector of nodes as an array, with alignment if pretty-printing.
    fn visit_vector(&self, items: &Vector<Arc<Node<'_>>>) {
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

    /// Formats a vector of key-value pairs as an array of maps.
    fn format_pairs(&self, pairs: &Vector<(Arc<Node<'a>>, Arc<Node<'a>>)>, key_name: &str, value_name: &str) {
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
            self.add_field(key_name, |p| { p.visit_node(key); });
            self.add_field(value_name, |p| { p.visit_node(value); });
            self.end_map();
        }
        self.append("]");
    }
}

impl<'a> Visitor for PrettyPrinter<'a> {
    fn visit_par<'b>(&self, node: &Arc<Node<'b>>, _base: &NodeBase<'b>, left: &Arc<Node<'b>>, right: &Arc<Node<'b>>, metadata: &Option<Arc<Metadata>>) -> Arc<Node<'b>> {
        self.start_map();
        self.add_field("type", |p| p.append("\"par\""));
        self.add_base_fields(node);
        self.add_field("left", |p| { p.visit_node(left); });
        self.add_field("right", |p| { p.visit_node(right); });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_sendsync<'b>(&self, node: &Arc<Node<'b>>, _base: &NodeBase<'b>, channel: &Arc<Node<'b>>, inputs: &Vector<Arc<Node<'b>>>, cont: &Arc<Node<'b>>, metadata: &Option<Arc<Metadata>>) -> Arc<Node<'b>> {
        self.start_map();
        self.add_field("type", |p| p.append("\"sendsync\""));
        self.add_base_fields(node);
        self.add_field("channel", |p| { p.visit_node(channel); });
        self.add_field("inputs", |p| p.visit_vector(inputs));
        self.add_field("cont", |p| { p.visit_node(cont); });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_send<'b>(&self, node: &Arc<Node<'b>>, _base: &NodeBase<'b>, channel: &Arc<Node<'b>>, send_type: &SendType, _send_type_end: &Position, inputs: &Vector<Arc<Node<'b>>>, metadata: &Option<Arc<Metadata>>) -> Arc<Node<'b>> {
        self.start_map();
        self.add_field("type", |p| p.append("\"send\""));
        self.add_base_fields(node);
        self.add_field("channel", |p| { p.visit_node(channel); });
        self.add_field("send_type", |p| p.append(&format!("\"{:?}\"", send_type)));
        self.add_field("inputs", |p| p.visit_vector(inputs));
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_new<'b>(&self, node: &Arc<Node<'b>>, _base: &NodeBase<'b>, decls: &Vector<Arc<Node<'b>>>, proc: &Arc<Node<'b>>, metadata: &Option<Arc<Metadata>>) -> Arc<Node<'b>> {
        self.start_map();
        self.add_field("type", |p| p.append("\"new\""));
        self.add_base_fields(node);
        self.add_field("decls", |p| p.visit_vector(decls));
        self.add_field("proc", |p| { p.visit_node(proc); });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_ifelse<'b>(&self, node: &Arc<Node<'b>>, _base: &NodeBase<'b>, condition: &Arc<Node<'b>>, consequence: &Arc<Node<'b>>, alternative: &Option<Arc<Node<'b>>>, metadata: &Option<Arc<Metadata>>) -> Arc<Node<'b>> {
        self.start_map();
        self.add_field("type", |p| p.append("\"ifelse\""));
        self.add_base_fields(node);
        self.add_field("condition", |p| { p.visit_node(condition); });
        self.add_field("consequence", |p| { p.visit_node(consequence); });
        if let Some(alt) = alternative {
            self.add_field("alternative", |p| { p.visit_node(alt); });
        }
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_let<'b>(&self, node: &Arc<Node<'b>>, _base: &NodeBase<'b>, decls: &Vector<Arc<Node<'b>>>, proc: &Arc<Node<'b>>, metadata: &Option<Arc<Metadata>>) -> Arc<Node<'b>> {
        self.start_map();
        self.add_field("type", |p| p.append("\"let\""));
        self.add_base_fields(node);
        self.add_field("decls", |p| p.visit_vector(decls));
        self.add_field("proc", |p| { p.visit_node(proc); });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_bundle<'b>(&self, node: &Arc<Node<'b>>, _base: &NodeBase<'b>, bundle_type: &BundleType, proc: &Arc<Node<'b>>, metadata: &Option<Arc<Metadata>>) -> Arc<Node<'b>> {
        self.start_map();
        self.add_field("type", |p| p.append("\"bundle\""));
        self.add_base_fields(node);
        self.add_field("bundle_type", |p| p.append(&format!("\"{:?}\"", bundle_type)));
        self.add_field("proc", |p| { p.visit_node(proc); });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_match<'b>(&self, node: &Arc<Node<'b>>, _base: &NodeBase<'b>, expression: &Arc<Node<'b>>, cases: &Vector<(Arc<Node<'b>>, Arc<Node<'b>>)>, metadata: &Option<Arc<Metadata>>) -> Arc<Node<'b>> {
        self.start_map();
        self.add_field("type", |p| p.append("\"match\""));
        self.add_base_fields(node);
        self.add_field("expression", |p| { p.visit_node(expression); });
        self.add_field("cases", |p| p.format_pairs(cases, "pattern", "proc"));
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_choice<'b>(&self, node: &Arc<Node<'b>>, _base: &NodeBase<'b>, branches: &Vector<(Vector<Arc<Node<'b>>>, Arc<Node<'b>>)>, metadata: &Option<Arc<Metadata>>) -> Arc<Node<'b>> {
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
                p.add_field("proc", |p| { p.visit_node(proc); });
                p.end_map();
            }
            p.append("]");
        });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_contract<'b>(&self, node: &Arc<Node<'b>>, _base: &NodeBase<'b>, name: &Arc<Node<'b>>, formals: &Vector<Arc<Node<'b>>>, proc: &Arc<Node<'b>>, metadata: &Option<Arc<Metadata>>) -> Arc<Node<'b>> {
        self.start_map();
        self.add_field("type", |p| p.append("\"contract\""));
        self.add_base_fields(node);
        self.add_field("name", |p| { p.visit_node(name); });
        self.add_field("formals", |p| p.visit_vector(formals));
        self.add_field("proc", |p| { p.visit_node(proc); });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_input<'b>(&self, node: &Arc<Node<'b>>, _base: &NodeBase<'b>, receipts: &Vector<Vector<Arc<Node<'b>>>>, proc: &Arc<Node<'b>>, metadata: &Option<Arc<Metadata>>) -> Arc<Node<'b>> {
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
        self.add_field("proc", |p| { p.visit_node(proc); });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_block<'b>(&self, node: &Arc<Node<'b>>, _base: &NodeBase<'b>, proc: &Arc<Node<'b>>, metadata: &Option<Arc<Metadata>>) -> Arc<Node<'b>> {
        self.start_map();
        self.add_field("type", |p| p.append("\"block\""));
        self.add_base_fields(node);
        self.add_field("proc", |p| { p.visit_node(proc); });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_binop<'b>(&self, node: &Arc<Node<'b>>, _base: &NodeBase<'b>, op: BinOperator, left: &Arc<Node<'b>>, right: &Arc<Node<'b>>, metadata: &Option<Arc<Metadata>>) -> Arc<Node<'b>> {
        self.start_map();
        self.add_field("type", |p| p.append("\"binop\""));
        self.add_base_fields(node);
        self.add_field("op", |p| p.append(&format!("\"{:?}\"", op)));
        self.add_field("left", |p| { p.visit_node(left); });
        self.add_field("right", |p| { p.visit_node(right); });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_unaryop<'b>(&self, node: &Arc<Node<'b>>, _base: &NodeBase<'b>, op: UnaryOperator, operand: &Arc<Node<'b>>, metadata: &Option<Arc<Metadata>>) -> Arc<Node<'b>> {
        self.start_map();
        self.add_field("type", |p| p.append("\"unaryop\""));
        self.add_base_fields(node);
        self.add_field("op", |p| p.append(&format!("\"{:?}\"", op)));
        self.add_field("operand", |p| { p.visit_node(operand); });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_method<'b>(&self, node: &Arc<Node<'b>>, _base: &NodeBase<'b>, receiver: &Arc<Node<'b>>, name: &String, args: &Vector<Arc<Node<'b>>>, metadata: &Option<Arc<Metadata>>) -> Arc<Node<'b>> {
        self.start_map();
        self.add_field("type", |p| p.append("\"method\""));
        self.add_base_fields(node);
        self.add_field("receiver", |p| { p.visit_node(receiver); });
        self.add_field("name", |p| p.append(&p.escape_json_string(name)));
        self.add_field("args", |p| p.visit_vector(args));
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_eval<'b>(&self, node: &Arc<Node<'b>>, _base: &NodeBase<'b>, name: &Arc<Node<'b>>, metadata: &Option<Arc<Metadata>>) -> Arc<Node<'b>> {
        self.start_map();
        self.add_field("type", |p| p.append("\"eval\""));
        self.add_base_fields(node);
        self.add_field("name", |p| { p.visit_node(name); });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_quote<'b>(&self, node: &Arc<Node<'b>>, _base: &NodeBase<'b>, quotable: &Arc<Node<'b>>, metadata: &Option<Arc<Metadata>>) -> Arc<Node<'b>> {
        self.start_map();
        self.add_field("type", |p| p.append("\"quote\""));
        self.add_base_fields(node);
        self.add_field("quotable", |p| { p.visit_node(quotable); });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_varref<'b>(&self, node: &Arc<Node<'b>>, _base: &NodeBase<'b>, kind: VarRefKind, var: &Arc<Node<'b>>, metadata: &Option<Arc<Metadata>>) -> Arc<Node<'b>> {
        self.start_map();
        self.add_field("type", |p| p.append("\"varref\""));
        self.add_base_fields(node);
        self.add_field("kind", |p| p.append(&format!("\"{:?}\"", kind)));
        self.add_field("var", |p| { p.visit_node(var); });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_bool_literal<'b>(&self, node: &Arc<Node<'b>>, _base: &NodeBase<'b>, _value: bool, metadata: &Option<Arc<Metadata>>) -> Arc<Node<'b>> {
        self.start_map();
        self.add_field("type", |p| p.append("\"bool\""));
        self.add_base_fields(node);
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_long_literal<'b>(&self, node: &Arc<Node<'b>>, _base: &NodeBase<'b>, _value: i64, metadata: &Option<Arc<Metadata>>) -> Arc<Node<'b>> {
        self.start_map();
        self.add_field("type", |p| p.append("\"long\""));
        self.add_base_fields(node);
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_string_literal<'b>(&self, node: &Arc<Node<'b>>, _base: &NodeBase<'b>, _value: &String, metadata: &Option<Arc<Metadata>>) -> Arc<Node<'b>> {
        self.start_map();
        self.add_field("type", |p| p.append("\"string\""));
        self.add_base_fields(node);
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_uri_literal<'b>(&self, node: &Arc<Node<'b>>, _base: &NodeBase<'b>, _value: &String, metadata: &Option<Arc<Metadata>>) -> Arc<Node<'b>> {
        self.start_map();
        self.add_field("type", |p| p.append("\"uri\""));
        self.add_base_fields(node);
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_nil<'b>(&self, node: &Arc<Node<'b>>, _base: &NodeBase<'b>, metadata: &Option<Arc<Metadata>>) -> Arc<Node<'b>> {
        self.start_map();
        self.add_field("type", |p| p.append("\"nil\""));
        self.add_base_fields(node);
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_list<'b>(&self, node: &Arc<Node<'b>>, _base: &NodeBase<'b>, elements: &Vector<Arc<Node<'b>>>, remainder: &Option<Arc<Node<'b>>>, metadata: &Option<Arc<Metadata>>) -> Arc<Node<'b>> {
        self.start_map();
        self.add_field("type", |p| p.append("\"list\""));
        self.add_base_fields(node);
        self.add_field("elements", |p| p.visit_vector(elements));
        if let Some(rem) = remainder {
            self.add_field("remainder", |p| { p.visit_node(rem); });
        }
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_set<'b>(&self, node: &Arc<Node<'b>>, _base: &NodeBase<'b>, elements: &Vector<Arc<Node<'b>>>, remainder: &Option<Arc<Node<'b>>>, metadata: &Option<Arc<Metadata>>) -> Arc<Node<'b>> {
        self.start_map();
        self.add_field("type", |p| p.append("\"set\""));
        self.add_base_fields(node);
        self.add_field("elements", |p| p.visit_vector(elements));
        if let Some(rem) = remainder {
            self.add_field("remainder", |p| { p.visit_node(rem); });
        }
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_map<'b>(&self, node: &Arc<Node<'b>>, _base: &NodeBase<'b>, pairs: &Vector<(Arc<Node<'b>>, Arc<Node<'b>>)>, remainder: &Option<Arc<Node<'b>>>, metadata: &Option<Arc<Metadata>>) -> Arc<Node<'b>> {
        self.start_map();
        self.add_field("type", |p| p.append("\"map\""));
        self.add_base_fields(node);
        self.add_field("pairs", |p| p.format_pairs(pairs, "key", "value"));
        if let Some(rem) = remainder {
            self.add_field("remainder", |p| { p.visit_node(rem); });
        }
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_tuple<'b>(&self, node: &Arc<Node<'b>>, _base: &NodeBase<'b>, elements: &Vector<Arc<Node<'b>>>, metadata: &Option<Arc<Metadata>>) -> Arc<Node<'b>> {
        self.start_map();
        self.add_field("type", |p| p.append("\"tuple\""));
        self.add_base_fields(node);
        self.add_field("elements", |p| p.visit_vector(elements));
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_var<'b>(&self, node: &Arc<Node<'b>>, _base: &NodeBase<'b>, name: &String, metadata: &Option<Arc<Metadata>>) -> Arc<Node<'b>> {
        self.start_map();
        self.add_field("type", |p| p.append("\"var\""));
        self.add_base_fields(node);
        self.add_field("name", |p| p.append(&p.escape_json_string(name)));
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_name_decl<'b>(&self, node: &Arc<Node<'b>>, _base: &NodeBase<'b>, var: &Arc<Node<'b>>, uri: &Option<Arc<Node<'b>>>, metadata: &Option<Arc<Metadata>>) -> Arc<Node<'b>> {
        self.start_map();
        self.add_field("type", |p| p.append("\"name_decl\""));
        self.add_base_fields(node);
        self.add_field("var", |p| { p.visit_node(var); });
        if let Some(u) = uri {
            self.add_field("uri", |p| { p.visit_node(u); });
        }
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_decl<'b>(&self, node: &Arc<Node<'b>>, _base: &NodeBase<'b>, names: &Vector<Arc<Node<'b>>>, procs: &Vector<Arc<Node<'b>>>, metadata: &Option<Arc<Metadata>>) -> Arc<Node<'b>> {
        self.start_map();
        self.add_field("type", |p| p.append("\"decl\""));
        self.add_base_fields(node);
        self.add_field("names", |p| p.visit_vector(names));
        self.add_field("procs", |p| p.visit_vector(procs));
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_linear_bind<'b>(&self, node: &Arc<Node<'b>>, _base: &NodeBase<'b>, names: &Vector<Arc<Node<'b>>>, source: &Arc<Node<'b>>, metadata: &Option<Arc<Metadata>>) -> Arc<Node<'b>> {
        self.start_map();
        self.add_field("type", |p| p.append("\"linear_bind\""));
        self.add_base_fields(node);
        self.add_field("names", |p| p.visit_vector(names));
        self.add_field("source", |p| { p.visit_node(source); });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_repeated_bind<'b>(&self, node: &Arc<Node<'b>>, _base: &NodeBase<'b>, names: &Vector<Arc<Node<'b>>>, source: &Arc<Node<'b>>, metadata: &Option<Arc<Metadata>>) -> Arc<Node<'b>> {
        self.start_map();
        self.add_field("type", |p| p.append("\"repeated_bind\""));
        self.add_base_fields(node);
        self.add_field("names", |p| p.visit_vector(names));
        self.add_field("source", |p| { p.visit_node(source); });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_peek_bind<'b>(&self, node: &Arc<Node<'b>>, _base: &NodeBase<'b>, names: &Vector<Arc<Node<'b>>>, source: &Arc<Node<'b>>, metadata: &Option<Arc<Metadata>>) -> Arc<Node<'b>> {
        self.start_map();
        self.add_field("type", |p| p.append("\"peek_bind\""));
        self.add_base_fields(node);
        self.add_field("names", |p| p.visit_vector(names));
        self.add_field("source", |p| { p.visit_node(source); });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_comment<'b>(&self, node: &Arc<Node<'b>>, _base: &NodeBase<'b>, kind: &CommentKind, metadata: &Option<Arc<Metadata>>) -> Arc<Node<'b>> {
        self.start_map();
        self.add_field("type", |p| p.append("\"comment\""));
        self.add_base_fields(node);
        self.add_field("kind", |p| p.append(&format!("\"{:?}\"", kind)));
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_wildcard<'b>(&self, node: &Arc<Node<'b>>, _base: &NodeBase<'b>, metadata: &Option<Arc<Metadata>>) -> Arc<Node<'b>> {
        self.start_map();
        self.add_field("type", |p| p.append("\"wildcard\""));
        self.add_base_fields(node);
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_simple_type<'b>(&self, node: &Arc<Node<'b>>, _base: &NodeBase<'b>, value: &String, metadata: &Option<Arc<Metadata>>) -> Arc<Node<'b>> {
        self.start_map();
        self.add_field("type", |p| p.append("\"simple_type\""));
        self.add_base_fields(node);
        self.add_field("value", |p| p.append(&p.escape_json_string(value)));
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_receive_send_source<'b>(&self, node: &Arc<Node<'b>>, _base: &NodeBase<'b>, name: &Arc<Node<'b>>, metadata: &Option<Arc<Metadata>>) -> Arc<Node<'b>> {
        self.start_map();
        self.add_field("type", |p| p.append("\"receive_send_source\""));
        self.add_base_fields(node);
        self.add_field("name", |p| { p.visit_node(name); });
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }

    fn visit_send_receive_source<'b>(&self, node: &Arc<Node<'b>>, _base: &NodeBase<'b>, name: &Arc<Node<'b>>, inputs: &Vector<Arc<Node<'b>>>, metadata: &Option<Arc<Metadata>>) -> Arc<Node<'b>> {
        self.start_map();
        self.add_field("type", |p| p.append("\"send_receive_source\""));
        self.add_base_fields(node);
        self.add_field("name", |p| { p.visit_node(name); });
        self.add_field("inputs", |p| p.visit_vector(inputs));
        self.add_metadata(metadata);
        self.end_map();
        Arc::clone(node)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::super::node::RelativePosition;
    use indoc::indoc;

    #[test]
    fn test_pretty_printer_aligned() {
        let _ = crate::logging::init_logger(false, Some("debug"));
        let rholang_code = r#"true|42"#;
        let tree = crate::tree_sitter::parse_code(rholang_code);
        let ir = crate::tree_sitter::parse_to_ir(&tree, rholang_code);
        let actual = format(&ir, true).expect("Failed to format tree");
        let expected = indoc! {r#"
            {:type "par"
             :start_line 0
             :start_column 0
             :end_line 0
             :end_column 7
             :position 0
             :length 7
             :text "true|42"
             :left {:type "bool"
                    :start_line 0
                    :start_column 0
                    :end_line 0
                    :end_column 4
                    :position 0
                    :length 4
                    :text "true"
                    :metadata {:version 0}}
             :right {:type "long"
                     :start_line 0
                     :start_column 5
                     :end_line 0
                     :end_column 7
                     :position 5
                     :length 2
                     :text "42"
                     :metadata {:version 0}}
             :metadata {:version 0}}"#}.trim();
        println!("{}", ir.text());
        println!("{}", actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_pretty_printer_unaligned() {
        let _ = crate::logging::init_logger(false, Some("debug"));
        let rholang_code = r#"true|42"#;
        let tree = crate::tree_sitter::parse_code(rholang_code);
        let ir = crate::tree_sitter::parse_to_ir(&tree, rholang_code);
        let actual = format(&ir, false).expect("Failed to format tree");
        let expected = r#"{:type "par",:start_line 0,:start_column 0,:end_line 0,:end_column 7,:position 0,:length 7,:text "true|42",:left {:type "bool",:start_line 0,:start_column 0,:end_line 0,:end_column 4,:position 0,:length 4,:text "true",:metadata {:version 0}},:right {:type "long",:start_line 0,:start_column 5,:end_line 0,:end_column 7,:position 5,:length 2,:text "42",:metadata {:version 0}},:metadata {:version 0}}"#;
        println!("{}", ir.text());
        println!("{}", actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_pretty_print_send() {
        let _ = crate::logging::init_logger(false, Some("debug"));
        let rholang_code = r#"ch!("msg")"#;
        let tree = crate::tree_sitter::parse_code(rholang_code);
        let ir = crate::tree_sitter::parse_to_ir(&tree, rholang_code);
        let actual = format(&ir, true).expect("Failed to format tree");
        let expected = indoc! {r#"
            {:type "send"
             :start_line 0
             :start_column 0
             :end_line 0
             :end_column 10
             :position 0
             :length 10
             :text "ch!(\"msg\")"
             :channel {:type "var"
                       :start_line 0
                       :start_column 0
                       :end_line 0
                       :end_column 2
                       :position 0
                       :length 2
                       :text "ch"
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
                       :text "\"msg\""
                       :metadata {:version 0}}]
             :metadata {:version 0}}"#}.trim();
        println!("{}", ir.text());
        println!("{}", actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_pretty_print_special_chars() {
        let _ = crate::logging::init_logger(false, Some("debug"));
        let rholang_code = r#"ch!("Hello\nWorld")"#;
        let tree = crate::tree_sitter::parse_code(rholang_code);
        let ir = crate::tree_sitter::parse_to_ir(&tree, rholang_code);
        let actual = format(&ir, true).expect("Failed to format tree");
        let expected = indoc! {r#"
            {:type "send"
             :start_line 0
             :start_column 0
             :end_line 0
             :end_column 19
             :position 0
             :length 19
             :text "ch!(\"Hello\\nWorld\")"
             :channel {:type "var"
                       :start_line 0
                       :start_column 0
                       :end_line 0
                       :end_column 2
                       :position 0
                       :length 2
                       :text "ch"
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
                       :text "\"Hello\\nWorld\""
                       :metadata {:version 0}}]
             :metadata {:version 0}}"#}.trim();
        println!("{}", ir.text());
        println!("{}", actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_pretty_print_decl() {
        let _ = crate::logging::init_logger(false, Some("debug"));
        let rholang_code = r#"let x = "hello" in { Nil }"#;
        let tree = crate::tree_sitter::parse_code(rholang_code);
        let ir = crate::tree_sitter::parse_to_ir(&tree, rholang_code);
        let positions = compute_absolute_positions(&ir);
        let printer = PrettyPrinter::new(true, &positions);
        printer.visit_node(&ir);
        let actual = printer.get_result();
        let expected = indoc! {r#"
            {:type "let"
             :start_line 0
             :start_column 0
             :end_line 0
             :end_column 26
             :position 0
             :length 26
             :text "let x = \"hello\" in { Nil }"
             :decls [{:type "decl"
                      :start_line 0
                      :start_column 4
                      :end_line 0
                      :end_column 15
                      :position 4
                      :length 11
                      :text "x = \"hello\""
                      :names [{:type "var"
                               :start_line 0
                               :start_column 4
                               :end_line 0
                               :end_column 5
                               :position 4
                               :length 1
                               :text "x"
                               :name "x"
                               :metadata {:version 0}}]
                      :procs [{:type "string"
                               :start_line 0
                               :start_column 8
                               :end_line 0
                               :end_column 15
                               :position 8
                               :length 7
                               :text "\"hello\""
                               :metadata {:version 0}}]
                      :metadata {:version 0}}]
             :proc {:type "block"
                    :start_line 0
                    :start_column 19
                    :end_line 0
                    :end_column 26
                    :position 19
                    :length 7
                    :text "{ Nil }"
                    :proc {:type "nil"
                           :start_line 0
                           :start_column 21
                           :end_line 0
                           :end_column 24
                           :position 21
                           :length 3
                           :text "Nil"
                           :metadata {:version 0}}
                    :metadata {:version 0}}
             :metadata {:version 0}}"#}.trim();
        println!("{}", ir.text());
        println!("{}", actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_pretty_print_new() {
        let _ = crate::logging::init_logger(false, Some("debug"));
        // Tests a New node with a declaration and a send operation
        let rholang_code = r#"new x in { x!("hello") }"#;
        let tree = crate::tree_sitter::parse_code(rholang_code);
        let ir = crate::tree_sitter::parse_to_ir(&tree, rholang_code);
        let actual = format(&ir, true).expect("Failed to format tree");
        let expected = indoc! {r#"
            {:type "new"
             :start_line 0
             :start_column 0
             :end_line 0
             :end_column 24
             :position 0
             :length 24
             :text "new x in { x!(\"hello\") }"
             :decls [{:type "name_decl"
                      :start_line 0
                      :start_column 4
                      :end_line 0
                      :end_column 5
                      :position 4
                      :length 1
                      :text "x"
                      :var {:type "var"
                            :start_line 0
                            :start_column 4
                            :end_line 0
                            :end_column 5
                            :position 4
                            :length 1
                            :text "x"
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
                    :text "{ x!(\"hello\") }"
                    :proc {:type "send"
                           :start_line 0
                           :start_column 11
                           :end_line 0
                           :end_column 22
                           :position 11
                           :length 11
                           :text "x!(\"hello\")"
                           :channel {:type "var"
                                     :start_line 0
                                     :start_column 11
                                     :end_line 0
                                     :end_column 12
                                     :position 11
                                     :length 1
                                     :text "x"
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
                                     :text "\"hello\""
                                     :metadata {:version 0}}]
                           :metadata {:version 0}}
                    :metadata {:version 0}}
             :metadata {:version 0}}"#}.trim();
        println!("{}", ir.text());
        println!("{}", actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_pretty_print_ifelse() {
        let _ = crate::logging::init_logger(false, Some("debug"));
        // Tests an IfElse node with condition, consequence, and alternative
        let rholang_code = r#"if (true) { Nil } else { Nil }"#;
        let tree = crate::tree_sitter::parse_code(rholang_code);
        let ir = crate::tree_sitter::parse_to_ir(&tree, rholang_code);
        let actual = format(&ir, true).expect("Failed to format tree");
        let expected = indoc! {r#"
            {:type "ifelse"
             :start_line 0
             :start_column 0
             :end_line 0
             :end_column 30
             :position 0
             :length 30
             :text "if (true) { Nil } else { Nil }"
             :condition {:type "bool"
                         :start_line 0
                         :start_column 4
                         :end_line 0
                         :end_column 8
                         :position 4
                         :length 4
                         :text "true"
                         :metadata {:version 0}}
             :consequence {:type "block"
                           :start_line 0
                           :start_column 10
                           :end_line 0
                           :end_column 17
                           :position 10
                           :length 7
                           :text "{ Nil }"
                           :proc {:type "nil"
                                  :start_line 0
                                  :start_column 12
                                  :end_line 0
                                  :end_column 15
                                  :position 12
                                  :length 3
                                  :text "Nil"
                                  :metadata {:version 0}}
                           :metadata {:version 0}}
             :alternative {:type "block"
                           :start_line 0
                           :start_column 23
                           :end_line 0
                           :end_column 30
                           :position 23
                           :length 7
                           :text "{ Nil }"
                           :proc {:type "nil"
                                  :start_line 0
                                  :start_column 25
                                  :end_line 0
                                  :end_column 28
                                  :position 25
                                  :length 3
                                  :text "Nil"
                                  :metadata {:version 0}}
                           :metadata {:version 0}}
             :metadata {:version 0}}"#}.trim();
        println!("{}", ir.text());
        println!("{}", actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_pretty_print_match() {
        let _ = crate::logging::init_logger(false, Some("debug"));
        // Tests a Match node with an expression and a single case
        let rholang_code = r#"match "hello" { "hello" => Nil }"#;
        let tree = crate::tree_sitter::parse_code(rholang_code);
        let ir = crate::tree_sitter::parse_to_ir(&tree, rholang_code);
        let actual = format(&ir, true).expect("Failed to format tree");
        let expected = indoc! {r#"
            {:type "match"
             :start_line 0
             :start_column 0
             :end_line 0
             :end_column 32
             :position 0
             :length 32
             :text "match \"hello\" { \"hello\" => Nil }"
             :expression {:type "string"
                          :start_line 0
                          :start_column 6
                          :end_line 0
                          :end_column 13
                          :position 6
                          :length 7
                          :text "\"hello\""
                          :metadata {:version 0}}
             :cases [{:pattern {:type "string"
                                :start_line 0
                                :start_column 16
                                :end_line 0
                                :end_column 23
                                :position 16
                                :length 7
                                :text "\"hello\""
                                :metadata {:version 0}}
                      :proc {:type "nil"
                             :start_line 0
                             :start_column 27
                             :end_line 0
                             :end_column 30
                             :position 27
                             :length 3
                             :text "Nil"
                             :metadata {:version 0}}}]
             :metadata {:version 0}}"#}.trim();
        println!("{}", ir.text());
        println!("{}", actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_pretty_print_contract() {
        let _ = crate::logging::init_logger(false, Some("debug"));
        // Tests a Contract node with a name, parameter, and body
        let rholang_code = r#"contract myContract(param) = { Nil }"#;
        let tree = crate::tree_sitter::parse_code(rholang_code);
        let ir = crate::tree_sitter::parse_to_ir(&tree, rholang_code);
        let actual = format(&ir, true).expect("Failed to format tree");
        let expected = indoc! {r#"
            {:type "contract"
             :start_line 0
             :start_column 0
             :end_line 0
             :end_column 36
             :position 0
             :length 36
             :text "contract myContract(param) = { Nil }"
             :name {:type "var"
                    :start_line 0
                    :start_column 9
                    :end_line 0
                    :end_column 19
                    :position 9
                    :length 10
                    :text "myContract"
                    :name "myContract"
                    :metadata {:version 0}}
             :formals [{:type "var"
                        :start_line 0
                        :start_column 20
                        :end_line 0
                        :end_column 25
                        :position 20
                        :length 5
                        :text "param"
                        :name "param"
                        :metadata {:version 0}}]
             :proc {:type "block"
                    :start_line 0
                    :start_column 29
                    :end_line 0
                    :end_column 36
                    :position 29
                    :length 7
                    :text "{ Nil }"
                    :proc {:type "nil"
                           :start_line 0
                           :start_column 31
                           :end_line 0
                           :end_column 34
                           :position 31
                           :length 3
                           :text "Nil"
                           :metadata {:version 0}}
                    :metadata {:version 0}}
             :metadata {:version 0}}"#}.trim();
        println!("{}", ir.text());
        println!("{}", actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_pretty_print_input() {
        let _ = crate::logging::init_logger(false, Some("debug"));
        // Tests an Input node with a linear binding
        let rholang_code = r#"for (x <- ch) { Nil }"#;
        let tree = crate::tree_sitter::parse_code(rholang_code);
        let ir = crate::tree_sitter::parse_to_ir(&tree, rholang_code);
        let actual = format(&ir, true).expect("Failed to format tree");
        let expected = indoc! {r#"
            {:type "input"
             :start_line 0
             :start_column 0
             :end_line 0
             :end_column 21
             :position 0
             :length 21
             :text "for (x <- ch) { Nil }"
             :receipts [[{:type "linear_bind"
                          :start_line 0
                          :start_column 5
                          :end_line 0
                          :end_column 12
                          :position 5
                          :length 7
                          :text "x <- ch"
                          :names [{:type "var"
                                   :start_line 0
                                   :start_column 5
                                   :end_line 0
                                   :end_column 6
                                   :position 5
                                   :length 1
                                   :text "x"
                                   :name "x"
                                   :metadata {:version 0}}]
                          :source {:type "var"
                                   :start_line 0
                                   :start_column 10
                                   :end_line 0
                                   :end_column 12
                                   :position 10
                                   :length 2
                                   :text "ch"
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
                    :text "{ Nil }"
                    :proc {:type "nil"
                           :start_line 0
                           :start_column 16
                           :end_line 0
                           :end_column 19
                           :position 16
                           :length 3
                           :text "Nil"
                           :metadata {:version 0}}
                    :metadata {:version 0}}
             :metadata {:version 0}}"#}.trim();
        println!("{}", ir.text());
        println!("{}", actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_pretty_print_binop() {
        let _ = crate::logging::init_logger(false, Some("debug"));
        // Tests a BinOp node for a simple addition
        let rholang_code = r#"1 + 2"#;
        let tree = crate::tree_sitter::parse_code(rholang_code);
        let ir = crate::tree_sitter::parse_to_ir(&tree, rholang_code);
        let actual = format(&ir, true).expect("Failed to format tree");
        let expected = indoc! {r#"
            {:type "binop"
             :start_line 0
             :start_column 0
             :end_line 0
             :end_column 5
             :position 0
             :length 5
             :text "1 + 2"
             :op "Add"
             :left {:type "long"
                    :start_line 0
                    :start_column 0
                    :end_line 0
                    :end_column 1
                    :position 0
                    :length 1
                    :text "1"
                    :metadata {:version 0}}
             :right {:type "long"
                     :start_line 0
                     :start_column 4
                     :end_line 0
                     :end_column 5
                     :position 4
                     :length 1
                     :text "2"
                     :metadata {:version 0}}
             :metadata {:version 0}}"#}.trim();
        println!("{}", ir.text());
        println!("{}", actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_pretty_print_list() {
        let _ = crate::logging::init_logger(false, Some("debug"));
        // Tests a List node with multiple elements
        let rholang_code = r#"[1, 2, 3]"#;
        let tree = crate::tree_sitter::parse_code(rholang_code);
        let ir = crate::tree_sitter::parse_to_ir(&tree, rholang_code);
        let actual = format(&ir, true).expect("Failed to format tree");
        let expected = indoc! {r#"
            {:type "list"
             :start_line 0
             :start_column 0
             :end_line 0
             :end_column 9
             :position 0
             :length 9
             :text "[1, 2, 3]"
             :elements [{:type "long"
                         :start_line 0
                         :start_column 1
                         :end_line 0
                         :end_column 2
                         :position 1
                         :length 1
                         :text "1"
                         :metadata {:version 0}}
                        {:type "long"
                         :start_line 0
                         :start_column 4
                         :end_line 0
                         :end_column 5
                         :position 4
                         :length 1
                         :text "2"
                         :metadata {:version 0}}
                        {:type "long"
                         :start_line 0
                         :start_column 7
                         :end_line 0
                         :end_column 8
                         :position 7
                         :length 1
                         :text "3"
                         :metadata {:version 0}}]
             :metadata {:version 0}}"#}.trim();
        println!("{}", ir.text());
        println!("{}", actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_pretty_print_comment() {
        let _ = crate::logging::init_logger(false, Some("debug"));
        // Tests a Comment node combined with a Nil process in a Par node
        let rholang_code = r#"// This is a comment
Nil"#;
        let tree = crate::tree_sitter::parse_code(rholang_code);
        let ir = crate::tree_sitter::parse_to_ir(&tree, rholang_code);
        let actual = format(&ir, true).expect("Failed to format tree");
        let expected = indoc! {r#"
            {:type "par"
             :start_line 0
             :start_column 0
             :end_line 1
             :end_column 3
             :position 0
             :length 24
             :text "// This is a comment\nNil"
             :left {:type "comment"
                    :start_line 0
                    :start_column 0
                    :end_line 0
                    :end_column 20
                    :position 0
                    :length 20
                    :text "// This is a comment"
                    :kind "Line"
                    :metadata {:version 0}}
             :right {:type "nil"
                     :start_line 1
                     :start_column 0
                     :end_line 1
                     :end_column 3
                     :position 21
                     :length 3
                     :text "Nil"
                     :metadata {:version 0}}
             :metadata {:version 0}}"#}.trim();
        println!("{}", ir.text());
        println!("{}", actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_pretty_print_match_fixed() {
        let _ = crate::logging::init_logger(false, Some("debug"));
        let code = r#"match "target" { "pat" => Nil }"#;
        let tree = crate::tree_sitter::parse_code(code);
        let ir = crate::tree_sitter::parse_to_ir(&tree, code);
        let actual = format(&ir, true).expect("Failed to format");
        let expected = indoc! {r#"
            {:type "match"
             :start_line 0
             :start_column 0
             :end_line 0
             :end_column 31
             :position 0
             :length 31
             :text "match \"target\" { \"pat\" => Nil }"
             :expression {:type "string"
                          :start_line 0
                          :start_column 6
                          :end_line 0
                          :end_column 14
                          :position 6
                          :length 8
                          :text "\"target\""
                          :metadata {:version 0}}
             :cases [{:pattern {:type "string"
                                :start_line 0
                                :start_column 17
                                :end_line 0
                                :end_column 22
                                :position 17
                                :length 5
                                :text "\"pat\""
                                :metadata {:version 0}}
                      :proc {:type "nil"
                             :start_line 0
                             :start_column 26
                             :end_line 0
                             :end_column 29
                             :position 26
                             :length 3
                             :text "Nil"
                             :metadata {:version 0}}}]
             :metadata {:version 0}}"#}.trim();
        println!("{}", ir.text());
        println!("{}", actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_pretty_print_input_fixed() {
        let _ = crate::logging::init_logger(false, Some("debug"));
        let code = r#"for (x <- ch) { Nil }"#;
        let tree = crate::tree_sitter::parse_code(code);
        let ir = crate::tree_sitter::parse_to_ir(&tree, code);
        let actual = format(&ir, true).expect("Failed to format");
        let expected = indoc! {r#"
            {:type "input"
             :start_line 0
             :start_column 0
             :end_line 0
             :end_column 21
             :position 0
             :length 21
             :text "for (x <- ch) { Nil }"
             :receipts [[{:type "linear_bind"
                          :start_line 0
                          :start_column 5
                          :end_line 0
                          :end_column 12
                          :position 5
                          :length 7
                          :text "x <- ch"
                          :names [{:type "var"
                                   :start_line 0
                                   :start_column 5
                                   :end_line 0
                                   :end_column 6
                                   :position 5
                                   :length 1
                                   :text "x"
                                   :name "x"
                                   :metadata {:version 0}}]
                          :source {:type "var"
                                   :start_line 0
                                   :start_column 10
                                   :end_line 0
                                   :end_column 12
                                   :position 10
                                   :length 2
                                   :text "ch"
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
                    :text "{ Nil }"
                    :proc {:type "nil"
                           :start_line 0
                           :start_column 16
                           :end_line 0
                           :end_column 19
                           :position 16
                           :length 3
                           :text "Nil"
                           :metadata {:version 0}}
                    :metadata {:version 0}}
             :metadata {:version 0}}"#}.trim();
        println!("{}", ir.text());
        println!("{}", actual);
        assert_eq!(actual, expected);
    }
}
