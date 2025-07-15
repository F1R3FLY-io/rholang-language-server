use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use rpds::Vector;
use archery::ArcK;

use tower_lsp::lsp_types::{DocumentSymbol, Range, SymbolKind, SymbolInformation, Location, Url};
use tracing::debug;

use crate::ir::node::{Metadata, Node, NodeBase, Position as IrPosition};
use crate::ir::symbol_table::{Symbol, SymbolTable, SymbolType};
use crate::ir::visitor::Visitor;

/// Collects hierarchical `DocumentSymbol`s from an IR tree for LSP document symbol requests.
#[derive(Debug)]
pub struct DocumentSymbolVisitor<'a> {
    positions: &'a HashMap<usize, (IrPosition, IrPosition)>, // Precomputed node positions
    symbols: RefCell<Vec<DocumentSymbol>>,                   // Accumulated symbols during traversal
}

impl<'a> DocumentSymbolVisitor<'a> {
    /// Creates a new visitor with a reference to precomputed node positions.
    pub fn new(positions: &'a HashMap<usize, (IrPosition, IrPosition)>) -> Self {
        Self {
            positions,
            symbols: RefCell::new(Vec::new()),
        }
    }

    /// Converts symbols from the current scope of a symbol table to DocumentSymbols.
    fn add_symbols_from_table(&self, table: &SymbolTable) -> Vec<DocumentSymbol> {
        table.current_symbols()
            .iter()
            .filter_map(|symbol| self.symbol_to_document_symbol(symbol.as_ref()))
            .collect()
    }

    /// Consumes the visitor and returns the collected symbols.
    pub fn into_symbols(self) -> Vec<DocumentSymbol> {
        self.symbols.into_inner()
    }

    /// Computes the LSP Range for a node using its precomputed positions.
    fn node_range(&self, node: &Arc<Node<'static>>) -> Range {
        if let Some(ts_node) = node.base().ts_node() {
            let node_id = ts_node.id();
            self.positions.get(&node_id).map_or_else(
                || {
                    debug!("No position found for node ID {}, using default range", node_id);
                    Range::default()
                },
                |(start, end)| Range {
                    start: tower_lsp::lsp_types::Position {
                        line: start.row as u32,
                        character: start.column as u32,
                    },
                    end: tower_lsp::lsp_types::Position {
                        line: end.row as u32,
                        character: end.column as u32,
                    },
                },
            )
        } else {
            debug!(
                "Node '{}' has no Tree-Sitter node, using default range",
                node.text()
            );
            Range::default()
        }
    }

    /// Converts a `Symbol` to a `DocumentSymbol` with an empty children vector, skipping empty names.
    fn symbol_to_document_symbol(&self, symbol: &Symbol) -> Option<DocumentSymbol> {
        if symbol.name.is_empty() {
            debug!("Skipping symbol with empty name at {:?}", symbol.declaration_location);
            return None;
        }
        let range = Range {
            start: tower_lsp::lsp_types::Position {
                line: symbol.declaration_location.row as u32,
                character: symbol.declaration_location.column as u32,
            },
            end: tower_lsp::lsp_types::Position {
                line: symbol.declaration_location.row as u32,
                character: (symbol.declaration_location.column + symbol.name.len()) as u32,
            },
        };
        let kind = match symbol.symbol_type {
            crate::ir::symbol_table::SymbolType::Variable => SymbolKind::VARIABLE,
            crate::ir::symbol_table::SymbolType::Contract => SymbolKind::FUNCTION,
            crate::ir::symbol_table::SymbolType::Parameter => SymbolKind::VARIABLE,
        };
        Some(DocumentSymbol {
            name: symbol.name.clone(),
            detail: None,
            range,
            selection_range: range,
            kind,
            tags: None,
            children: Some(vec![]),
            #[allow(deprecated)]
            deprecated: None,
        })
    }
}

