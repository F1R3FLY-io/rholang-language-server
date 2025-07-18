use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use archery::ArcK;
use rpds::Vector;
use tower_lsp::lsp_types::Url;
use tracing::{debug, trace};

use crate::ir::node::{Metadata, Node, NodeBase, Position};
use crate::ir::symbol_table::{Symbol, SymbolTable, SymbolType};
use crate::ir::visitor::Visitor;

/// Maps symbol declaration positions to their usage locations within a file.
pub type InvertedIndex = HashMap<Position, Vec<Position>>;

/// Builds hierarchical symbol tables and an inverted index for Rholang IR trees.
/// Manages scope creation for nodes like `new`, `let`, `contract`, `input`, `case`, and `branch`.
#[derive(Debug)]
pub struct SymbolTableBuilder {
    root: Arc<Node<'static>>,  // Root IR node with static lifetime
    current_uri: Url,          // URI of the current file
    current_table: RwLock<Arc<SymbolTable>>,  // Current scope's symbol table
    inverted_index: RwLock<InvertedIndex>,    // Tracks local symbol usages
    potential_global_refs: RwLock<Vec<(String, Position)>>,  // Potential unresolved global contract calls (name, use_pos)
    global_table: Arc<SymbolTable>,  // Global scope for cross-file symbols (passed but not used as parent)
}

impl SymbolTableBuilder {
    /// Creates a new builder with a root IR node, file URI, and global symbol table.
    pub fn new(root: Arc<Node<'static>>, uri: Url, global_table: Arc<SymbolTable>) -> Self {
        let local_table = Arc::new(SymbolTable::new(Some(global_table.clone())));  // Chain local to global
        Self {
            root,
            current_uri: uri,
            current_table: RwLock::new(local_table),
            inverted_index: RwLock::new(HashMap::new()),
            potential_global_refs: RwLock::new(Vec::new()),
            global_table,
        }
    }

    /// Returns a clone of the local inverted index.
    pub fn get_inverted_index(&self) -> InvertedIndex {
        self.inverted_index.read().unwrap().clone()
    }

    /// Returns a clone of the potential global references.
    pub fn get_potential_global_refs(&self) -> Vec<(String, Position)> {
        self.potential_global_refs.read().unwrap().clone()
    }

    /// Resolves potentials that can be bound locally after full traversal (e.g., forward references in same file).
    pub fn resolve_local_potentials(&self, symbol_table: &Arc<SymbolTable>) {
        let potentials = self.potential_global_refs.write().expect("Failed to lock potential_global_refs").clone();
        let mut to_remove = Vec::new();
        for (i, (name, use_pos)) in potentials.iter().enumerate() {
            if let Some(symbol) = symbol_table.lookup(name) {
                if symbol.declaration_uri == self.current_uri {
                    self.inverted_index.write().expect("Failed to lock inverted_index")
                        .entry(symbol.declaration_location)
                        .or_insert_with(Vec::new)
                        .push(*use_pos);
                    trace!("Resolved local potential '{}' at {:?}", name, use_pos);
                    to_remove.push(i);
                }
            }
        }
        let mut potentials_guard = self.potential_global_refs.write().expect("Failed to lock potential_global_refs");
        for &i in to_remove.iter().rev() {
            potentials_guard.swap_remove(i);
        }
    }

    /// Pushes a new scope onto the stack, linking it to the current scope as its parent.
    fn push_scope(&self) -> Arc<SymbolTable> {
        let current = self.current_table.read().expect("Failed to lock current_table").clone();
        let new_table = Arc::new(SymbolTable::new(Some(current)));
        *self.current_table.write().expect("Failed to lock current_table") = new_table.clone();
        trace!("Pushed new scope");
        new_table
    }

    /// Pops the current scope, reverting to its parent if one exists.
    fn pop_scope(&self) {
        let current = self.current_table.read().expect("Failed to lock current_table").clone();
        if let Some(parent) = current.parent() {
            *self.current_table.write().expect("Failed to lock current_table") = parent;
            trace!("Popped scope");
        } else {
            trace!("No parent scope to pop to; retaining current scope");
        }
    }

    /// Updates a node's metadata with a specific symbol table and optional symbol.
    fn update_metadata<'b>(
        &self,
        node: Arc<Node<'b>>,
        table: Arc<SymbolTable>,
        symbol: Option<Arc<Symbol>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'b>> {
        let mut data = metadata.as_ref().map_or(HashMap::new(), |m| m.data.clone());
        data.insert("symbol_table".to_string(), Arc::new(table) as Arc<dyn Any + Send + Sync>);
        if let Some(sym) = symbol {
            data.insert("referenced_symbol".to_string(), Arc::new(sym) as Arc<dyn Any + Send + Sync>);
        }
        node.with_metadata(Some(Arc::new(Metadata { data })))
    }

    /// Updates a node's metadata with the current symbol table and optional symbol.
    fn update_with_current_table<'b>(
        &self,
        node: Arc<Node<'b>>,
        symbol: Option<Arc<Symbol>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'b>> {
        let current_table = self.current_table.read().expect("Failed to lock current_table").clone();
        self.update_metadata(node, current_table, symbol, metadata)
    }

    /// Collects variables bound in pattern nodes (e.g., in `match` cases).
    fn collect_bound_vars<'b>(&self, pattern: &'b Arc<Node<'b>>) -> Vec<Arc<Node<'b>>> {
        match &**pattern {
            Node::Var { .. } => vec![pattern.clone()],
            Node::Wildcard { .. } => vec![],
            Node::Tuple { elements, .. } => elements.iter().flat_map(|e| self.collect_bound_vars(e)).collect(),
            Node::List { elements, remainder, .. } => {
                let mut vars: Vec<_> = elements.iter().flat_map(|e| self.collect_bound_vars(e)).collect();
                if let Some(rem) = remainder {
                    vars.extend(self.collect_bound_vars(rem));
                }
                vars
            }
            Node::Set { elements, remainder, .. } => {
                let mut vars: Vec<_> = elements.iter().flat_map(|e| self.collect_bound_vars(e)).collect();
                if let Some(rem) = remainder {
                    vars.extend(self.collect_bound_vars(rem));
                }
                vars
            }
            Node::Map { pairs, remainder, .. } => {
                let mut vars: Vec<_> = pairs.iter().flat_map(|(k, v)| {
                    self.collect_bound_vars(k).into_iter().chain(self.collect_bound_vars(v))
                }).collect();
                if let Some(rem) = remainder {
                    vars.extend(self.collect_bound_vars(rem));
                }
                vars
            }
            Node::Quote { quotable, .. } => self.collect_bound_vars(quotable),
            Node::Disjunction { left, right, .. } => {
                let mut vars = self.collect_bound_vars(left);
                vars.extend(self.collect_bound_vars(right));
                vars
            }
            Node::Conjunction { left, right, .. } => {
                let mut vars = self.collect_bound_vars(left);
                vars.extend(self.collect_bound_vars(right));
                vars
            }
            Node::Negation { operand, .. } => self.collect_bound_vars(operand),
            Node::Parenthesized { expr, .. } => self.collect_bound_vars(expr),
            _ => vec![],
        }
    }

}

