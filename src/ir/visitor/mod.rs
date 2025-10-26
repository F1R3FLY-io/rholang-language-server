//! Visitor pattern for Rholang IR traversal and transformation
//!
//! The Visitor trait provides methods for visiting each RholangNode variant,
//! enabling tree transformations while preserving structural sharing via Arc.
//!
//! # Architecture
//!
//! The visitor module is organized for future refinement:
//! - `visitor_trait`: Main Visitor trait with all 42 visit methods
//! - `literals`: (Future) Literal-specific visitor trait
//!
//! # Usage
//!
//! ```ignore
//! use rholang_language_server::ir::visitor::Visitor;
//! use std::sync::Arc;
//!
//! struct MyVisitor;
//!
//! impl Visitor for MyVisitor {
//!     // Override specific methods to transform nodes
//!     fn visit_bool_literal(
//!         &self,
//!         node: &Arc<RholangNode>,
//!         base: &NodeBase,
//!         value: bool,
//!         metadata: &Option<Arc<Metadata>>,
//!     ) -> Arc<RholangNode> {
//!         // Custom transformation logic
//!         Arc::clone(node)
//!     }
//! }
//! ```
//!
//! # Pattern
//!
//! Each visitor method:
//! 1. Visits all child nodes recursively via `visit_node()`
//! 2. Checks if any children changed using `Arc::ptr_eq()`
//! 3. Returns original node if unchanged, new node if changed
//!
//! This pattern enables efficient structural sharing.

mod visitor_trait;

// Experimental submodules for future trait composition
// These are not yet integrated but show the planned structure
pub mod literals;

// Re-export the main Visitor trait
pub use visitor_trait::Visitor;
