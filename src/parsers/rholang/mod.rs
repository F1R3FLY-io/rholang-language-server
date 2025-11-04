//! Rholang parser - Tree-Sitter based parsing and IR conversion
//!
//! This module provides parsing functionality for Rholang code, converting
//! Tree-Sitter concrete syntax trees into our intermediate representation (IR).
//!
//! # Architecture
//!
//! - `parsing`: Public API for parsing Rholang code using Tree-Sitter
//! - `helpers`: Utility functions for node collection and processing
//! - `conversion`: CST to IR conversion logic
//!
//! # Usage
//!
//! ```ignore
//! use rholang_language_server::parsers::rholang::{parse_code, parse_to_ir};
//! use ropey::Rope;
//!
//! let code = r#"Nil | Nil"#;
//! let tree = parse_code(code);
//! let rope = Rope::from_str(code);
//! let ir = parse_to_ir(&tree, &rope);
//! ```

pub mod parsing;
pub mod helpers;
pub mod conversion;

// Re-export public API for backward compatibility
pub use parsing::{parse_code, parse_to_ir, parse_to_document_ir, update_tree};

// Note: helpers and conversion are internal implementation details
// and are not re-exported at the module level
