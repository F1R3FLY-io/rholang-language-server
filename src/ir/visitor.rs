use std::sync::Arc;
use rpds::Vector;
use archery::ArcK;

use super::rholang_node::{RholangNode, NodeBase, Metadata, CommentKind, RholangSendType, RholangBundleType, BinOperator, UnaryOperator, RholangVarRefKind, RelativePosition};

/// Provides a visitor pattern for traversing and transforming the Rholang Intermediate Representation (IR) tree.
/// This module enables implementors to define custom logic for processing each node type, facilitating operations
/// such as optimization, analysis, or formatting of the IR tree.
pub trait Visitor {

    /// Entry point for visiting an IR node, dispatching to the appropriate type-specific method.
    /// Implementors typically do not override this method unless custom dispatching is needed.
    ///
    /// # Arguments
    /// * node - The node to visit.
    ///
    /// # Returns
    /// The transformed node, or the original if unchanged.
    fn visit_node(&self, node: &Arc<RholangNode>) -> Arc<RholangNode> {
        match &**node {
            RholangNode::Par { base, left, right, metadata } => self.visit_par(node, base, left, right, metadata),
            RholangNode::SendSync { base, channel, inputs, cont, metadata } => self.visit_send_sync(node, base, channel, inputs, cont, metadata),
            RholangNode::Send { base, channel, send_type, send_type_delta, inputs, metadata } => self.visit_send(node, base, channel, send_type, send_type_delta, inputs, metadata),
            RholangNode::New { base, decls, proc, metadata } => self.visit_new(node, base, decls, proc, metadata),
            RholangNode::IfElse { base, condition, consequence, alternative, metadata } => self.visit_ifelse(node, base, condition, consequence, alternative, metadata),
            RholangNode::Let { base, decls, proc, metadata } => self.visit_let(node, base, decls, proc, metadata),
            RholangNode::Bundle { base, bundle_type, proc, metadata } => self.visit_bundle(node, base, bundle_type, proc, metadata),
            RholangNode::Match { base, expression, cases, metadata } => self.visit_match(node, base, expression, cases, metadata),
            RholangNode::Choice { base, branches, metadata } => self.visit_choice(node, base, branches, metadata),
            RholangNode::Contract { base, name, formals, formals_remainder, proc, metadata } => self.visit_contract(node, base, name, formals, formals_remainder, proc, metadata),
            RholangNode::Input { base, receipts, proc, metadata } => self.visit_input(node, base, receipts, proc, metadata),
            RholangNode::Block { base, proc, metadata } => self.visit_block(node, base, proc, metadata),
            RholangNode::Parenthesized { base, expr, metadata } => self.visit_parenthesized(node, base, expr, metadata),
            RholangNode::BinOp { base, op, left, right, metadata } => self.visit_binop(node, base, op.clone(), left, right, metadata),
            RholangNode::UnaryOp { base, op, operand, metadata } => self.visit_unaryop(node, base, op.clone(), operand, metadata),
            RholangNode::Method { base, receiver, name, args, metadata } => self.visit_method(node, base, receiver, name, args, metadata),
            RholangNode::Eval { base, name, metadata } => self.visit_eval(node, base, name, metadata),
            RholangNode::Quote { base, quotable, metadata } => self.visit_quote(node, base, quotable, metadata),
            RholangNode::VarRef { base, kind, var, metadata } => self.visit_varref(node, base, kind.clone(), var, metadata),
            RholangNode::BoolLiteral { base, value, metadata } => self.visit_bool_literal(node, base, *value, metadata),
            RholangNode::LongLiteral { base, value, metadata } => self.visit_long_literal(node, base, *value, metadata),
            RholangNode::StringLiteral { base, value, metadata } => self.visit_string_literal(node, base, value, metadata),
            RholangNode::UriLiteral { base, value, metadata } => self.visit_uri_literal(node, base, value, metadata),
            RholangNode::Nil { base, metadata } => self.visit_nil(node, base, metadata),
            RholangNode::List { base, elements, remainder, metadata } => self.visit_list(node, base, elements, remainder, metadata),
            RholangNode::Set { base, elements, remainder, metadata } => self.visit_set(node, base, elements, remainder, metadata),
            RholangNode::Map { base, pairs, remainder, metadata } => self.visit_map(node, base, pairs, remainder, metadata),
            RholangNode::Tuple { base, elements, metadata } => self.visit_tuple(node, base, elements, metadata),
            RholangNode::Var { base, name, metadata } => self.visit_var(node, base, name, metadata),
            RholangNode::NameDecl { base, var, uri, metadata } => self.visit_name_decl(node, base, var, uri, metadata),
            RholangNode::Decl { base, names, names_remainder, procs, metadata } => self.visit_decl(node, base, names, names_remainder, procs, metadata),
            RholangNode::LinearBind { base, names, remainder, source, metadata } => self.visit_linear_bind(node, base, names, remainder, source, metadata),
            RholangNode::RepeatedBind { base, names, remainder, source, metadata } => self.visit_repeated_bind(node, base, names, remainder, source, metadata),
            RholangNode::PeekBind { base, names, remainder, source, metadata } => self.visit_peek_bind(node, base, names, remainder, source, metadata),
            RholangNode::Comment { base, kind, metadata } => self.visit_comment(node, base, kind, metadata),
            RholangNode::Wildcard { base, metadata } => self.visit_wildcard(node, base, metadata),
            RholangNode::SimpleType { base, value, metadata } => self.visit_simple_type(node, base, value, metadata),
            RholangNode::ReceiveSendSource { base, name, metadata } => self.visit_receive_send_source(node, base, name, metadata),
            RholangNode::SendReceiveSource { base, name, inputs, metadata } => self.visit_send_receive_source(node, base, name, inputs, metadata),
            RholangNode::Error { base, children, metadata } => self.visit_error(node, base, children, metadata),
            RholangNode::Disjunction { base, left, right, metadata } => self.visit_disjunction(node, base, left, right, metadata),
            RholangNode::Conjunction { base, left, right, metadata } => self.visit_conjunction(node, base, left, right, metadata),
            RholangNode::Negation { base, operand, metadata } => self.visit_negation(node, base, operand, metadata),
            RholangNode::Unit { base, metadata } => self.visit_unit(node, base, metadata),
        }
    }

