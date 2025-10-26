use std::sync::Arc;
use std::collections::HashMap;

use tracing::debug;
use ropey::Rope;

use crate::ir::rholang_node::{RholangNode, Position, compute_absolute_positions};
use crate::ir::visitor::Visitor;

pub mod printer;
pub mod json_formatters;

pub use printer::PrettyPrinter;
pub use json_formatters::JsonStringFormatter;

/// Formats the Rholang IR tree into a JSON-like string representation.
/// Supports both compact and pretty-printed output with alignment and indentation.
///
/// # Arguments
/// * tree - The root node of the IR tree.
/// * pretty_print - If true, enables indentation and newlines for readability.
/// * rope - The Rope containing the source text for on-demand text extraction.
///
/// # Returns
/// A Result containing the formatted string or an error if validation fails.
pub fn format(tree: &Arc<RholangNode>, pretty_print: bool, _rope: &Rope) -> Result<String, String> {
    tree.validate()?;
    let positions = compute_absolute_positions(tree);
    let printer = PrettyPrinter::new(pretty_print, positions);
    printer.visit_node(tree);
    let result = printer.get_result();
    let (start, _) = printer.positions().get(&(&**tree as *const RholangNode as usize)).unwrap();
    debug!("Formatted IR at {}:{} (length={})", start.row, start.column, result.len());
    Ok(result)
}
