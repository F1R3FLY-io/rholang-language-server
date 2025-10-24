//! Generic symbol collector using SemanticNode and GenericVisitor
//!
//! This module demonstrates how symbol table building can work in a language-agnostic
//! way using the SemanticNode trait and GenericVisitor pattern. It serves as a
//! proof-of-concept for cross-language symbol analysis.
//!
//! Unlike the Rholang-specific SymbolTableBuilder, this collector works with any
//! language that implements SemanticNode (Rholang, MeTTa, etc.).

use std::collections::HashMap;
use tower_lsp::lsp_types::Url;

use crate::ir::rholang_node::{Position, RholangNode};
use crate::ir::metta_node::MettaNode;
use crate::ir::semantic_node::{GenericVisitor, SemanticCategory, SemanticNode};
use crate::ir::symbol_table::SymbolType;

/// Represents a collected symbol with language-agnostic information
#[derive(Debug, Clone)]
pub struct CollectedSymbol {
    /// Symbol name
    pub name: String,
    /// Symbol type (binding, variable, etc.)
    pub symbol_type: SymbolType,
    /// Source file URI
    pub uri: Url,
    /// Position in source
    pub location: Position,
    /// Semantic category
    pub category: SemanticCategory,
}

/// Generic symbol collector that works with any SemanticNode implementation
///
/// This collector demonstrates language-agnostic symbol collection using
/// semantic categories rather than variant matching.
#[derive(Debug)]
pub struct GenericSymbolCollector {
    /// URI of the document being processed
    uri: Url,
    /// Collected symbols
    symbols: Vec<CollectedSymbol>,
    /// Symbol references (name -> usage positions)
    references: HashMap<String, Vec<Position>>,
    /// Traversal depth (for debugging stack overflow)
    depth: usize,
    /// Maximum depth before bailing out
    max_depth: usize,
}

impl GenericSymbolCollector {
    /// Creates a new collector for the given document
    pub fn new(uri: Url) -> Self {
        Self {
            uri,
            symbols: Vec::new(),
            references: HashMap::new(),
            depth: 0,
            max_depth: 1000, // Generous limit to handle deep nesting while preventing stack overflow
        }
    }

    /// Returns the collected symbols
    pub fn symbols(&self) -> &[CollectedSymbol] {
        &self.symbols
    }

    /// Returns the symbol references
    pub fn references(&self) -> &HashMap<String, Vec<Position>> {
        &self.references
    }

    /// Extracts symbol name from a node (language-aware)
    fn extract_name(&self, node: &dyn SemanticNode) -> Option<String> {
        // Try Rholang
        if let Some(rho) = node.as_any().downcast_ref::<RholangNode>() {
            return match rho {
                RholangNode::Var { name, .. } => Some(name.clone()),
                RholangNode::NameDecl { var, .. } => {
                    if let RholangNode::Var { name, .. } = &**var {
                        Some(name.clone())
                    } else {
                        None
                    }
                }
                _ => None,
            };
        }

        // Try MeTTa
        if let Some(metta) = node.as_any().downcast_ref::<MettaNode>() {
            return metta.name().map(|s| s.to_string());
        }

        None
    }

    /// Computes absolute position for a node (simplified - uses delta as position)
    fn get_position(&self, node: &dyn SemanticNode) -> Position {
        let base = node.base();
        let rel = base.relative_start();
        // Simplified: treat relative position as absolute
        // In a full implementation, this would track parent positions
        Position {
            row: rel.delta_lines as usize,
            column: rel.delta_columns as usize,
            byte: rel.delta_bytes,
        }
    }
}

impl GenericVisitor for GenericSymbolCollector {
    /// Process binding nodes (variable declarations, function definitions, etc.)
    fn visit_binding(&mut self, node: &dyn SemanticNode) {
        if let Some(name) = self.extract_name(node) {
            if !name.is_empty() {
                let symbol = CollectedSymbol {
                    name: name.clone(),
                    symbol_type: SymbolType::Variable,
                    uri: self.uri.clone(),
                    location: self.get_position(node),
                    category: node.semantic_category(),
                };
                self.symbols.push(symbol);
                tracing::trace!("Collected binding: {}", name);
            }
        }

        // Continue traversal
        self.visit_children(node);
    }