impl<'a> Visitor for DocumentSymbolVisitor<'a> {
    fn visit_contract<'b>(
        &self,
        node: &Arc<Node<'b>>,
        _base: &NodeBase<'b>,
        name: &Arc<Node<'b>>,
        formals: &Vector<Arc<Node<'b>>, ArcK>,
        formals_remainder: &Option<Arc<Node<'b>>>,
        proc: &Arc<Node<'b>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'b>> {
        let static_node: Arc<Node<'static>> = unsafe { std::mem::transmute(node.clone()) };
        let range = self.node_range(&static_node);
        let static_name: Arc<Node<'static>> = unsafe { std::mem::transmute(name.clone()) };
        let selection_range = self.node_range(&static_name);
        let contract_name = if let Node::Var { name, .. } = &**name {
            name.clone()
        } else {
            "contract".to_string()
        };

        let visitor = DocumentSymbolVisitor::new(self.positions);
        for formal in formals {
            visitor.visit_node(formal);
        }
        if let Some(rem) = formals_remainder {
            visitor.visit_node(rem);
        }
        visitor.visit_node(proc);

        if let Some(metadata) = metadata {
            if let Some(symbol_table) = metadata
                .data
                .get("symbol_table")
                .and_then(|st| st.downcast_ref::<Arc<SymbolTable>>())
            {
                let params: Vec<_> = symbol_table.current_symbols().iter().map(|s| s.name.clone()).collect();
                debug!("Collecting symbols for contract '{}': parameters {:?}", contract_name, params);
                let mut contract_symbol = DocumentSymbol {
                    name: contract_name.clone(),
                    detail: None,
                    range,
                    selection_range,
                    kind: SymbolKind::FUNCTION,
                    tags: None,
                    children: Some(
                        symbol_table
                            .current_symbols()
                            .iter()
                            .filter_map(|s| self.symbol_to_document_symbol(s.as_ref()))
                            .collect(),
                    ),
                    #[allow(deprecated)]
                    deprecated: None,
                };
                let children = visitor.into_symbols();
                debug!("Adding {} child symbols from process body to '{}'", children.len(), contract_name);
                contract_symbol
                    .children
                    .as_mut()
                    .unwrap()
                    .extend(children);
                self.symbols.borrow_mut().push(contract_symbol);
            }
        }
        node.clone()
    }

    fn visit_new<'b>(
        &self,
        node: &Arc<Node<'b>>,
        _base: &NodeBase<'b>,
        decls: &Vector<Arc<Node<'b>>, ArcK>,
        proc: &Arc<Node<'b>>,
        _metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'b>> {
        let static_node: Arc<Node<'static>> = unsafe { std::mem::transmute(node.clone()) };
        let range = self.node_range(&static_node);
        let visitor = DocumentSymbolVisitor::new(self.positions);
        for decl in decls {
            visitor.visit_node(decl);
        }
        visitor.visit_node(proc);
        let children = visitor.into_symbols();

