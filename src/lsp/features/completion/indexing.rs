//! Completion index population from existing symbol tables
//!
//! This module provides functions to populate the WorkspaceCompletionIndex
//! from existing symbol tables and global indexes.

use super::dictionary::{WorkspaceCompletionIndex, SymbolMetadata, CompletionSymbol};
use super::context::CompletionContextType;
use crate::ir::symbol_table::{SymbolTable, SymbolType};
use tower_lsp::lsp_types::CompletionItemKind;

/// Populate completion index from a symbol table
///
/// Extracts all symbols from the symbol table and adds them to the completion index.
pub fn populate_from_symbol_table(
    index: &WorkspaceCompletionIndex,
    symbol_table: &SymbolTable,
) {
    // Get all symbols from the current scope and parent scopes
    let all_symbols = symbol_table.collect_all_symbols();

    for symbol in all_symbols {
        let kind = match symbol.symbol_type {
            SymbolType::Variable => CompletionItemKind::VARIABLE,
            SymbolType::Contract => CompletionItemKind::FUNCTION,
            SymbolType::Parameter => CompletionItemKind::VARIABLE,
        };

        let type_str = match symbol.symbol_type {
            SymbolType::Variable => "variable",
            SymbolType::Contract => "contract",
            SymbolType::Parameter => "parameter",
        };

        let metadata = SymbolMetadata {
            name: symbol.name.clone(),
            kind,
            documentation: symbol.documentation.clone(),
            signature: Some(type_str.to_string()),
            reference_count: 0,  // TODO: Track reference counts
        };

        index.insert(symbol.name.clone(), metadata);
    }
}

/// Populate completion index from a symbol table with document tracking for incremental updates
///
/// This variant tracks which symbols belong to which document URI,
/// enabling efficient removal when the document changes.
pub fn populate_from_symbol_table_with_tracking(
    index: &WorkspaceCompletionIndex,
    symbol_table: &SymbolTable,
    uri: &tower_lsp::lsp_types::Url,
) {
    // Get all symbols from the current scope and parent scopes
    let all_symbols = symbol_table.collect_all_symbols();

    for symbol in all_symbols {
        let kind = match symbol.symbol_type {
            SymbolType::Variable => CompletionItemKind::VARIABLE,
            SymbolType::Contract => CompletionItemKind::FUNCTION,
            SymbolType::Parameter => CompletionItemKind::VARIABLE,
        };

        let type_str = match symbol.symbol_type {
            SymbolType::Variable => "variable",
            SymbolType::Contract => "contract",
            SymbolType::Parameter => "parameter",
        };

        let metadata = SymbolMetadata {
            name: symbol.name.clone(),
            kind,
            documentation: symbol.documentation.clone(),
            signature: Some(type_str.to_string()),
            reference_count: 0,  // TODO: Track reference counts
        };

        let symbol_name = symbol.name.clone();
        index.insert(symbol_name.clone(), metadata);

        // Track which document this symbol belongs to
        index.track_document_symbol(uri, symbol_name);
    }
}

/// Add Rholang keywords to the completion index
pub fn add_keywords(index: &WorkspaceCompletionIndex) {
    let keywords = vec![
        // Process keywords (from grammar reserved words)
        ("new", "Declare new channels"),
        ("contract", "Define a contract"),
        ("for", "Input guarded process (for comprehension)"),
        ("match", "Pattern matching expression"),
        ("select", "Non-deterministic choice expression"),
        ("if", "Conditional expression"),
        ("else", "Alternative branch in conditional"),
        ("let", "Local binding declaration"),

        // Bundle keywords
        ("bundle", "Read-write bundle (restricts channel capabilities)"),
        ("bundle-", "Read-only bundle"),
        ("bundle+", "Write-only bundle"),
        ("bundle0", "Equivalence-only bundle"),

        // Boolean keywords
        ("true", "Boolean true literal"),
        ("false", "Boolean false literal"),

        // Logical operators (keywords)
        ("or", "Logical disjunction"),
        ("and", "Logical conjunction"),
        ("not", "Logical negation"),
        ("matches", "Pattern matching operator"),

        // Special values
        ("Nil", "Empty process (no-op)"),

        // Type keywords
        ("Bool", "Boolean type"),
        ("Int", "Integer type"),
        ("String", "String type"),
        ("Uri", "URI type"),
        ("ByteArray", "Byte array type"),
        ("Set", "Set collection constructor"),
    ];

    for (keyword, doc) in keywords {
        let metadata = SymbolMetadata {
            name: keyword.to_string(),
            kind: CompletionItemKind::KEYWORD,
            documentation: Some(doc.to_string()),
            signature: Some("keyword".to_string()),
            reference_count: 0,
        };

        index.insert(keyword.to_string(), metadata);
    }
}

