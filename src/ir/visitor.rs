use std::sync::Arc;
use rpds::Vector;
use super::node::{Node, NodeBase, Metadata, CommentKind, SendType, BundleType, BinOperator, UnaryOperator, VarRefKind, Position};

/// Provides a visitor pattern for traversing and transforming the Rholang Intermediate Representation (IR) tree.
/// This module enables implementors to define custom logic for processing each node type, facilitating operations
/// such as optimization, analysis, or formatting of the IR tree.
pub trait Visitor {
    /// Entry point for visiting an IR node, dispatching to the appropriate type-specific method.
    /// Implementors typically do not override this method unless custom dispatching is needed.
    ///
    /// # Arguments
    /// * `node` - The node to visit.
    ///
    /// # Returns
    /// The transformed node, or the original if unchanged.
    fn visit_node<'a>(&self, node: &Arc<Node<'a>>) -> Arc<Node<'a>> {
        match &**node {
            Node::Par { base, left, right, metadata } => self.visit_par(node, base, left, right, metadata),
            Node::SendSync { base, channel, inputs, cont, metadata } => self.visit_sendsync(node, base, channel, inputs, cont, metadata),
            Node::Send { base, channel, send_type, send_type_end, inputs, metadata } => {
                self.visit_send(node, base, channel, send_type, send_type_end, inputs, metadata)
            }
            Node::New { base, decls, proc, metadata } => self.visit_new(node, base, decls, proc, metadata),
            Node::IfElse { base, condition, consequence, alternative, metadata } => self.visit_ifelse(node, base, condition, consequence, alternative, metadata),
            Node::Let { base, decls, proc, metadata } => self.visit_let(node, base, decls, proc, metadata),
            Node::Bundle { base, bundle_type, proc, metadata } => self.visit_bundle(node, base, bundle_type, proc, metadata),
            Node::Match { base, expression, cases, metadata } => self.visit_match(node, base, expression, cases, metadata),
            Node::Choice { base, branches, metadata } => self.visit_choice(node, base, branches, metadata),
            Node::Contract { base, name, formals, proc, metadata } => self.visit_contract(node, base, name, formals, proc, metadata),
            Node::Input { base, receipts, proc, metadata } => self.visit_input(node, base, receipts, proc, metadata),
            Node::Block { base, proc, metadata } => self.visit_block(node, base, proc, metadata),
            Node::BinOp { base, op, left, right, metadata } => self.visit_binop(node, base, op.clone(), left, right, metadata),
            Node::UnaryOp { base, op, operand, metadata } => self.visit_unaryop(node, base, op.clone(), operand, metadata),
            Node::Method { base, receiver, name, args, metadata } => self.visit_method(node, base, receiver, name, args, metadata),
            Node::Eval { base, name, metadata } => self.visit_eval(node, base, name, metadata),
            Node::Quote { base, quotable, metadata } => self.visit_quote(node, base, quotable, metadata),
            Node::VarRef { base, kind, var, metadata } => self.visit_varref(node, base, kind.clone(), var, metadata),
            Node::BoolLiteral { base, value, metadata } => self.visit_bool_literal(node, base, *value, metadata),
            Node::LongLiteral { base, value, metadata } => self.visit_long_literal(node, base, *value, metadata),
            Node::StringLiteral { base, value, metadata } => self.visit_string_literal(node, base, value, metadata),
            Node::UriLiteral { base, value, metadata } => self.visit_uri_literal(node, base, value, metadata),
            Node::Nil { base, metadata } => self.visit_nil(node, base, metadata),
            Node::List { base, elements, remainder, metadata } => self.visit_list(node, base, elements, remainder, metadata),
            Node::Set { base, elements, remainder, metadata } => self.visit_set(node, base, elements, remainder, metadata),
            Node::Map { base, pairs, remainder, metadata } => self.visit_map(node, base, pairs, remainder, metadata),
            Node::Tuple { base, elements, metadata } => self.visit_tuple(node, base, elements, metadata),
            Node::Var { base, name, metadata } => self.visit_var(node, base, name, metadata),
            Node::NameDecl { base, var, uri, metadata } => self.visit_name_decl(node, base, var, uri, metadata),
            Node::Decl { base, names, procs, metadata } => self.visit_decl(node, base, names, procs, metadata),
            Node::LinearBind { base, names, source, metadata } => self.visit_linear_bind(node, base, names, source, metadata),
            Node::RepeatedBind { base, names, source, metadata } => self.visit_repeated_bind(node, base, names, source, metadata),
            Node::PeekBind { base, names, source, metadata } => self.visit_peek_bind(node, base, names, source, metadata),
            Node::Comment { base, kind, metadata } => self.visit_comment(node, base, kind, metadata),
            Node::Wildcard { base, metadata } => self.visit_wildcard(node, base, metadata),
            Node::SimpleType { base, value, metadata } => self.visit_simple_type(node, base, value, metadata),
            Node::ReceiveSendSource { base, name, metadata } => self.visit_receive_send_source(node, base, name, metadata),
            Node::SendReceiveSource { base, name, inputs, metadata } => self.visit_send_receive_source(node, base, name, inputs, metadata),
        }
    }

    /// Visits a parallel composition node (`Par`), processing its subprocesses.
    ///
    /// # Arguments
    /// * `node` - The original `Par` node for reference.
    /// * `base` - Metadata including position and text.
    /// * `left` - Left subprocess.
    /// * `right` - Right subprocess.
    /// * `metadata` - Optional node metadata.
    ///
    /// # Returns
    /// A new `Par` node if subprocesses change, otherwise the original.
    ///
    /// # Examples
    /// For `P | Q`, visits `P` and `Q`, reconstructing the node if either changes.
    fn visit_par<'a>(
        &self,
        node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        left: &Arc<Node<'a>>,
        right: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_left = self.visit_node(left);
        let new_right = self.visit_node(right);
        if Arc::ptr_eq(left, &new_left) && Arc::ptr_eq(right, &new_right) {
            Arc::clone(node)
        } else {
            Arc::new(Node::Par {
                base: base.clone(),
                left: new_left,
                right: new_right,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a synchronous send node (`SendSync`), handling channel, inputs, and continuation.
    ///
    /// # Arguments
    /// * `node` - The original `SendSync` node.
    /// * `base` - Metadata including position and text.
    /// * `channel` - The channel expression.
    /// * `inputs` - Arguments sent synchronously.
    /// * `cont` - Continuation process after sending.
    /// * `metadata` - Optional node metadata.
    ///
    /// # Returns
    /// A new `SendSync` node if any component changes, otherwise the original.
    ///
    /// # Examples
    /// For `ch!?("msg"; Nil)`, processes `ch`, `"msg"`, and `Nil`.
    fn visit_sendsync<'a>(
        &self,
        node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        channel: &Arc<Node<'a>>,
        inputs: &Vector<Arc<Node<'a>>>,
        cont: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_channel = self.visit_node(channel);
        let new_inputs = inputs.iter().map(|i| self.visit_node(i)).collect::<Vector<_>>();
        let new_cont = self.visit_node(cont);
        if Arc::ptr_eq(channel, &new_channel) &&
           inputs.iter().zip(&new_inputs).all(|(a, b)| Arc::ptr_eq(a, b)) &&
           Arc::ptr_eq(cont, &new_cont) {
            Arc::clone(node)
        } else {
            Arc::new(Node::SendSync {
                base: base.clone(),
                channel: new_channel,
                inputs: new_inputs,
                cont: new_cont,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits an asynchronous send node (`Send`), processing its channel and inputs.
    ///
    /// # Arguments
    /// * `node` - The original `Send` node.
    /// * `base` - Metadata including position and text.
    /// * `channel` - The channel expression.
    /// * `send_type` - Single (`!`) or multiple (`!!`) send type.
    /// * `send_type_end` - Position after the send type token.
    /// * `inputs` - Arguments sent asynchronously.
    /// * `metadata` - Optional node metadata.
    ///
    /// # Returns
    /// A new `Send` node if components change, otherwise the original.
    ///
    /// # Examples
    /// For `ch!("msg")`, visits `ch` and `"msg"`.
    fn visit_send<'a>(
        &self,
        node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        channel: &Arc<Node<'a>>,
        send_type: &SendType,
        send_type_end: &Position,
        inputs: &Vector<Arc<Node<'a>>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_channel = self.visit_node(channel);
        let new_inputs = inputs.iter().map(|i| self.visit_node(i)).collect::<Vector<_>>();
        if Arc::ptr_eq(channel, &new_channel) &&
            inputs.iter().zip(&new_inputs).all(|(a, b)| Arc::ptr_eq(a, b)) {
            Arc::clone(node)
        } else {
            Arc::new(Node::Send {
                base: base.clone(),
                channel: new_channel,
                send_type: send_type.clone(),
                send_type_end: *send_type_end,
                inputs: new_inputs,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a new name declaration node (`New`), handling declarations and scoped process.
    ///
    /// # Arguments
    /// * `node` - The original `New` node.
    /// * `base` - Metadata including position and text.
    /// * `decls` - Vector of name declarations.
    /// * `proc` - The scoped process.
    /// * `metadata` - Optional node metadata.
    ///
    /// # Returns
    /// A new `New` node if components change, otherwise the original.
    ///
    /// # Examples
    /// For `new x in { P }`, visits `x` and `P`.
    fn visit_new<'a>(
        &self,
        node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        decls: &Vector<Arc<Node<'a>>>,
        proc: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_decls = decls.iter().map(|d| self.visit_node(d)).collect::<Vector<_>>();
        let new_proc = self.visit_node(proc);
        if decls.iter().zip(&new_decls).all(|(a, b)| Arc::ptr_eq(a, b)) &&
           Arc::ptr_eq(proc, &new_proc) {
            Arc::clone(node)
        } else {
            Arc::new(Node::New {
                base: base.clone(),
                decls: new_decls,
                proc: new_proc,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a conditional node (`IfElse`), processing condition, consequence, and optional alternative.
    ///
    /// # Arguments
    /// * `node` - The original `IfElse` node.
    /// * `base` - Metadata including position and text.
    /// * `condition` - The condition expression.
    /// * `consequence` - Process if condition is true.
    /// * `alternative` - Optional process if condition is false.
    /// * `metadata` - Optional node metadata.
    ///
    /// # Returns
    /// A new `IfElse` node if components change, otherwise the original.
    ///
    /// # Examples
    /// For `if (cond) { P } else { Q }`, visits `cond`, `P`, and `Q`.
    fn visit_ifelse<'a>(
        &self,
        node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        condition: &Arc<Node<'a>>,
        consequence: &Arc<Node<'a>>,
        alternative: &Option<Arc<Node<'a>>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_condition = self.visit_node(condition);
        let new_consequence = self.visit_node(consequence);
        let new_alternative = alternative.as_ref().map(|a| self.visit_node(a));
        let condition_changed = !Arc::ptr_eq(condition, &new_condition);
        let consequence_changed = !Arc::ptr_eq(consequence, &new_consequence);
        let alternative_changed = match (alternative, &new_alternative) {
            (Some(a), Some(na)) => !Arc::ptr_eq(a, na),
            (None, None) => false,
            _ => true,
        };
        if !condition_changed && !consequence_changed && !alternative_changed {
            Arc::clone(node)
        } else {
            Arc::new(Node::IfElse {
                base: base.clone(),
                condition: new_condition,
                consequence: new_consequence,
                alternative: new_alternative,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a variable binding node (`Let`), handling declarations and process.
    ///
    /// # Arguments
    /// * `node` - The original `Let` node.
    /// * `base` - Metadata including position and text.
    /// * `decls` - Vector of variable declarations.
    /// * `proc` - The process using the bindings.
    /// * `metadata` - Optional node metadata.
    ///
    /// # Returns
    /// A new `Let` node if components change, otherwise the original.
    ///
    /// # Examples
    /// For `let x = "val" in { P }`, visits `x = "val"` and `P`.
    fn visit_let<'a>(
        &self,
        node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        decls: &Vector<Arc<Node<'a>>>,
        proc: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_decls = decls.iter().map(|d| self.visit_node(d)).collect::<Vector<_>>();
        let new_proc = self.visit_node(proc);
        if decls.iter().zip(&new_decls).all(|(a, b)| Arc::ptr_eq(a, b)) &&
           Arc::ptr_eq(proc, &new_proc) {
            Arc::clone(node)
        } else {
            Arc::new(Node::Let {
                base: base.clone(),
                decls: new_decls,
                proc: new_proc,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits an access-controlled process node (`Bundle`), processing its process.
    ///
    /// # Arguments
    /// * `node` - The original `Bundle` node.
    /// * `base` - Metadata including position and text.
    /// * `bundle_type` - Type of access control (e.g., Read, Write).
    /// * `proc` - The bundled process.
    /// * `metadata` - Optional node metadata.
    ///
    /// # Returns
    /// A new `Bundle` node if the process changes, otherwise the original.
    ///
    /// # Examples
    /// For `bundle+ { P }`, visits `P`.
    fn visit_bundle<'a>(
        &self,
        node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        bundle_type: &BundleType,
        proc: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_proc = self.visit_node(proc);
        if Arc::ptr_eq(proc, &new_proc) {
            Arc::clone(node)
        } else {
            Arc::new(Node::Bundle {
                base: base.clone(),
                bundle_type: bundle_type.clone(),
                proc: new_proc,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a pattern matching node (`Match`), processing expression and cases.
    ///
    /// # Arguments
    /// * `node` - The original `Match` node.
    /// * `base` - Metadata including position and text.
    /// * `expression` - The expression to match against.
    /// * `cases` - Vector of (pattern, process) pairs.
    /// * `metadata` - Optional node metadata.
    ///
    /// # Returns
    /// A new `Match` node if components change, otherwise the original.
    ///
    /// # Examples
    /// For `match expr { pat => P }`, visits `expr`, `pat`, and `P`.
    fn visit_match<'a>(
        &self,
        node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        expression: &Arc<Node<'a>>,
        cases: &Vector<(Arc<Node<'a>>, Arc<Node<'a>>)>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_expression = self.visit_node(expression);
        let new_cases = cases.iter().map(|(pat, proc)| {
            (self.visit_node(pat), self.visit_node(proc))
        }).collect::<Vector<_>>();
        let expression_changed = !Arc::ptr_eq(expression, &new_expression);
        let cases_changed = cases.iter().zip(&new_cases).any(|((p1, r1), (p2, r2))| {
            !Arc::ptr_eq(p1, p2) || !Arc::ptr_eq(r1, r2)
        });
        if !expression_changed && !cases_changed {
            Arc::clone(node)
        } else {
            Arc::new(Node::Match {
                base: base.clone(),
                expression: new_expression,
                cases: new_cases,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a non-deterministic choice node (`Choice`), processing its branches.
    ///
    /// # Arguments
    /// * `node` - The original `Choice` node.
    /// * `base` - Metadata including position and text.
    /// * `branches` - Vector of (inputs, process) pairs.
    /// * `metadata` - Optional node metadata.
    ///
    /// # Returns
    /// A new `Choice` node if branches change, otherwise the original.
    ///
    /// # Examples
    /// For `select { x <- ch => P }`, visits `x <- ch` and `P`.
    fn visit_choice<'a>(
        &self,
        node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        branches: &Vector<(Vector<Arc<Node<'a>>>, Arc<Node<'a>>)>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_branches = branches.iter().map(|(inputs, proc)| {
            let new_inputs = inputs.iter().map(|i| self.visit_node(i)).collect::<Vector<_>>();
            let new_proc = self.visit_node(proc);
            (new_inputs, new_proc)
        }).collect::<Vector<_>>();
        let branches_changed = branches.iter().zip(&new_branches).any(|((i1, p1), (i2, p2))| {
            i1.iter().zip(i2).any(|(a, b)| !Arc::ptr_eq(a, b)) || !Arc::ptr_eq(p1, p2)
        });
        if !branches_changed {
            Arc::clone(node)
        } else {
            Arc::new(Node::Choice {
                base: base.clone(),
                branches: new_branches,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a contract definition node (`Contract`), processing name, parameters, and body.
    ///
    /// # Arguments
    /// * `node` - The original `Contract` node.
    /// * `base` - Metadata including position and text.
    /// * `name` - The contract’s name.
    /// * `formals` - Vector of formal parameters.
    /// * `proc` - The contract body process.
    /// * `metadata` - Optional node metadata.
    ///
    /// # Returns
    /// A new `Contract` node if components change, otherwise the original.
    ///
    /// # Examples
    /// For `contract name(args) = { P }`, visits `name`, `args`, and `P`.
    fn visit_contract<'a>(
        &self,
        node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        name: &Arc<Node<'a>>,
        formals: &Vector<Arc<Node<'a>>>,
        proc: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_name = self.visit_node(name);
        let new_formals = formals.iter().map(|f| self.visit_node(f)).collect::<Vector<_>>();
        let new_proc = self.visit_node(proc);
        if Arc::ptr_eq(name, &new_name) &&
           formals.iter().zip(&new_formals).all(|(a, b)| Arc::ptr_eq(a, b)) &&
           Arc::ptr_eq(proc, &new_proc) {
            Arc::clone(node)
        } else {
            Arc::new(Node::Contract {
                base: base.clone(),
                name: new_name,
                formals: new_formals,
                proc: new_proc,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits an input binding node (`Input`), processing receipts and process.
    ///
    /// # Arguments
    /// * `node` - The original `Input` node.
    /// * `base` - Metadata including position and text.
    /// * `receipts` - Vector of binding groups from channels.
    /// * `proc` - The process after receiving.
    /// * `metadata` - Optional node metadata.
    ///
    /// # Returns
    /// A new `Input` node if components change, otherwise the original.
    ///
    /// # Examples
    /// For `for (x <- ch) { P }`, visits `x <- ch` and `P`.
    fn visit_input<'a>(
        &self,
        node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        receipts: &Vector<Vector<Arc<Node<'a>>>>,
        proc: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_receipts = receipts.iter().map(|r| {
            r.iter().map(|n| self.visit_node(n)).collect::<Vector<_>>()
        }).collect::<Vector<_>>();
        let new_proc = self.visit_node(proc);
        let receipts_changed = receipts.iter().zip(&new_receipts).any(|(r1, r2)| {
            r1.iter().zip(r2).any(|(a, b)| !Arc::ptr_eq(a, b))
        });
        if !receipts_changed && Arc::ptr_eq(proc, &new_proc) {
            Arc::clone(node)
        } else {
            Arc::new(Node::Input {
                base: base.clone(),
                receipts: new_receipts,
                proc: new_proc,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a block node (`Block`), processing its contained process.
    ///
    /// # Arguments
    /// * `node` - The original `Block` node.
    /// * `base` - Metadata including position and text.
    /// * `proc` - The process within the block.
    /// * `metadata` - Optional node metadata.
    ///
    /// # Returns
    /// A new `Block` node if the process changes, otherwise the original.
    ///
    /// # Examples
    /// For `{ P }`, visits `P`.
    fn visit_block<'a>(
        &self,
        node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        proc: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_proc = self.visit_node(proc);
        if Arc::ptr_eq(proc, &new_proc) {
            Arc::clone(node)
        } else {
            Arc::new(Node::Block {
                base: base.clone(),
                proc: new_proc,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a binary operation node (`BinOp`), processing its operands.
    ///
    /// # Arguments
    /// * `node` - The original `BinOp` node.
    /// * `base` - Metadata including position and text.
    /// * `op` - The binary operator (e.g., `Add`, `Eq`).
    /// * `left` - Left operand.
    /// * `right` - Right operand.
    /// * `metadata` - Optional node metadata.
    ///
    /// # Returns
    /// A new `BinOp` node if operands change, otherwise the original.
    ///
    /// # Examples
    /// For `a + b`, visits `a` and `b`.
    fn visit_binop<'a>(
        &self,
        node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        op: BinOperator,
        left: &Arc<Node<'a>>,
        right: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_left = self.visit_node(left);
        let new_right = self.visit_node(right);
        if Arc::ptr_eq(left, &new_left) && Arc::ptr_eq(right, &new_right) {
            Arc::clone(node)
        } else {
            Arc::new(Node::BinOp {
                base: base.clone(),
                op,
                left: new_left,
                right: new_right,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a unary operation node (`UnaryOp`), processing its operand.
    ///
    /// # Arguments
    /// * `node` - The original `UnaryOp` node.
    /// * `base` - Metadata including position and text.
    /// * `op` - The unary operator (e.g., `Neg`, `Not`).
    /// * `operand` - The operand expression.
    /// * `metadata` - Optional node metadata.
    ///
    /// # Returns
    /// A new `UnaryOp` node if the operand changes, otherwise the original.
    ///
    /// # Examples
    /// For `not P`, visits `P`.
    fn visit_unaryop<'a>(
        &self,
        node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        op: UnaryOperator,
        operand: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_operand = self.visit_node(operand);
        if Arc::ptr_eq(operand, &new_operand) {
            Arc::clone(node)
        } else {
            Arc::new(Node::UnaryOp {
                base: base.clone(),
                op,
                operand: new_operand,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a method call node (`Method`), processing receiver and arguments.
    ///
    /// # Arguments
    /// * `node` - The original `Method` node.
    /// * `base` - Metadata including position and text.
    /// * `receiver` - The receiver expression.
    /// * `name` - Method name as a string.
    /// * `args` - Vector of argument expressions.
    /// * `metadata` - Optional node metadata.
    ///
    /// # Returns
    /// A new `Method` node if components change, otherwise the original.
    ///
    /// # Examples
    /// For `obj.method(args)`, visits `obj` and `args`.
    fn visit_method<'a>(
        &self,
        node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        receiver: &Arc<Node<'a>>,
        name: &String,
        args: &Vector<Arc<Node<'a>>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_receiver = self.visit_node(receiver);
        let new_args = args.iter().map(|a| self.visit_node(a)).collect::<Vector<_>>();
        if Arc::ptr_eq(receiver, &new_receiver) &&
           args.iter().zip(&new_args).all(|(a, b)| Arc::ptr_eq(a, b)) {
            Arc::clone(node)
        } else {
            Arc::new(Node::Method {
                base: base.clone(),
                receiver: new_receiver,
                name: name.clone(),
                args: new_args,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits an evaluation node (`Eval`), processing the name to evaluate.
    ///
    /// # Arguments
    /// * `node` - The original `Eval` node.
    /// * `base` - Metadata including position and text.
    /// * `name` - The name expression to evaluate.
    /// * `metadata` - Optional node metadata.
    ///
    /// # Returns
    /// A new `Eval` node if the name changes, otherwise the original.
    ///
    /// # Examples
    /// For `*name`, visits `name`.
    fn visit_eval<'a>(
        &self,
        node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        name: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_name = self.visit_node(name);
        if Arc::ptr_eq(name, &new_name) {
            Arc::clone(node)
        } else {
            Arc::new(Node::Eval {
                base: base.clone(),
                name: new_name,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a quotation node (`Quote`), processing the quoted process.
    ///
    /// # Arguments
    /// * `node` - The original `Quote` node.
    /// * `base` - Metadata including position and text.
    /// * `quotable` - The process being quoted.
    /// * `metadata` - Optional node metadata.
    ///
    /// # Returns
    /// A new `Quote` node if the quotable changes, otherwise the original.
    ///
    /// # Examples
    /// For `@P`, visits `P`.
    fn visit_quote<'a>(
        &self,
        node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        quotable: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_quotable = self.visit_node(quotable);
        if Arc::ptr_eq(quotable, &new_quotable) {
            Arc::clone(node)
        } else {
            Arc::new(Node::Quote {
                base: base.clone(),
                quotable: new_quotable,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a variable reference node (`VarRef`), processing the referenced variable.
    ///
    /// # Arguments
    /// * `node` - The original `VarRef` node.
    /// * `base` - Metadata including position and text.
    /// * `kind` - Reference kind (e.g., `Assign`, `AssignStar`).
    /// * `var` - The variable expression.
    /// * `metadata` - Optional node metadata.
    ///
    /// # Returns
    /// A new `VarRef` node if the variable changes, otherwise the original.
    ///
    /// # Examples
    /// For `=x` or `=*x`, visits `x`.
    fn visit_varref<'a>(
        &self,
        node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        kind: VarRefKind,
        var: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_var = self.visit_node(var);
        if Arc::ptr_eq(var, &new_var) {
            Arc::clone(node)
        } else {
            Arc::new(Node::VarRef {
                base: base.clone(),
                kind,
                var: new_var,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a boolean literal node (`BoolLiteral`).
    ///
    /// # Arguments
    /// * `node` - The original `BoolLiteral` node.
    /// * `base` - Metadata including position and text.
    /// * `value` - The boolean value (`true` or `false`).
    /// * `metadata` - Optional node metadata.
    ///
    /// # Returns
    /// The original node by default; override to transform.
    ///
    /// # Examples
    /// For `true`, returns unchanged unless overridden.
    fn visit_bool_literal<'a>(
        &self,
        node: &Arc<Node<'a>>,
        _base: &NodeBase<'a>,
        _value: bool,
        _metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        Arc::clone(node)
    }

    /// Visits an integer literal node (`LongLiteral`).
    ///
    /// # Arguments
    /// * `node` - The original `LongLiteral` node.
    /// * `base` - Metadata including position and text.
    /// * `value` - The integer value.
    /// * `metadata` - Optional node metadata.
    ///
    /// # Returns
    /// The original node by default; override to transform.
    ///
    /// # Examples
    /// For `42`, returns unchanged unless overridden.
    fn visit_long_literal<'a>(
        &self,
        node: &Arc<Node<'a>>,
        _base: &NodeBase<'a>,
        _value: i64,
        _metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        Arc::clone(node)
    }

    /// Visits a string literal node (`StringLiteral`).
    ///
    /// # Arguments
    /// * `node` - The original `StringLiteral` node.
    /// * `base` - Metadata including position and text.
    /// * `value` - The string content.
    /// * `metadata` - Optional node metadata.
    ///
    /// # Returns
    /// The original node by default; override to transform.
    ///
    /// # Examples
    /// For `"hello"`, returns unchanged unless overridden.
    fn visit_string_literal<'a>(
        &self,
        node: &Arc<Node<'a>>,
        _base: &NodeBase<'a>,
        _value: &String,
        _metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        Arc::clone(node)
    }

    /// Visits a URI literal node (`UriLiteral`).
    ///
    /// # Arguments
    /// * `node` - The original `UriLiteral` node.
    /// * `base` - Metadata including position and text.
    /// * `value` - The URI string.
    /// * `metadata` - Optional node metadata.
    ///
    /// # Returns
    /// The original node by default; override to transform.
    ///
    /// # Examples
    /// For `` `http://example.com` ``, returns unchanged unless overridden.
    fn visit_uri_literal<'a>(
        &self,
        node: &Arc<Node<'a>>,
        _base: &NodeBase<'a>,
        _value: &String,
        _metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        Arc::clone(node)
    }

    /// Visits an empty process node (`Nil`).
    ///
    /// # Arguments
    /// * `node` - The original `Nil` node.
    /// * `base` - Metadata including position and text.
    /// * `metadata` - Optional node metadata.
    ///
    /// # Returns
    /// The original node by default; override to transform.
    ///
    /// # Examples
    /// For `Nil`, returns unchanged unless overridden.
    fn visit_nil<'a>(
        &self,
        node: &Arc<Node<'a>>,
        _base: &NodeBase<'a>,
        _metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        Arc::clone(node)
    }

    /// Visits a list collection node (`List`), processing elements and remainder.
    ///
    /// # Arguments
    /// * `node` - The original `List` node.
    /// * `base` - Metadata including position and text.
    /// * `elements` - Vector of list elements.
    /// * `remainder` - Optional remainder (e.g., `...rest`).
    /// * `metadata` - Optional node metadata.
    ///
    /// # Returns
    /// A new `List` node if components change, otherwise the original.
    ///
    /// # Examples
    /// For `[a, b, ...rest]`, visits `a`, `b`, and `rest`.
    fn visit_list<'a>(
        &self,
        node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        elements: &Vector<Arc<Node<'a>>>,
        remainder: &Option<Arc<Node<'a>>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_elements = elements.iter().map(|e| self.visit_node(e)).collect::<Vector<_>>();
        let new_remainder = remainder.as_ref().map(|r| self.visit_node(r));
        let elements_changed = elements.iter().zip(&new_elements).any(|(a, b)| !Arc::ptr_eq(a, b));
        let remainder_changed = match (remainder, &new_remainder) {
            (Some(r), Some(nr)) => !Arc::ptr_eq(r, nr),
            (None, None) => false,
            _ => true,
        };
        if !elements_changed && !remainder_changed {
            Arc::clone(node)
        } else {
            Arc::new(Node::List {
                base: base.clone(),
                elements: new_elements,
                remainder: new_remainder,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a set collection node (`Set`), processing elements and remainder.
    ///
    /// # Arguments
    /// * `node` - The original `Set` node.
    /// * `base` - Metadata including position and text.
    /// * `elements` - Vector of set elements.
    /// * `remainder` - Optional remainder (e.g., `...rest`).
    /// * `metadata` - Optional node metadata.
    ///
    /// # Returns
    /// A new `Set` node if components change, otherwise the original.
    ///
    /// # Examples
    /// For `Set(a, b, ...rest)`, visits `a`, `b`, and `rest`.
    fn visit_set<'a>(
        &self,
        node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        elements: &Vector<Arc<Node<'a>>>,
        remainder: &Option<Arc<Node<'a>>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_elements = elements.iter().map(|e| self.visit_node(e)).collect::<Vector<_>>();
        let new_remainder = remainder.as_ref().map(|r| self.visit_node(r));
        let elements_changed = elements.iter().zip(&new_elements).any(|(a, b)| !Arc::ptr_eq(a, b));
        let remainder_changed = match (remainder, &new_remainder) {
            (Some(r), Some(nr)) => !Arc::ptr_eq(r, nr),
            (None, None) => false,
            _ => true,
        };
        if !elements_changed && !remainder_changed {
            Arc::clone(node)
        } else {
            Arc::new(Node::Set {
                base: base.clone(),
                elements: new_elements,
                remainder: new_remainder,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a map collection node (`Map`), processing key-value pairs and remainder.
    ///
    /// # Arguments
    /// * `node` - The original `Map` node.
    /// * `base` - Metadata including position and text.
    /// * `pairs` - Vector of (key, value) pairs.
    /// * `remainder` - Optional remainder (e.g., `...rest`).
    /// * `metadata` - Optional node metadata.
    ///
    /// # Returns
    /// A new `Map` node if components change, otherwise the original.
    ///
    /// # Examples
    /// For `{k: v, ...rest}`, visits `k`, `v`, and `rest`.
    fn visit_map<'a>(
        &self,
        node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        pairs: &Vector<(Arc<Node<'a>>, Arc<Node<'a>>)>,
        remainder: &Option<Arc<Node<'a>>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_pairs = pairs.iter().map(|(k, v)| {
            (self.visit_node(k), self.visit_node(v))
        }).collect::<Vector<_>>();
        let new_remainder = remainder.as_ref().map(|r| self.visit_node(r));
        let pairs_changed = pairs.iter().zip(&new_pairs).any(|((k1, v1), (k2, v2))| {
            !Arc::ptr_eq(k1, k2) || !Arc::ptr_eq(v1, v2)
        });
        let remainder_changed = match (remainder, &new_remainder) {
            (Some(r), Some(nr)) => !Arc::ptr_eq(r, nr),
            (None, None) => false,
            _ => true,
        };
        if !pairs_changed && !remainder_changed {
            Arc::clone(node)
        } else {
            Arc::new(Node::Map {
                base: base.clone(),
                pairs: new_pairs,
                remainder: new_remainder,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a tuple collection node (`Tuple`), processing its elements.
    ///
    /// # Arguments
    /// * `node` - The original `Tuple` node.
    /// * `base` - Metadata including position and text.
    /// * `elements` - Vector of tuple elements.
    /// * `metadata` - Optional node metadata.
    ///
    /// # Returns
    /// A new `Tuple` node if elements change, otherwise the original.
    ///
    /// # Examples
    /// For `(a, b)`, visits `a` and `b`.
    fn visit_tuple<'a>(
        &self,
        node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        elements: &Vector<Arc<Node<'a>>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_elements = elements.iter().map(|e| self.visit_node(e)).collect::<Vector<_>>();
        if elements.iter().zip(&new_elements).all(|(a, b)| Arc::ptr_eq(a, b)) {
            Arc::clone(node)
        } else {
            Arc::new(Node::Tuple {
                base: base.clone(),
                elements: new_elements,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a variable identifier node (`Var`).
    ///
    /// # Arguments
    /// * `node` - The original `Var` node.
    /// * `base` - Metadata including position and text.
    /// * `name` - The variable name.
    /// * `metadata` - Optional node metadata.
    ///
    /// # Returns
    /// The original node by default; override to transform.
    ///
    /// # Examples
    /// For `x`, returns unchanged unless overridden.
    fn visit_var<'a>(
        &self,
        node: &Arc<Node<'a>>,
        _base: &NodeBase<'a>,
        _name: &String,
        _metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        Arc::clone(node)
    }

    /// Visits a name declaration node (`NameDecl`) in a `new` construct.
    ///
    /// # Arguments
    /// * `node` - The original `NameDecl` node.
    /// * `base` - Metadata including position and text.
    /// * `var` - The variable being declared.
    /// * `uri` - Optional URI associated with the name.
    /// * `metadata` - Optional node metadata.
    ///
    /// # Returns
    /// A new `NameDecl` node if components change, otherwise the original.
    ///
    /// # Examples
    /// For `x` or `x(uri)` in `new`, visits `x` and `uri`.
    fn visit_name_decl<'a>(
        &self,
        node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        var: &Arc<Node<'a>>,
        uri: &Option<Arc<Node<'a>>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
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
            Arc::new(Node::NameDecl {
                base: base.clone(),
                var: new_var,
                uri: new_uri,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a declaration node (`Decl`) in a `let` statement.
    ///
    /// # Arguments
    /// * `node` - The original `Decl` node.
    /// * `base` - Metadata including position and text.
    /// * `names` - Vector of variables being bound.
    /// * `procs` - Vector of processes bound to variables.
    /// * `metadata` - Optional node metadata.
    ///
    /// # Returns
    /// A new `Decl` node if components change, otherwise the original.
    ///
    /// # Examples
    /// For `x = P` in `let`, visits `x` and `P`.
    fn visit_decl<'a>(
        &self,
        node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        names: &Vector<Arc<Node<'a>>>,
        procs: &Vector<Arc<Node<'a>>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_names = names.iter().map(|n| self.visit_node(n)).collect::<Vector<_>>();
        let new_procs = procs.iter().map(|p| self.visit_node(p)).collect::<Vector<_>>();
        let names_changed = names.iter().zip(&new_names).any(|(a, b)| !Arc::ptr_eq(a, b));
        let procs_changed = procs.iter().zip(&new_procs).any(|(a, b)| !Arc::ptr_eq(a, b));
        if !names_changed && !procs_changed {
            Arc::clone(node)
        } else {
            Arc::new(Node::Decl {
                base: base.clone(),
                names: new_names,
                procs: new_procs,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a linear binding node (`LinearBind`) in a `for` comprehension.
    ///
    /// # Arguments
    /// * `node` - The original `LinearBind` node.
    /// * `base` - Metadata including position and text.
    /// * `names` - Vector of variables to bind.
    /// * `source` - The source channel expression.
    /// * `metadata` - Optional node metadata.
    ///
    /// # Returns
    /// A new `LinearBind` node if components change, otherwise the original.
    ///
    /// # Examples
    /// For `x <- ch`, visits `x` and `ch`.
    fn visit_linear_bind<'a>(
        &self,
        node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        names: &Vector<Arc<Node<'a>>>,
        source: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_names = names.iter().map(|n| self.visit_node(n)).collect::<Vector<_>>();
        let new_source = self.visit_node(source);
        let names_changed = names.iter().zip(&new_names).any(|(a, b)| !Arc::ptr_eq(a, b));
        if !names_changed && Arc::ptr_eq(source, &new_source) {
            Arc::clone(node)
        } else {
            Arc::new(Node::LinearBind {
                base: base.clone(),
                names: new_names,
                source: new_source,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a repeated binding node (`RepeatedBind`) in a `for` comprehension.
    ///
    /// # Arguments
    /// * `node` - The original `RepeatedBind` node.
    /// * `base` - Metadata including position and text.
    /// * `names` - Vector of variables to bind repeatedly.
    /// * `source` - The source channel expression.
    /// * `metadata` - Optional node metadata.
    ///
    /// # Returns
    /// A new `RepeatedBind` node if components change, otherwise the original.
    ///
    /// # Examples
    /// For `x <= ch`, visits `x` and `ch`.
    fn visit_repeated_bind<'a>(
        &self,
        node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        names: &Vector<Arc<Node<'a>>>,
        source: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_names = names.iter().map(|n| self.visit_node(n)).collect::<Vector<_>>();
        let new_source = self.visit_node(source);
        let names_changed = names.iter().zip(&new_names).any(|(a, b)| !Arc::ptr_eq(a, b));
        if !names_changed && Arc::ptr_eq(source, &new_source) {
            Arc::clone(node)
        } else {
            Arc::new(Node::RepeatedBind {
                base: base.clone(),
                names: new_names,
                source: new_source,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a peek binding node (`PeekBind`) in a `for` comprehension.
    ///
    /// # Arguments
    /// * `node` - The original `PeekBind` node.
    /// * `base` - Metadata including position and text.
    /// * `names` - Vector of variables to peek.
    /// * `source` - The source channel expression.
    /// * `metadata` - Optional node metadata.
    ///
    /// # Returns
    /// A new `PeekBind` node if components change, otherwise the original.
    ///
    /// # Examples
    /// For `x <<- ch`, visits `x` and `ch`.
    fn visit_peek_bind<'a>(
        &self,
        node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        names: &Vector<Arc<Node<'a>>>,
        source: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_names = names.iter().map(|n| self.visit_node(n)).collect::<Vector<_>>();
        let new_source = self.visit_node(source);
        let names_changed = names.iter().zip(&new_names).any(|(a, b)| !Arc::ptr_eq(a, b));
        if !names_changed && Arc::ptr_eq(source, &new_source) {
            Arc::clone(node)
        } else {
            Arc::new(Node::PeekBind {
                base: base.clone(),
                names: new_names,
                source: new_source,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a comment node (`Comment`).
    ///
    /// # Arguments
    /// * `node` - The original `Comment` node.
    /// * `base` - Metadata including position and text.
    /// * `kind` - Type of comment (Line or Block).
    /// * `metadata` - Optional node metadata.
    ///
    /// # Returns
    /// The original node by default; override to transform.
    ///
    /// # Examples
    /// For `// text` or `/* text */`, returns unchanged unless overridden.
    fn visit_comment<'a>(
        &self,
        node: &Arc<Node<'a>>,
        _base: &NodeBase<'a>,
        _kind: &CommentKind,
        _metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        Arc::clone(node)
    }

    /// Visits a wildcard pattern node (`Wildcard`).
    ///
    /// # Arguments
    /// * `node` - The original `Wildcard` node.
    /// * `base` - Metadata including position and text.
    /// * `metadata` - Optional node metadata.
    ///
    /// # Returns
    /// The original node by default; override to transform.
    ///
    /// # Examples
    /// For `_`, returns unchanged unless overridden.
    fn visit_wildcard<'a>(
        &self,
        node: &Arc<Node<'a>>,
        _base: &NodeBase<'a>,
        _metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        Arc::clone(node)
    }

    /// Visits a simple type annotation node (`SimpleType`).
    ///
    /// # Arguments
    /// * `node` - The original `SimpleType` node.
    /// * `base` - Metadata including position and text.
    /// * `value` - The type name (e.g., "Bool").
    /// * `metadata` - Optional node metadata.
    ///
    /// # Returns
    /// The original node by default; override to transform.
    ///
    /// # Examples
    /// For `Bool`, returns unchanged unless overridden.
    fn visit_simple_type<'a>(
        &self,
        node: &Arc<Node<'a>>,
        _base: &NodeBase<'a>,
        _value: &String,
        _metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        Arc::clone(node)
    }

    /// Visits a receive-send source node (`ReceiveSendSource`).
    ///
    /// # Arguments
    /// * `node` - The original `ReceiveSendSource` node.
    /// * `base` - Metadata including position and text.
    /// * `name` - The source name or channel.
    /// * `metadata` - Optional node metadata.
    ///
    /// # Returns
    /// A new `ReceiveSendSource` node if the name changes, otherwise the original.
    ///
    /// # Examples
    /// For `ch?!`, visits `ch`.
    fn visit_receive_send_source<'a>(
        &self,
        node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        name: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_name = self.visit_node(name);
        if Arc::ptr_eq(name, &new_name) {
            Arc::clone(node)
        } else {
            Arc::new(Node::ReceiveSendSource {
                base: base.clone(),
                name: new_name,
                metadata: metadata.clone(),
            })
        }
    }

    /// Visits a send-receive source node (`SendReceiveSource`).
    ///
    /// # Arguments
    /// * `node` - The original `SendReceiveSource` node.
    /// * `base` - Metadata including position and text.
    /// * `name` - The source name or channel.
    /// * `inputs` - Vector of arguments for the operation.
    /// * `metadata` - Optional node metadata.
    ///
    /// # Returns
    /// A new `SendReceiveSource` node if components change, otherwise the original.
    ///
    /// # Examples
    /// For `ch!?(args)`, visits `ch` and `args`.
    fn visit_send_receive_source<'a>(
        &self,
        node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        name: &Arc<Node<'a>>,
        inputs: &Vector<Arc<Node<'a>>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_name = self.visit_node(name);
        let new_inputs = inputs.iter().map(|i| self.visit_node(i)).collect::<Vector<_>>();
        if Arc::ptr_eq(name, &new_name) &&
           inputs.iter().zip(&new_inputs).all(|(a, b)| Arc::ptr_eq(a, b)) {
            Arc::clone(node)
        } else {
            Arc::new(Node::SendReceiveSource {
                base: base.clone(),
                name: new_name,
                inputs: new_inputs,
                metadata: metadata.clone(),
            })
        }
    }
}
