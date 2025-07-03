use rholang_language_server::ir::node::{
    BinOperator, BundleType, CommentKind, Metadata, Node, NodeBase, Position,
    SendType, UnaryOperator, VarRefKind
};
use rholang_language_server::ir::visitor::Visitor;
use rholang_language_server::ir::pipeline::{Pipeline, Transform};
use rholang_language_server::tree_sitter::{parse_code, parse_to_ir};
use quickcheck::{QuickCheck, TestResult};
use std::sync::Arc;
use tracing::{debug, info};
use test_utils::ir::generator::RholangProc;
use rpds::Vector;

// Simplifies double unary operations (e.g., --x to x, not not x to x).
struct SimplifyDoubleUnary;

impl Visitor for SimplifyDoubleUnary {
    fn visit_unaryop<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        op: UnaryOperator,
        operand: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        // Recurse into the tree, first:
        let new_operand = self.visit_node(&operand);

        // Simplify double unary operations (e.g., --x, not not x)
        if let Node::UnaryOp { op: inner_op, operand: inner_operand, .. } = &*new_operand {
            if *inner_op == op {
                debug!("Simplifying double unary operation: {:?} {}", op, inner_operand.text());
                let new_base = NodeBase::new(
                    None, // Transformed node
                    base.relative_start(),
                    inner_operand.base().length(),
                    Some(inner_operand.text()),
                );
                return inner_operand.with_base(new_base);
            }
        }

        // Simplify unary operation on literals only if explicitly required
        match op {
            UnaryOperator::Neg => {
                if let Node::LongLiteral { value, .. } = &*new_operand {
                    let new_value = -value;
                    let new_text = new_value.to_string();
                    let new_length = new_text.len();
                    debug!("Simplifying neg(long_literal({})) to {}", value, new_value);
                    let new_base = NodeBase::new(
                        None, // Transformed node
                        base.relative_start(),
                        new_length,
                        Some(new_text),
                    );
                    return Arc::new(Node::LongLiteral {
                        base: new_base,
                        value: new_value,
                        metadata: metadata.clone(),
                    });
                }
            }
            UnaryOperator::Not => {
                if let Node::BoolLiteral { value, .. } = &*new_operand {
                    let new_value = !value;
                    let new_text = if new_value { "true" } else { "false" }.to_string();
                    let new_length = new_text.len();
                    debug!("Simplifying not(bool_literal({})) to {}", value, new_value);
                    let new_base = NodeBase::new(
                        None,
                        base.relative_start(),
                        new_length,
                        Some(new_text),
                    );
                    return Arc::new(Node::BoolLiteral {
                        base: new_base,
                        value: new_value,
                        metadata: metadata.clone(),
                    });
                }
            }
            _ => {}
        }

        // Default: visit operand and reconstruct without simplification
        Arc::new(Node::UnaryOp {
            base: base.clone(),
            op,
            operand: new_operand,
            metadata: metadata.clone(),
        })
    }
}

// Increments the version number in node metadata.
struct IncrementVersion;

