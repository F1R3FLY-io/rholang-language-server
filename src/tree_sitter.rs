//! Legacy re-export module for backward compatibility
//!
//! This module re-exports the Rholang parser functionality from its new location
//! at `crate::parsers::rholang`. This maintains backward compatibility for existing code.
//!
//! **Note**: New code should use `crate::parsers::rholang` directly.

pub use crate::parsers::rholang::{parse_code, parse_to_ir, update_tree};
