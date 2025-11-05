//! Rholang language adapter
//!
//! Provides Rholang-specific implementations of LSP features using the
//! unified LanguageAdapter architecture.

use std::sync::Arc;
use tower_lsp::lsp_types::{HoverContents, CompletionItem, Documentation, MarkupContent, MarkupKind, CompletionItemKind};

use crate::lsp::features::traits::{
    LanguageAdapter, HoverProvider, CompletionProvider, DocumentationProvider,
    HoverContext, CompletionContext, DocumentationContext,
};
use crate::ir::semantic_node::SemanticNode;
use crate::ir::symbol_resolution::{
    SymbolResolver,
    ComposableSymbolResolver,
    PatternAwareContractResolver,
    lexical_scope::LexicalScopeResolver,
};
use crate::ir::symbol_table::SymbolTable;
use crate::ir::global_index::GlobalSymbolIndex;

/// Rholang-specific hover provider
pub struct RholangHoverProvider;

impl HoverProvider for RholangHoverProvider {
    fn hover_for_symbol(
        &self,
        symbol_name: &str,
        node: &dyn SemanticNode,
        context: &HoverContext,
    ) -> Option<HoverContents> {
        use crate::ir::transforms::documentation_attacher::DOC_METADATA_KEY;

        // Check for documentation in context first (may be from parent node)
        let doc_text = if let Some(ref doc) = context.documentation {
            Some(doc.as_str())
        } else {
            // Fall back to checking node metadata directly
            node.metadata()
                .and_then(|m| m.get(DOC_METADATA_KEY))
                .and_then(|doc_any| doc_any.downcast_ref::<String>())
                .map(|s| s.as_str())
        };

        // Format hover content with documentation if available
        let content = if let Some(doc) = doc_text {
            format!("**{}**\n\n{}\n\n---\n\n*Rholang symbol*", symbol_name, doc)
        } else {
            format!("**{}**\n\n*Rholang symbol*", symbol_name)
        };

        Some(HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: content,
        }))
    }
}

/// Rholang-specific completion provider
pub struct RholangCompletionProvider;

impl CompletionProvider for RholangCompletionProvider {
    fn complete_at(
        &self,
        _node: &dyn SemanticNode,
        _context: &CompletionContext,
    ) -> Vec<CompletionItem> {
        // Return Rholang keywords as completions
        self.keywords()
            .iter()
            .map(|&kw| CompletionItem {
                label: kw.to_string(),
                kind: Some(CompletionItemKind::KEYWORD),
                ..Default::default()
            })
            .collect()
    }

    fn keywords(&self) -> &[&str] {
        &[
            "contract",
            "new",
            "for",
            "match",
            "select",
            "if",
            "else",
            "let",
            "true",
            "false",
            "Nil",
        ]
    }
}

/// Rholang-specific documentation provider
pub struct RholangDocumentationProvider;

impl DocumentationProvider for RholangDocumentationProvider {
    fn documentation_for(
        &self,
        symbol_name: &str,
        _context: &DocumentationContext,
    ) -> Option<Documentation> {
        // Basic documentation lookup
        let doc_text = match symbol_name {
            "contract" => "Defines a new contract that can receive messages",
            "new" => "Creates new private names",
            "for" => "Pattern matches on channels and continues execution",
            "match" => "Pattern matches a process against cases",
            _ => return None,
        };

        Some(Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value: doc_text.to_string(),
        }))
    }
}

/// Rholang symbol resolver using traditional symbol table
///
/// This resolver performs lexical scope lookup in Rholang's hierarchical symbol table.
struct RholangSymbolResolver {
    symbol_table: Arc<SymbolTable>,
}

