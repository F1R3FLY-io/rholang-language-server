//! Tree-Sitter parsing interface for Rholang
//!
//! This module provides the public API for parsing Rholang code using Tree-Sitter
//! and converting the concrete syntax tree (CST) to our intermediate representation (IR).
//!
//! ## Parse Tree Caching (Phase 2 Optimization)
//!
//! This module integrates parse tree caching to eliminate ~3.5% CPU overhead from
//! re-parsing unchanged documents. Cache hits provide 1,000-10,000x speedup over
//! re-parsing (20-30ns cache lookup vs 37-263µs parsing).

use std::sync::Arc;
use tree_sitter::{InputEdit, Parser, Tree};
use tracing::{debug, trace, warn};
use ropey::Rope;
use once_cell::sync::Lazy;

use crate::ir::rholang_node::{RholangNode, Position};
use crate::ir::{CommentNode, DocumentIR};
use crate::parsers::ParseCache;
use super::conversion::convert_ts_node_to_ir;
use super::helpers::walk_for_comments;

/// Global parse tree cache (shared across all parse operations)
///
/// Uses once_cell::Lazy for thread-safe lazy initialization.
/// Default capacity: 1000 entries (~60-110MB memory).
static PARSE_CACHE: Lazy<ParseCache> = Lazy::new(|| ParseCache::default());

/// Parse Rholang code into a Tree-Sitter syntax tree (with caching)
///
/// **Phase 2 Optimization**: This function now uses parse tree caching to avoid
/// re-parsing unchanged code. Cache lookups take ~20-30ns vs ~37-263µs for parsing.
///
/// # Arguments
/// * `code` - The Rholang source code to parse
///
/// # Returns
/// A Tree-Sitter Tree representing the parsed code
///
/// # Panics
/// Panics if the Tree-Sitter language cannot be set or parsing fails completely
///
/// # Performance
/// - Cache hit: ~20-30ns (1,000-10,000x faster than parsing)
/// - Cache miss: ~37-263µs (parsing) + ~15ns cache insertion overhead
pub fn parse_code(code: &str) -> Tree {
    // Check cache first (Phase 2 optimization)
    if let Some(cached_tree) = PARSE_CACHE.get(code) {
        trace!("Parse cache hit for {} byte code", code.len());
        return cached_tree;
    }

    // Cache miss - parse normally
    trace!("Parse cache miss for {} byte code, parsing...", code.len());
    let mut parser = Parser::new();
    parser
        .set_language(&rholang_tree_sitter::LANGUAGE.into())
        .expect("Failed to set Tree-Sitter language");

    let tree = parser
        .parse(code, None)
        .expect("Failed to parse Rholang code");

    // Store in cache for future use
    PARSE_CACHE.insert(code.to_string(), tree.clone());

    tree
}

/// Collect all comments from the Tree-Sitter tree
///
/// This function walks the entire parse tree, extracts all comment nodes,
/// and converts them to `CommentNode` instances with position tracking.
///
/// # Arguments
/// * `tree` - The Tree-Sitter tree to collect comments from
/// * `rope` - The source code as a Rope for text extraction
///
/// # Returns
/// A vector of `CommentNode` instances, sorted by position
///
/// # Performance
/// - Uses efficient tree walking with cursors
/// - Comments are already sorted by traversal order
/// - Position deltas computed relative to previous comment
fn collect_comments(tree: &Tree, rope: &Rope) -> Vec<CommentNode> {
    let mut ts_comments = Vec::new();
    walk_for_comments(tree.root_node(), &mut ts_comments);

    let mut comment_nodes = Vec::new();
    let mut prev_end = Position {
        row: 0,
        column: 0,
        byte: 0,
    };

    for ts_node in ts_comments {
        let comment_node = CommentNode::from_ts_node(ts_node, rope, prev_end);
        // Update prev_end to the end of this comment
        let comment_start = comment_node.absolute_position(prev_end);
        prev_end = comment_node.absolute_end(comment_start);
        comment_nodes.push(comment_node);
    }

    comment_nodes
}