        let new_symbol = DocumentSymbol {
            name: "new".to_string(),
            detail: None,
            range,
            selection_range: range,
            kind: SymbolKind::NAMESPACE,
            tags: None,
            children: Some(children),
            #[allow(deprecated)]
            deprecated: None,
        };
        self.symbols.borrow_mut().push(new_symbol);
        node.clone()
    }

    fn visit_let<'b>(
        &self,
        node: &Arc<Node<'b>>,
        _base: &NodeBase<'b>,
        decls: &Vector<Arc<Node<'b>>, ArcK>,
        proc: &Arc<Node<'b>>,
        _metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'b>> {
        let static_node: Arc<Node<'static>> = unsafe { std::mem::transmute(node.clone()) };
        let range = self.node_range(&static_node);
        let visitor = DocumentSymbolVisitor::new(self.positions);
        for decl in decls {
            visitor.visit_node(decl);
        }
        visitor.visit_node(proc);
        let children = visitor.into_symbols();

        let let_symbol = DocumentSymbol {
            name: "let".to_string(),
            detail: None,
            range,
            selection_range: range,
            kind: SymbolKind::NAMESPACE,
            tags: None,
            children: Some(children),
            #[allow(deprecated)]
            deprecated: None,
        };
        self.symbols.borrow_mut().push(let_symbol);
        node.clone()
    }

    fn visit_name_decl<'b>(
        &self,
        node: &Arc<Node<'b>>,
        _base: &NodeBase<'b>,
        var: &Arc<Node<'b>>,
        _uri: &Option<Arc<Node<'b>>>,
        _metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'b>> {
        if let Node::Var { name, .. } = &**var {
            let static_node: Arc<Node<'static>> = unsafe { std::mem::transmute(node.clone()) };
            let range = self.node_range(&static_node);
            let symbol = DocumentSymbol {
                name: name.clone(),
                detail: None,
                range,
                selection_range: range,
                kind: SymbolKind::VARIABLE,
                tags: None,
                children: Some(vec![]),
                #[allow(deprecated)]
                deprecated: None,
            };
            debug!("Added variable symbol '{}' from NameDecl at {:?}", name, range.start);
            self.symbols.borrow_mut().push(symbol);
        }
        node.clone()
    }

    fn visit_decl<'b>(
        &self,
        node: &Arc<Node<'b>>,
        _base: &NodeBase<'b>,
        names: &Vector<Arc<Node<'b>>, ArcK>,
        names_remainder: &Option<Arc<Node<'b>>>,
        _procs: &Vector<Arc<Node<'b>>, ArcK>,
        _metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'b>> {
        for name in names {
            if let Node::Var { name: var_name, .. } = &**name {
                let static_name: Arc<Node<'static>> = unsafe { std::mem::transmute(name.clone()) };
                let range = self.node_range(&static_name);
                let symbol = DocumentSymbol {
                    name: var_name.clone(),
                    detail: None,
                    range,
                    selection_range: range,
                    kind: SymbolKind::VARIABLE,
                    tags: None,
                    children: Some(vec![]),
                    #[allow(deprecated)]
                    deprecated: None,
                };
                debug!("Added variable symbol '{}' from Decl at {:?}", var_name, range.start);
                self.symbols.borrow_mut().push(symbol);
            }
        }
        if let Some(rem) = names_remainder {
            if let Node::Var { name: var_name, .. } = &**rem {
                let static_rem: Arc<Node<'static>> = unsafe { std::mem::transmute(rem.clone()) };
                let range = self.node_range(&static_rem);
                let symbol = DocumentSymbol {
                    name: var_name.clone(),
                    detail: Some("(remainder)".to_string()),
                    range,
                    selection_range: range,
                    kind: SymbolKind::VARIABLE,
                    tags: None,
                    children: Some(vec![]),
                    #[allow(deprecated)]
                    deprecated: None,
                };
                debug!("Added remainder variable symbol '{}' from Decl at {:?}", var_name, range.start);
                self.symbols.borrow_mut().push(symbol);
            }
        }
        node.clone()
    }

    fn visit_input<'b>(
        &self,
        node: &Arc<Node<'b>>,
        _base: &NodeBase<'b>,
        _receipts: &Vector<Vector<Arc<Node<'b>>, ArcK>, ArcK>,
        proc: &Arc<Node<'b>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'b>> {
        let static_node: Arc<Node<'static>> = unsafe { std::mem::transmute(node.clone()) };
        let range = self.node_range(&static_node);
        let mut children = Vec::new();

        // Add bound variables from the symbol table
        if let Some(metadata) = metadata {
            if let Some(symbol_table) = metadata.data.get("symbol_table")
                .and_then(|st| st.downcast_ref::<Arc<SymbolTable>>()) {
                children.extend(self.add_symbols_from_table(symbol_table));
            }
        }

        // Visit process body for additional symbols
        let visitor = DocumentSymbolVisitor::new(self.positions);
        visitor.visit_node(proc);
        children.extend(visitor.into_symbols());

        let input_symbol = DocumentSymbol {
            name: "for".to_string(),
            detail: None,
            range,
            selection_range: range,
            kind: SymbolKind::NAMESPACE,
            tags: None,
            children: Some(children),
            #[allow(deprecated)]
            deprecated: None,
        };
        self.symbols.borrow_mut().push(input_symbol);
        node.clone()
    }

    fn visit_match<'b>(
        &self,
        node: &Arc<Node<'b>>,
        _base: &NodeBase<'b>,
        expression: &Arc<Node<'b>>,
        cases: &Vector<(Arc<Node<'b>>, Arc<Node<'b>>), ArcK>,
        _metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'b>> {
        let static_node: Arc<Node<'static>> = unsafe { std::mem::transmute(node.clone()) };
        let range = self.node_range(&static_node);
        let mut match_children = Vec::new();

        // Visit the expression
        let expr_visitor = DocumentSymbolVisitor::new(self.positions);
        expr_visitor.visit_node(expression);
        match_children.extend(expr_visitor.into_symbols());

        // Process each case
        for (i, (pattern, proc)) in cases.iter().enumerate() {
            // let static_pattern: Arc<Node<'static>> = unsafe { std::mem::transmute(pattern.clone()) };
            // let static_proc: Arc<Node<'static>> = unsafe { std::mem::transmute(proc.clone()) };
            let case_start = self.positions.get(&pattern.base().ts_node().unwrap().id()).unwrap().0;
            let case_end = self.positions.get(&proc.base().ts_node().unwrap().id()).unwrap().1;
            let case_range = Range {
                start: tower_lsp::lsp_types::Position {
                    line: case_start.row as u32,
                    character: case_start.column as u32,
                },
                end: tower_lsp::lsp_types::Position {
                    line: case_end.row as u32,
                    character: case_end.column as u32,
                },
            };

            let mut case_children = Vec::new();
            // Add bound variables from the process's symbol table
            if let Some(proc_metadata) = proc.metadata() {
                if let Some(symbol_table) = proc_metadata.data.get("symbol_table")
                    .and_then(|st| st.downcast_ref::<Arc<SymbolTable>>()) {
                    case_children.extend(self.add_symbols_from_table(symbol_table));
                }
            }

            // Visit the process for additional symbols
            let proc_visitor = DocumentSymbolVisitor::new(self.positions);
            proc_visitor.visit_node(proc);
            case_children.extend(proc_visitor.into_symbols());

            let case_symbol = DocumentSymbol {
                name: format!("case {}", i + 1),
                detail: None,
                range: case_range,
                selection_range: case_range,
                kind: SymbolKind::NAMESPACE,
                tags: None,
                children: Some(case_children),
                #[allow(deprecated)]
                deprecated: None,
            };
            match_children.push(case_symbol);
        }

        let match_symbol = DocumentSymbol {
            name: "match".to_string(),
            detail: None,
            range,
            selection_range: range,
            kind: SymbolKind::NAMESPACE,
            tags: None,
            children: Some(match_children),
            #[allow(deprecated)]
            deprecated: None,
        };
        self.symbols.borrow_mut().push(match_symbol);
        node.clone()
    }

    fn visit_choice<'b>(
        &self,
        node: &Arc<Node<'b>>,
        _base: &NodeBase<'b>,
        branches: &Vector<(Vector<Arc<Node<'b>>, ArcK>, Arc<Node<'b>>), ArcK>,
        _metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'b>> {
        let static_node: Arc<Node<'static>> = unsafe { std::mem::transmute(node.clone()) };
        let range = self.node_range(&static_node);
        let mut select_children = Vec::new();

        // Process each branch
        for (i, (inputs, proc)) in branches.iter().enumerate() {
            // let static_input: Arc<Node<'static>> = unsafe { std::mem::transmute(inputs[0].clone()) };
            // let static_proc: Arc<Node<'static>> = unsafe { std::mem::transmute(proc.clone()) };
            let branch_start = self.positions.get(&inputs[0].base().ts_node().unwrap().id()).unwrap().0;
            let branch_end = self.positions.get(&proc.base().ts_node().unwrap().id()).unwrap().1;
            let branch_range = Range {
                start: tower_lsp::lsp_types::Position {
                    line: branch_start.row as u32,
                    character: branch_start.column as u32,
                },
                end: tower_lsp::lsp_types::Position {
                    line: branch_end.row as u32,
                    character: branch_end.column as u32,
                },
            };

            let mut branch_children = Vec::new();
            // Add bound variables from the process's symbol table
            if let Some(proc_metadata) = proc.metadata() {
                if let Some(symbol_table) = proc_metadata.data.get("symbol_table")
                    .and_then(|st| st.downcast_ref::<Arc<SymbolTable>>()) {
                    branch_children.extend(self.add_symbols_from_table(symbol_table));
                }
            }

            // Visit the process for additional symbols
            let proc_visitor = DocumentSymbolVisitor::new(self.positions);
            proc_visitor.visit_node(proc);
            branch_children.extend(proc_visitor.into_symbols());

            let branch_symbol = DocumentSymbol {
                name: format!("branch {}", i + 1),
                detail: None,
                range: branch_range,
                selection_range: branch_range,
                kind: SymbolKind::NAMESPACE,
                tags: None,
                children: Some(branch_children),
                #[allow(deprecated)]
                deprecated: None,
            };
            select_children.push(branch_symbol);
        }

        let choice_symbol = DocumentSymbol {
            name: "select".to_string(),
            detail: None,
            range,
            selection_range: range,
            kind: SymbolKind::NAMESPACE,
            tags: None,
            children: Some(select_children),
            #[allow(deprecated)]
            deprecated: None,
        };
        self.symbols.borrow_mut().push(choice_symbol);
        node.clone()
    }

    fn visit_block<'b>(
        &self,
        node: &Arc<Node<'b>>,
        _base: &NodeBase<'b>,
        proc: &Arc<Node<'b>>,
        _metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'b>> {
        let visitor = DocumentSymbolVisitor::new(self.positions);
        visitor.visit_node(proc);
        self.symbols.borrow_mut().extend(visitor.into_symbols());
        node.clone()
    }

    fn visit_par<'b>(
        &self,
        node: &Arc<Node<'b>>,
        _base: &NodeBase<'b>,
        left: &Arc<Node<'b>>,
        right: &Arc<Node<'b>>,
        _metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'b>> {
        let visitor = DocumentSymbolVisitor::new(self.positions);
        visitor.visit_node(left);
        visitor.visit_node(right);
        self.symbols.borrow_mut().extend(visitor.into_symbols());
        node.clone()
    }
}