    /// Process variable usage nodes
    fn visit_variable(&mut self, node: &dyn SemanticNode) {
        if let Some(name) = self.extract_name(node) {
            if !name.is_empty() {
                let location = self.get_position(node);
                self.references
                    .entry(name.clone())
                    .or_insert_with(Vec::new)
                    .push(location);
                tracing::trace!("Collected variable reference: {}", name);
            }
        }

        // Continue traversal
        self.visit_children(node);
    }

    /// Process invocation nodes (function calls, sends, etc.)
    fn visit_invocation(&mut self, node: &dyn SemanticNode) {
        // For invocations, we might want to record the target
        // For now, just traverse children
        self.visit_children(node);
    }

    /// Override the default visit_node to handle all categories
    fn visit_node(&mut self, node: &dyn SemanticNode) {
        // Check depth limit to prevent stack overflow
        if self.depth >= self.max_depth {
            eprintln!("WARNING: Maximum depth {} reached at node type: {}",
                     self.max_depth, node.type_name());
            eprintln!("  Category: {:?}, Children: {}",
                     node.semantic_category(), node.children_count());
            return;
        }

        self.depth += 1;

        // Print debug info for deep traversals
        if self.depth > 50 {
            eprintln!("Depth {}: Visiting {} (category: {:?}, children: {})",
                     self.depth, node.type_name(), node.semantic_category(),
                     node.children_count());
        }

        // Dispatch based on semantic category
        match node.semantic_category() {
            SemanticCategory::Binding => self.visit_binding(node),
            SemanticCategory::Variable => self.visit_variable(node),
            SemanticCategory::Invocation => self.visit_invocation(node),
            SemanticCategory::Literal => self.visit_literal(node),
            SemanticCategory::Collection => self.visit_collection(node),
            SemanticCategory::Match => self.visit_match(node),
            SemanticCategory::Conditional => self.visit_conditional(node),
            SemanticCategory::Block => self.visit_block(node),
            SemanticCategory::LanguageSpecific => {
                // For language-specific nodes, just traverse children
                self.visit_children(node);
            }
            SemanticCategory::Unknown => {
                // Unknown nodes - still traverse
                self.visit_children(node);
            }
        }

        self.depth -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree_sitter::{parse_code, parse_to_ir};
    use ropey::Rope;

    #[test]
    fn test_generic_symbol_collector_rholang() {
        let _ = crate::logging::init_logger(false, Some("warn"), false);

        let rho_code = r#"new x, y in { x!(42) | y!(100) }"#;
        let tree = parse_code(rho_code);
        let rope = Rope::from_str(rho_code);
        let ir = parse_to_ir(&tree, &rope);

        let uri = Url::parse("file:///test.rho").unwrap();
        let mut collector = GenericSymbolCollector::new(uri);

        // Visit the IR tree
        collector.visit_node(&*ir);

        // Check collected symbols
        println!("Collected {} symbols", collector.symbols().len());
        for symbol in collector.symbols() {
            println!("  Symbol: {} ({:?})", symbol.name, symbol.category);
        }

        // Should have found variable declarations
        assert!(
            collector.symbols().len() > 0,
            "Should have collected at least one symbol"
        );

        // Check references
        println!("Collected {} reference groups", collector.references().len());
        for (name, positions) in collector.references() {
            println!("  Ref '{}': {} usages", name, positions.len());
        }
    }

    #[test]
    fn test_generic_symbol_collector_with_unified_ir() {
        let _ = crate::logging::init_logger(false, Some("warn"), false);

        let rho_code = r#"new ch in { ch!(true, 42) }"#;
        let tree = parse_code(rho_code);
        let rope = Rope::from_str(rho_code);
        let rho_ir = parse_to_ir(&tree, &rope);

        // Convert to UnifiedIR
        use crate::ir::unified_ir::UnifiedIR;
        let unified_ir = UnifiedIR::from_rholang(&rho_ir);

        let uri = Url::parse("file:///test.rho").unwrap();
        let mut collector = GenericSymbolCollector::new(uri);

        // Visit the UnifiedIR tree
        collector.visit_node(&*unified_ir);

        println!(
            "Collected {} symbols from UnifiedIR",
            collector.symbols().len()
        );
        println!(
            "Collected {} reference groups from UnifiedIR",
            collector.references().len()
        );

        // UnifiedIR should work with GenericVisitor
        // Note: May collect fewer symbols due to RholangExt wrapping
    }
}