/// Convert a Tree-Sitter syntax tree to DocumentIR with comment channel
///
/// **Phase 1**: This is the new primary parsing function that returns `DocumentIR`
/// containing both the semantic tree and the comment channel.
///
/// # Arguments
/// * `tree` - The Tree-Sitter tree to convert
/// * `rope` - The source code as a Rope for efficient slicing
///
/// # Returns
/// A `DocumentIR` containing:
/// - `root`: The semantic IR tree (without comments)
/// - `comments`: All comments sorted by position
///
/// # Examples
/// ```rust,ignore
/// let tree = parse_code(source);
/// let rope = Rope::from_str(source);
/// let doc_ir = parse_to_document_ir(&tree, &rope);
///
/// // Access semantic tree
/// let root = &doc_ir.root;
///
/// // Access comments
/// for comment in &doc_ir.comments {
///     if let Some(lang) = comment.parse_directive() {
///         println!("Found directive: {}", lang);
///     }
/// }
/// ```
pub fn parse_to_document_ir(tree: &Tree, rope: &Rope) -> Arc<DocumentIR> {
    debug!("Parsing Tree-Sitter tree into DocumentIR");
    if tree.root_node().has_error() {
        debug!("Parse tree contains errors");
    }

    // Parse semantic tree (without comments)
    let initial_prev_end = Position {
        row: 0,
        column: 0,
        byte: 0,
    };
    let (semantic_root, _) = convert_ts_node_to_ir(tree.root_node(), rope, initial_prev_end);

    // Collect comments into separate channel
    let comments = collect_comments(tree, rope);

    debug!("Parsed {} comments into comment channel", comments.len());

    Arc::new(DocumentIR::new(semantic_root, comments))
}

/// Convert a Tree-Sitter syntax tree to RholangNode IR
///
/// **DEPRECATED**: This function is maintained for backward compatibility.
/// New code should use `parse_to_document_ir()` to access the comment channel.
///
/// # Arguments
/// * `tree` - The Tree-Sitter tree to convert
/// * `rope` - The source code as a Rope for efficient slicing
///
/// # Returns
/// The root IR node representing the parsed program (comments excluded)
#[deprecated(since = "0.1.0", note = "Use parse_to_document_ir() to access comment channel")]
pub fn parse_to_ir(tree: &Tree, rope: &Rope) -> Arc<RholangNode> {
    parse_to_document_ir(tree, rope).root.clone()
}

/// Update a syntax tree incrementally based on text changes
///
/// This enables efficient re-parsing by reusing unchanged portions of the tree.
///
/// # Arguments
/// * `tree` - The existing syntax tree
/// * `new_text` - The updated source code
/// * `start_byte` - Byte offset where the edit starts
/// * `old_end_byte` - Byte offset where the edit ended in the old text
/// * `new_length` - Length of the new text inserted
///
/// # Returns
/// A new Tree reflecting the incremental edit, or a full parse if incremental fails
pub fn update_tree(
    tree: &Tree,
    new_text: &str,
    start_byte: usize,
    old_end_byte: usize,
    new_length: usize,
) -> Tree {
    let mut parser = Parser::new();
    parser
        .set_language(&rholang_tree_sitter::LANGUAGE.into())
        .expect("Failed to set Tree-Sitter language");

    let edit = InputEdit {
        start_byte,
        old_end_byte,
        new_end_byte: start_byte + new_length,
        start_position: tree.root_node().start_position(),
        old_end_position: tree.root_node().end_position(),
        new_end_position: tree.root_node().end_position(),
    };

    let mut new_tree = tree.clone();
    new_tree.edit(&edit);

    parser.parse(new_text, Some(&new_tree)).unwrap_or_else(|| {
        warn!("Incremental parse failed, performing full parse");
        parse_code(new_text)
    })
}
