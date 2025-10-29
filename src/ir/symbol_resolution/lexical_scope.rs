//! Default lexical scope resolver
//!
//! Implements standard lexical scoping with scope chain traversal.
//! Supports both MeTTa and Rholang symbol tables.

use std::sync::Arc;

use tower_lsp::lsp_types::{Range, Position as LspPosition};
use tracing::debug;

use crate::ir::semantic_node::Position;
use crate::ir::transforms::metta_symbol_table_builder::MettaSymbolTable;

use super::{
    SymbolResolver, SymbolLocation, ResolutionContext, ResolutionConfidence, SymbolKind,
};

/// Lexical scope resolver using MettaSymbolTable
///
/// Implements standard lexical scoping:
/// 1. Look up symbol in current scope
/// 2. Traverse parent scopes until found or reach global scope
/// 3. Return all matching symbols ordered by scope proximity
pub struct LexicalScopeResolver {
    /// The symbol table to query
    symbol_table: Arc<MettaSymbolTable>,
    /// Language this resolver handles
    language: String,
}

impl LexicalScopeResolver {
    /// Create a new lexical scope resolver
    pub fn new(symbol_table: Arc<MettaSymbolTable>, language: String) -> Self {
        Self {
            symbol_table,
            language,
        }
    }

    /// Find symbols in scope hierarchy
    fn find_in_scope_chain(
        &self,
        symbol_name: &str,
        scope_id: usize,
    ) -> Vec<SymbolLocation> {
        let mut locations = Vec::new();
        let mut current_scope_id = Some(scope_id);

        // Traverse scope chain from current to global
        while let Some(sid) = current_scope_id {
            if sid >= self.symbol_table.scopes.len() {
                break;
            }

            let scope = &self.symbol_table.scopes[sid];

            // Check if symbol exists in this scope
            if let Some(occurrences) = scope.symbols.get(symbol_name) {
                for occ in occurrences {
                    if occ.is_definition {
                        locations.push(SymbolLocation {
                            uri: self.symbol_table.uri.clone(),
                            range: occ.range,
                            kind: match occ.kind {
                                crate::ir::transforms::metta_symbol_table_builder::MettaSymbolKind::Variable => SymbolKind::Variable,
                                crate::ir::transforms::metta_symbol_table_builder::MettaSymbolKind::Definition => SymbolKind::Function,
                                crate::ir::transforms::metta_symbol_table_builder::MettaSymbolKind::Parameter => SymbolKind::Parameter,
                                crate::ir::transforms::metta_symbol_table_builder::MettaSymbolKind::LetBinding => SymbolKind::Variable,
                                crate::ir::transforms::metta_symbol_table_builder::MettaSymbolKind::MatchPattern => SymbolKind::Variable,
                            },
                            confidence: ResolutionConfidence::Exact,
                            metadata: None,
                        });
                    }
                }
            }

            // Move to parent scope
            current_scope_id = scope.parent_id;
        }

        locations
    }
}

impl SymbolResolver for LexicalScopeResolver {
    fn resolve_symbol(
        &self,
        symbol_name: &str,
        position: &Position,
        context: &ResolutionContext,
    ) -> Vec<SymbolLocation> {
        debug!(
            "LexicalScopeResolver: Resolving '{}' at {:?}",
            symbol_name, position
        );

        // Convert Position to LspPosition for symbol table lookup
        let lsp_pos = LspPosition {
            line: position.row as u32,
            character: position.column as u32,
        };

        // Find symbol at position to get scope_id
        let symbol = match self.symbol_table.find_symbol_at_position(&lsp_pos) {
            Some(sym) => sym,
            None => {
                // If we can't find symbol at position, try using context scope_id
                if let Some(scope_id) = context.scope_id {
                    return self.find_in_scope_chain(symbol_name, scope_id);
                }
                debug!("No symbol found at position and no scope_id in context");
                return Vec::new();
            }
        };

        // Use the symbol's scope to start the chain traversal
        self.find_in_scope_chain(symbol_name, symbol.scope_id)
    }

    fn supports_language(&self, language: &str) -> bool {
        self.language == language
    }

    fn name(&self) -> &'static str {
        "LexicalScopeResolver"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tests will be added when we integrate with actual symbol tables
}