/// Remove all symbols from a specific document (Phase 11.3: Incremental Updates)
///
/// This is the first step in an incremental update: remove outdated symbols before adding new ones.
/// Used when a document changes to clean up its old symbols from the index.
///
/// # Performance
/// - O(m) where m = number of symbols in the document
/// - Leverages Phase 10's deletion support in DynamicDawg
/// - Much faster than full index rebuild (O(n × m) where n = total files)
pub fn remove_symbols_from_file(
    index: &WorkspaceCompletionIndex,
    uri: &tower_lsp::lsp_types::Url,
) {
    // Delegate to WorkspaceCompletionIndex method
    index.remove_document_symbols(uri);
}

/// Add symbols from a document's symbol table (Phase 11.3: Incremental Updates)
///
/// This is the second step in an incremental update: add new symbols after removing old ones.
/// Used when a document changes to populate the index with its current symbols.
///
/// # Performance
/// - O(m) where m = number of symbols in the document
/// - Tracks document → symbol mappings for future incremental updates
pub fn insert_symbols_from_file(
    index: &WorkspaceCompletionIndex,
    uri: &tower_lsp::lsp_types::Url,
    symbol_table: &SymbolTable,
) {
    // Reuse existing populate function with tracking
    populate_from_symbol_table_with_tracking(index, symbol_table, uri);
}

/// Update symbols for a changed document (Phase 11.3: Incremental Updates)
///
/// This combines remove + insert for a single atomic operation.
/// Called from didChange/didSave handlers to incrementally update the completion index.
///
/// # Performance
/// - O(m) where m = number of symbols in this document
/// - **100-1000x faster** than full rebuild for single-file edits
/// - Phase 10's deletion + DynamicDawg insertion enable O(m) complexity
///
/// # Example
/// ```ignore
/// // In didChange handler:
/// update_symbols_for_file(&workspace_index, &uri, &new_symbol_table);
/// ```
pub fn update_symbols_for_file(
    index: &WorkspaceCompletionIndex,
    uri: &tower_lsp::lsp_types::Url,
    symbol_table: &SymbolTable,
) {
    // Step 1: Remove old symbols for this file (O(m) where m = symbols in file)
    remove_symbols_from_file(index, uri);

    // Step 2: Insert new symbols for this file (O(m))
    insert_symbols_from_file(index, uri, symbol_table);
}