fn traverse<'a>(
    node: &Arc<Node<'a>>,
    position: Position,
    positions: &HashMap<usize, (Position, Position)>,
    best: &mut Option<(Arc<Node<'a>>, Position, usize)>,
    depth: usize,
) {
    let key = node.base().ts_node().map_or(0, |n| n.id());
    if let Some(&(start, end)) = positions.get(&key) {
        if start.byte <= position.byte && position.byte <= end.byte {
            // let size = (end.byte - start.byte) as usize;
            let is_better = best.as_ref().map_or(true, |(_, _, b_depth)| depth > *b_depth);
            if is_better {
                *best = Some((node.clone(), start, depth));
            }
        }
    }
    match &**node {
        Node::Par { left, right, .. } => {
            traverse(left, position, positions, best, depth + 1);
            traverse(right, position, positions, best, depth + 1);
        }
        Node::SendSync { channel, inputs, cont, .. } => {
            traverse(channel, position, positions, best, depth + 1);
            for input in inputs {
                traverse(input, position, positions, best, depth + 1);
            }
            traverse(cont, position, positions, best, depth + 1);
        }
        Node::Send { channel, inputs, .. } => {
            traverse(channel, position, positions, best, depth + 1);
            for input in inputs {
                traverse(input, position, positions, best, depth + 1);
            }
        }
        Node::New { decls, proc, .. } => {
            for decl in decls {
                traverse(decl, position, positions, best, depth + 1);
            }
            traverse(proc, position, positions, best, depth + 1);
        }
        Node::IfElse { condition, consequence, alternative, .. } => {
            traverse(condition, position, positions, best, depth + 1);
            traverse(consequence, position, positions, best, depth + 1);
            if let Some(alt) = alternative {
                traverse(alt, position, positions, best, depth + 1);
            }
        }
        Node::Let { decls, proc, .. } => {
            for decl in decls {
                traverse(decl, position, positions, best, depth + 1);
            }
            traverse(proc, position, positions, best, depth + 1);
        }
        Node::Bundle { proc, .. } => traverse(proc, position, positions, best, depth + 1),
        Node::Match { expression, cases, .. } => {
            traverse(expression, position, positions, best, depth + 1);
            for (pat, proc) in cases {
                traverse(pat, position, positions, best, depth + 1);
                traverse(proc, position, positions, best, depth + 1);
            }
        }
        Node::Choice { branches, .. } => {
            for (inputs, proc) in branches {
                for input in inputs {
                    traverse(input, position, positions, best, depth + 1);
                }
                traverse(proc, position, positions, best, depth + 1);
            }
        }
        Node::Contract { name, formals, formals_remainder, proc, .. } => {
            traverse(name, position, positions, best, depth + 1);
            for formal in formals {
                traverse(formal, position, positions, best, depth + 1);
            }
            if let Some(rem) = formals_remainder {
                traverse(rem, position, positions, best, depth + 1);
            }
            traverse(proc, position, positions, best, depth + 1);
        }
        Node::Input { receipts, proc, .. } => {
            for receipt in receipts {
                for bind in receipt {
                    traverse(bind, position, positions, best, depth + 1);
                }
            }
            traverse(proc, position, positions, best, depth + 1);
        }
        Node::Block { proc, .. } => traverse(proc, position, positions, best, depth + 1),
        Node::Parenthesized { expr, .. } => traverse(expr, position, positions, best, depth + 1),
        Node::BinOp { left, right, .. } => {
            traverse(left, position, positions, best, depth + 1);
            traverse(right, position, positions, best, depth + 1);
        }
        Node::UnaryOp { operand, .. } => traverse(operand, position, positions, best, depth + 1),
        Node::Method { receiver, args, .. } => {
            traverse(receiver, position, positions, best, depth + 1);
            for arg in args {
                traverse(arg, position, positions, best, depth + 1);
            }
        }
        Node::Eval { name, .. } => traverse(name, position, positions, best, depth + 1),
        Node::Quote { quotable, .. } => traverse(quotable, position, positions, best, depth + 1),
        Node::VarRef { var, .. } => traverse(var, position, positions, best, depth + 1),
        Node::List { elements, remainder, .. } => {
            for elem in elements {
                traverse(elem, position, positions, best, depth + 1);
            }
            if let Some(rem) = remainder {
                traverse(rem, position, positions, best, depth + 1);
            }
        }
        Node::Set { elements, remainder, .. } => {
            for elem in elements {
                traverse(elem, position, positions, best, depth + 1);
            }
            if let Some(rem) = remainder {
                traverse(rem, position, positions, best, depth + 1);
            }
        }
        Node::Map { pairs, remainder, .. } => {
            for (key, value) in pairs {
                traverse(key, position, positions, best, depth + 1);
                traverse(value, position, positions, best, depth + 1);
            }
            if let Some(rem) = remainder {
                traverse(rem, position, positions, best, depth + 1);
            }
        }
        Node::Tuple { elements, .. } => {
            for elem in elements {
                traverse(elem, position, positions, best, depth + 1);
            }
        }
        Node::NameDecl { var, uri, .. } => {
            traverse(var, position, positions, best, depth + 1);
            if let Some(u) = uri {
                traverse(u, position, positions, best, depth + 1);
            }
        }
        Node::Decl { names, names_remainder, procs, .. } => {
            for name in names {
                traverse(name, position, positions, best, depth + 1);
            }
            if let Some(rem) = names_remainder {
                traverse(rem, position, positions, best, depth + 1);
            }
            for proc in procs {
                traverse(proc, position, positions, best, depth + 1);
            }
        }
        Node::LinearBind { names, remainder, source, .. } => {
            for name in names {
                traverse(name, position, positions, best, depth + 1);
            }
            if let Some(rem) = remainder {
                traverse(rem, position, positions, best, depth + 1);
            }
            traverse(source, position, positions, best, depth + 1);
        }
        Node::RepeatedBind { names, remainder, source, .. } => {
            for name in names {
                traverse(name, position, positions, best, depth + 1);
            }
            if let Some(rem) = remainder {
                traverse(rem, position, positions, best, depth + 1);
            }
            traverse(source, position, positions, best, depth + 1);
        }
        Node::PeekBind { names, remainder, source, .. } => {
            for name in names {
                traverse(name, position, positions, best, depth + 1);
            }
            if let Some(rem) = remainder {
                traverse(rem, position, positions, best, depth + 1);
            }
            traverse(source, position, positions, best, depth + 1);
        }
        Node::ReceiveSendSource { name, .. } => traverse(name, position, positions, best, depth + 1),
        Node::SendReceiveSource { name, inputs, .. } => {
            traverse(name, position, positions, best, depth + 1);
            for input in inputs {
                traverse(input, position, positions, best, depth + 1);
            }
        }
        Node::Error { children, .. } => {
            for child in children {
                traverse(child, position, positions, best, depth + 1);
            }
        }
        Node::Disjunction { left, right, .. } => {
            traverse(left, position, positions, best, depth + 1);
            traverse(right, position, positions, best, depth + 1);
        }
        Node::Conjunction { left, right, .. } => {
            traverse(left, position, positions, best, depth + 1);
            traverse(right, position, positions, best, depth + 1);
        }
        Node::Negation { operand, .. } => traverse(operand, position, positions, best, depth + 1),
        _ => {},
    }
}

