//! Position-indexed AST for O(log n) node lookups
//!
//! This module provides a position-based index over the AST to enable fast
//! node lookups by position. Instead of traversing the entire tree (O(n)),
//! we can use a BTreeMap to find nodes in O(log n) time.
//!
//! # Performance
//!
//! - **Current (linear)**: O(n) where n = total nodes in AST
//!   - 50 nodes: ~8.7 µs
//!   - 200 nodes: ~34 µs
//!
//! - **Target (indexed)**: O(log n) + O(k) where k = nodes at position
//!   - Expected: <5 µs for typical files
//!   - 60-70% improvement for large files
//!
//! # Usage
//!
//! ```ignore
//! let index = PositionIndex::build(&ir);
//! let node = index.find_at_position(&position);
//! ```

use crate::ir::rholang_node::{Position, RholangNode};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Position-indexed AST for fast node lookups
///
/// Maps positions to nodes, allowing O(log n) lookup by position.
/// Multiple nodes can exist at the same position (e.g., parent and child),
/// so we store a Vec of nodes per position.
#[derive(Debug, Clone)]
pub struct PositionIndex {
    /// Ordered map: Position -> Nodes at that position
    /// BTreeMap provides O(log n) lookups and maintains position ordering
    index: BTreeMap<Position, Vec<Arc<RholangNode>>>,

    /// Total number of nodes indexed
    node_count: usize,
}

impl PositionIndex {
    /// Create a new empty position index
    pub fn new() -> Self {
        Self {
            index: BTreeMap::new(),
            node_count: 0,
        }
    }

    /// Build a position index from an IR tree
    ///
    /// # Arguments
    ///
    /// * `root` - The root node of the AST to index
    ///
    /// # Returns
    ///
    /// A position index containing all nodes in the tree
    ///
    /// # Performance
    ///
    /// - Time: O(n) where n = number of nodes (one-time cost)
    /// - Space: O(n) for storing node references
    pub fn build(root: &Arc<RholangNode>) -> Self {
        let mut index = Self::new();
        index.index_node(root);
        index
    }

    /// Recursively index a node and all its children
    fn index_node(&mut self, node: &Arc<RholangNode>) {
        let position = node.base().start();

        // Add this node to the index
        self.index
            .entry(position)
            .or_insert_with(Vec::new)
            .push(node.clone());

        self.node_count += 1;

        // Recursively index children
        self.index_children(node);
    }