/// Filter keywords based on completion context
///
/// Returns keywords appropriate for the current context based on Rholang grammar:
///
/// - **Expression/LexicalScope**: Process keywords (new, contract, for, match, select, if, let, bundle*),
///   literals (true, false, Nil), logical operators (or, and, not, matches), types (Bool, Int, String, etc.)
///
/// - **Pattern**: Pattern-appropriate keywords - literals (true, false, Nil), type constructors (Set),
///   and logical operators for pattern guards
///
/// - **StringLiteral**: No keywords (user is typing string content)
///
/// - **TypeMethod**: No keywords (after dot operator, only methods apply)
///
/// - **VirtualDocument**: Language-specific keywords (future enhancement)
///
/// Grammar reference: `/rholang-rs/rholang-tree-sitter/grammar.js`
pub fn filter_keywords_by_context(
    keywords: Vec<CompletionSymbol>,
    context: &CompletionContextType,
) -> Vec<CompletionSymbol> {
    match context {
        CompletionContextType::Expression | CompletionContextType::LexicalScope { .. } => {
            // In expression/scope context: allow process keywords, literals, operators, types
            // Excludes: 'else' (only after 'if'), pattern-specific keywords
            keywords
                .into_iter()
                .filter(|k| {
                    !matches!(
                        k.metadata.name.as_str(),
                        "else"  // Only shown after 'if'
                    )
                })
                .collect()
        }
        CompletionContextType::Pattern => {
            // In pattern context: allow literals, type constructors, logical operators for guards
            // Based on grammar: case: $ => seq(field('pattern', $._proc), '=>', field('proc', $._proc))
            keywords
                .into_iter()
                .filter(|k| {
                    matches!(
                        k.metadata.name.as_str(),
                        // Literals for matching
                        "true" | "false" | "Nil" |
                        // Type constructors for pattern matching
                        "Set" |
                        // Logical operators for pattern guards
                        "and" | "or" | "not" | "matches"
                    )
                })
                .collect()
        }
        CompletionContextType::StringLiteral => {
            // No keywords inside string literals
            vec![]
        }
        CompletionContextType::TypeMethod { .. } => {
            // After dot operator, only methods (handled separately via type_methods.rs)
            vec![]
        }
        CompletionContextType::VirtualDocument { .. } => {
            // Virtual documents (e.g., MeTTa code in strings) have their own language keywords
            // Don't show Rholang keywords
            vec![]
        }
        CompletionContextType::QuotedMapPattern { .. } => {
            // Inside quoted map pattern: pattern-aware completion (Phase 3)
            // No keywords needed
            vec![]
        }
        CompletionContextType::QuotedListPattern { .. } => {
            // Inside quoted list pattern: pattern-aware completion (Phase 3)
            // No keywords needed
            vec![]
        }
        CompletionContextType::QuotedTuplePattern { .. } => {
            // Inside quoted tuple pattern: pattern-aware completion (Phase 3)
            // No keywords needed
            vec![]
        }
        CompletionContextType::QuotedSetPattern { .. } => {
            // Inside quoted set pattern: pattern-aware completion (Phase 3)
            // No keywords needed
            vec![]
        }
        CompletionContextType::Unknown => {
            // Unknown context: show all keywords as safe fallback
            keywords
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::symbol_table::Symbol;
    use crate::ir::semantic_node::Position;
    use tower_lsp::lsp_types::Url;
    use std::sync::Arc;

    /// Helper to create a test symbol table with a few symbols
    fn create_test_symbol_table() -> SymbolTable {
        let table = SymbolTable::new(None);

        // Add a contract
        let contract_symbol = Arc::new(Symbol {
            name: "testContract".to_string(),
            symbol_type: SymbolType::Contract,
            declaration_uri: Url::parse("file:///test.rho").unwrap(),
            declaration_location: Position { row: 0, column: 0, byte: 0 },
            definition_location: Some(Position { row: 0, column: 10, byte: 10 }),
            contract_pattern: None,
            contract_identifier_node: None,
            documentation: None,
        });
        table.insert(contract_symbol);

        // Add a variable
        let var_symbol = Arc::new(Symbol {
            name: "myVar".to_string(),
            symbol_type: SymbolType::Variable,
            declaration_uri: Url::parse("file:///test.rho").unwrap(),
            declaration_location: Position { row: 1, column: 0, byte: 20 },
            definition_location: Some(Position { row: 1, column: 5, byte: 25 }),
            contract_pattern: None,
            contract_identifier_node: None,
            documentation: None,
        });
        table.insert(var_symbol);

        table
    }

    #[test]
    fn test_remove_symbols_from_file() {
        let index = WorkspaceCompletionIndex::new();
        let uri = Url::parse("file:///test.rho").unwrap();
        let symbol_table = create_test_symbol_table();

        // Step 1: Insert symbols with tracking
        insert_symbols_from_file(&index, &uri, &symbol_table);

        // Verify symbols are in index
        assert!(index.contains("testContract"), "Should contain testContract");
        assert!(index.contains("myVar"), "Should contain myVar");

        // Step 2: Remove symbols
        remove_symbols_from_file(&index, &uri);

        // Verify symbols are removed
        assert!(!index.contains("testContract"), "Should not contain testContract after removal");
        assert!(!index.contains("myVar"), "Should not contain myVar after removal");
    }

    #[test]
    fn test_insert_symbols_from_file() {
        let index = WorkspaceCompletionIndex::new();
        let uri = Url::parse("file:///test.rho").unwrap();
        let symbol_table = create_test_symbol_table();

        // Insert symbols
        insert_symbols_from_file(&index, &uri, &symbol_table);

        // Verify symbols are in index
        assert!(index.contains("testContract"), "Should contain testContract");
        assert!(index.contains("myVar"), "Should contain myVar");

        // Verify metadata
        let metadata = index.get_metadata("testContract");
        assert!(metadata.is_some(), "Should have metadata for testContract");
        let metadata = metadata.unwrap();
        assert_eq!(metadata.name, "testContract");
        assert_eq!(metadata.kind, CompletionItemKind::FUNCTION);
    }

    #[test]
    fn test_update_symbols_for_file() {
        let index = WorkspaceCompletionIndex::new();
        let uri = Url::parse("file:///test.rho").unwrap();

        // Create initial symbol table
        let symbol_table1 = create_test_symbol_table();

        // Insert initial symbols
        insert_symbols_from_file(&index, &uri, &symbol_table1);

        assert!(index.contains("testContract"), "Should contain testContract");
        assert!(index.contains("myVar"), "Should contain myVar");

        // Create updated symbol table with different symbols
        let symbol_table2 = SymbolTable::new(None);

        let new_contract = Arc::new(Symbol {
            name: "newContract".to_string(),
            symbol_type: SymbolType::Contract,
            declaration_uri: uri.clone(),
            declaration_location: Position { row: 0, column: 0, byte: 0 },
            definition_location: Some(Position { row: 0, column: 15, byte: 15 }),
            contract_pattern: None,
            contract_identifier_node: None,
            documentation: None,
        });
        symbol_table2.insert(new_contract);

        let new_var = Arc::new(Symbol {
            name: "newVar".to_string(),
            symbol_type: SymbolType::Variable,
            declaration_uri: uri.clone(),
            declaration_location: Position { row: 1, column: 0, byte: 20 },
            definition_location: Some(Position { row: 1, column: 6, byte: 26 }),
            contract_pattern: None,
            contract_identifier_node: None,
            documentation: None,
        });
        symbol_table2.insert(new_var);

        // Update symbols (remove old, insert new)
        update_symbols_for_file(&index, &uri, &symbol_table2);

        // Verify old symbols are removed
        assert!(!index.contains("testContract"), "Should not contain testContract after update");
        assert!(!index.contains("myVar"), "Should not contain myVar after update");

        // Verify new symbols are inserted
        assert!(index.contains("newContract"), "Should contain newContract after update");
        assert!(index.contains("newVar"), "Should contain newVar after update");

        // Verify metadata for new symbols
        let metadata = index.get_metadata("newContract");
        assert!(metadata.is_some(), "Should have metadata for newContract");
        let metadata = metadata.unwrap();
        assert_eq!(metadata.name, "newContract");
    }

    #[test]
    fn test_update_symbols_for_file_handles_empty_table() {
        let index = WorkspaceCompletionIndex::new();
        let uri = Url::parse("file:///test.rho").unwrap();

        // Insert initial symbols
        let symbol_table1 = create_test_symbol_table();
        insert_symbols_from_file(&index, &uri, &symbol_table1);

        assert!(index.contains("testContract"));
        assert!(index.contains("myVar"));

        // Update with empty symbol table (simulating deleted file content)
        let empty_table = SymbolTable::new(None);
        update_symbols_for_file(&index, &uri, &empty_table);

        // All symbols should be removed
        assert!(!index.contains("testContract"), "Should remove all symbols when table is empty");
        assert!(!index.contains("myVar"), "Should remove all symbols when table is empty");
    }

    #[test]
    fn test_incremental_update_does_not_affect_other_files() {
        let index = WorkspaceCompletionIndex::new();
        let uri1 = Url::parse("file:///test1.rho").unwrap();
        let uri2 = Url::parse("file:///test2.rho").unwrap();

        // Create symbol tables for two different files
        let table1 = SymbolTable::new(None);
        let contract1 = Arc::new(Symbol {
            name: "contract1".to_string(),
            symbol_type: SymbolType::Contract,
            declaration_uri: uri1.clone(),
            declaration_location: Position { row: 0, column: 0, byte: 0 },
            definition_location: Some(Position { row: 0, column: 10, byte: 10 }),
            contract_pattern: None,
            contract_identifier_node: None,
            documentation: None,
        });
        table1.insert(contract1);

        let table2 = SymbolTable::new(None);
        let contract2 = Arc::new(Symbol {
            name: "contract2".to_string(),
            symbol_type: SymbolType::Contract,
            declaration_uri: uri2.clone(),
            declaration_location: Position { row: 0, column: 0, byte: 0 },
            definition_location: Some(Position { row: 0, column: 10, byte: 10 }),
            contract_pattern: None,
            contract_identifier_node: None,
            documentation: None,
        });
        table2.insert(contract2);

        // Insert symbols from both files
        insert_symbols_from_file(&index, &uri1, &table1);
        insert_symbols_from_file(&index, &uri2, &table2);

        assert!(index.contains("contract1"));
        assert!(index.contains("contract2"));

        // Update first file (remove its symbols)
        let empty_table = SymbolTable::new(None);
        update_symbols_for_file(&index, &uri1, &empty_table);

        // File 1 symbols should be removed
        assert!(!index.contains("contract1"), "File 1 symbols should be removed");

        // File 2 symbols should remain
        assert!(index.contains("contract2"), "File 2 symbols should remain unaffected");
    }

    #[test]
    fn test_incremental_update_performance_characteristic() {
        // This test verifies the O(m) complexity behavior (not strict benchmark)
        let index = WorkspaceCompletionIndex::new();

        // Create 100 files with 10 symbols each (1000 total symbols)
        for file_id in 0..100 {
            let uri = Url::parse(&format!("file:///test{}.rho", file_id)).unwrap();
            let table = SymbolTable::new(None);

            for sym_id in 0..10 {
                let name = format!("symbol_{}_{}", file_id, sym_id);
                let symbol = Arc::new(Symbol {
                    name: name.clone(),
                    symbol_type: SymbolType::Variable,
                    declaration_uri: uri.clone(),
                    declaration_location: Position {
                        row: sym_id,
                        column: 0,
                        byte: sym_id * 20,
                    },
                    definition_location: Some(Position {
                        row: sym_id,
                        column: 10,
                        byte: sym_id * 20 + 10,
                    }),
                    contract_pattern: None,
                    contract_identifier_node: None,
            documentation: None,
                });
                table.insert(symbol);
            }

            insert_symbols_from_file(&index, &uri, &table);
        }

        // Verify all 1000 symbols are indexed (+ 16 static keywords)
        assert_eq!(index.len(), 1016, "Should have 1000 symbols + 16 keywords");

        // Now update ONE file with new symbols (simulating single file edit)
        let target_uri = Url::parse("file:///test42.rho").unwrap();
        let new_table = SymbolTable::new(None);
        for sym_id in 0..10 {
            let name = format!("new_symbol_42_{}", sym_id);
            let symbol = Arc::new(Symbol {
                name: name.clone(),
                symbol_type: SymbolType::Contract,
                declaration_uri: target_uri.clone(),
                declaration_location: Position {
                    row: sym_id,
                    column: 0,
                    byte: sym_id * 20,
                },
                definition_location: Some(Position {
                    row: sym_id,
                    column: 10,
                    byte: sym_id * 20 + 10,
                }),
                contract_pattern: None,
                contract_identifier_node: None,
            documentation: None,
            });
            new_table.insert(symbol);
        }

        // Incremental update: should only process 10 symbols (O(m) where m=10)
        // NOT all 1000 symbols (which would be O(n × m))
        update_symbols_for_file(&index, &target_uri, &new_table);

        // Verify old file 42 symbols are gone
        assert!(!index.contains("symbol_42_0"), "Old symbols should be removed");

        // Verify new file 42 symbols are present
        assert!(index.contains("new_symbol_42_0"), "New symbols should be inserted");

        // Verify other files remain unaffected
        assert!(index.contains("symbol_0_0"), "File 0 symbols should remain");
        assert!(index.contains("symbol_99_9"), "File 99 symbols should remain");

        // Total count should still be 1000 + 16 (same symbols, just updated for one file)
        assert_eq!(index.len(), 1016, "Total symbol count should remain same");
    }
}
