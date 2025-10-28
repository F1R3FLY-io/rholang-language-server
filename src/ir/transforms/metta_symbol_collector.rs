//! Symbol collection for MeTTa documents
//!
//! Collects symbols from MeTTa IR for LSP document/workspace symbol features.

use std::sync::Arc;
use tower_lsp::lsp_types::{DocumentSymbol, SymbolKind, Range, Position as LspPosition};

use crate::ir::metta_node::MettaNode;
use crate::ir::semantic_node::SemanticNode;

/// Collect document symbols from MeTTa IR nodes
pub fn collect_metta_document_symbols(nodes: &[Arc<MettaNode>]) -> Vec<DocumentSymbol> {
    let mut symbols = Vec::new();

    for (index, node) in nodes.iter().enumerate() {
        if let Some(symbol) = collect_symbol_from_node(node, index) {
            symbols.push(symbol);
        }
    }

    symbols
}

/// Collect a symbol from a single MeTTa node
fn collect_symbol_from_node(node: &Arc<MettaNode>, index: usize) -> Option<DocumentSymbol> {
    match &**node {
        // Definitions: (= name body)
        MettaNode::Definition { pattern, body: _, .. } => {
            let name = extract_name(pattern).unwrap_or_else(|| format!("definition_{}", index));
            let range = node_to_range(node);

            Some(DocumentSymbol {
                name: name.clone(),
                detail: Some("MeTTa definition".to_string()),
                kind: SymbolKind::FUNCTION,
                range,
                selection_range: range,
                children: Some(vec![]),
                tags: None,
                deprecated: None,
            })
        }

        // Type annotations: (: expr type)
        MettaNode::TypeAnnotation { expr, .. } => {
            let name = extract_name(expr).unwrap_or_else(|| format!("typed_expr_{}", index));
            let range = node_to_range(node);

            Some(DocumentSymbol {
                name: name.clone(),
                detail: Some("Type annotation".to_string()),
                kind: SymbolKind::VARIABLE,
                range,
                selection_range: range,
                children: Some(vec![]),
                tags: None,
                deprecated: None,
            })
        }

        // Lambda: (lambda (params) body)
        MettaNode::Lambda { params, .. } => {
            let param_names = params.iter()
                .filter_map(|p| extract_name(p))
                .collect::<Vec<_>>()
                .join(", ");
            let name = format!("λ({})", param_names);
            let range = node_to_range(node);

            Some(DocumentSymbol {
                name,
                detail: Some("Lambda function".to_string()),
                kind: SymbolKind::FUNCTION,
                range,
                selection_range: range,
                children: Some(vec![]),
                tags: None,
                deprecated: None,
            })
        }

        // Let bindings: (let ((var val) ...) body)
        MettaNode::Let { bindings, .. } => {
            let binding_names = bindings.iter()
                .filter_map(|(var, _)| extract_name(var))
                .collect::<Vec<_>>()
                .join(", ");
            let name = format!("let {}", binding_names);
            let range = node_to_range(node);

            Some(DocumentSymbol {
                name,
                detail: Some("Let binding".to_string()),
                kind: SymbolKind::VARIABLE,
                range,
                selection_range: range,
                children: Some(vec![]),
                tags: None,
                deprecated: None,
            })
        }

        // Match expressions
        MettaNode::Match { scrutinee, .. } => {
            let scrutinee_name = extract_name(scrutinee).unwrap_or_else(|| "value".to_string());
            let name = format!("match {}", scrutinee_name);
            let range = node_to_range(node);

            Some(DocumentSymbol {
                name,
                detail: Some("Pattern match".to_string()),
                kind: SymbolKind::ENUM,
                range,
                selection_range: range,
                children: Some(vec![]),
                tags: None,
                deprecated: None,
            })
        }

        // For top-level atoms and s-expressions, only show if they look like definitions
        MettaNode::SExpr { elements, .. } if elements.len() >= 2 => {
            // Check if this looks like a function call or important expression
            if let Some(first) = elements.first() {
                if let Some(op_name) = extract_name(first) {
                    if is_important_operation(&op_name) {
                        let range = node_to_range(node);
                        return Some(DocumentSymbol {
                            name: format!("({} ...)", op_name),
                            detail: Some("Expression".to_string()),
                            kind: SymbolKind::FUNCTION,
                            range,
                            selection_range: range,
                            children: Some(vec![]),
                            tags: None,
                            deprecated: None,
                        });
                    }
                }
            }
            None
        }

        // Skip other node types from document symbols
        _ => None,
    }
}

