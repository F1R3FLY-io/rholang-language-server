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
        CompletionContextType::Unknown => {
            // Unknown context: show all keywords as safe fallback
            keywords
        }
    }
}