/// Collects document symbols using the visitor pattern.
/// Assumes `node` and `positions` have `'static` lifetimes from the backend processing.
pub fn collect_document_symbols(
    node: &Arc<Node<'static>>,
    positions: &HashMap<usize, (IrPosition, IrPosition)>,
) -> Vec<DocumentSymbol> {
    // Extend positions lifetime to 'static safely, as it's derived from a 'static tree
    let positions_static: &'static HashMap<usize, (IrPosition, IrPosition)> =
        unsafe { std::mem::transmute(positions) };
    let visitor = DocumentSymbolVisitor::new(positions_static);
    visitor.visit_node(node);
    visitor.into_symbols()
}

/// Collects all symbols from a SymbolTable as SymbolInformation for workspace-wide search.
pub fn collect_workspace_symbols(symbol_table: &SymbolTable, uri: &Url) -> Vec<SymbolInformation> {
    symbol_table.collect_all_symbols().into_iter()
        .filter(|symbol| symbol.declaration_uri == *uri && matches!(symbol.symbol_type, SymbolType::Contract)) // Only symbols defined in this document
        .map(|symbol| {
            let location = Location {
                uri: uri.clone(),
                range: Range {
                    start: tower_lsp::lsp_types::Position {
                        line: symbol.declaration_location.row as u32,
                        character: symbol.declaration_location.column as u32,
                    },
                    end: tower_lsp::lsp_types::Position {
                        line: symbol.declaration_location.row as u32,
                        character: (symbol.declaration_location.column + symbol.name.len()) as u32,
                    },
                },
            };
            let kind = match symbol.symbol_type {
                SymbolType::Variable => tower_lsp::lsp_types::SymbolKind::VARIABLE,
                SymbolType::Contract => tower_lsp::lsp_types::SymbolKind::FUNCTION,
                SymbolType::Parameter => tower_lsp::lsp_types::SymbolKind::VARIABLE,
            };
            debug!("Collected workspace symbol: {} at {:?}", symbol.name, location);
            SymbolInformation {
                name: symbol.name.clone(),
                kind,
                location,
                container_name: None,
                tags: None,
                #[allow(deprecated)]
                deprecated: None,
            }
        }).collect()
}
