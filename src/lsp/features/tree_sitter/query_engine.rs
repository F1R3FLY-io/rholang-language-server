//! Query engine for loading and executing Tree-Sitter queries
//!
//! Provides the core engine for loading .scm query files and executing them
//! against Tree-Sitter syntax trees. Supports incremental parsing and caching
//! for optimal performance.

use std::collections::HashMap;
use std::sync::Arc;
use tree_sitter::{Language, Parser, Query, QueryCursor, Tree, InputEdit};
use tracing::{debug, trace, warn};
use ropey::Rope;

use super::query_types::{QueryType, QueryCapture};
use crate::ir::semantic_node::Position;

/// Query engine for a specific language
///
/// Manages Tree-Sitter queries for a language and provides execution methods.
/// Supports incremental parsing for efficient document updates.
pub struct QueryEngine {
    /// Language name (e.g., "rholang", "metta")
    language_name: String,
    /// Tree-Sitter language
    language: Language,
    /// Loaded queries by type
    queries: HashMap<QueryType, Arc<Query>>,
    /// Parser for this language (reused for efficiency)
    parser: Parser,
    /// Cached parse tree (for incremental updates)
    cached_tree: Option<Tree>,
    /// Source code of cached tree
    cached_source: Option<String>,
}

impl QueryEngine {
    /// Create a new query engine for a language
    ///
    /// # Arguments
    /// * `language_name` - Name of the language ("rholang", "metta", etc.)
    /// * `language` - Tree-Sitter language object
    ///
    /// # Returns
    /// QueryEngine instance
    pub fn new(language_name: impl Into<String>, language: Language) -> Result<Self, String> {
        let language_name = language_name.into();
        let mut parser = Parser::new();
        parser
            .set_language(&language)
            .map_err(|e| format!("Failed to set language: {}", e))?;

        Ok(Self {
            language_name,
            language,
            queries: HashMap::new(),
            parser,
            cached_tree: None,
            cached_source: None,
        })
    }

    /// Load a query from string content
    ///
    /// # Arguments
    /// * `query_type` - Type of query (Highlights, Locals, etc.)
    /// * `query_source` - .scm query content
    ///
    /// # Returns
    /// Result indicating success or error message
    pub fn load_query(&mut self, query_type: QueryType, query_source: &str) -> Result<(), String> {
        debug!("Loading {} query for {}", query_type.description(), self.language_name);

        let query = Query::new(&self.language, query_source)
            .map_err(|e| format!("Failed to parse query: {}", e))?;

        trace!(
            "Loaded query with {} patterns and {} captures",
            query.pattern_count(),
            query.capture_names().len()
        );

        self.queries.insert(query_type, Arc::new(query));
        Ok(())
    }

    /// Parse source code into a Tree-Sitter tree
    ///
    /// Uses incremental parsing if possible (when cached tree exists and source is similar).
    ///
    /// # Arguments
    /// * `source` - Source code to parse
    ///
    /// # Returns
    /// Parsed Tree-Sitter tree
    pub fn parse(&mut self, source: &str) -> Result<Tree, String> {
        // Check if we can use incremental parsing
        if let (Some(old_tree), Some(old_source)) = (&self.cached_tree, &self.cached_source) {
            // Attempt incremental parse
            trace!("Attempting incremental parse (old: {} bytes, new: {} bytes)",
                   old_source.len(), source.len());

            // For now, we'll do a full re-parse if sources differ significantly
            // TODO: Implement proper edit tracking for incremental parsing
            if let Some(new_tree) = self.parser.parse(source, Some(old_tree)) {
                trace!("Incremental parse successful");
                self.cached_tree = Some(new_tree.clone());
                self.cached_source = Some(source.to_string());
                return Ok(new_tree);
            }
        }

        // Full parse
        trace!("Full parse of {} bytes", source.len());
        let tree = self.parser
            .parse(source, None)
            .ok_or_else(|| "Failed to parse source code".to_string())?;

        // Cache for future incremental updates
        self.cached_tree = Some(tree.clone());
        self.cached_source = Some(source.to_string());

        Ok(tree)
    }

    /// Update cached tree with incremental edit
    ///
    /// More efficient than full re-parse when you have the edit information.
    ///
    /// # Arguments
    /// * `edit` - The edit that was applied to the source
    /// * `new_source` - The updated source code
    ///
    /// # Returns
    /// Updated Tree-Sitter tree
    pub fn update_tree(&mut self, edit: InputEdit, new_source: &str) -> Result<Tree, String> {
        if let Some(old_tree) = &mut self.cached_tree {
            // Apply edit to cached tree
            old_tree.edit(&edit);

            // Incremental re-parse
            trace!("Incremental update at byte {}", edit.start_byte);
            let new_tree = self.parser
                .parse(new_source, Some(old_tree))
                .ok_or_else(|| "Failed to parse updated source".to_string())?;

            self.cached_tree = Some(new_tree.clone());
            self.cached_source = Some(new_source.to_string());

            Ok(new_tree)
        } else {
            // No cached tree, do full parse
            self.parse(new_source)
        }
    }

