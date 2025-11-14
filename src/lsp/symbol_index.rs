//! Suffix array-based symbol index for O(m log n + k) substring search
//!
//! This module provides a generalized suffix array over all workspace symbols,
//! enabling fast substring matching without iterating through all symbols.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use suffix::SuffixTable;
use tower_lsp::lsp_types::SymbolInformation;
use std::sync::Arc;

/// Suffix array-based index for fast symbol substring search
///
/// Phase B-3: Custom Serialize/Deserialize implementation.
/// The suffix_table is reconstructed on deserialization since it can't be serialized
/// (uses leaked memory for 'static lifetime requirements).
#[derive(Debug)]
pub struct SymbolIndex {
    /// Pre-computed workspace symbols (for returning results)
    symbols: Arc<Vec<SymbolInformation>>,

    /// Concatenated symbol names with null byte separators (must be owned for 'static lifetime)
    text: Box<String>,

    /// Suffix table for O(m log n) substring search
    /// Using Box to keep text and table together for lifetime requirements
    /// Phase B-3: Skipped during serialization, rebuilt on deserialization
    suffix_table: Box<SuffixTable<'static, 'static>>,

    /// Maps character positions in concatenated text back to symbol indices
    position_to_symbol: Vec<usize>,
}

// Phase B-3: Custom serialization for SymbolIndex
impl Serialize for SymbolIndex {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("SymbolIndex", 3)?;

        // Serialize symbols as plain Vec (unwrap Arc)
        state.serialize_field("symbols", self.symbols.as_ref())?;
        state.serialize_field("text", &*self.text)?;
        state.serialize_field("position_to_symbol", &self.position_to_symbol)?;
        // Skip suffix_table - will be reconstructed on deserialization
        state.end()
    }
}

impl<'de> Deserialize<'de> for SymbolIndex {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct SymbolIndexHelper {
            symbols: Vec<SymbolInformation>,
            text: String,
            position_to_symbol: Vec<usize>,
        }

        let helper = SymbolIndexHelper::deserialize(deserializer)?;

        // Rebuild suffix table from text (same as in new())
        let leaked_text: &'static str = Box::leak(helper.text.clone().into_boxed_str());
        let suffix_table = Box::new(SuffixTable::new(leaked_text));

        Ok(SymbolIndex {
            symbols: Arc::new(helper.symbols),
            text: Box::new(helper.text),
            suffix_table,
            position_to_symbol: helper.position_to_symbol,
        })
    }
}

impl SymbolIndex {
    /// Build a new symbol index from a list of symbols
    pub fn new(symbols: Vec<SymbolInformation>) -> Self {
        if symbols.is_empty() {
            // For empty index, leak an empty string to get 'static lifetime
            let leaked_text: &'static str = Box::leak(Box::new(String::new())).as_str();
            return Self {
                symbols: Arc::new(Vec::new()),
                text: Box::new(String::new()),
                suffix_table: Box::new(SuffixTable::new(leaked_text)),
                position_to_symbol: Vec::new(),
            };
        }

        // Build concatenated string with null separators
        // e.g., "FooContract\0BarService\0BazModule\0"
        let mut text = String::new();
        let mut position_to_symbol = Vec::new();

        for (idx, symbol) in symbols.iter().enumerate() {
            let name_len = symbol.name.len();

            // Convert to lowercase for case-insensitive search
            text.push_str(&symbol.name.to_lowercase());

            // Map each character position to its symbol index
            for _ in 0..name_len {
                position_to_symbol.push(idx);
            }

            // Add null separator
            text.push('\0');
            position_to_symbol.push(usize::MAX); // Sentinel for separator
        }

        // Leak the string to get 'static lifetime for SuffixTable
        // This is safe because the SymbolIndex owns the data and lives as long as needed
        let leaked_text: &'static str = Box::leak(text.clone().into_boxed_str());

        // Build suffix table - this is O(n log n) but done once during indexing
        let suffix_table = Box::new(SuffixTable::new(leaked_text));

        Self {
            symbols: Arc::new(symbols),
            text: Box::new(text),
            suffix_table,
            position_to_symbol,
        }
    }