/// Extract a name from a MeTTa node (for atoms and variables)
fn extract_name(node: &Arc<MettaNode>) -> Option<String> {
    match &**node {
        MettaNode::Atom { name, .. } => Some(name.clone()),
        MettaNode::Variable { name, .. } => Some(format!("${}", name)),
        _ => None,
    }
}

/// Check if an operation is important enough to show in symbols
fn is_important_operation(op: &str) -> bool {
    matches!(op, "import" | "pragma" | "module" | "define" | "defun")
}

/// Convert a MeTTa node to an LSP Range
fn node_to_range(node: &Arc<MettaNode>) -> Range {
    let base = node.base();
    let rel_start = base.relative_start();
    let start_line = rel_start.delta_lines.max(0) as u32;
    let start_char = rel_start.delta_columns.max(0) as u32;

    // Approximate end position (we don't have precise end positions yet)
    let end_line = start_line;
    let end_char = start_char + base.length() as u32;

    Range {
        start: LspPosition {
            line: start_line,
            character: start_char,
        },
        end: LspPosition {
            line: end_line,
            character: end_char,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::semantic_node::{NodeBase, RelativePosition};

    fn test_base() -> NodeBase {
        NodeBase::new_simple(
            RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 },
            10,
            0,
            10,
        )
    }

    #[test]
    fn test_collect_definition_symbol() {
        let pattern = Arc::new(MettaNode::Atom {
            base: test_base(),
            name: "factorial".to_string(),
            metadata: None,
        });
        let body = Arc::new(MettaNode::Integer {
            base: test_base(),
            value: 42,
            metadata: None,
        });
        let def = Arc::new(MettaNode::Definition {
            base: test_base(),
            pattern,
            body,
            metadata: None,
        });

        let symbol = collect_symbol_from_node(&def, 0);
        assert!(symbol.is_some());

        let sym = symbol.unwrap();
        assert_eq!(sym.name, "factorial");
        assert_eq!(sym.kind, SymbolKind::FUNCTION);
    }

    #[test]
    fn test_collect_lambda_symbol() {
        let param = Arc::new(MettaNode::Atom {
            base: test_base(),
            name: "x".to_string(),
            metadata: None,
        });
        let body = Arc::new(MettaNode::Atom {
            base: test_base(),
            name: "x".to_string(),
            metadata: None,
        });
        let lambda = Arc::new(MettaNode::Lambda {
            base: test_base(),
            params: vec![param],
            body,
            metadata: None,
        });

        let symbol = collect_symbol_from_node(&lambda, 0);
        assert!(symbol.is_some());

        let sym = symbol.unwrap();
        assert_eq!(sym.name, "λ(x)");
        assert_eq!(sym.kind, SymbolKind::FUNCTION);
    }

    #[test]
    fn test_collect_symbols_from_multiple_nodes() {
        let def1 = Arc::new(MettaNode::Definition {
            base: test_base(),
            pattern: Arc::new(MettaNode::Atom {
                base: test_base(),
                name: "foo".to_string(),
                metadata: None,
            }),
            body: Arc::new(MettaNode::Integer {
                base: test_base(),
                value: 1,
                metadata: None,
            }),
            metadata: None,
        });

        let def2 = Arc::new(MettaNode::Definition {
            base: test_base(),
            pattern: Arc::new(MettaNode::Atom {
                base: test_base(),
                name: "bar".to_string(),
                metadata: None,
            }),
            body: Arc::new(MettaNode::Integer {
                base: test_base(),
                value: 2,
                metadata: None,
            }),
            metadata: None,
        });

        let symbols = collect_metta_document_symbols(&[def1, def2]);
        assert_eq!(symbols.len(), 2);
        assert_eq!(symbols[0].name, "foo");
        assert_eq!(symbols[1].name, "bar");
    }
}
