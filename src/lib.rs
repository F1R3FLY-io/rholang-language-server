#![recursion_limit = "1024"]
pub mod ir;
pub mod language_regions;
pub mod logging;
pub mod lsp;
pub mod metrics;
pub mod parsers;
pub mod rnode_apis;
pub mod serde_helpers;  // Phase B-3: Custom serialization helpers for persistent cache
pub mod tree_sitter;
pub mod validators;
pub mod wire_logger;
pub mod wire_logger_middleware;
