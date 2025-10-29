//! Tree-Sitter query system for language-agnostic LSP features
//!
//! This module provides a composable system for using Tree-Sitter queries (.scm files)
//! to implement LSP features without writing language-specific code. It bridges the gap
//! between Tree-Sitter's concrete syntax trees and our SemanticNode-based IR.
//!
//! # Architecture
//!
//! ```text
//! .scm Query Files
//!       ↓
//! QueryEngine (loads and executes queries)
//!       ↓
//! QueryCaptures (Tree-Sitter nodes + metadata)
//!       ↓
//! TreeSitterAdapter (converts to SemanticNode)
//!       ↓
//! Generic LSP Features (uses LanguageAdapter)
//! ```
//!
//! # Supported Query Types
//!
//! 1. **highlights.scm** - Syntax highlighting (semantic tokens)
//! 2. **folds.scm** - Code folding regions
//! 3. **indents.scm** - Indentation rules
//! 4. **injections.scm** - Embedded language detection
//! 5. **locals.scm** - Local scope and symbol tracking
//! 6. **textobjects.scm** - Text object navigation
//!
//! # Query-Driven LSP Features
//!
//! Many LSP features can be implemented purely from Tree-Sitter queries:
//!
//! ## From highlights.scm
//! - **Semantic Tokens**: Map highlight captures to LSP semantic token types
//! - **DocumentSymbols**: Extract symbols from function/class captures
//!
//! ## From folds.scm
//! - **Folding Ranges**: Convert @fold captures to LSP folding ranges
//!
//! ## From locals.scm
//! - **Goto Definition**: Follow @local.reference to @local.definition
//! - **References**: Find all @local.reference for a @local.definition
//! - **Rename**: Update all @local.reference and @local.definition
//! - **Document Highlight**: Highlight all references in current scope
//! - **Hover**: Show definition info from @local.definition metadata
//!
//! ## From indents.scm
//! - **Formatting**: Apply indentation rules to document
//! - **On-Type Formatting**: Auto-indent on newline
//!
//! ## From injections.scm
//! - **Virtual Documents**: Detect embedded languages (like MeTTa in Rholang)
//! - **Multi-Language Support**: Route LSP requests to appropriate language
//!
//! ## From textobjects.scm
//! - **Selection Range**: Expand selection to semantic boundaries
//! - **Document Symbols**: Extract function/class boundaries
//!
//! # Usage Example
//!
//! ```rust,ignore
//! use crate::lsp::features::tree_sitter::{QueryEngine, QueryType};
//!
//! // Load queries for a language
//! let engine = QueryEngine::new("rholang")?;
//! engine.load_query(QueryType::Highlights, include_str!("highlights.scm"))?;
//! engine.load_query(QueryType::Locals, include_str!("locals.scm"))?;
//!
//! // Execute query on a tree
//! let tree = parse_code(source);
//! let captures = engine.execute(&tree, QueryType::Locals)?;
//!
//! // Convert captures to semantic nodes
//! let adapter = TreeSitterAdapter::new(engine);
//! let semantic_nodes = adapter.captures_to_nodes(&captures)?;
//!
//! // Use with generic LSP features
//! let goto_def = GenericGotoDefinition;
//! let response = goto_def.goto_definition(
//!     &semantic_nodes,
//!     &position,
//!     &uri,
//!     &adapter,
//! ).await?;
//! ```
//!
//! # Design Principles
//!
//! 1. **Query-First**: Maximize use of .scm queries, minimize custom code
//! 2. **Language-Agnostic**: Same code works for Rholang, MeTTa, etc.
//! 3. **Composable**: Mix Tree-Sitter queries with manual IR construction
//! 4. **Incremental**: Can adopt query-driven features one at a time
//! 5. **Compatible**: Integrates with existing SemanticNode/LanguageAdapter architecture
//!
//! # Translation Strategy
//!
//! Tree-Sitter nodes → SemanticNode translation:
//!
//! - **NodeBase**: Computed from Tree-Sitter byte ranges + relative positions
//! - **Metadata**: Populated from query capture names (e.g., @function → type=Function)
//! - **Category**: Inferred from capture type (@local.definition → Binding, etc.)
//! - **Children**: Recursively converted from Tree-Sitter children
//!
//! This allows generic LSP features to work with both:
//! - Manually constructed IR (existing Rholang/MeTTa code)
//! - Query-derived IR (new languages or feature enhancements)

pub mod query_engine;
pub mod query_types;
pub mod adapter;
pub mod captures;

// Re-export main types
pub use query_engine::QueryEngine;
pub use query_types::{QueryType, QueryCapture, CaptureType};
pub use adapter::TreeSitterAdapter;
pub use captures::CaptureProcessor;