impl Visitor for IncrementVersion {
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
        let new_metadata = metadata.as_ref().map(|m| Arc::new(Metadata { version: m.version + 1 }));
        Arc::new(Node::Par {
            base: base.clone(),
            left: new_left,
            right: new_right,
            metadata: new_metadata,
        })
    }

    fn visit_sendsync<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        channel: &Arc<Node<'a>>,
        inputs: &Vector<Arc<Node<'a>>>,
        cont: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_channel = self.visit_node(channel);
        let new_inputs = inputs.iter().map(|i| self.visit_node(i)).collect::<Vector<_>>();
        let new_cont = self.visit_node(cont);
        let new_metadata = metadata.as_ref().map(|m| Arc::new(Metadata { version: m.version + 1 }));
        Arc::new(Node::SendSync {
            base: base.clone(),
            channel: new_channel,
            inputs: new_inputs,
            cont: new_cont,
            metadata: new_metadata,
        })
    }

    fn visit_send<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        channel: &Arc<Node<'a>>,
        send_type: &SendType,
        send_type_end: &Position,
        inputs: &Vector<Arc<Node<'a>>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_channel = self.visit_node(channel);
        let new_inputs = inputs.iter().map(|i| self.visit_node(i)).collect::<Vector<_>>();
        let new_metadata = metadata.as_ref().map(|m| Arc::new(Metadata { version: m.version + 1 }));
        Arc::new(Node::Send {
            base: base.clone(),
            channel: new_channel,
            send_type: send_type.clone(),
            send_type_end: *send_type_end,
            inputs: new_inputs,
            metadata: new_metadata,
        })
    }

    fn visit_new<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        decls: &Vector<Arc<Node<'a>>>,
        proc: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_decls = decls.iter().map(|d| self.visit_node(d)).collect::<Vector<_>>();
        let new_proc = self.visit_node(proc);
        let new_metadata = metadata.as_ref().map(|m| Arc::new(Metadata { version: m.version + 1 }));
        Arc::new(Node::New {
            base: base.clone(),
            decls: new_decls,
            proc: new_proc,
            metadata: new_metadata,
        })
    }

    fn visit_ifelse<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        condition: &Arc<Node<'a>>,
        consequence: &Arc<Node<'a>>,
        alternative: &Option<Arc<Node<'a>>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_condition = self.visit_node(condition);
        let new_consequence = self.visit_node(consequence);
        let new_alternative = alternative.as_ref().map(|a| self.visit_node(a));
        let new_metadata = metadata.as_ref().map(|m| Arc::new(Metadata { version: m.version + 1 }));
        Arc::new(Node::IfElse {
            base: base.clone(),
            condition: new_condition,
            consequence: new_consequence,
            alternative: new_alternative,
            metadata: new_metadata,
        })
    }

    fn visit_let<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        decls: &Vector<Arc<Node<'a>>>,
        proc: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_decls = decls.iter().map(|d| self.visit_node(d)).collect::<Vector<_>>();
        let new_proc = self.visit_node(proc);
        let new_metadata = metadata.as_ref().map(|m| Arc::new(Metadata { version: m.version + 1 }));
        Arc::new(Node::Let {
            base: base.clone(),
            decls: new_decls,
            proc: new_proc,
            metadata: new_metadata,
        })
    }

    fn visit_bundle<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        bundle_type: &BundleType,
        proc: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_proc = self.visit_node(proc);
        let new_metadata = metadata.as_ref().map(|m| Arc::new(Metadata { version: m.version + 1 }));
        Arc::new(Node::Bundle {
            base: base.clone(),
            bundle_type: bundle_type.clone(),
            proc: new_proc,
            metadata: new_metadata,
        })
    }

    fn visit_match<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        expression: &Arc<Node<'a>>,
        cases: &Vector<(Arc<Node<'a>>, Arc<Node<'a>>)>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_expression = self.visit_node(expression);
        let new_cases = cases.iter().map(|(pat, proc)| (self.visit_node(pat), self.visit_node(proc))).collect::<Vector<_>>();
        let new_metadata = metadata.as_ref().map(|m| Arc::new(Metadata { version: m.version + 1 }));
        Arc::new(Node::Match {
            base: base.clone(),
            expression: new_expression,
            cases: new_cases,
            metadata: new_metadata,
        })
    }

    fn visit_choice<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        branches: &Vector<(Vector<Arc<Node<'a>>>, Arc<Node<'a>>)>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_branches = branches.iter().map(|(inputs, proc)| {
            let new_inputs = inputs.iter().map(|i| self.visit_node(i)).collect::<Vector<_>>();
            let new_proc = self.visit_node(proc);
            (new_inputs, new_proc)
        }).collect::<Vector<_>>();
        let new_metadata = metadata.as_ref().map(|m| Arc::new(Metadata { version: m.version + 1 }));
        Arc::new(Node::Choice {
            base: base.clone(),
            branches: new_branches,
            metadata: new_metadata,
        })
    }

    fn visit_contract<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        name: &Arc<Node<'a>>,
        formals: &Vector<Arc<Node<'a>>>,
        proc: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_name = self.visit_node(name);
        let new_formals = formals.iter().map(|f| self.visit_node(f)).collect::<Vector<_>>();
        let new_proc = self.visit_node(proc);
        let new_metadata = metadata.as_ref().map(|m| Arc::new(Metadata { version: m.version + 1 }));
        Arc::new(Node::Contract {
            base: base.clone(),
            name: new_name,
            formals: new_formals,
            proc: new_proc,
            metadata: new_metadata,
        })
    }

    fn visit_input<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        receipts: &Vector<Vector<Arc<Node<'a>>>>,
        proc: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_receipts = receipts.iter().map(|r| r.iter().map(|n| self.visit_node(n)).collect::<Vector<_>>()).collect::<Vector<_>>();
        let new_proc = self.visit_node(proc);
        let new_metadata = metadata.as_ref().map(|m| Arc::new(Metadata { version: m.version + 1 }));
        Arc::new(Node::Input {
            base: base.clone(),
            receipts: new_receipts,
            proc: new_proc,
            metadata: new_metadata,
        })
    }

    fn visit_block<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        proc: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_proc = self.visit_node(proc);
        let new_metadata = metadata.as_ref().map(|m| Arc::new(Metadata { version: m.version + 1 }));
        Arc::new(Node::Block {
            base: base.clone(),
            proc: new_proc,
            metadata: new_metadata,
        })
    }

    fn visit_binop<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        op: BinOperator,
        left: &Arc<Node<'a>>,
        right: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_left = self.visit_node(left);
        let new_right = self.visit_node(right);
        let new_metadata = metadata.as_ref().map(|m| Arc::new(Metadata { version: m.version + 1 }));
        Arc::new(Node::BinOp {
            base: base.clone(),
            op,
            left: new_left,
            right: new_right,
            metadata: new_metadata,
        })
    }

    fn visit_unaryop<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        op: UnaryOperator,
        operand: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_operand = self.visit_node(operand);
        let new_metadata = metadata.as_ref().map(|m| Arc::new(Metadata { version: m.version + 1 }));
        Arc::new(Node::UnaryOp {
            base: base.clone(),
            op,
            operand: new_operand,
            metadata: new_metadata,
        })
    }

    fn visit_method<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        receiver: &Arc<Node<'a>>,
        name: &String,
        args: &Vector<Arc<Node<'a>>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_receiver = self.visit_node(receiver);
        let new_args = args.iter().map(|a| self.visit_node(a)).collect::<Vector<_>>();
        let new_metadata = metadata.as_ref().map(|m| Arc::new(Metadata { version: m.version + 1 }));
        Arc::new(Node::Method {
            base: base.clone(),
            receiver: new_receiver,
            name: name.clone(),
            args: new_args,
            metadata: new_metadata,
        })
    }

    fn visit_eval<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        name: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_name = self.visit_node(name);
        let new_metadata = metadata.as_ref().map(|m| Arc::new(Metadata { version: m.version + 1 }));
        Arc::new(Node::Eval {
            base: base.clone(),
            name: new_name,
            metadata: new_metadata,
        })
    }

    fn visit_quote<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        quotable: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_quotable = self.visit_node(quotable);
        let new_metadata = metadata.as_ref().map(|m| Arc::new(Metadata { version: m.version + 1 }));
        Arc::new(Node::Quote {
            base: base.clone(),
            quotable: new_quotable,
            metadata: new_metadata,
        })
    }

    fn visit_varref<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        kind: VarRefKind,
        var: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_var = self.visit_node(var);
        let new_metadata = metadata.as_ref().map(|m| Arc::new(Metadata { version: m.version + 1 }));
        Arc::new(Node::VarRef {
            base: base.clone(),
            kind,
            var: new_var,
            metadata: new_metadata,
        })
    }

    fn visit_bool_literal<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        value: bool,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_metadata = metadata.as_ref().map(|m| Arc::new(Metadata { version: m.version + 1 }));
        Arc::new(Node::BoolLiteral {
            base: base.clone(),
            value,
            metadata: new_metadata,
        })
    }

    fn visit_long_literal<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        value: i64,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_metadata = metadata.as_ref().map(|m| Arc::new(Metadata { version: m.version + 1 }));
        Arc::new(Node::LongLiteral {
            base: base.clone(),
            value,
            metadata: new_metadata,
        })
    }

    fn visit_string_literal<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        value: &String,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_metadata = metadata.as_ref().map(|m| Arc::new(Metadata { version: m.version + 1 }));
        Arc::new(Node::StringLiteral {
            base: base.clone(),
            value: value.clone(),
            metadata: new_metadata,
        })
    }

    fn visit_uri_literal<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        value: &String,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_metadata = metadata.as_ref().map(|m| Arc::new(Metadata { version: m.version + 1 }));
        Arc::new(Node::UriLiteral {
            base: base.clone(),
            value: value.clone(),
            metadata: new_metadata,
        })
    }

    fn visit_nil<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_metadata = metadata.as_ref().map(|m| Arc::new(Metadata { version: m.version + 1 }));
        Arc::new(Node::Nil {
            base: base.clone(),
            metadata: new_metadata,
        })
    }

    fn visit_list<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        elements: &Vector<Arc<Node<'a>>>,
        remainder: &Option<Arc<Node<'a>>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_elements = elements.iter().map(|e| self.visit_node(e)).collect::<Vector<_>>();
        let new_remainder = remainder.as_ref().map(|r| self.visit_node(r));
        let new_metadata = metadata.as_ref().map(|m| Arc::new(Metadata { version: m.version + 1 }));
        Arc::new(Node::List {
            base: base.clone(),
            elements: new_elements,
            remainder: new_remainder,
            metadata: new_metadata,
        })
    }

    fn visit_set<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        elements: &Vector<Arc<Node<'a>>>,
        remainder: &Option<Arc<Node<'a>>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_elements = elements.iter().map(|e| self.visit_node(e)).collect::<Vector<_>>();
        let new_remainder = remainder.as_ref().map(|r| self.visit_node(r));
        let new_metadata = metadata.as_ref().map(|m| Arc::new(Metadata { version: m.version + 1 }));
        Arc::new(Node::Set {
            base: base.clone(),
            elements: new_elements,
            remainder: new_remainder,
            metadata: new_metadata,
        })
    }

    fn visit_map<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        pairs: &Vector<(Arc<Node<'a>>, Arc<Node<'a>>)>,
        remainder: &Option<Arc<Node<'a>>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_pairs = pairs.iter().map(|(k, v)| (self.visit_node(k), self.visit_node(v))).collect::<Vector<_>>();
        let new_remainder = remainder.as_ref().map(|r| self.visit_node(r));
        let new_metadata = metadata.as_ref().map(|m| Arc::new(Metadata { version: m.version + 1 }));
        Arc::new(Node::Map {
            base: base.clone(),
            pairs: new_pairs,
            remainder: new_remainder,
            metadata: new_metadata,
        })
    }

    fn visit_tuple<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        elements: &Vector<Arc<Node<'a>>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_elements = elements.iter().map(|e| self.visit_node(e)).collect::<Vector<_>>();
        let new_metadata = metadata.as_ref().map(|m| Arc::new(Metadata { version: m.version + 1 }));
        Arc::new(Node::Tuple {
            base: base.clone(),
            elements: new_elements,
            metadata: new_metadata,
        })
    }

    fn visit_var<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        name: &String,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_metadata = metadata.as_ref().map(|m| Arc::new(Metadata { version: m.version + 1 }));
        Arc::new(Node::Var {
            base: base.clone(),
            name: name.clone(),
            metadata: new_metadata,
        })
    }

    fn visit_name_decl<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        var: &Arc<Node<'a>>,
        uri: &Option<Arc<Node<'a>>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_var = self.visit_node(var);
        let new_uri = uri.as_ref().map(|u| self.visit_node(u));
        let new_metadata = metadata.as_ref().map(|m| Arc::new(Metadata { version: m.version + 1 }));
        Arc::new(Node::NameDecl {
            base: base.clone(),
            var: new_var,
            uri: new_uri,
            metadata: new_metadata,
        })
    }

    fn visit_decl<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        names: &Vector<Arc<Node<'a>>>,
        procs: &Vector<Arc<Node<'a>>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_names = names.iter().map(|n| self.visit_node(n)).collect::<Vector<_>>();
        let new_procs = procs.iter().map(|p| self.visit_node(p)).collect::<Vector<_>>();
        let new_metadata = metadata.as_ref().map(|m| Arc::new(Metadata { version: m.version + 1 }));
        Arc::new(Node::Decl {
            base: base.clone(),
            names: new_names,
            procs: new_procs,
            metadata: new_metadata,
        })
    }

    fn visit_linear_bind<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        names: &Vector<Arc<Node<'a>>>,
        source: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_names = names.iter().map(|n| self.visit_node(n)).collect::<Vector<_>>();
        let new_source = self.visit_node(source);
        let new_metadata = metadata.as_ref().map(|m| Arc::new(Metadata { version: m.version + 1 }));
        Arc::new(Node::LinearBind {
            base: base.clone(),
            names: new_names,
            source: new_source,
            metadata: new_metadata,
        })
    }

    fn visit_repeated_bind<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        names: &Vector<Arc<Node<'a>>>,
        source: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_names = names.iter().map(|n| self.visit_node(n)).collect::<Vector<_>>();
        let new_source = self.visit_node(source);
        let new_metadata = metadata.as_ref().map(|m| Arc::new(Metadata { version: m.version + 1 }));
        Arc::new(Node::RepeatedBind {
            base: base.clone(),
            names: new_names,
            source: new_source,
            metadata: new_metadata,
        })
    }

    fn visit_peek_bind<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        names: &Vector<Arc<Node<'a>>>,
        source: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_names = names.iter().map(|n| self.visit_node(n)).collect::<Vector<_>>();
        let new_source = self.visit_node(source);
        let new_metadata = metadata.as_ref().map(|m| Arc::new(Metadata { version: m.version + 1 }));
        Arc::new(Node::PeekBind {
            base: base.clone(),
            names: new_names,
            source: new_source,
            metadata: new_metadata,
        })
    }

    fn visit_comment<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        kind: &CommentKind,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_metadata = metadata.as_ref().map(|m| Arc::new(Metadata { version: m.version + 1 }));
        Arc::new(Node::Comment {
            base: base.clone(),
            kind: kind.clone(),
            metadata: new_metadata,
        })
    }

    fn visit_receive_send_source<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        name: &Arc<Node<'a>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_name = self.visit_node(name);
        let new_metadata = metadata.as_ref().map(|m| Arc::new(Metadata { version: m.version + 1 }));
        Arc::new(Node::ReceiveSendSource {
            base: base.clone(),
            name: new_name,
            metadata: new_metadata,
        })
    }

    fn visit_send_receive_source<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        name: &Arc<Node<'a>>,
        inputs: &Vector<Arc<Node<'a>>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_name = self.visit_node(name);
        let new_inputs = inputs.iter().map(|i| self.visit_node(i)).collect::<Vector<_>>();
        let new_metadata = metadata.as_ref().map(|m| Arc::new(Metadata { version: m.version + 1 }));
        Arc::new(Node::SendReceiveSource {
            base: base.clone(),
            name: new_name,
            inputs: new_inputs,
            metadata: new_metadata,
        })
    }

    fn visit_wildcard<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_metadata = metadata.as_ref().map(|m| Arc::new(Metadata { version: m.version + 1 }));
        Arc::new(Node::Wildcard {
            base: base.clone(),
            metadata: new_metadata,
        })
    }

    fn visit_simple_type<'a>(
        &self,
        _node: &Arc<Node<'a>>,
        base: &NodeBase<'a>,
        value: &String,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<Node<'a>> {
        let new_metadata = metadata.as_ref().map(|m| Arc::new(Metadata { version: m.version + 1 }));
        Arc::new(Node::SimpleType {
            base: base.clone(),
            value: value.clone(),
            metadata: new_metadata,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simplify_double_negation() {
        let code = "--x";
        let tree = parse_code(code);
        let ir = parse_to_ir(&tree, code);
        let simplifier = SimplifyDoubleUnary;
        let transformed = simplifier.visit_node(&ir);
        assert_eq!(transformed.text(), "x");
        assert!(!transformed.metadata().is_none(), "Transformed node should have metadata");
    }

    #[test]
    fn test_single_negation_unchanged() {
        let code = "-x";
        let tree = parse_code(code);
        let ir = parse_to_ir(&tree, code);
        let simplifier = SimplifyDoubleUnary;
        let transformed = simplifier.visit_node(&ir);
        assert_eq!(transformed.text(), "-x");
        assert!(!transformed.metadata().is_none(), "Transformed node should have metadata");
    }

    #[test]
    fn test_simplify_double_not() {
        let simplifier = SimplifyDoubleUnary;

        let code = "true";
        let tree_single = parse_code(code);
        let ir = parse_to_ir(&tree_single, code);
        let transformed_single = simplifier.visit_node(&ir);
        assert_eq!(transformed_single.text(), "true", "Non-negated should remain unchanged");
        assert!(!transformed_single.metadata().is_none(), "Transformed node should have metadata");

        let code = "not true";
        let tree_single = parse_code(code);
        let ir = parse_to_ir(&tree_single, code);
        let transformed_single = simplifier.visit_node(&ir);
        assert_eq!(transformed_single.text(), "false", "Single not be negated");
        assert!(!transformed_single.metadata().is_none(), "Transformed node should have metadata");

        let code = "not false";
        let tree_single = parse_code(code);
        let ir = parse_to_ir(&tree_single, code);
        let transformed_single = simplifier.visit_node(&ir);
        assert_eq!(transformed_single.text(), "true", "Single not be negated");
        assert!(!transformed_single.metadata().is_none(), "Transformed node should have metadata");

        let code = "not not true";
        let tree = parse_code(code);
        let ir = parse_to_ir(&tree, code);
        let transformed = simplifier.visit_node(&ir);
        assert_eq!(transformed.text(), "true", "Double not should simplify to original value");
        assert!(!transformed.metadata().is_none(), "Transformed node should have metadata");

        let code = "not not not true";
        let tree = parse_code(code);
        let ir = parse_to_ir(&tree, code);
        let transformed = simplifier.visit_node(&ir);
        assert_eq!(transformed.text(), "false", "Triple not should simplify to the negation of the original value");
        assert!(!transformed.metadata().is_none(), "Transformed node should have metadata");
    }

    #[test]
    fn test_simplify_within_par() {
        let code = "--42 | x";
        let tree = parse_code(code);
        let ir = parse_to_ir(&tree, code);
        let simplifier = SimplifyDoubleUnary;
        let transformed = simplifier.visit_node(&ir);
        if let Node::Par { left, right, .. } = &*transformed {
            assert_eq!(left.text(), "42", "Double negation in par should simplify");
            assert_eq!(right.text(), "x");
        } else {
            panic!("Expected Par node");
        }
        assert!(!transformed.metadata().is_none(), "Transformed node should have metadata");
    }

    #[test]
    fn test_pipeline_with_simplification() {
        let mut pipeline = Pipeline::new();
        pipeline.add_transform(Transform {
            id: "simplify_double_unary".to_string(),
            dependencies: vec![],
            visitor: Arc::new(SimplifyDoubleUnary),
        });
        pipeline.add_transform(Transform {
            id: "increment_version".to_string(),
            dependencies: vec!["simplify_double_unary".to_string()],
            visitor: Arc::new(IncrementVersion),
        });

        let code = "--42";
        let tree = parse_code(code);
        let ir = parse_to_ir(&tree, code);
        let transformed = pipeline.apply(&ir);
        assert_eq!(transformed.text(), "42", "Pipeline should simplify double negation");
        if let Some(metadata) = transformed.metadata() {
            assert_eq!(metadata.version, 1, "Metadata version should be incremented");
        } else {
            panic!("Expected metadata on transformed node");
        }
    }

    #[test]
    fn test_property_double_unary_simplification() {
        fn prop(proc: RholangProc) -> TestResult {
            let code = proc.to_code();
            info!("Testing code: {}", code);
            let tree = parse_code(&code);
            if tree.root_node().has_error() {
                debug!("Parse error in code: {}", code);
                return TestResult::discard();
            }
            let ir = parse_to_ir(&tree, &code);
            let simplifier = SimplifyDoubleUnary;
            let transformed = simplifier.visit_node(&ir);

            let transformed_twice = simplifier.visit_node(&transformed);
            if transformed.text() != transformed_twice.text() {
                debug!(
                    "Non-idempotent transformation: {} -> {} -> {}",
                    code, transformed.text(), transformed_twice.text()
                );
                return TestResult::failed();
            }

            TestResult::passed()
        }

        QuickCheck::new()
            .tests(1000)
            .max_tests(10000)
            .quickcheck(prop as fn(RholangProc) -> TestResult);
    }

    #[test]
    fn test_relative_positioning() {
        let code = "Nil | x";
        let tree = parse_code(code);
        let ir = parse_to_ir(&tree, code);

        if let Node::Par { left, right, .. } = &*ir {
            let nil_start = left.absolute_start(&ir);
            assert_eq!(nil_start, Position { row: 0, column: 0, byte: 0 });
            let nil_end = left.absolute_end(&ir);
            assert_eq!(nil_end, Position { row: 0, column: 3, byte: 3 });

            let x_start = right.absolute_start(&ir);
            assert_eq!(x_start.row, 0);
            assert_eq!(x_start.column, 6);
            assert_eq!(x_start.byte, 6);
        } else {
            panic!("Expected Par node");
        }
    }

    #[test]
    fn test_transformation_preserves_position() {
        let code = "--x";
        let tree = parse_code(code);
        let ir = parse_to_ir(&tree, code);
        let simplifier = SimplifyDoubleUnary;
        let transformed = simplifier.visit_node(&ir);
        let original_start = ir.absolute_start(&ir);
        let transformed_start = transformed.absolute_start(&transformed);
        assert_eq!(original_start, transformed_start);
    }
}