    /// Index all children of a node based on its variant
    fn index_children(&mut self, node: &Arc<RholangNode>) {
        match node.as_ref() {
            // Parallel composition
            RholangNode::Par { left, right, processes, .. } => {
                // Handle legacy binary form
                if let Some(l) = left {
                    self.index_node(l);
                }
                if let Some(r) = right {
                    self.index_node(r);
                }
                // Handle n-ary form
                if let Some(procs) = processes {
                    for child in procs.iter() {
                        self.index_node(child);
                    }
                }
            }

            // Send operations
            RholangNode::Send { channel, inputs, .. } => {
                self.index_node(channel);
                for input in inputs.iter() {
                    self.index_node(input);
                }
            }
            RholangNode::SendSync { channel, inputs, cont, .. } => {
                self.index_node(channel);
                for input in inputs.iter() {
                    self.index_node(input);
                }
                self.index_node(cont);
            }

            // Name declaration and scoping
            RholangNode::New { decls, proc, .. } => {
                for decl in decls.iter() {
                    self.index_node(decl);
                }
                self.index_node(proc);
            }
            RholangNode::Let { decls, proc, .. } => {
                for decl in decls.iter() {
                    self.index_node(decl);
                }
                self.index_node(proc);
            }

            // Control flow
            RholangNode::IfElse { condition, consequence, alternative, .. } => {
                self.index_node(condition);
                self.index_node(consequence);
                if let Some(alt) = alternative {
                    self.index_node(alt);
                }
            }
            RholangNode::Match { expression, cases, .. } => {
                self.index_node(expression);
                for (pattern, body) in cases.iter() {
                    self.index_node(pattern);
                    self.index_node(body);
                }
            }
            RholangNode::Choice { branches, .. } => {
                for (patterns, body) in branches.iter() {
                    for pattern in patterns.iter() {
                        self.index_node(pattern);
                    }
                    self.index_node(body);
                }
            }

            // Contract and input
            RholangNode::Contract { name, formals, formals_remainder, proc, .. } => {
                self.index_node(name);
                for formal in formals.iter() {
                    self.index_node(formal);
                }
                if let Some(remainder) = formals_remainder {
                    self.index_node(remainder);
                }
                self.index_node(proc);
            }
            RholangNode::Input { receipts, proc, .. } => {
                for receipt in receipts.iter() {
                    for binding in receipt.iter() {
                        self.index_node(binding);
                    }
                }
                self.index_node(proc);
            }

            // Structural nodes
            RholangNode::Block { proc, .. } => {
                self.index_node(proc);
            }
            RholangNode::Parenthesized { expr, .. } => {
                self.index_node(expr);
            }
            RholangNode::Bundle { proc, .. } => {
                self.index_node(proc);
            }

            // Binary and unary operations
            RholangNode::BinOp { left, right, .. } => {
                self.index_node(left);
                self.index_node(right);
            }
            RholangNode::UnaryOp { operand, .. } => {
                self.index_node(operand);
            }

            // Method calls
            RholangNode::Method { receiver, args, .. } => {
                self.index_node(receiver);
                for arg in args.iter() {
                    self.index_node(arg);
                }
            }

            // Name operations
            RholangNode::Eval { name, .. } => {
                self.index_node(name);
            }
            RholangNode::Quote { quotable, .. } => {
                self.index_node(quotable);
            }
            RholangNode::VarRef { var, .. } => {
                self.index_node(var);
            }

            // Collections
            RholangNode::List { elements, remainder, .. } => {
                for elem in elements.iter() {
                    self.index_node(elem);
                }
                if let Some(rem) = remainder {
                    self.index_node(rem);
                }
            }
            RholangNode::Set { elements, remainder, .. } => {
                for elem in elements.iter() {
                    self.index_node(elem);
                }
                if let Some(rem) = remainder {
                    self.index_node(rem);
                }
            }
            RholangNode::Map { pairs, remainder, .. } => {
                for (key, value) in pairs.iter() {
                    self.index_node(key);
                    self.index_node(value);
                }
                if let Some(rem) = remainder {
                    self.index_node(rem);
                }
            }
            RholangNode::Pathmap { elements, remainder, .. } => {
                for elem in elements.iter() {
                    self.index_node(elem);
                }
                if let Some(rem) = remainder {
                    self.index_node(rem);
                }
            }
            RholangNode::Tuple { elements, .. } => {
                for elem in elements.iter() {
                    self.index_node(elem);
                }
            }

            // Declaration nodes
            RholangNode::NameDecl { var, uri, .. } => {
                self.index_node(var);
                if let Some(u) = uri {
                    self.index_node(u);
                }
            }
            RholangNode::Decl { names, names_remainder, procs, .. } => {
                for name in names.iter() {
                    self.index_node(name);
                }
                if let Some(rem) = names_remainder {
                    self.index_node(rem);
                }
                for proc in procs.iter() {
                    self.index_node(proc);
                }
            }

            // Binding nodes
            RholangNode::LinearBind { names, remainder, source, .. }
            | RholangNode::RepeatedBind { names, remainder, source, .. }
            | RholangNode::PeekBind { names, remainder, source, .. } => {
                for name in names.iter() {
                    self.index_node(name);
                }
                if let Some(rem) = remainder {
                    self.index_node(rem);
                }
                self.index_node(source);
            }

            // Pattern operations
            RholangNode::Disjunction { left, right, .. }
            | RholangNode::Conjunction { left, right, .. } => {
                self.index_node(left);
                self.index_node(right);
            }
            RholangNode::Negation { operand, .. } => {
                self.index_node(operand);
            }

            // Source nodes
            RholangNode::ReceiveSendSource { name, .. } => {
                self.index_node(name);
            }
            RholangNode::SendReceiveSource { name, inputs, .. } => {
                self.index_node(name);
                for input in inputs.iter() {
                    self.index_node(input);
                }
            }

            // Error nodes
            RholangNode::Error { children, .. } => {
                for child in children.iter() {
                    self.index_node(child);
                }
            }

            // Leaf nodes - no children to index
            RholangNode::Var { .. }
            | RholangNode::BoolLiteral { .. }
            | RholangNode::LongLiteral { .. }
            | RholangNode::StringLiteral { .. }
            | RholangNode::UriLiteral { .. }
            | RholangNode::Nil { .. }
            | RholangNode::Wildcard { .. }
            | RholangNode::SimpleType { .. }
            | RholangNode::Comment { .. }
            | RholangNode::Unit { .. } => {
                // No children
            }
        }
    }

