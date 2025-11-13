pub mod comment;
pub mod document_ir;
pub mod formatter;
pub mod global_index;
pub mod metta_node;
pub mod metta_pattern_matching;
pub mod mork_canonical;
pub mod mork_convert;
pub mod pattern_matching;
pub mod pattern_matching_debug;
pub mod pipeline;
pub mod rholang_node;
pub mod rholang_pattern_index;
pub mod semantic_node;
pub mod space_pool;
pub mod structured_documentation;
pub mod symbol_resolution;
pub mod symbol_table;
pub mod transforms;
pub mod type_extraction;
pub mod unified_ir;
pub mod visitor;

// Re-export comment channel types for convenience
pub use comment::CommentNode;
pub use document_ir::DocumentIR;
pub use structured_documentation::StructuredDocumentation;