impl SymbolResolver for RholangSymbolResolver {
    fn resolve_symbol(
        &self,
        symbol_name: &str,
        _position: &crate::ir::semantic_node::Position,
        context: &crate::ir::symbol_resolution::ResolutionContext,
    ) -> Vec<crate::ir::symbol_resolution::SymbolLocation> {
        use crate::ir::symbol_resolution::{SymbolLocation, SymbolKind, ResolutionConfidence};
        use tower_lsp::lsp_types::{Position as LspPosition, Range};

        tracing::debug!(
            "RholangSymbolResolver: Looking up symbol '{}' (language={}, uri={})",
            symbol_name,
            context.language,
            context.uri
        );

        // Try to get symbol table from the IR node's metadata (for nested scopes)
        // Otherwise fall back to the root symbol table
        let symbol_table = if let Some(ir_node) = &context.ir_node {
            if let Some(node) = ir_node.downcast_ref::<Arc<crate::ir::rholang_node::RholangNode>>() {
                if let Some(metadata) = node.metadata() {
                    if let Some(table) = metadata.get("symbol_table") {
                        table.downcast_ref::<Arc<crate::ir::symbol_table::SymbolTable>>()
                            .map(|t| t.clone())
                            .unwrap_or_else(|| self.symbol_table.clone())
                    } else {
                        self.symbol_table.clone()
                    }
                } else {
                    self.symbol_table.clone()
                }
            } else {
                self.symbol_table.clone()
            }
        } else {
            self.symbol_table.clone()
        };

        // Look up symbol in the symbol table (walks parent chain automatically)
        if let Some(symbol) = symbol_table.lookup(symbol_name) {
            tracing::debug!(
                "Found symbol '{}' at line {}, col {} (type={:?}, uri={})",
                symbol.name,
                symbol.declaration_location.row,
                symbol.declaration_location.column,
                symbol.symbol_type,
                symbol.declaration_uri
            );

            // Determine symbol kind from Rholang symbol type
            let kind = match symbol.symbol_type {
                crate::ir::symbol_table::SymbolType::Contract => SymbolKind::Function,
                crate::ir::symbol_table::SymbolType::Variable => SymbolKind::Variable,
                crate::ir::symbol_table::SymbolType::Parameter => SymbolKind::Parameter,
                _ => SymbolKind::Other,
            };

            // For goto-definition: return only the definition location
            // (or declaration if no separate definition exists)
            let target_location = symbol.definition_location.as_ref().unwrap_or(&symbol.declaration_location);

            let lsp_pos = LspPosition {
                line: target_location.row as u32,
                character: target_location.column as u32,
            };
            let range = Range {
                start: lsp_pos,
                end: LspPosition {
                    line: lsp_pos.line,
                    character: lsp_pos.character + symbol.name.len() as u32,
                },
            };

            vec![SymbolLocation {
                uri: symbol.declaration_uri.clone(),
                range,
                kind,
                confidence: ResolutionConfidence::Exact,
                metadata: None,
            }]
        } else {
            tracing::debug!("Symbol '{}' not found in symbol table", symbol_name);
            // Symbol not found in scope
            Vec::new()
        }
    }

    fn supports_language(&self, language: &str) -> bool {
        language == "rholang"
    }
}

/// Create a Rholang language adapter with symbol table
///
/// # Arguments
/// * `symbol_table` - Symbol table for the Rholang document
///
/// # Returns
/// Configured LanguageAdapter for Rholang with working symbol resolution
///
/// # Implementation
/// Uses ComposableSymbolResolver with pattern-aware contract matching as primary
/// resolver and lexical scope lookup as fallback. This enables overload resolution
/// and parameter-aware goto-definition for contracts.
pub fn create_rholang_adapter(
    symbol_table: Arc<SymbolTable>,
    global_index: Arc<std::sync::RwLock<GlobalSymbolIndex>>,
) -> LanguageAdapter {
    // Create pattern-aware resolver (primary: contract pattern matching)
    let pattern_resolver = Box::new(PatternAwareContractResolver::new(
        global_index.clone()
    )) as Box<dyn SymbolResolver>;

    // Create lexical scope resolver (fallback: standard symbol table lookup)
    let lexical_resolver = Box::new(RholangSymbolResolver {
        symbol_table: symbol_table.clone()
    }) as Box<dyn SymbolResolver>;

    // Chain resolvers: pattern matching first, then lexical scope
    // This allows pattern matching to override for contracts while
    // falling back to normal symbol table for variables/channels
    let resolver: Arc<dyn SymbolResolver> = Arc::new(
        ComposableSymbolResolver::new(
            pattern_resolver,
            vec![], // No filters needed (pattern matching is in base)
            Some(lexical_resolver), // Fallback to lexical scope
        )
    );

    // Create providers
    let hover = Arc::new(RholangHoverProvider);
    let completion = Arc::new(RholangCompletionProvider);
    let documentation = Arc::new(RholangDocumentationProvider);

    // Bundle into adapter
    LanguageAdapter::new(
        "rholang",
        resolver,
        hover,
        completion,
        documentation,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::symbol_table::SymbolTable;

    #[test]
    fn test_create_rholang_adapter() {
        let symbol_table = Arc::new(SymbolTable::new(None));
        let global_index = Arc::new(std::sync::RwLock::new(GlobalSymbolIndex::new()));
        let adapter = create_rholang_adapter(symbol_table, global_index);

        assert_eq!(adapter.language_name(), "rholang");
    }

    #[test]
    fn test_rholang_completion_provider() {
        let provider = RholangCompletionProvider;
        let keywords = provider.keywords();

        assert!(keywords.contains(&"contract"));
        assert!(keywords.contains(&"new"));
        assert!(keywords.contains(&"for"));
    }

    #[test]
    fn test_rholang_documentation_provider() {
        use crate::ir::semantic_node::SemanticCategory;

        let provider = RholangDocumentationProvider;

        let context = DocumentationContext {
            language: "rholang".to_string(),
            category: SemanticCategory::Variable,
            qualified_name: None,
        };

        let doc = provider.documentation_for("contract", &context);
        assert!(doc.is_some());

        let doc = provider.documentation_for("unknown_symbol", &context);
        assert!(doc.is_none());
    }
}