    /// Find the most specific (deepest) node at a given position
    ///
    /// # Arguments
    ///
    /// * `position` - The position to search for
    ///
    /// # Returns
    ///
    /// The deepest node at the given position, or None if no node exists
    ///
    /// # Performance
    ///
    /// - Best case: O(log n) if exact position match with single node
    /// - Worst case: O(log n) + O(k) where k = nodes at position
    ///   - k is typically small (1-5 for deeply nested positions)
    pub fn find_at_position(&self, position: &Position) -> Option<Arc<RholangNode>> {
        // First, try exact position match
        if let Some(nodes) = self.index.get(position) {
            return Self::find_deepest_node(nodes, position);
        }

        // If no exact match, find the range of positions that could contain this position
        // We need to find nodes whose range includes this position
        self.find_containing_node(position)
    }

    /// Find the deepest (most specific) node from a list of candidates
    ///
    /// When multiple nodes share the same start position, the deepest one
    /// in the tree (smallest span) is most specific.
    fn find_deepest_node(nodes: &[Arc<RholangNode>], _position: &Position) -> Option<Arc<RholangNode>> {
        if nodes.is_empty() {
            return None;
        }

        // Find node with smallest span (most specific)
        // This is a heuristic - the smallest node is likely the most specific
        nodes.iter()
            .min_by_key(|node| {
                let base = node.base();
                let start = base.start();
                let end = base.end();

                // Calculate span size (rough heuristic)
                let row_span = end.row.saturating_sub(start.row);
                let col_span = if row_span == 0 {
                    end.column.saturating_sub(start.column)
                } else {
                    end.column // Multi-line, use end column as proxy
                };

                (row_span, col_span)
            })
            .cloned()
    }

    /// Find a node that contains the given position
    ///
    /// Uses BTreeMap's range queries to efficiently find candidate nodes.
    fn find_containing_node(&self, position: &Position) -> Option<Arc<RholangNode>> {
        // Find all nodes that start at or before this position
        let candidates: Vec<_> = self.index
            .range(..=position.clone())
            .rev() // Start from closest position
            .take(100) // Limit search to avoid performance issues
            .flat_map(|(_, nodes)| nodes.iter().cloned())
            .filter(|node| {
                let base = node.base();
                Self::position_in_range(position, &base.start(), &base.end())
            })
            .collect();

        if candidates.is_empty() {
            return None;
        }

        // Return the most specific (smallest span) candidate
        Self::find_deepest_node(&candidates, position)
    }

    /// Check if a position falls within a range [start, end]
    fn position_in_range(pos: &Position, start: &Position, end: &Position) -> bool {
        pos >= start && pos <= end
    }

    /// Get the total number of nodes indexed
    pub fn node_count(&self) -> usize {
        self.node_count
    }

    /// Get the number of unique positions indexed
    pub fn position_count(&self) -> usize {
        self.index.len()
    }
}

impl Default for PositionIndex {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::rholang_node::NodeBase;

    #[test]
    fn test_empty_index() {
        let index = PositionIndex::new();
        let pos = Position { row: 0, column: 0, byte: 0 };
        assert!(index.find_at_position(&pos).is_none());
        assert_eq!(index.node_count(), 0);
    }

    #[test]
    fn test_single_node() {
        let node = Arc::new(RholangNode::Var {
            base: NodeBase::new_simple(
                Position { row: 0, column: 0, byte: 0 },  // start
                3,  // length
                0,  // span_lines (single line)
                3,  // span_columns
            ),
            name: "foo".to_string(),
            metadata: None,
        });

        let index = PositionIndex::build(&node);
        assert_eq!(index.node_count(), 1);

        let found = index.find_at_position(&Position { row: 0, column: 0, byte: 0 });
        assert!(found.is_some());
    }

    #[test]
    fn test_position_ordering() {
        let pos1 = Position { row: 0, column: 0, byte: 0 };
        let pos2 = Position { row: 0, column: 5, byte: 5 };
        let pos3 = Position { row: 1, column: 0, byte: 10 };

        assert!(pos1 < pos2);
        assert!(pos2 < pos3);
    }
}