    /// Visits a parallel composition node Par), processing its subprocesses.
    ///
    /// # Arguments
    /// * node - The original Par node for reference.
    /// * base - Metadata including position and text.
    /// * left - Left subprocess.
    /// * right - Right subprocess.
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// A new Par node if subprocesses change, otherwise the original.
    ///
    /// # Examples
    /// For P | Q, visits P and Q, reconstructing the node if either changes.
    fn visit_par(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        left: &Arc<RholangNode>,
        right: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_left = self.visit_node(left);
        let new_right = self.visit_node(right);
        if Arc::ptr_eq(left, &new_left) && Arc::ptr_eq(right, &new_right) {
            Arc::clone(node)
        } else {
            Arc::new(RholangNode::Par {
                base: base.clone(),
                left: new_left,
                right: new_right,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a synchronous send node SendSync), handling channel, inputs, and continuation.
    ///
    /// # Arguments
    /// * node - The original SendSync node.
    /// * base - Metadata including position and text.
    /// * channel - The channel expression.
    /// * inputs - Arguments sent synchronously.
    /// * cont - Continuation process after sending.
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// A new SendSync node if any any component changes, otherwise the original.
    ///
    /// # Examples
    /// For ch!?("msg"; Nil), processes ch, "msg", and Nil.
    fn visit_send_sync(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        channel: &Arc<RholangNode>,
        inputs: &Vector<Arc<RholangNode>, ArcK>,
        cont: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_channel = self.visit_node(channel);
        let new_inputs = inputs.iter().map(|i| self.visit_node(i)).collect::<Vector<Arc<RholangNode>, ArcK>>();
        let new_cont = self.visit_node(cont);
        if Arc::ptr_eq(channel, &new_channel) &&
            inputs.iter().zip(new_inputs.iter()).all(|(a, b)| Arc::ptr_eq(a, b)) &&
            Arc::ptr_eq(cont, &new_cont) {
            Arc::clone(node)
        } else {
            Arc::new(RholangNode::SendSync {
                base: base.clone(),
                channel: new_channel,
                inputs: new_inputs,
                cont: new_cont,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits an asynchronous send node Send), processing its channel and inputs.
    ///
    /// # Arguments
    /// * node - The original Send node.
    /// * base - Metadata including position and text.
    /// * channel - The channel expression.
    /// * send_type - Single !) or multiple !!) send type.
    /// * send_type_delta - Relative position after the send type token.
    /// * inputs - Arguments sent asynchronously.
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// A new Send node if any any component changes, otherwise the original.
    ///
    /// # Examples
    /// For ch!("msg"), visits ch and "msg".
    fn visit_send(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        channel: &Arc<RholangNode>,
        send_type: &RholangSendType,
        send_type_delta: &RelativePosition,
        inputs: &Vector<Arc<RholangNode>, ArcK>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_channel = self.visit_node(channel);
        let new_inputs = inputs.iter().map(|i| self.visit_node(i)).collect::<Vector<Arc<RholangNode>, ArcK>>();
        if Arc::ptr_eq(channel, &new_channel) &&
            inputs.iter().zip(new_inputs.iter()).all(|(a, b)| Arc::ptr_eq(a, b)) {
            Arc::clone(node)
        } else {
            Arc::new(RholangNode::Send {
                base: base.clone(),
                channel: new_channel,
                send_type: send_type.clone(),
                send_type_delta: *send_type_delta,
                inputs: new_inputs,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a new name declaration node New), handling declarations and scoped process.
    ///
    /// # Arguments
    /// * node - The original New node.
    /// * base - Metadata including position and text.
    /// * decls - Vector of name declarations.
    /// * proc - The scoped process.
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// A new New node if any any component changes, otherwise the original.
    ///
    /// # Examples
    /// For new x in { P }, visits x and P.
    fn visit_new(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        decls: &Vector<Arc<RholangNode>, ArcK>,
        proc: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_decls = decls.iter().map(|d| self.visit_node(d)).collect::<Vector<Arc<RholangNode>, ArcK>>();
        let new_proc = self.visit_node(proc);
        if decls.iter().zip(new_decls.iter()).all(|(a, b)| Arc::ptr_eq(a, b)) &&
            Arc::ptr_eq(proc, &new_proc) {
            Arc::clone(node)
        } else {
            Arc::new(RholangNode::New {
                base: base.clone(),
                decls: new_decls,
                proc: new_proc,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a conditional node IfElse), processing condition, consequence, and optional alternative.
    ///
    /// # Arguments
    /// * node - The original IfElse node.
    /// * base - Metadata including position and text.
    /// * condition - The condition expression.
    /// * consequence - Process if condition is true.
    /// * alternative - Optional process if condition is false.
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// A new IfElse node if any any component changes, otherwise the original.
    ///
    /// # Examples
    /// For if (cond) { P } else { Q }, visits cond, P, and Q.
    fn visit_ifelse(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        condition: &Arc<RholangNode>,
        consequence: &Arc<RholangNode>,
        alternative: &Option<Arc<RholangNode>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_condition = self.visit_node(condition);
        let new_consequence = self.visit_node(consequence);
        let new_alternative = alternative.as_ref().map(|a| self.visit_node(a));
        if Arc::ptr_eq(condition, &new_condition) && Arc::ptr_eq(consequence, &new_consequence) &&
            alternative.as_ref().map_or(true, |a| new_alternative.as_ref().map_or(false, |na| Arc::ptr_eq(a, na))) {
            Arc::clone(node)
        } else {
            Arc::new(RholangNode::IfElse {
                base: base.clone(),
                condition: new_condition,
                consequence: new_consequence,
                alternative: new_alternative,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a variable binding node Let), handling declarations and process.
    ///
    /// # Arguments
    /// * node - The original Let node.
    /// * base - Metadata including position and text.
    /// * decls - Vector of variable declarations.
    /// * proc - The process using the bindings.
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// A new Let node if any any component changes, otherwise the original.
    ///
    /// # Examples
    /// For let x = "val" in { P }, visits x = "val" and P.
    fn visit_let(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        decls: &Vector<Arc<RholangNode>, ArcK>,
        proc: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_decls = decls.iter().map(|d| self.visit_node(d)).collect::<Vector<Arc<RholangNode>, ArcK>>();
        let new_proc = self.visit_node(proc);
        if decls.iter().zip(new_decls.iter()).all(|(a, b)| Arc::ptr_eq(a, b)) &&
            Arc::ptr_eq(proc, &new_proc) {
            Arc::clone(node)
        } else {
            Arc::new(RholangNode::Let {
                base: base.clone(),
                decls: new_decls,
                proc: new_proc,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits an access-controlled process node Bundle), processing its process.
    ///
    /// # Arguments
    /// * node - The original Bundle node.
    /// * base - Metadata including position and text.
    /// * bundle_type - Type of access control (e.g., Read, Write).
    /// * proc - The bundled process.
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// A new Bundle node if the process changes, otherwise the original.
    ///
    /// # Examples
    /// For bundle+ { P }, visits P.
    fn visit_bundle(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        bundle_type: &RholangBundleType,
        proc: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_proc = self.visit_node(proc);
        if Arc::ptr_eq(proc, &new_proc) {
            Arc::clone(node)
        } else {
            Arc::new(RholangNode::Bundle {
                base: base.clone(),
                bundle_type: bundle_type.clone(),
                proc: new_proc,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a pattern matching node Match), processing expression and cases.
    ///
    /// # Arguments
    /// * node - The original Match node.
    /// * base - Metadata including position and text.
    /// * expression - The expression to match against.
    /// * cases - Vector of (pattern, process) pairs.
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// A new Match node if any any component changes, otherwise the original.
    ///
    /// # Examples
    /// For match expr { pat => P }, visits expr, pat, and P.
    fn visit_match(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        expression: &Arc<RholangNode>,
        cases: &Vector<(Arc<RholangNode>, Arc<RholangNode>), ArcK>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_expression = self.visit_node(expression);
        let new_cases = cases.iter().map(|(p, r)| (self.visit_node(p), self.visit_node(r))).collect::<Vector<(Arc<RholangNode>, Arc<RholangNode>), ArcK>>();
        if Arc::ptr_eq(expression, &new_expression) && cases.iter().zip(new_cases.iter()).all(|((p1, r1), (p2, r2))| Arc::ptr_eq(p1, p2) && Arc::ptr_eq(r1, r2)) {
            Arc::clone(node)
        } else {
            Arc::new(RholangNode::Match {
                base: base.clone(),
                expression: new_expression,
                cases: new_cases,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a non-deterministic choice node Choice), processing its branches.
    ///
    /// # Arguments
    /// * node - The original Choice node.
    /// * base - Metadata including position and text.
    /// * branches - Vector of (inputs, process) pairs.
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// A new Choice node if branches change, otherwise the original.
    ///
    /// # Examples
    /// For select { x <- ch => P }, visits x <- ch and P.
    fn visit_choice(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        branches: &Vector<(Vector<Arc<RholangNode>, ArcK>, Arc<RholangNode>), ArcK>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_branches = branches.iter().map(|(i, p)| {
            let new_inputs = i.iter().map(|n| self.visit_node(n)).collect::<Vector<Arc<RholangNode>, ArcK>>();
            (new_inputs, self.visit_node(p))
        }).collect::<Vector<(Vector<Arc<RholangNode>, ArcK>, Arc<RholangNode>), ArcK>>();
        if branches.iter().zip(new_branches.iter()).all(|((i1, p1), (i2, p2))| i1.iter().zip(i2.iter()).all(|(a, b)| Arc::ptr_eq(a, b)) && Arc::ptr_eq(p1, p2)) {
            Arc::clone(node)
        } else {
            Arc::new(RholangNode::Choice {
                base: base.clone(),
                branches: new_branches,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a contract definition node Contract), processing name, parameters, and body.
    ///
    /// # Arguments
    /// * node - The original Contract node.
    /// * base - Metadata including position and text.
    /// * name - The contract’s name.
    /// * formals - Vector of formal parameters.
    /// * formals_remainder - Optional remainder for formal parameters.
    /// * proc - The contract body process.
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// A new Contract node if any any component changes, otherwise the original.
    ///
    /// # Examples
    /// For contract name(args) = { P }, visits name, args, and P.
    fn visit_contract(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        name: &Arc<RholangNode>,
        formals: &Vector<Arc<RholangNode>, ArcK>,
        formals_remainder: &Option<Arc<RholangNode>>,
        proc: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_name = self.visit_node(name);
        let new_formals = formals.iter().map(|f| self.visit_node(f)).collect::<Vector<Arc<RholangNode>, ArcK>>();
        let new_formals_remainder = formals_remainder.as_ref().map(|r| self.visit_node(r));
        let new_proc = self.visit_node(proc);
        if Arc::ptr_eq(name, &new_name) &&
            formals.iter().zip(new_formals.iter()).all(|(a, b)| Arc::ptr_eq(a, b)) &&
            formals_remainder.as_ref().map_or(true, |r| new_formals_remainder.as_ref().map_or(false, |nr| Arc::ptr_eq(r, nr))) &&
            Arc::ptr_eq(proc, &new_proc) {
            Arc::clone(node)
        } else {
            Arc::new(RholangNode::Contract {
                base: base.clone(),
                name: new_name,
                formals: new_formals,
                formals_remainder: new_formals_remainder,
                proc: new_proc,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits an input binding node Input), processing receipts and process.
    ///
    /// # Arguments
    /// * node - The original Input node.
    /// * base - Metadata including position and text.
    /// * receipts - Vector of binding groups from channels.
    /// * proc - The process after receiving.
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// A new Input node if any any component changes, otherwise the original.
    ///
    /// # Examples
    /// For for (x <- ch) { P }, visits x <- ch and P.
    fn visit_input(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        receipts: &Vector<Vector<Arc<RholangNode>, ArcK>, ArcK>,
        proc: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_receipts = receipts.iter().map(|r| {
            r.iter().map(|b| self.visit_node(b)).collect::<Vector<Arc<RholangNode>, ArcK>>()
        }).collect::<Vector<Vector<Arc<RholangNode>, ArcK>, ArcK>>();
        let new_proc = self.visit_node(proc);
        if receipts.iter().zip(new_receipts.iter()).all(|(r1, r2)| r1.iter().zip(r2.iter()).all(|(a, b)| Arc::ptr_eq(a, b))) && Arc::ptr_eq(proc, &new_proc) {
            Arc::clone(node)
        } else {
            Arc::new(RholangNode::Input {
                base: base.clone(),
                receipts: new_receipts,
                proc: new_proc,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a block node Block), processing its contained process.
    ///
    /// # Arguments
    /// * node - The original Block node.
    /// * base - Metadata including position and text.
    /// * proc - The process within the block.
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// A new Block node if the process changes, otherwise the original.
    ///
    /// # Examples
    /// For { P }, visits P.
    fn visit_block(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        proc: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_proc = self.visit_node(proc);
        if Arc::ptr_eq(proc, &new_proc) {
            Arc::clone(node)
        } else {
            Arc::new(RholangNode::Block {
                base: base.clone(),
                proc: new_proc,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a parenthesized expression node Parenthesized), processing its expression.
    ///
    /// # Arguments
    /// * node - The original Parenthesized node.
    /// * base - Metadata including position and text.
    /// * expr - The expression inside parentheses.
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// A new Parenthesized node if the expression changes, otherwise the original.
    ///
    /// # Examples
    /// For (P), visits P.
    fn visit_parenthesized(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        expr: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_expr = self.visit_node(expr);
        if Arc::ptr_eq(expr, &new_expr) {
            Arc::clone(node)
        } else {
            Arc::new(RholangNode::Parenthesized {
                base: base.clone(),
                expr: new_expr,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a binary operation node BinOp), processing its operands.
    ///
    /// # Arguments
    /// * node - The original BinOp node.
    /// * base - Metadata including position and text.
    /// * op - The binary operator (e.g., Add, Eq).
    /// * left - Left operand.
    /// * right - Right operand.
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// A new BinOp node if operands change, otherwise the original.
    ///
    /// # Examples
    /// For a + b, visits a and b.
    fn visit_binop(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        op: BinOperator,
        left: &Arc<RholangNode>,
        right: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_left = self.visit_node(left);
        let new_right = self.visit_node(right);
        if Arc::ptr_eq(left, &new_left) && Arc::ptr_eq(right, &new_right) {
            Arc::clone(node)
        } else {
            Arc::new(RholangNode::BinOp {
                base: base.clone(),
                op,
                left: new_left,
                right: new_right,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a unary operation node UnaryOp), processing its operand.
    ///
    /// # Arguments
    /// * node - The original UnaryOp node.
    /// * base - Metadata including position and text.
    /// * op - The unary operator (e.g., Neg, Not).
    /// * operand - The operand expression.
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// A new UnaryOp node if the operand changes, otherwise the original.
    ///
    /// # Examples
    /// For not P, visits P.
    fn visit_unaryop(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        op: UnaryOperator,
        operand: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_operand = self.visit_node(operand);
        if Arc::ptr_eq(operand, &new_operand) {
            Arc::clone(node)
        } else {
            Arc::new(RholangNode::UnaryOp {
                base: base.clone(),
                op,
                operand: new_operand,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a method call node Method), processing receiver and arguments.
    ///
    /// # Arguments
    /// * node - The original Method node.
    /// * base - Metadata including position and text.
    /// * receiver - The receiver expression.
    /// * name - Method name as a string.
    /// * args - Vector of argument expressions.
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// A new Method node if any any component changes, otherwise the original.
    ///
    /// # Examples
    /// For obj.method(args), visits obj and args.
    fn visit_method(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        receiver: &Arc<RholangNode>,
        name: &String,
        args: &Vector<Arc<RholangNode>, ArcK>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_receiver = self.visit_node(receiver);
        let new_args = args.iter().map(|a| self.visit_node(a)).collect::<Vector<Arc<RholangNode>, ArcK>>();
        if Arc::ptr_eq(receiver, &new_receiver) && args.iter().zip(new_args.iter()).all(|(a, b)| Arc::ptr_eq(a, b)) {
            Arc::clone(node)
        } else {
            Arc::new(RholangNode::Method {
                base: base.clone(),
                receiver: new_receiver,
                name: name.clone(),
                args: new_args,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits an evaluation node Eval), processing the name to evaluate.
    ///
    /// # Arguments
    /// * node - The original Eval node.
    /// * base - Metadata including position and text.
    /// * name - The name expression to evaluate.
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// A new Eval node if the name changes, otherwise the original.
    ///
    /// # Examples
    /// For *name, visits name.
    fn visit_eval(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        name: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_name = self.visit_node(name);
        if Arc::ptr_eq(name, &new_name) {
            Arc::clone(node)
        } else {
            Arc::new(RholangNode::Eval {
                base: base.clone(),
                name: new_name,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a quotation node Quote), processing the quoted process.
    ///
    /// # Arguments
    /// * node - The original Quote node.
    /// * base - Metadata including position and text.
    /// * quotable - The process being quoted.
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// A new Quote node if the quotable changes, otherwise the original.
    ///
    /// # Examples
    /// For @P, visits P.
    fn visit_quote(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        quotable: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_quotable = self.visit_node(quotable);
        if Arc::ptr_eq(quotable, &new_quotable) {
            Arc::clone(node)
        } else {
            Arc::new(RholangNode::Quote {
                base: base.clone(),
                quotable: new_quotable,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a variable reference node VarRef), processing the referenced variable.
    ///
    /// # Arguments
    /// * node - The original VarRef node.
    /// * base - Metadata including position and text.
    /// * kind - Reference kind (e.g., Assign, AssignStar).
    /// * var - The variable expression.
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// A new VarRef node if the variable changes, otherwise the original.
    ///
    /// # Examples
    /// For =x or =*x, visits x.
    fn visit_varref(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        kind: RholangVarRefKind,
        var: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_var = self.visit_node(var);
        if Arc::ptr_eq(var, &new_var) {
            Arc::clone(node)
        } else {
            Arc::new(RholangNode::VarRef {
                base: base.clone(),
                kind,
                var: new_var,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a boolean literal node BoolLiteral).
    ///
    /// # Arguments
    /// * node - The original BoolLiteral node.
    /// * base - Metadata including position and text.
    /// * value - The boolean value true or false).
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// The original node by default; override to transform.
    ///
    /// # Examples
    /// For true, returns unchanged unless overridden.
    fn visit_bool_literal(
        &self,
        node: &Arc<RholangNode>,
        _base: &NodeBase,
        _value: bool,
        _metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        Arc::clone(node)
    }

    /// Visits an integer literal node LongLiteral).
    ///
    /// # Arguments
    /// * node - The original LongLiteral node.
    /// * base - Metadata including position and text.
    /// * value - The integer value.
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// The original node by default; override to transform.
    ///
    /// # Examples
    /// For 42, returns unchanged unless overridden.
    fn visit_long_literal(
        &self,
        node: &Arc<RholangNode>,
        _base: &NodeBase,
        _value: i64,
        _metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        Arc::clone(node)
    }

    /// Visits a string literal node StringLiteral).
    ///
    /// # Arguments
    /// * node - The original StringLiteral node.
    /// * base - Metadata including position and text.
    /// * value - The string content.
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// The original node by default; override to transform.
    ///
    /// # Examples
    /// For "hello", returns unchanged unless overridden.
    fn visit_string_literal(
        &self,
        node: &Arc<RholangNode>,
        _base: &NodeBase,
        _value: &String,
        _metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        Arc::clone(node)
    }

    /// Visits a URI literal node UriLiteral).
    ///
    /// # Arguments
    /// * node - The original UriLiteral node.
    /// * base - Metadata including position and text.
    /// * value - The URI string.
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// The original node by default; override to transform.
    ///
    /// # Examples
    /// For `` http://example.com ``, returns unchanged unless overridden.
    fn visit_uri_literal(
        &self,
        node: &Arc<RholangNode>,
        _base: &NodeBase,
        _value: &String,
        _metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        Arc::clone(node)
    }

    /// Visits an empty process node Nil).
    ///
    /// # Arguments
    /// * node - The original Nil node.
    /// * base - Metadata including position and text.
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// The original node by default; override to transform.
    ///
    /// # Examples
    /// For Nil, returns unchanged unless overridden.
    fn visit_nil(
        &self,
        node: &Arc<RholangNode>,
        _base: &NodeBase,
        _metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        Arc::clone(node)
    }

    /// Visits a list collection node List), processing elements and remainder.
    ///
    /// # Arguments
    /// * node - The original List node.
    /// * base - Metadata including position and text.
    /// * elements - Vector of list elements.
    /// * remainder - Optional remainder (e.g., ...rest).
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// A new List node if components change, otherwise the original.
    ///
    /// # Examples
    /// For [a, b, ...rest], visits a, b, and rest.
    fn visit_list(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        elements: &Vector<Arc<RholangNode>, ArcK>,
        remainder: &Option<Arc<RholangNode>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_elements = elements.iter().map(|e| self.visit_node(e)).collect::<Vector<Arc<RholangNode>, ArcK>>();
        let new_remainder = remainder.as_ref().map(|r| self.visit_node(r));
        if elements.iter().zip(new_elements.iter()).all(|(a, b)| Arc::ptr_eq(a, b)) &&
            remainder.as_ref().map_or(true, |r| new_remainder.as_ref().map_or(false, |nr| Arc::ptr_eq(r, nr))) {
            Arc::clone(node)
        } else {
            Arc::new(RholangNode::List {
                base: base.clone(),
                elements: new_elements,
                remainder: new_remainder,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a set collection node Set), processing elements and remainder.
    ///
    /// # Arguments
    /// * node - The original Set node.
    /// * base - Metadata including position and text.
    /// * elements - Vector of set elements.
    /// * remainder - Optional remainder (e.g., ...rest).
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// A new Set node if components change, otherwise the original.
    ///
    /// # Examples
    /// For Set(a, b, ...rest), visits a, b, and rest.
    fn visit_set(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        elements: &Vector<Arc<RholangNode>, ArcK>,
        remainder: &Option<Arc<RholangNode>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_elements = elements.iter().map(|e| self.visit_node(e)).collect::<Vector<Arc<RholangNode>, ArcK>>();
        let new_remainder = remainder.as_ref().map(|r| self.visit_node(r));
        if elements.iter().zip(new_elements.iter()).all(|(a, b)| Arc::ptr_eq(a, b)) &&
            remainder.as_ref().map_or(true, |r| new_remainder.as_ref().map_or(false, |nr| Arc::ptr_eq(r, nr))) {
            Arc::clone(node)
        } else {
            Arc::new(RholangNode::Set {
                base: base.clone(),
                elements: new_elements,
                remainder: new_remainder,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a map collection node Map), processing key-value pairs and remainder.
    ///
    /// # Arguments
    /// * node - The original Map node.
    /// * base - Metadata including position and text.
    /// * pairs - Vector of (key, value) pairs.
    /// * remainder - Optional remainder (e.g., ...rest).
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// A new Map node if components change, otherwise the original.
    ///
    /// # Examples
    /// For {k: v, ...rest}, visits k, v, and rest.
    fn visit_map(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        pairs: &Vector<(Arc<RholangNode>, Arc<RholangNode>), ArcK>,
        remainder: &Option<Arc<RholangNode>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_pairs = pairs.iter().map(|(k, v)| (self.visit_node(k), self.visit_node(v))).collect::<Vector<(Arc<RholangNode>, Arc<RholangNode>), ArcK>>();
        let new_remainder = remainder.as_ref().map(|r| self.visit_node(r));
        if pairs.iter().zip(new_pairs.iter()).all(|((k1, v1), (k2, v2))| Arc::ptr_eq(k1, k2) && Arc::ptr_eq(v1, v2)) &&
            remainder.as_ref().map_or(true, |r| new_remainder.as_ref().map_or(false, |nr| Arc::ptr_eq(r, nr))) {
            Arc::clone(node)
        } else {
            Arc::new(RholangNode::Map {
                base: base.clone(),
                pairs: new_pairs,
                remainder: new_remainder,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a tuple collection node Tuple), processing its elements.
    ///
    /// # Arguments
    /// * node - The original Tuple node.
    /// * base - Metadata including position and text.
    /// * elements - Vector of tuple elements.
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// A new Tuple node if elements change, otherwise the original.
    ///
    /// # Examples
    /// For (a, b), visits a and b.
    fn visit_tuple(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        elements: &Vector<Arc<RholangNode>, ArcK>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_elements = elements.iter().map(|e| self.visit_node(e)).collect::<Vector<Arc<RholangNode>, ArcK>>();
        if elements.iter().zip(new_elements.iter()).all(|(a, b)| Arc::ptr_eq(a, b)) {
            Arc::clone(node)
        } else {
            Arc::new(RholangNode::Tuple {
                base: base.clone(),
                elements: new_elements,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a variable identifier node Var).
    ///
    /// # Arguments
    /// * node - The original Var node.
    /// * base - Metadata including position and text.
    /// * name - The variable name.
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// The original node by default; override to transform.
    ///
    /// # Examples
    /// For x, returns unchanged unless overridden.
    fn visit_var(
        &self,
        node: &Arc<RholangNode>,
        _base: &NodeBase,
        _name: &String,
        _metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        Arc::clone(node)
    }

    /// Visits a name declaration node NameDecl) in a new construct.
    ///
    /// # Arguments
    /// * node - The original NameDecl node.
    /// * base - Metadata including position and text.
    /// * var - The variable being declared.
    /// * uri - Optional URI associated with the name.
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// A new NameDecl node if components change, otherwise the original.
    ///
    /// # Examples
    /// For x or x(uri) in new, visits x and uri.
    fn visit_name_decl(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        var: &Arc<RholangNode>,
        uri: &Option<Arc<RholangNode>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_var = self.visit_node(var);
        let new_uri = uri.as_ref().map(|u| self.visit_node(u));
        let var_changed = !Arc::ptr_eq(var, &new_var);
        let uri_changed = match (uri, &new_uri) {
            (Some(u), Some(nu)) => !Arc::ptr_eq(u, nu),
            (None, None) => false,
            _ => true,
        };
        if !var_changed && !uri_changed {
            Arc::clone(node)
        } else {
            Arc::new(RholangNode::NameDecl {
                base: base.clone(),
                var: new_var,
                uri: new_uri,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a declaration node Decl) in a let statement.
    ///
    /// # Arguments
    /// * node - The original Decl node.
    /// * base - Metadata including position and text.
    /// * names - Vector of variables being bound.
    /// * names_remainder - Optional remainder for names.
    /// * procs - Vector of processes bound to variables.
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// A new Decl node if components change, otherwise the original.
    ///
    /// # Examples
    /// For x = P in let, visits x and P.
    fn visit_decl(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        names: &Vector<Arc<RholangNode>, ArcK>,
        names_remainder: &Option<Arc<RholangNode>>,
        procs: &Vector<Arc<RholangNode>, ArcK>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_names = names.iter().map(|n| self.visit_node(n)).collect::<Vector<Arc<RholangNode>, ArcK>>();
        let new_names_remainder = names_remainder.as_ref().map(|r| self.visit_node(r));
        let new_procs = procs.iter().map(|p| self.visit_node(p)).collect::<Vector<Arc<RholangNode>, ArcK>>();
        if names.iter().zip(new_names.iter()).all(|(a, b)| Arc::ptr_eq(a, b)) &&
            names_remainder.as_ref().map_or(true, |r| new_names_remainder.as_ref().map_or(false, |nr| Arc::ptr_eq(r, nr))) &&
            procs.iter().zip(new_procs.iter()).all(|(a, b)| Arc::ptr_eq(a, b)) {
            Arc::clone(node)
        } else {
            Arc::new(RholangNode::Decl {
                base: base.clone(),
                names: new_names,
                names_remainder: new_names_remainder,
                procs: new_procs,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a linear binding node LinearBind) in a for comprehension.
    ///
    /// # Arguments
    /// * node - The original LinearBind node.
    /// * base - Metadata including position and text.
    /// * names - Vector of variables to bind.
    /// * remainder - Optional remainder for the binding.
    /// * source - The source channel expression.
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// A new LinearBind node if components change, otherwise the original.
    ///
    /// # Examples
    /// For x <- ch, visits x and ch.
    fn visit_linear_bind(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        names: &Vector<Arc<RholangNode>, ArcK>,
        remainder: &Option<Arc<RholangNode>>,
        source: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_names = names.iter().map(|n| self.visit_node(n)).collect::<Vector<Arc<RholangNode>, ArcK>>();
        let new_remainder = remainder.as_ref().map(|r| self.visit_node(r));
        let new_source = self.visit_node(source);
        if names.iter().zip(new_names.iter()).all(|(a, b)| Arc::ptr_eq(a, b)) &&
            remainder.as_ref().map_or(true, |r| new_remainder.as_ref().map_or(false, |nr| Arc::ptr_eq(r, nr))) &&
            Arc::ptr_eq(source, &new_source) {
            Arc::clone(node)
        } else {
            Arc::new(RholangNode::LinearBind {
                base: base.clone(),
                names: new_names,
                remainder: new_remainder,
                source: new_source,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a repeated binding node RepeatedBind) in a for comprehension.
    ///
    /// # Arguments
    /// * node - The original RepeatedBind node.
    /// * base - Metadata including position and text.
    /// * names - Vector of variables to bind repeatedly.
    /// * remainder - Optional remainder for the binding.
    /// * source - The source channel expression.
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// A new RepeatedBind node if components change, otherwise the original.
    ///
    /// # Examples
    /// For x <= ch, visits x and ch.
    fn visit_repeated_bind(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        names: &Vector<Arc<RholangNode>, ArcK>,
        remainder: &Option<Arc<RholangNode>>,
        source: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_names = names.iter().map(|n| self.visit_node(n)).collect::<Vector<Arc<RholangNode>, ArcK>>();
        let new_remainder = remainder.as_ref().map(|r| self.visit_node(r));
        let new_source = self.visit_node(source);
        if names.iter().zip(new_names.iter()).all(|(a, b)| Arc::ptr_eq(a, b)) &&
            remainder.as_ref().map_or(true, |r| new_remainder.as_ref().map_or(false, |nr| Arc::ptr_eq(r, nr))) &&
            Arc::ptr_eq(source, &new_source) {
            Arc::clone(node)
        } else {
            Arc::new(RholangNode::RepeatedBind {
                base: base.clone(),
                names: new_names,
                remainder: new_remainder,
                source: new_source,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a peek binding node PeekBind) in a for comprehension.
    ///
    /// # Arguments
    /// * node - The original PeekBind node.
    /// * base - Metadata including position and text.
    /// * names - Vector of variables to peek.
    /// * remainder - Optional remainder for the binding.
    /// * source - The source channel expression.
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// A new PeekBind node if components change, otherwise the original.
    ///
    /// # Examples
    /// For x <<- ch, visits x and ch.
    fn visit_peek_bind(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        names: &Vector<Arc<RholangNode>, ArcK>,
        remainder: &Option<Arc<RholangNode>>,
        source: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_names = names.iter().map(|n| self.visit_node(n)).collect::<Vector<Arc<RholangNode>, ArcK>>();
        let new_remainder = remainder.as_ref().map(|r| self.visit_node(r));
        let new_source = self.visit_node(source);
        if names.iter().zip(new_names.iter()).all(|(a, b)| Arc::ptr_eq(a, b)) &&
            remainder.as_ref().map_or(true, |r| new_remainder.as_ref().map_or(false, |nr| Arc::ptr_eq(r, nr))) &&
            Arc::ptr_eq(source, &new_source) {
            Arc::clone(node)
        } else {
            Arc::new(RholangNode::PeekBind {
                base: base.clone(),
                names: new_names,
                remainder: new_remainder,
                source: new_source,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a comment node Comment).
    ///
    /// # Arguments
    /// * node - The original Comment node.
    /// * base - Metadata including position and text.
    /// * kind - Type of comment (Line or Block).
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// The original node by default; override to transform.
    ///
    /// # Examples
    /// For // text or /* text */, returns unchanged unless overridden.
    fn visit_comment(
        &self,
        node: &Arc<RholangNode>,
        _base: &NodeBase,
        _kind: &CommentKind,
        _metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        Arc::clone(node)
    }

    /// Visits a wildcard pattern node Wildcard).
    ///
    /// # Arguments
    /// * node - The original Wildcard node.
    /// * base - Metadata including position and text.
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// The original node by default; override to transform.
    ///
    /// # Examples
    /// For _, returns unchanged unless overridden.
    fn visit_wildcard(
        &self,
        node: &Arc<RholangNode>,
        _base: &NodeBase,
        _metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        Arc::clone(node)
    }

    /// Visits a simple type annotation node SimpleType).
    ///
    /// # Arguments
    /// * node - The original SimpleType node.
    /// * base - Metadata including position and text.
    /// * value - The type name (e.g., "Bool").
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// The original node by default; override to transform.
    ///
    /// # Examples
    /// For Bool, returns unchanged unless overridden.
    fn visit_simple_type(
        &self,
        node: &Arc<RholangNode>,
        _base: &NodeBase,
        _value: &String,
        _metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        Arc::clone(node)
    }

    /// Visits a receive-send source node ReceiveSendSource).
    ///
    /// # Arguments
    /// * node - The original ReceiveSendSource node.
    /// * base - Metadata including position and text.
    /// * name - The source name or channel.
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// A new ReceiveSendSource node if the name changes, otherwise the original.
    ///
    /// # Examples
    /// For ch?!, visits ch.
    fn visit_receive_send_source(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        name: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_name = self.visit_node(name);
        if Arc::ptr_eq(name, &new_name) {
            Arc::clone(node)
        } else {
            Arc::new(RholangNode::ReceiveSendSource {
                base: base.clone(),
                name: new_name,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a send-receive source node SendReceiveSource).
    ///
    /// # Arguments
    /// * node - The original SendReceiveSource node.
    /// * base - Metadata including position and text.
    /// * name - The source name or channel.
    /// * inputs - Vector of arguments for the operation.
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// A new SendReceiveSource node if components change, otherwise the original.
    ///
    /// # Examples
    /// For ch!?(args), visits ch and args.
    fn visit_send_receive_source(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        name: &Arc<RholangNode>,
        inputs: &Vector<Arc<RholangNode>, ArcK>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_name = self.visit_node(name);
        let new_inputs = inputs.iter().map(|i| self.visit_node(i)).collect::<Vector<Arc<RholangNode>, ArcK>>();
        if Arc::ptr_eq(name, &new_name) && inputs.iter().zip(new_inputs.iter()).all(|(a, b)| Arc::ptr_eq(a, b)) {
            Arc::clone(node)
        } else {
            Arc::new(RholangNode::SendReceiveSource {
                base: base.clone(),
                name: new_name,
                inputs: new_inputs,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits an error node Error), processing its children.
    ///
    /// # Arguments
    /// * node - The original Error node.
    /// * base - Metadata including position and text.
    /// * children - Vector of child nodes within the erroneous subtree.
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// A new Error node with transformed children if any change, otherwise the original.
    ///
    /// # Examples
    /// For an erroneous construct containing send, recurses into its children.
    fn visit_error(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        children: &Vector<Arc<RholangNode>, ArcK>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_children = children.iter().map(|c| self.visit_node(c)).collect::<Vector<Arc<RholangNode>, ArcK>>();
        if children.iter().zip(new_children.iter()).all(|(a, b)| Arc::ptr_eq(a, b)) {
            Arc::clone(node)
        } else {
            Arc::new(RholangNode::Error {
                base: base.clone(),
                children: new_children,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a disjunction pattern node Disjunction).
    ///
    /// # Arguments
    /// * node - The original Disjunction node.
    /// * base - Metadata including position and text.
    /// * left - Left pattern.
    /// * right - Right pattern.
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// A new Disjunction node if patterns change, otherwise the original.
    ///
    /// # Examples
    /// For P \/ Q in patterns, visits P and Q.
    fn visit_disjunction(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        left: &Arc<RholangNode>,
        right: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_left = self.visit_node(left);
        let new_right = self.visit_node(right);
        if Arc::ptr_eq(left, &new_left) && Arc::ptr_eq(right, &new_right) {
            Arc::clone(node)
        } else {
            Arc::new(RholangNode::Disjunction {
                base: base.clone(),
                left: new_left,
                right: new_right,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a conjunction pattern node Conjunction).
    ///
    /// # Arguments
    /// * node - The original Conjunction node.
    /// * base - Metadata including position and text.
    /// * left - Left pattern.
    /// * right - Right pattern.
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// A new Conjunction node if patterns change, otherwise the original.
    ///
    /// # Examples
    /// For P /\ Q in patterns, visits P and Q.
    fn visit_conjunction(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        left: &Arc<RholangNode>,
        right: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_left = self.visit_node(left);
        let new_right = self.visit_node(right);
        if Arc::ptr_eq(left, &new_left) && Arc::ptr_eq(right, &new_right) {
            Arc::clone(node)
        } else {
            Arc::new(RholangNode::Conjunction {
                base: base.clone(),
                left: new_left,
                right: new_right,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a negation pattern node Negation).
    ///
    /// # Arguments
    /// * node - The original Negation node.
    /// * base - Metadata including position and text.
    /// * operand - The pattern being negated.
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// A new Negation node if the operand changes, otherwise the original.
    ///
    /// # Examples
    /// For ~P in patterns, visits P.
    fn visit_negation(
        &self,
        node: &Arc<RholangNode>,
        base: &NodeBase,
        operand: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_operand = self.visit_node(operand);
        if Arc::ptr_eq(operand, &new_operand) {
            Arc::clone(node)
        } else {
            Arc::new(RholangNode::Negation {
                base: base.clone(),
                operand: new_operand,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a unit value node Unit).
    ///
    /// # Arguments
    /// * node - The original Unit node.
    /// * base - Metadata including position and text.
    /// * metadata - Optional node metadata.
    ///
    /// # Returns
    /// The original node by default; override to transform.
    ///
    /// # Examples
    /// For (), returns unchanged unless overridden.
    fn visit_unit(
        &self,
        node: &Arc<RholangNode>,
        _base: &NodeBase,
        _metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        Arc::clone(node)
    }
}