pub fn find_node_at_position<'a>(root: &Arc<Node<'a>>, positions: &HashMap<usize, (Position, Position)>, position: Position) -> Option<Arc<Node<'a>>> {
    let mut best: Option<(Arc<Node<'a>>, Position, usize)> = None;
    traverse(root, position, positions, &mut best, 0);
    if let Some(node) = best.map(|(node, _, _)| node) {
        debug!("Found best match: '{}'", node.text());
        Some(node)
    } else {
        debug!("No node found at position {:?}", position);
        None
    }
}

impl Visitor for SymbolTableBuilder {
    fn visit_par<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        left: &Arc<Node<'a>>,
        right: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_left = self.visit_node(left);
        let new_right = self.visit_node(right);
        let new_node = Arc::new(Node::Par {
            base: base.clone(),
            left: new_left,
            right: new_right,
            metadata: metadata.clone(),
        });
        self.update_with_current_table(new_node, None, metadata)
    }

    /// Visits a `new` node, ensuring declarations are added to the symbol table before processing.
    fn visit_new<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        decls: &Vector<Arc<Node<'a>>, ArcK>,
        proc: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_table = self.push_scope();
        for d in decls {
            if let Node::NameDecl { var, .. } = &**d {
                if let Node::Var { name, .. } = &**var {
                    if !name.is_empty() {  // Skip empty variable names
                        let location = var.absolute_start(&self.root);
                        let symbol = Arc::new(Symbol::new(
                            name.clone(),
                            SymbolType::Variable,
                            self.current_uri.clone(),
                            location,
                        ));
                        new_table.insert(symbol);
                        trace!("Declared variable '{}' in new scope at {:?}", name, location);
                    } else {
                        trace!("Skipped empty variable name in new declaration at {:?}", var.absolute_start(&self.root));
                    }
                }
            }
        }
        let new_decls = decls.iter().map(|d| self.visit_node(d)).collect();
        let new_proc = self.visit_node(proc);
        let new_node = Arc::new(Node::New {
            base: base.clone(),
            decls: new_decls,
            proc: new_proc,
            metadata: metadata.clone(),
        });
        let updated_node = self.update_metadata(new_node, new_table.clone(), None, metadata);
        self.pop_scope();
        updated_node
    }

    /// Visits a `let` node, adding declarations to the symbol table before processing.
    fn visit_let<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        decls: &Vector<Arc<Node<'a>>, ArcK>,
        proc: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let outer_table = self.current_table.read().unwrap().clone();
        let new_table = self.push_scope();
        for d in decls {
            if let Node::Decl { names, names_remainder, procs, .. } = &**d {
                for (name, proc) in names.iter().zip(procs.iter()) {
                    if let Node::Var { name: var_name, .. } = &**name {
                        if !var_name.is_empty() {  // Skip empty variable names
                            let decl_loc = name.absolute_start(&self.root);
                            let def_loc = proc.absolute_start(&self.root);
                            let symbol = Arc::new(Symbol {
                                name: var_name.clone(),
                                symbol_type: SymbolType::Variable,
                                declaration_uri: self.current_uri.clone(),
                                declaration_location: decl_loc,
                                definition_location: Some(def_loc),
                            });
                            new_table.insert(symbol);
                            trace!("Declared variable '{}' in let scope at {:?}", var_name, decl_loc);
                        } else {
                            trace!("Skipped empty variable name in let declaration at {:?}", name.absolute_start(&self.root));
                        }
                    }
                }
                if let Some(rem) = names_remainder {
                    if let Node::Var { name: var_name, .. } = &**rem {
                        if !var_name.is_empty() {
                            let decl_loc = rem.absolute_start(&self.root);
                            let symbol = Arc::new(Symbol::new(
                                var_name.clone(),
                                SymbolType::Variable,
                                self.current_uri.clone(),
                                decl_loc,
                            ));
                            new_table.insert(symbol);
                            trace!("Declared remainder variable '{}' in let scope at {:?}", var_name, decl_loc);
                        }
                    }
                }
            }
        }
        let new_decls = decls.iter().map(|d| {
            if let Node::Decl { names, names_remainder, procs, base: decl_base, metadata: decl_metadata } = &**d {
                let new_names = names.iter().map(|n| self.visit_node(n)).collect::<Vector<_, ArcK>>();
                let new_names_remainder = names_remainder.as_ref().map(|r| self.visit_node(r));
                let new_procs = procs.iter().map(|p| {
                    let prev_table = self.current_table.write().unwrap().clone();
                    *self.current_table.write().unwrap() = outer_table.clone();
                    let new_p = self.visit_node(p);
                    *self.current_table.write().unwrap() = prev_table;
                    new_p
                }).collect::<Vector<_, ArcK>>();
                Arc::new(Node::Decl {
                    base: decl_base.clone(),
                    names: new_names,
                    names_remainder: new_names_remainder,
                    procs: new_procs,
                    metadata: decl_metadata.clone(),
                })
            } else {
                self.visit_node(d)
            }
        }).collect::<Vector<_, ArcK>>();
        let new_proc = self.visit_node(proc);
        let new_node = Arc::new(Node::Let {
            base: base.clone(),
            decls: new_decls,
            proc: new_proc,
            metadata: metadata.clone(),
        });
        let updated_node = self.update_metadata(new_node, new_table.clone(), None, metadata);
        self.pop_scope();
        updated_node
    }

    /// Visits a `contract` node, registering the contract globally and parameters locally.
    fn visit_contract<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        name: &Arc<Node<'a>>,
        formals: &Vector<Arc<Node<'a>>, ArcK>,
        formals_remainder: &Option<Arc<Node<'a>>>,
        proc: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let contract_pos = name.absolute_start(&self.root);
        let contract_name = if let Node::Var { name, .. } = &**name {
            name.clone()
        } else {
            String::new()
        };

        let symbol = if !contract_name.is_empty() {
            #[allow(unused_assignments)]
            let mut symbol_opt: Option<Arc<Symbol>> = None;
            {
                let current_table_guard = self.current_table.read().unwrap();
                // let is_top_level = current_table_guard.parent().as_ref().map_or(false, |p| p.parent().is_none());
                if let Some(existing) = current_table_guard.lookup(&contract_name) {
                    trace!("Updating variable '{}' to contract at {:?}", contract_name, contract_pos);
                    let updated = Arc::new(Symbol {
                        name: existing.name.clone(),
                        symbol_type: SymbolType::Contract,
                        declaration_uri: existing.declaration_uri.clone(),
                        declaration_location: existing.declaration_location,
                        definition_location: Some(contract_pos),
                    });
                    symbol_opt = Some(updated);
                } else {
                    trace!("Declaring new contract '{}' at {:?}", contract_name, contract_pos);
                    let symbol = Arc::new(Symbol {
                        name: contract_name.clone(),
                        symbol_type: SymbolType::Contract,
                        declaration_uri: self.current_uri.clone(),
                        declaration_location: contract_pos,
                        definition_location: Some(contract_pos),
                    });
                    symbol_opt = Some(symbol);
                }
            }
            if let Some(symbol) = symbol_opt {
                let current_table_guard = self.current_table.read().unwrap();
                let is_top_level = current_table_guard.parent().as_ref().map_or(false, |p| p.parent().is_none());
                drop(current_table_guard);
                let insert_table = if is_top_level { self.global_table.clone() } else { self.current_table.read().unwrap().clone() };
                insert_table.symbols.write().unwrap().insert(contract_name.clone(), symbol.clone());
                Some(symbol)
            } else {
                None
            }
        } else {
            trace!("Skipped empty contract name at {:?}", contract_pos);
            None
        };

        let new_name = self.visit_node(name);

        let new_table = self.push_scope();
        for f in formals {
            if let Node::Var { name, .. } = &**f {
                if !name.is_empty() {  // Skip empty parameter names
                    let location = f.absolute_start(&self.root);
                    let symbol = Arc::new(Symbol::new(
                        name.clone(),
                        SymbolType::Parameter,
                        self.current_uri.clone(),
                        location,
                    ));
                    new_table.insert(symbol);
                    trace!("Declared parameter '{}' in contract scope at {:?}", name, location);
                } else {
                    trace!("Skipped empty parameter name in contract formals at {:?}", f.absolute_start(&self.root));
                }
            }
        }
        if let Some(rem) = formals_remainder {
            if let Node::Var { name, .. } = &**rem {
                if !name.is_empty() {
                    let location = rem.absolute_start(&self.root);
                    let symbol = Arc::new(Symbol::new(
                        name.clone(),
                        SymbolType::Parameter,
                        self.current_uri.clone(),
                        location,
                    ));
                    new_table.insert(symbol);
                    trace!("Declared remainder parameter '{}' in contract scope at {:?}", name, location);
                }
            }
        }

        let new_formals = formals.iter().map(|f| self.visit_node(f)).collect();
        let new_formals_remainder = formals_remainder.as_ref().map(|r| self.visit_node(r));
        let new_proc = self.visit_node(proc);

        let new_node = Arc::new(Node::Contract {
            base: base.clone(),
            name: new_name,
            formals: new_formals,
            formals_remainder: new_formals_remainder,
            proc: new_proc,
            metadata: metadata.clone(),
        });
        let updated_node = self.update_metadata(new_node, new_table.clone(), symbol, metadata);
        self.pop_scope();
        updated_node
    }

    /// Visits an `input` node, adding bindings to the symbol table before processing.
    fn visit_input<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        receipts: &Vector<Vector<Arc<Node<'a>>, ArcK>, ArcK>,
        proc: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_table = self.push_scope();
        for r in receipts {
            for b in r {
                match &**b {
                    Node::LinearBind { names, remainder, .. } | Node::RepeatedBind { names, remainder, .. } | Node::PeekBind { names, remainder, .. } => {
                        for name in names {
                            let bound_vars = self.collect_bound_vars(name);
                            for var in bound_vars {
                                if let Node::Var { name: var_name, .. } = &*var {
                                    if !var_name.is_empty() {  // Skip empty variable names
                                        let location = var.absolute_start(&self.root);
                                        let symbol = Arc::new(Symbol::new(
                                            var_name.clone(),
                                            SymbolType::Variable,
                                            self.current_uri.clone(),
                                            location,
                                        ));
                                        new_table.insert(symbol);
                                        trace!("Declared variable '{}' in input scope at {:?}", var_name, location);
                                    } else {
                                        trace!("Skipped empty variable name in input binding at {:?}", var.absolute_start(&self.root));
                                    }
                                }
                            }
                        }
                        if let Some(rem) = remainder {
                            if let Node::Var { name: var_name, .. } = &**rem {
                                if !var_name.is_empty() {
                                    let location = rem.absolute_start(&self.root);
                                    let symbol = Arc::new(Symbol::new(
                                        var_name.clone(),
                                        SymbolType::Variable,
                                        self.current_uri.clone(),
                                        location,
                                    ));
                                    new_table.insert(symbol);
                                    trace!("Declared remainder variable '{}' in input scope at {:?}", var_name, location);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        let new_receipts = receipts.iter().map(|r| {
            r.iter().map(|b| self.visit_node(b)).collect()
        }).collect();
        let new_proc = self.visit_node(proc);
        let new_node = Arc::new(Node::Input {
            base: base.clone(),
            receipts: new_receipts,
            proc: new_proc,
            metadata: metadata.clone(),
        });
        let updated_node = self.update_metadata(new_node, new_table.clone(), None, metadata);
        self.pop_scope();
        updated_node
    }

    /// Visits a `match` node, adding pattern variables to the symbol table before processing cases.
    fn visit_match<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        expression: &Arc<Node<'a>>,
        cases: &Vector<(Arc<Node<'a>>, Arc<Node<'a>>), ArcK>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_expression = self.visit_node(expression);
        let new_cases = cases.iter().map(|(pattern, proc)| {
            let new_table = self.push_scope();
            let bound_vars = self.collect_bound_vars(pattern);
            for var in bound_vars {
                if let Node::Var { name, .. } = &*var {
                    if !name.is_empty() {  // Skip empty variable names
                        let location = var.absolute_start(&self.root);
                        let symbol = Arc::new(Symbol::new(
                            name.clone(),
                            SymbolType::Variable,
                            self.current_uri.clone(),
                            location,
                        ));
                        new_table.insert(symbol);
                        trace!("Declared variable '{}' in match case scope at {:?}", name, location);
                    } else {
                        trace!("Skipped empty variable name in match pattern at {:?}", var.absolute_start(&self.root));
                    }
                }
            }
            let new_pattern = self.visit_node(pattern);
            let new_proc = self.visit_node(proc);
            let case_node = Arc::new(Node::Match {
                base: base.clone(),
                expression: new_expression.clone(),
                cases: Vector::new_with_ptr_kind().push_back((new_pattern.clone(), new_proc.clone())),
                metadata: metadata.clone(),
            });
            let _updated_case = self.update_metadata(case_node, new_table.clone(), None, metadata);
            self.pop_scope();
            (new_pattern, new_proc)
        }).collect();

        let new_node = Arc::new(Node::Match {
            base: base.clone(),
            expression: new_expression,
            cases: new_cases,
            metadata: metadata.clone(),
        });
        self.update_with_current_table(new_node, None, metadata)
    }

    /// Visits a `choice` node, adding input variables to the symbol table before processing branches.
    fn visit_choice<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        branches: &Vector<(Vector<Arc<Node<'a>>, ArcK>, Arc<Node<'a>>), ArcK>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_branches = branches.iter().map(|(inputs, proc)| {
            let new_table = self.push_scope();
            for i in inputs {
                if let Node::LinearBind { names, remainder, .. } | Node::RepeatedBind { names, remainder, .. } | Node::PeekBind { names, remainder, .. } = &**i {
                    for name in names {
                        if let Node::Var { name: var_name, .. } = &**name {
                            if !var_name.is_empty() {  // Skip empty variable names
                                let location = name.absolute_start(&self.root);
                                let symbol = Arc::new(Symbol::new(
                                    var_name.clone(),
                                    SymbolType::Variable,
                                    self.current_uri.clone(),
                                    location,
                                ));
                                new_table.insert(symbol);
                                trace!("Declared variable '{}' in choice branch scope at {:?}", var_name, location);
                            } else {
                                trace!("Skipped empty variable name in choice branch at {:?}", name.absolute_start(&self.root));
                            }
                        }
                    }
                    if let Some(rem) = remainder {
                        if let Node::Var { name: var_name, .. } = &**rem {
                            if !var_name.is_empty() {
                                let location = rem.absolute_start(&self.root);
                                let symbol = Arc::new(Symbol::new(
                                    var_name.clone(),
                                    SymbolType::Variable,
                                    self.current_uri.clone(),
                                    location,
                                ));
                                new_table.insert(symbol);
                                trace!("Declared remainder variable '{}' in choice branch scope at {:?}", var_name, location);
                            }
                        }
                    }
                }
            }
            let new_inputs: Vector<Arc<Node<'a>>, ArcK> = inputs.iter().map(|i| self.visit_node(i)).collect();
            let new_proc = self.visit_node(proc);
            let branch_node = Arc::new(Node::Choice {
                base: base.clone(),
                branches: Vector::new_with_ptr_kind().push_back((new_inputs.clone(), new_proc.clone())),
                metadata: metadata.clone(),
            });
            let _updated_branch = self.update_metadata(branch_node, new_table.clone(), None, metadata);
            self.pop_scope();
            (new_inputs, new_proc)
        }).collect();

        let new_node = Arc::new(Node::Choice {
            base: base.clone(),
            branches: new_branches,
            metadata: metadata.clone(),
        });
        self.update_with_current_table(new_node, None, metadata)
    }

    /// Visits a `var` node, recording usages only if they differ from the declaration location.
    fn visit_var<'a>(
        &self,
        node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        name: &String,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let mut referenced_symbol: Option<Arc<Symbol>> = None;
        if !name.is_empty() {  // Only process non-empty variable names
            if let Some(symbol) = self.current_table.read().unwrap().lookup(name) {
                referenced_symbol = Some(symbol.clone());
                let usage_location = node.absolute_start(&self.root);
                let is_declaration = usage_location == symbol.declaration_location;
                let is_definition = symbol.definition_location.map_or(false, |def| usage_location == def);
                if is_declaration || is_definition {
                    trace!("Skipped recording for {} of '{}' at {:?}", if is_declaration {"declaration"} else {"definition"}, name, usage_location);
                } else {
                    if symbol.declaration_uri == self.current_uri {
                        self.inverted_index.write().unwrap()
                            .entry(symbol.declaration_location)
                            .or_insert(Vec::new())
                            .push(usage_location);
                        trace!("Recorded local usage of '{}' at {:?}", name, usage_location);
                    } else {
                        self.potential_global_refs.write().unwrap().push((name.clone(), usage_location));
                        trace!("Added global reference for '{}' at {:?}", name, usage_location);
                    }
                }
            } else {
                let usage_location = node.absolute_start(&self.root);
                self.potential_global_refs.write().unwrap().push((name.clone(), usage_location));
                trace!("Added potential unbound reference for '{}' at {:?}", name, usage_location);
            }
        } else {
            trace!("Skipped empty variable name in var usage at {:?}", node.absolute_start(&self.root));
        }
        let new_node = Arc::new(Node::Var {
            base: base.clone(),
            name: name.clone(),
            metadata: metadata.clone(),
        });
        self.update_with_current_table(new_node, referenced_symbol, metadata)
    }

    fn visit_disjunction<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        left: &Arc<Node<'a>>,
        right: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_left = self.visit_node(left);
        let new_right = self.visit_node(right);
        let new_node = Arc::new(Node::Disjunction {
            base: base.clone(),
            left: new_left,
            right: new_right,
            metadata: metadata.clone(),
        });
        self.update_with_current_table(new_node, None, metadata)
    }

    fn visit_conjunction<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        left: &Arc<Node<'a>>,
        right: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_left = self.visit_node(left);
        let new_right = self.visit_node(right);
        let new_node = Arc::new(Node::Conjunction {
            base: base.clone(),
            left: new_left,
            right: new_right,
            metadata: metadata.clone(),
        });
        self.update_with_current_table(new_node, None, metadata)
    }

    fn visit_negation<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        operand: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_operand = self.visit_node(operand);
        let new_node = Arc::new(Node::Negation {
            base: base.clone(),
            operand: new_operand,
            metadata: metadata.clone(),
        });
        self.update_with_current_table(new_node, None, metadata)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree_sitter::{parse_code, parse_to_ir};
    use crate::ir::node::{RelativePosition, compute_absolute_positions};
    use tower_lsp::lsp_types::Url;

    #[test]
    fn test_hierarchical_symbol_table() {
        let code = "new x in { let y = 42 in { contract z() = { x!(y) } } }";
        let uri = Url::parse("file:///test.rho").unwrap();
        let tree = parse_code(code);
        let ir = parse_to_ir(&tree, code);
        let root: Arc<Node<'static>> = unsafe { std::mem::transmute(ir) };
        let global_table = Arc::new(SymbolTable::new(None));

        let builder = SymbolTableBuilder::new(root.clone(), uri.clone(), global_table.clone());
        let transformed = builder.visit_node(&root);

        // Check nested scopes
        if let Node::New { proc, .. } = &*transformed {
            if let Node::Block { proc: let_node, .. } = &**proc {
                if let Node::Let { proc: contract_block, .. } = &**let_node {
                    if let Node::Block { proc: contract_node, .. } = &**contract_block {
                        let contract_table = contract_node.metadata()
                            .and_then(|m| m.data.get("symbol_table"))
                            .and_then(|t| t.downcast_ref::<Arc<SymbolTable>>())
                            .cloned()
                            .unwrap();
                        assert!(contract_table.lookup("x").is_some(), "x should be in scope");
                        assert!(contract_table.lookup("y").is_some(), "y should be in scope");
                        assert!(contract_table.lookup("z").is_some(), "z should be in global scope");
                    }
                }
            }
        }
    }

    #[test]
    fn test_symbol_table_new() {
        let code = "new x in { contract x() = { Nil } }";
        let uri = Url::parse("file:///test.rho").unwrap();
        let tree = parse_code(code);
        let ir = parse_to_ir(&tree, code);
        let root: Arc<Node<'static>> = unsafe { std::mem::transmute(ir) };
        let global_table = Arc::new(SymbolTable::new(None));
        let builder = SymbolTableBuilder::new(root.clone(), uri, global_table);
        let transformed = builder.visit_node(&root);

        if let Node::New { proc, .. } = &*transformed {
            if let Node::Block { proc: contract_node, .. } = &**proc {
                let contract_table = contract_node.metadata()
                    .and_then(|m| m.data.get("symbol_table"))
                    .and_then(|t| t.downcast_ref::<Arc<SymbolTable>>())
                    .cloned()
                    .unwrap();
                assert!(contract_table.lookup("x").is_some());
            }
        }
    }

    #[test]
    fn test_inverted_index() {
        let code = "new x in { x!() | x!() }";
        let uri = Url::parse("file:///test.rho").unwrap();
        let tree = parse_code(code);
        let ir = parse_to_ir(&tree, code);
        let root: Arc<Node<'static>> = unsafe { std::mem::transmute(ir) };
        let global_table = Arc::new(SymbolTable::new(None));
        let builder = SymbolTableBuilder::new(root.clone(), uri, global_table);
        builder.visit_node(&root);
        let index = builder.get_inverted_index();

        let decl_pos = if let Node::New { decls, .. } = &*root {
            decls[0].absolute_start(&root)
        } else { unreachable!() };
        assert!(index.contains_key(&decl_pos));
        assert_eq!(index[&decl_pos].len(), 2, "x should have two usages");
    }

    #[test]
    fn test_position_lookup() {
        let code = "new x in { x!() }";
        let uri = Url::parse("file:///test.rho").unwrap();
        let tree = parse_code(code);
        let ir = parse_to_ir(&tree, code);
        let root: Arc<Node<'static>> = unsafe { std::mem::transmute(ir) };
        let global_table = Arc::new(SymbolTable::new(None));
        let positions = Arc::new(compute_absolute_positions(&root));
        let builder = SymbolTableBuilder::new(root.clone(), uri, global_table);
        builder.visit_node(&root);
        let position = Position { row: 0, column: 11, byte: 11 };
        let node = find_node_at_position(&root, &positions, position).unwrap();
        assert_eq!(node.text(), "x");
    }

    #[test]
    fn test_symbol_table_scoping() {
        let code = "new x in { let y = x in { y } }";
        let uri = Url::parse("file:///test.rho").unwrap();
        let tree = parse_code(code);
        let ir = parse_to_ir(&tree, code);
        let root: Arc<Node<'static>> = unsafe { std::mem::transmute(ir) };
        let global_table = Arc::new(SymbolTable::new(None));
        let builder = SymbolTableBuilder::new(root.clone(), uri.clone(), global_table.clone());
        let transformed = builder.visit_node(&root);

        let x_decl = if let Node::New { decls, .. } = &*transformed {
            if let Node::NameDecl { var, .. } = &*decls[0] {
                var.absolute_start(&root)
            } else { unreachable!() }
        } else { unreachable!() };
        assert!(global_table.lookup("x").is_none(), "x should be in local scope");
        if let Node::New { proc, .. } = &*transformed {
            if let Node::Block { proc: let_node, .. } = &**proc {
                let let_table = let_node.metadata().unwrap().data.get("symbol_table").unwrap()
                    .downcast_ref::<Arc<SymbolTable>>().unwrap();
                assert!(let_table.lookup("x").is_some(), "x should be accessible in let scope");
                if let Node::Let { decls, proc, .. } = &**let_node {
                    if let Some(decl) = decls.first() {
                        if let Node::Decl { procs, .. } = &**decl {
                            if let Some(x_node) = procs.first() {
                                let x_usage = x_node.absolute_start(&root);
                                let index = builder.get_inverted_index();
                                assert!(index.get(&x_decl).unwrap().contains(&x_usage), "x usage in let should be recorded");
                            }
                        }
                    }
                    if let Node::Block { proc: y_node, .. } = &**proc {
                        // Use the Var node's symbol table instead of let_table
                        let y_table = y_node.metadata().unwrap().data.get("symbol_table").unwrap()
                            .downcast_ref::<Arc<SymbolTable>>().unwrap();
                        assert!(y_table.lookup("y").is_some(), "y should be in the Var node's symbol table");
                        let y_usage = y_node.absolute_start(&root);
                        let y_symbol = y_table.lookup("y").unwrap();
                        let y_decl = y_symbol.declaration_location;
                        let index = builder.get_inverted_index();
                        assert!(index.get(&y_decl).unwrap().contains(&y_usage), "y usage should be recorded");
                    }
                }
            }
        }
    }

    #[test]
    fn test_cross_file_reference() {
        let code1 = "contract foo() = { Nil }";
        let code2 = "new x in { foo!() }";
        let uri1 = Url::parse("file:///file1.rho").unwrap();
        let uri2 = Url::parse("file:///file2.rho").unwrap();
        let tree1 = parse_code(code1);
        let tree2 = parse_code(code2);
        let ir1 = parse_to_ir(&tree1, code1);
        let ir2 = parse_to_ir(&tree2, code2);
        let root1: Arc<Node<'static>> = unsafe { std::mem::transmute(ir1) };
        let root2: Arc<Node<'static>> = unsafe { std::mem::transmute(ir2) };
        let global_table = Arc::new(SymbolTable::new(None));

        let builder1 = SymbolTableBuilder::new(root1.clone(), uri1.clone(), global_table.clone());
        builder1.visit_node(&root1);
        let builder2 = SymbolTableBuilder::new(root2.clone(), uri2.clone(), global_table.clone());
        builder2.visit_node(&root2);

        let potentials = builder2.get_potential_global_refs();
        assert_eq!(potentials.len(), 1);
        assert_eq!(potentials[0].0, "foo");
    }

    #[test]
    fn test_contract_parameters_in_symbol_table() {
        let code = "contract foo(x) = { Nil }";
        let uri = Url::parse("file:///test.rho").unwrap();
        let tree = parse_code(code);
        let ir = parse_to_ir(&tree, code);
        let root: Arc<Node<'static>> = unsafe { std::mem::transmute(ir) };
        let global_table = Arc::new(SymbolTable::new(None));
        let builder = SymbolTableBuilder::new(root.clone(), uri.clone(), global_table.clone());
        let transformed = builder.visit_node(&root);

        if let Node::Contract { metadata, .. } = &*transformed {
            let symbol_table = metadata.as_ref().unwrap()
                .data.get("symbol_table").unwrap()
                .downcast_ref::<Arc<SymbolTable>>().unwrap();
            assert!(symbol_table.lookup("x").is_some(), "Parameter 'x' should be in contract's symbol table");
        } else {
            panic!("Expected Contract node");
        }
    }

    #[test]
    fn test_match_pat_set() {
        let p_e = Vector::new_with_ptr_kind().push_back(Arc::new(Node::LongLiteral { base: NodeBase::new(None, RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 }, 1, Some("1".to_string())), value: 1, metadata: None })).push_back(Arc::new(Node::LongLiteral { base: NodeBase::new(None, RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 }, 1, Some("2".to_string())), value: 2, metadata: None }));
        let pat = Arc::new(Node::Set { base: NodeBase::new(None, RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 }, 0, None), elements: p_e, remainder: None, metadata: None });
        let c_e = Vector::new_with_ptr_kind().push_back(Arc::new(Node::LongLiteral { base: NodeBase::new(None, RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 }, 1, Some("2".to_string())), value: 2, metadata: None })).push_back(Arc::new(Node::LongLiteral { base: NodeBase::new(None, RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 }, 1, Some("1".to_string())), value: 1, metadata: None }));
        let concrete = Arc::new(Node::Set { base: NodeBase::new(None, RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 }, 0, None), elements: c_e, remainder: None, metadata: None });
        let mut subst = HashMap::new();
        assert!(crate::ir::node::match_pat(&pat, &concrete, &mut subst));
    }

    #[test]
    fn test_match_pat_map() {
        let p_pair1 = (Arc::new(Node::StringLiteral { base: NodeBase::new(None, RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 }, 3, Some("\"a\"".to_string())), value: "a".to_string(), metadata: None }), Arc::new(Node::LongLiteral { base: NodeBase::new(None, RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 }, 1, Some("1".to_string())), value: 1, metadata: None }));
        let p_pair2 = (Arc::new(Node::StringLiteral { base: NodeBase::new(None, RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 }, 3, Some("\"b\"".to_string())), value: "b".to_string(), metadata: None }), Arc::new(Node::LongLiteral { base: NodeBase::new(None, RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 }, 1, Some("2".to_string())), value: 2, metadata: None }));
        let p_pairs = Vector::new_with_ptr_kind().push_back(p_pair1).push_back(p_pair2);
        let pat = Arc::new(Node::Map { base: NodeBase::new(None, RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 }, 0, None), pairs: p_pairs, remainder: None, metadata: None });
        let c_pair1 = (Arc::new(Node::StringLiteral { base: NodeBase::new(None, RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 }, 3, Some("\"b\"".to_string())), value: "b".to_string(), metadata: None }), Arc::new(Node::LongLiteral { base: NodeBase::new(None, RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 }, 1, Some("2".to_string())), value: 2, metadata: None }));
        let c_pair2 = (Arc::new(Node::StringLiteral { base: NodeBase::new(None, RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 }, 3, Some("\"a\"".to_string())), value: "a".to_string(), metadata: None }), Arc::new(Node::LongLiteral { base: NodeBase::new(None, RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 }, 1, Some("1".to_string())), value: 1, metadata: None }));
        let c_pairs = Vector::new_with_ptr_kind().push_back(c_pair1).push_back(c_pair2);
        let concrete = Arc::new(Node::Map { base: NodeBase::new(None, RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 }, 0, None), pairs: c_pairs, remainder: None, metadata: None });
        let mut subst = HashMap::new();
        assert!(crate::ir::node::match_pat(&pat, &concrete, &mut subst));
    }

    #[test]
    fn test_match_pat_disjunction() {
        let p_left = Arc::new(Node::LongLiteral { base: NodeBase::new(None, RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 }, 1, Some("1".to_string())), value: 1, metadata: None });
        let p_right = Arc::new(Node::LongLiteral { base: NodeBase::new(None, RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 }, 1, Some("2".to_string())), value: 2, metadata: None });
        let pat = Arc::new(Node::Disjunction { base: NodeBase::new(None, RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 }, 0, None), left: p_left, right: p_right, metadata: None });
        let c_left = Arc::new(Node::LongLiteral { base: NodeBase::new(None, RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 }, 1, Some("1".to_string())), value: 1, metadata: None });
        let c_right = Arc::new(Node::LongLiteral { base: NodeBase::new(None, RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 }, 1, Some("2".to_string())), value: 2, metadata: None });
        let concrete = Arc::new(Node::Disjunction { base: NodeBase::new(None, RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 }, 0, None), left: c_left, right: c_right, metadata: None });
        let mut subst = HashMap::new();
        assert!(crate::ir::node::match_pat(&pat, &concrete, &mut subst));
    }
}