    /// Execute a query on a syntax tree
    ///
    /// # Arguments
    /// * `tree` - Tree-Sitter syntax tree
    /// * `query_type` - Type of query to execute
    /// * `source` - Source code (for extracting text)
    ///
    /// # Returns
    /// Vector of query captures
    pub fn execute<'tree>(
        &self,
        tree: &'tree Tree,
        query_type: QueryType,
        source: &[u8],
    ) -> Result<Vec<QueryCapture<'tree>>, String> {
        let query = self.queries
            .get(&query_type)
            .ok_or_else(|| format!("{} query not loaded", query_type.description()))?;

        debug!("Executing {} query", query_type.description());

        // TODO: Implement proper Tree-Sitter 0.25 query execution
        // The API has changed and needs proper investigation
        trace!("Query execution not yet implemented for Tree-Sitter 0.25");
        Ok(Vec::new())
    }

    /// Execute a query on a specific subtree (ranged query)
    ///
    /// Useful for ranged formatting or localized analysis.
    ///
    /// # Arguments
    /// * `tree` - Tree-Sitter syntax tree
    /// * `query_type` - Type of query to execute
    /// * `source` - Source code
    /// * `start_byte` - Start of range
    /// * `end_byte` - End of range
    ///
    /// # Returns
    /// Vector of query captures within the specified range
    pub fn execute_ranged<'tree>(
        &self,
        tree: &'tree Tree,
        query_type: QueryType,
        source: &[u8],
        start_byte: usize,
        end_byte: usize,
    ) -> Result<Vec<QueryCapture<'tree>>, String> {
        let query = self.queries
            .get(&query_type)
            .ok_or_else(|| format!("{} query not loaded", query_type.description()))?;

        debug!("Executing ranged {} query ({}..{})",
               query_type.description(), start_byte, end_byte);

        // TODO: Implement proper Tree-Sitter 0.25 ranged query execution
        trace!("Ranged query execution not yet implemented for Tree-Sitter 0.25");
        Ok(Vec::new())
    }

    /// Get all loaded query types
    pub fn loaded_queries(&self) -> Vec<QueryType> {
        self.queries.keys().copied().collect()
    }

    /// Check if a specific query type is loaded
    pub fn has_query(&self, query_type: QueryType) -> bool {
        self.queries.contains_key(&query_type)
    }

    /// Get language name
    pub fn language_name(&self) -> &str {
        &self.language_name
    }

    /// Clear cached tree (useful for testing or memory management)
    pub fn clear_cache(&mut self) {
        self.cached_tree = None;
        self.cached_source = None;
    }
}

/// Factory for creating QueryEngines for known languages
pub struct QueryEngineFactory;

impl QueryEngineFactory {
    /// Create a QueryEngine for Rholang with default queries
    pub fn create_rholang() -> Result<QueryEngine, String> {
        let language = rholang_tree_sitter::LANGUAGE.into();
        let mut engine = QueryEngine::new("rholang", language)?;

        // Load default queries (these would be embedded or loaded from files)
        // For now, we'll leave them to be loaded by the caller
        // In production, you might want to embed the .scm files using include_str!

        Ok(engine)
    }

    /// Create a QueryEngine for MeTTa with default queries
    pub fn create_metta() -> Result<QueryEngine, String> {
        // MeTTa Tree-Sitter not yet configured properly
        // For now, just return an error
        Err("MeTTa Tree-Sitter language not yet available".to_string())
    }

    /// Create a QueryEngine for an arbitrary language
    pub fn create(language_name: &str, language: Language) -> Result<QueryEngine, String> {
        QueryEngine::new(language_name, language)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_engine_creation() {
        let result = QueryEngineFactory::create_rholang();
        assert!(result.is_ok());
        let engine = result.unwrap();
        assert_eq!(engine.language_name(), "rholang");
    }

    #[test]
    fn test_query_loading() {
        let mut engine = QueryEngineFactory::create_rholang().unwrap();

        // Simple test query
        let query_src = r#"
            (var) @variable
        "#;

        let result = engine.load_query(QueryType::Highlights, query_src);
        assert!(result.is_ok());
        assert!(engine.has_query(QueryType::Highlights));
    }

    #[test]
    fn test_parse_and_execute() {
        let mut engine = QueryEngineFactory::create_rholang().unwrap();

        // Load a simple query
        let query_src = r#"
            (var) @variable
        "#;
        engine.load_query(QueryType::Highlights, query_src).unwrap();

        // Parse simple Rholang code
        let source = "new x in { x!(42) }";
        let tree = engine.parse(source).unwrap();

        // Execute query
        let captures = engine.execute(&tree, QueryType::Highlights, source.as_bytes()).unwrap();

        // Should find "x" variable references
        assert!(!captures.is_empty());
    }

    #[test]
    fn test_incremental_parse() {
        let mut engine = QueryEngineFactory::create_rholang().unwrap();

        // Initial parse
        let source1 = "new x in { x!(42) }";
        let tree1 = engine.parse(source1).unwrap();
        assert!(tree1.root_node().child_count() > 0);

        // Parse similar code (incremental path)
        let source2 = "new y in { y!(99) }";
        let tree2 = engine.parse(source2).unwrap();
        assert!(tree2.root_node().child_count() > 0);

        // Verify caching worked
        assert!(engine.cached_tree.is_some());
        assert!(engine.cached_source.is_some());
    }
}