    /// Search for symbols containing the query substring
    ///
    /// # Performance
    /// O(m log n + k) where:
    /// - m = query length
    /// - n = total characters in all symbol names
    /// - k = number of matches
    ///
    /// This is significantly faster than O(symbols × avg_name_length) linear search
    pub fn search(&self, query: &str) -> Vec<SymbolInformation> {
        if query.is_empty() {
            // Empty query returns all symbols
            return (*self.symbols).clone();
        }

        if self.symbols.is_empty() {
            return Vec::new();
        }

        // Convert query to lowercase for case-insensitive matching
        let query_lower = query.to_lowercase();

        // Use suffix table to find all positions containing the query
        // This is O(m log n) binary search + O(k) to collect matches
        let positions = self.suffix_table.positions(&query_lower);

        // Map positions back to symbol indices and deduplicate
        let mut symbol_indices = std::collections::HashSet::new();
        for pos in positions {
            let pos_usize = *pos as usize;
            if pos_usize < self.position_to_symbol.len() {
                let symbol_idx = self.position_to_symbol[pos_usize];
                if symbol_idx != usize::MAX {  // Skip separators
                    symbol_indices.insert(symbol_idx);
                }
            }
        }

        // Collect matching symbols
        let mut results: Vec<SymbolInformation> = symbol_indices
            .into_iter()
            .filter_map(|idx| self.symbols.get(idx).cloned())
            .collect();

        // Sort by name for consistent ordering
        results.sort_by(|a, b| a.name.cmp(&b.name));

        results
    }

    /// Get the number of symbols in the index
    pub fn len(&self) -> usize {
        self.symbols.len()
    }

    /// Check if the index is empty
    pub fn is_empty(&self) -> bool {
        self.symbols.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp::lsp_types::{Location, Position, Range, SymbolKind, Url};

    fn create_test_symbol(name: &str) -> SymbolInformation {
        SymbolInformation {
            name: name.to_string(),
            kind: SymbolKind::FUNCTION,
            location: Location {
                uri: Url::parse("file:///test.rho").unwrap(),
                range: Range {
                    start: Position { line: 0, character: 0 },
                    end: Position { line: 0, character: 10 },
                },
            },
            tags: None,
            deprecated: None,
            container_name: None,
        }
    }

    #[test]
    fn test_exact_match() {
        let symbols = vec![
            create_test_symbol("FooContract"),
            create_test_symbol("BarService"),
            create_test_symbol("BazModule"),
        ];

        let index = SymbolIndex::new(symbols);
        let results = index.search("FooContract");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "FooContract");
    }

    #[test]
    fn test_prefix_match() {
        let symbols = vec![
            create_test_symbol("FooContract"),
            create_test_symbol("FooService"),
            create_test_symbol("BarModule"),
        ];

        let index = SymbolIndex::new(symbols);
        let results = index.search("Foo");

        assert_eq!(results.len(), 2);
        assert!(results.iter().any(|s| s.name == "FooContract"));
        assert!(results.iter().any(|s| s.name == "FooService"));
    }

    #[test]
    fn test_substring_match() {
        let symbols = vec![
            create_test_symbol("MyFooContract"),
            create_test_symbol("BarService"),
            create_test_symbol("FooBarBaz"),
        ];

        let index = SymbolIndex::new(symbols);
        let results = index.search("Foo");

        assert_eq!(results.len(), 2);
        assert!(results.iter().any(|s| s.name == "MyFooContract"));
        assert!(results.iter().any(|s| s.name == "FooBarBaz"));
    }

    #[test]
    fn test_case_insensitive() {
        let symbols = vec![
            create_test_symbol("FooContract"),
            create_test_symbol("FOOSERVICE"),
            create_test_symbol("fooModule"),
        ];

        let index = SymbolIndex::new(symbols);
        let results = index.search("foo");

        assert_eq!(results.len(), 3);
    }

    #[test]
    fn test_empty_query() {
        let symbols = vec![
            create_test_symbol("Foo"),
            create_test_symbol("Bar"),
        ];

        let index = SymbolIndex::new(symbols);
        let results = index.search("");

        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_no_matches() {
        let symbols = vec![
            create_test_symbol("FooContract"),
            create_test_symbol("BarService"),
        ];

        let index = SymbolIndex::new(symbols);
        let results = index.search("Xyz");

        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_empty_index() {
        let index = SymbolIndex::new(Vec::new());
        let results = index.search("Foo");

        assert_eq!(results.len(), 0);
    }
}
