use std::collections::HashMap;
use std::any::Any;
use rholang_language_server::ir::rholang_node::{
    BinOperator, RholangBundleType, CommentKind, Metadata, RholangNode, NodeBase, Position,
    RelativePosition, RholangSendType, UnaryOperator, RholangVarRefKind
};
use rholang_language_server::ir::visitor::Visitor;
use rholang_language_server::ir::pipeline::{Pipeline, Transform};
use rholang_language_server::tree_sitter::{parse_code, parse_to_ir};
use quickcheck::{QuickCheck, TestResult};
use std::sync::Arc;
use tracing::{debug, info};
use test_utils::ir::generator::RholangProc;
use rpds::Vector;
use archery::ArcK;
use ropey::Rope;

// Simplifies double unary operations (e.g., --x to x, not not x to x).
struct SimplifyDoubleUnary;

impl Visitor for SimplifyDoubleUnary {
    fn visit_unaryop(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        op: UnaryOperator,
        operand: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        // Recurse into the tree, first:
        let new_operand = self.visit_node(&operand);

        // Simplify double unary operations (e.g., --x, not not x)
        if let RholangNode::UnaryOp { op: inner_op, operand: inner_operand, .. } = &*new_operand {
            if *inner_op == op {
                debug!("Simplifying double unary operation: {:?}", op);
                let new_base = NodeBase::new(
                    base.relative_start(),
                    inner_operand.base().length(),
                    0,
                    inner_operand.base().length(),
                );
                return inner_operand.with_base(new_base);
            }
        }

        // Simplify unary operation on literals only if explicitly required
        match op {
            UnaryOperator::Neg => {
                if let RholangNode::LongLiteral { value, .. } = &*new_operand {
                    let new_value = -value;
                    let new_text = new_value.to_string();
                    let new_length = new_text.len();
                    debug!("Simplifying neg(long_literal({})) to {}", value, new_value);
                    let new_base = NodeBase::new(
                        base.relative_start(),
                        new_length,
                        0,
                        new_length,
                    );
                    return Arc::new(RholangNode::LongLiteral {
                        base: new_base,
                        value: new_value,
                        metadata: metadata.clone(),
                    });
                }
            }
            UnaryOperator::Not => {
                if let RholangNode::BoolLiteral { value, .. } = &*new_operand {
                    let new_value = !value;
                    let new_text = if new_value { "true" } else { "false" }.to_string();
                    let new_length = new_text.len();
                    debug!("Simplifying not(bool_literal({})) to {}", value, new_value);
                    let new_base = NodeBase::new(
                        base.relative_start(),
                        new_length,
                        0,
                        new_length,
                    );
                    return Arc::new(RholangNode::BoolLiteral {
                        base: new_base,
                        value: new_value,
                        metadata: metadata.clone(),
                    });
                }
            }
            _ => {}
        }

        // Default: visit operand and reconstruct without simplification
        Arc::new(RholangNode::UnaryOp {
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
    fn visit_par(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        left: &Arc<RholangNode>,
        right: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_left = self.visit_node(left);
        let new_right = self.visit_node(right);
        let new_metadata = metadata.clone().unwrap_or_else(|| Arc::new(HashMap::new()));
        let mut data = new_metadata.as_ref().clone();
        let version = data.get("version").and_then(|v| v.downcast_ref::<usize>()).unwrap_or(&0) + 1;
        data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
        Arc::new(RholangNode::Par {
            base: base.clone(),
            left: new_left,
            right: new_right,
            metadata: Some(Arc::new(data)),
        })
    }

    fn visit_send_sync(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        channel: &Arc<RholangNode>,
        inputs: &Vector<Arc<RholangNode>, ArcK>,
        cont: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_channel = self.visit_node(channel);
        let new_inputs = inputs.iter().map(|i| self.visit_node(i)).collect::<Vector<_, ArcK>>();
        let new_cont = self.visit_node(cont);
        let new_metadata = metadata.clone().unwrap_or_else(|| Arc::new(HashMap::new()));
        let mut data = new_metadata.as_ref().clone();
        let version = data.get("version").and_then(|v| v.downcast_ref::<usize>()).unwrap_or(&0) + 1;
        data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
        Arc::new(RholangNode::SendSync {
            base: base.clone(),
            channel: new_channel,
            inputs: new_inputs,
            cont: new_cont,
            metadata: Some(Arc::new(data)),
        })
    }

    fn visit_send(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        channel: &Arc<RholangNode>,
        send_type: &RholangSendType,
        send_type_delta: &RelativePosition,
        inputs: &Vector<Arc<RholangNode>, ArcK>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_channel = self.visit_node(channel);
        let new_inputs = inputs.iter().map(|i| self.visit_node(i)).collect::<Vector<_, ArcK>>();
        let new_metadata = metadata.clone().unwrap_or_else(|| Arc::new(HashMap::new()));
        let mut data = new_metadata.as_ref().clone();
        let version = data.get("version").and_then(|v| v.downcast_ref::<usize>()).unwrap_or(&0) + 1;
        data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
        Arc::new(RholangNode::Send {
            base: base.clone(),
            channel: new_channel,
            send_type: send_type.clone(),
            send_type_delta: *send_type_delta,
            inputs: new_inputs,
            metadata: Some(Arc::new(data)),
        })
    }

    fn visit_new(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        decls: &Vector<Arc<RholangNode>, ArcK>,
        proc: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_decls = decls.iter().map(|d| self.visit_node(d)).collect::<Vector<_, ArcK>>();
        let new_proc = self.visit_node(proc);
        let new_metadata = metadata.clone().unwrap_or_else(|| Arc::new(HashMap::new()));
        let mut data = new_metadata.as_ref().clone();
        let version = data.get("version").and_then(|v| v.downcast_ref::<usize>()).unwrap_or(&0) + 1;
        data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
        Arc::new(RholangNode::New {
            base: base.clone(),
            decls: new_decls,
            proc: new_proc,
            metadata: Some(Arc::new(data)),
        })
    }

    fn visit_ifelse(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        condition: &Arc<RholangNode>,
        consequence: &Arc<RholangNode>,
        alternative: &Option<Arc<RholangNode>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_condition = self.visit_node(condition);
        let new_consequence = self.visit_node(consequence);
        let new_alternative = alternative.as_ref().map(|a| self.visit_node(a));
        let new_metadata = metadata.clone().unwrap_or_else(|| Arc::new(HashMap::new()));
        let mut data = new_metadata.as_ref().clone();
        let version = data.get("version").and_then(|v| v.downcast_ref::<usize>()).unwrap_or(&0) + 1;
        data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
        Arc::new(RholangNode::IfElse {
            base: base.clone(),
            condition: new_condition,
            consequence: new_consequence,
            alternative: new_alternative,
            metadata: Some(Arc::new(data)),
        })
    }

    fn visit_let(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        decls: &Vector<Arc<RholangNode>, ArcK>,
        proc: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_decls = decls.iter().map(|d| self.visit_node(d)).collect::<Vector<_, ArcK>>();
        let new_proc = self.visit_node(proc);
        let new_metadata = metadata.clone().unwrap_or_else(|| Arc::new(HashMap::new()));
        let mut data = new_metadata.as_ref().clone();
        let version = data.get("version").and_then(|v| v.downcast_ref::<usize>()).unwrap_or(&0) + 1;
        data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
        Arc::new(RholangNode::Let {
            base: base.clone(),
            decls: new_decls,
            proc: new_proc,
            metadata: Some(Arc::new(data)),
        })
    }

    fn visit_bundle(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        bundle_type: &RholangBundleType,
        proc: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_proc = self.visit_node(proc);
        let new_metadata = metadata.clone().unwrap_or_else(|| Arc::new(HashMap::new()));
        let mut data = new_metadata.as_ref().clone();
        let version = data.get("version").and_then(|v| v.downcast_ref::<usize>()).unwrap_or(&0) + 1;
        data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
        Arc::new(RholangNode::Bundle {
            base: base.clone(),
            bundle_type: bundle_type.clone(),
            proc: new_proc,
            metadata: Some(Arc::new(data)),
        })
    }

    fn visit_match(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        expression: &Arc<RholangNode>,
        cases: &Vector<(Arc<RholangNode>, Arc<RholangNode>), ArcK>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_expression = self.visit_node(expression);
        let new_cases = cases.iter().map(|(pat, proc)| (self.visit_node(pat), self.visit_node(proc))).collect::<Vector<_, ArcK>>();
        let new_metadata = metadata.clone().unwrap_or_else(|| Arc::new(HashMap::new()));
        let mut data = new_metadata.as_ref().clone();
        let version = data.get("version").and_then(|v| v.downcast_ref::<usize>()).unwrap_or(&0) + 1;
        data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
        Arc::new(RholangNode::Match {
            base: base.clone(),
            expression: new_expression,
            cases: new_cases,
            metadata: Some(Arc::new(data)),
        })
    }

    fn visit_choice(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        branches: &Vector<(Vector<Arc<RholangNode>, ArcK>, Arc<RholangNode>), ArcK>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_branches = branches.iter().map(|(inputs, proc)| {
            let new_inputs = inputs.iter().map(|i| self.visit_node(i)).collect::<Vector<_, ArcK>>();
            let new_proc = self.visit_node(proc);
            (new_inputs, new_proc)
        }).collect::<Vector<_, ArcK>>();
        let new_metadata = metadata.clone().unwrap_or_else(|| Arc::new(HashMap::new()));
        let mut data = new_metadata.as_ref().clone();
        let version = data.get("version").and_then(|v| v.downcast_ref::<usize>()).unwrap_or(&0) + 1;
        data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
        Arc::new(RholangNode::Choice {
            base: base.clone(),
            branches: new_branches,
            metadata: Some(Arc::new(data)),
        })
    }

    fn visit_contract(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        name: &Arc<RholangNode>,
        formals: &Vector<Arc<RholangNode>, ArcK>,
        formals_remainder: &Option<Arc<RholangNode>>,
        proc: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_name = self.visit_node(name);
        let new_formals = formals.iter().map(|f| self.visit_node(f)).collect::<Vector<_, ArcK>>();
        let new_formals_remainder = formals_remainder.as_ref().map(|r| self.visit_node(r));
        let new_proc = self.visit_node(proc);
        let new_metadata = metadata.clone().unwrap_or_else(|| Arc::new(HashMap::new()));
        let mut data = new_metadata.as_ref().clone();
        let version = data.get("version").and_then(|v| v.downcast_ref::<usize>()).unwrap_or(&0) + 1;
        data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
        Arc::new(RholangNode::Contract {
            base: base.clone(),
            name: new_name,
            formals: new_formals,
            formals_remainder: new_formals_remainder,
            proc: new_proc,
            metadata: Some(Arc::new(data)),
        })
    }

    fn visit_input(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        receipts: &Vector<Vector<Arc<RholangNode>, ArcK>, ArcK>,
        proc: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_receipts = receipts.iter().map(|r| r.iter().map(|n| self.visit_node(n)).collect::<Vector<_, ArcK>>()).collect::<Vector<_, ArcK>>();
        let new_proc = self.visit_node(proc);
        let new_metadata = metadata.clone().unwrap_or_else(|| Arc::new(HashMap::new()));
        let mut data = new_metadata.as_ref().clone();
        let version = data.get("version").and_then(|v| v.downcast_ref::<usize>()).unwrap_or(&0) + 1;
        data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
        Arc::new(RholangNode::Input {
            base: base.clone(),
            receipts: new_receipts,
            proc: new_proc,
            metadata: Some(Arc::new(data)),
        })
    }

    fn visit_block(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        proc: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_proc = self.visit_node(proc);
        let new_metadata = metadata.clone().unwrap_or_else(|| Arc::new(HashMap::new()));
        let mut data = new_metadata.as_ref().clone();
        let version = data.get("version").and_then(|v| v.downcast_ref::<usize>()).unwrap_or(&0) + 1;
        data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
        Arc::new(RholangNode::Block {
            base: base.clone(),
            proc: new_proc,
            metadata: Some(Arc::new(data)),
        })
    }

    fn visit_binop(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        op: BinOperator,
        left: &Arc<RholangNode>,
        right: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_left = self.visit_node(left);
        let new_right = self.visit_node(right);
        let new_metadata = metadata.clone().unwrap_or_else(|| Arc::new(HashMap::new()));
        let mut data = new_metadata.as_ref().clone();
        let version = data.get("version").and_then(|v| v.downcast_ref::<usize>()).unwrap_or(&0) + 1;
        data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
        Arc::new(RholangNode::BinOp {
            base: base.clone(),
            op,
            left: new_left,
            right: new_right,
            metadata: Some(Arc::new(data)),
        })
    }

    fn visit_unaryop(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        op: UnaryOperator,
        operand: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_operand = self.visit_node(operand);
        let new_metadata = metadata.clone().unwrap_or_else(|| Arc::new(HashMap::new()));
        let mut data = new_metadata.as_ref().clone();
        let version = data.get("version").and_then(|v| v.downcast_ref::<usize>()).unwrap_or(&0) + 1;
        data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
        Arc::new(RholangNode::UnaryOp {
            base: base.clone(),
            op,
            operand: new_operand,
            metadata: Some(Arc::new(data)),
        })
    }

    fn visit_method(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        receiver: &Arc<RholangNode>,
        name: &String,
        args: &Vector<Arc<RholangNode>, ArcK>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_receiver = self.visit_node(receiver);
        let new_args = args.iter().map(|a| self.visit_node(a)).collect::<Vector<_, ArcK>>();
        let new_metadata = metadata.clone().unwrap_or_else(|| Arc::new(HashMap::new()));
        let mut data = new_metadata.as_ref().clone();
        let version = data.get("version").and_then(|v| v.downcast_ref::<usize>()).unwrap_or(&0) + 1;
        data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
        Arc::new(RholangNode::Method {
            base: base.clone(),
            receiver: new_receiver,
            name: name.clone(),
            args: new_args,
            metadata: Some(Arc::new(data)),
        })
    }

    fn visit_eval(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        name: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_name = self.visit_node(name);
        let new_metadata = metadata.clone().unwrap_or_else(|| Arc::new(HashMap::new()));
        let mut data = new_metadata.as_ref().clone();
        let version = data.get("version").and_then(|v| v.downcast_ref::<usize>()).unwrap_or(&0) + 1;
        data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
        Arc::new(RholangNode::Eval {
            base: base.clone(),
            name: new_name,
            metadata: Some(Arc::new(data)),
        })
    }

    fn visit_quote(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        quotable: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_quotable = self.visit_node(quotable);
        let new_metadata = metadata.clone().unwrap_or_else(|| Arc::new(HashMap::new()));
        let mut data = new_metadata.as_ref().clone();
        let version = data.get("version").and_then(|v| v.downcast_ref::<usize>()).unwrap_or(&0) + 1;
        data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
        Arc::new(RholangNode::Quote {
            base: base.clone(),
            quotable: new_quotable,
            metadata: Some(Arc::new(data)),
        })
    }

    fn visit_varref(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        kind: RholangVarRefKind,
        var: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_var = self.visit_node(var);
        let new_metadata = metadata.clone().unwrap_or_else(|| Arc::new(HashMap::new()));
        let mut data = new_metadata.as_ref().clone();
        let version = data.get("version").and_then(|v| v.downcast_ref::<usize>()).unwrap_or(&0) + 1;
        data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
        Arc::new(RholangNode::VarRef {
            base: base.clone(),
            kind,
            var: new_var,
            metadata: Some(Arc::new(data)),
        })
    }

    fn visit_bool_literal(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        value: bool,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_metadata = metadata.clone().unwrap_or_else(|| Arc::new(HashMap::new()));
        let mut data = new_metadata.as_ref().clone();
        let version = data.get("version").and_then(|v| v.downcast_ref::<usize>()).unwrap_or(&0) + 1;
        data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
        Arc::new(RholangNode::BoolLiteral {
            base: base.clone(),
            value,
            metadata: Some(Arc::new(data)),
        })
    }

    fn visit_long_literal(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        value: i64,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_metadata = metadata.clone().unwrap_or_else(|| Arc::new(HashMap::new()));
        let mut data = new_metadata.as_ref().clone();
        let version = data.get("version").and_then(|v| v.downcast_ref::<usize>()).unwrap_or(&0) + 1;
        data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
        Arc::new(RholangNode::LongLiteral {
            base: base.clone(),
            value,
            metadata: Some(Arc::new(data)),
        })
    }

    fn visit_string_literal(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        value: &String,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_metadata = metadata.clone().unwrap_or_else(|| Arc::new(HashMap::new()));
        let mut data = new_metadata.as_ref().clone();
        let version = data.get("version").and_then(|v| v.downcast_ref::<usize>()).unwrap_or(&0) + 1;
        data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
        Arc::new(RholangNode::StringLiteral {
            base: base.clone(),
            value: value.clone(),
            metadata: Some(Arc::new(data)),
        })
    }

    fn visit_uri_literal(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        value: &String,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_metadata = metadata.clone().unwrap_or_else(|| Arc::new(HashMap::new()));
        let mut data = new_metadata.as_ref().clone();
        let version = data.get("version").and_then(|v| v.downcast_ref::<usize>()).unwrap_or(&0) + 1;
        data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
        Arc::new(RholangNode::UriLiteral {
            base: base.clone(),
            value: value.clone(),
            metadata: Some(Arc::new(data)),
        })
    }

    fn visit_nil(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_metadata = metadata.clone().unwrap_or_else(|| Arc::new(HashMap::new()));
        let mut data = new_metadata.as_ref().clone();
        let version = data.get("version").and_then(|v| v.downcast_ref::<usize>()).unwrap_or(&0) + 1;
        data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
        Arc::new(RholangNode::Nil {
            base: base.clone(),
            metadata: Some(Arc::new(data)),
        })
    }

    fn visit_list(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        elements: &Vector<Arc<RholangNode>, ArcK>,
        remainder: &Option<Arc<RholangNode>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_elements = elements.iter().map(|e| self.visit_node(e)).collect::<Vector<_, ArcK>>();
        let new_remainder = remainder.as_ref().map(|r| self.visit_node(r));
        let new_metadata = metadata.clone().unwrap_or_else(|| Arc::new(HashMap::new()));
        let mut data = new_metadata.as_ref().clone();
        let version = data.get("version").and_then(|v| v.downcast_ref::<usize>()).unwrap_or(&0) + 1;
        data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
        Arc::new(RholangNode::List {
            base: base.clone(),
            elements: new_elements,
            remainder: new_remainder,
            metadata: Some(Arc::new(data)),
        })
    }

    fn visit_set(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        elements: &Vector<Arc<RholangNode>, ArcK>,
        remainder: &Option<Arc<RholangNode>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_elements = elements.iter().map(|e| self.visit_node(e)).collect::<Vector<_, ArcK>>();
        let new_remainder = remainder.as_ref().map(|r| self.visit_node(r));
        let new_metadata = metadata.clone().unwrap_or_else(|| Arc::new(HashMap::new()));
        let mut data = new_metadata.as_ref().clone();
        let version = data.get("version").and_then(|v| v.downcast_ref::<usize>()).unwrap_or(&0) + 1;
        data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
        Arc::new(RholangNode::Set {
            base: base.clone(),
            elements: new_elements,
            remainder: new_remainder,
            metadata: Some(Arc::new(data)),
        })
    }

    fn visit_map(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        pairs: &Vector<(Arc<RholangNode>, Arc<RholangNode>), ArcK>,
        remainder: &Option<Arc<RholangNode>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_pairs = pairs.iter().map(|(k, v)| (self.visit_node(k), self.visit_node(v))).collect::<Vector<_, ArcK>>();
        let new_remainder = remainder.as_ref().map(|r| self.visit_node(r));
        let new_metadata = metadata.clone().unwrap_or_else(|| Arc::new(HashMap::new()));
        let mut data = new_metadata.as_ref().clone();
        let version = data.get("version").and_then(|v| v.downcast_ref::<usize>()).unwrap_or(&0) + 1;
        data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
        Arc::new(RholangNode::Map {
            base: base.clone(),
            pairs: new_pairs,
            remainder: new_remainder,
            metadata: Some(Arc::new(data)),
        })
    }

    fn visit_tuple(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        elements: &Vector<Arc<RholangNode>, ArcK>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_elements = elements.iter().map(|e| self.visit_node(e)).collect::<Vector<_, ArcK>>();
        let new_metadata = metadata.clone().unwrap_or_else(|| Arc::new(HashMap::new()));
        let mut data = new_metadata.as_ref().clone();
        let version = data.get("version").and_then(|v| v.downcast_ref::<usize>()).unwrap_or(&0) + 1;
        data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
        Arc::new(RholangNode::Tuple {
            base: base.clone(),
            elements: new_elements,
            metadata: Some(Arc::new(data)),
        })
    }

    fn visit_var(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        name: &String,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_metadata = metadata.clone().unwrap_or_else(|| Arc::new(HashMap::new()));
        let mut data = new_metadata.as_ref().clone();
        let version = data.get("version").and_then(|v| v.downcast_ref::<usize>()).unwrap_or(&0) + 1;
        data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
        Arc::new(RholangNode::Var {
            base: base.clone(),
            name: name.clone(),
            metadata: Some(Arc::new(data)),
        })
    }

    fn visit_name_decl(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        var: &Arc<RholangNode>,
        uri: &Option<Arc<RholangNode>>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_var = self.visit_node(var);
        let new_uri = uri.as_ref().map(|u| self.visit_node(u));
        let new_metadata = metadata.clone().unwrap_or_else(|| Arc::new(HashMap::new()));
        let mut data = new_metadata.as_ref().clone();
        let version = data.get("version").and_then(|v| v.downcast_ref::<usize>()).unwrap_or(&0) + 1;
        data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
        Arc::new(RholangNode::NameDecl {
            base: base.clone(),
            var: new_var,
            uri: new_uri,
            metadata: Some(Arc::new(data)),
        })
    }

    fn visit_decl(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        names: &Vector<Arc<RholangNode>, ArcK>,
        names_remainder: &Option<Arc<RholangNode>>,
        procs: &Vector<Arc<RholangNode>, ArcK>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_names = names.iter().map(|n| self.visit_node(n)).collect::<Vector<_, ArcK>>();
        let new_names_remainder = names_remainder.as_ref().map(|r| self.visit_node(r));
        let new_procs = procs.iter().map(|p| self.visit_node(p)).collect::<Vector<_, ArcK>>();
        let new_metadata = metadata.clone().unwrap_or_else(|| Arc::new(HashMap::new()));
        let mut data = new_metadata.as_ref().clone();
        let version = data.get("version").and_then(|v| v.downcast_ref::<usize>()).unwrap_or(&0) + 1;
        data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
        Arc::new(RholangNode::Decl {
            base: base.clone(),
            names: new_names,
            names_remainder: new_names_remainder,
            procs: new_procs,
            metadata: Some(Arc::new(data)),
        })
    }

    fn visit_linear_bind(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        names: &Vector<Arc<RholangNode>, ArcK>,
        remainder: &Option<Arc<RholangNode>>,
        source: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_names = names.iter().map(|n| self.visit_node(n)).collect::<Vector<_, ArcK>>();
        let new_remainder = remainder.as_ref().map(|r| self.visit_node(r));
        let new_source = self.visit_node(source);
        let new_metadata = metadata.clone().unwrap_or_else(|| Arc::new(HashMap::new()));
        let mut data = new_metadata.as_ref().clone();
        let version = data.get("version").and_then(|v| v.downcast_ref::<usize>()).unwrap_or(&0) + 1;
        data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
        Arc::new(RholangNode::LinearBind {
            base: base.clone(),
            names: new_names,
            remainder: new_remainder,
            source: new_source,
            metadata: Some(Arc::new(data)),
        })
    }

    fn visit_repeated_bind(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        names: &Vector<Arc<RholangNode>, ArcK>,
        remainder: &Option<Arc<RholangNode>>,
        source: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_names = names.iter().map(|n| self.visit_node(n)).collect::<Vector<_, ArcK>>();
        let new_remainder = remainder.as_ref().map(|r| self.visit_node(r));
        let new_source = self.visit_node(source);
        let new_metadata = metadata.clone().unwrap_or_else(|| Arc::new(HashMap::new()));
        let mut data = new_metadata.as_ref().clone();
        let version = data.get("version").and_then(|v| v.downcast_ref::<usize>()).unwrap_or(&0) + 1;
        data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
        Arc::new(RholangNode::RepeatedBind {
            base: base.clone(),
            names: new_names,
            remainder: new_remainder,
            source: new_source,
            metadata: Some(Arc::new(data)),
        })
    }

    fn visit_peek_bind(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        names: &Vector<Arc<RholangNode>, ArcK>,
        remainder: &Option<Arc<RholangNode>>,
        source: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_names = names.iter().map(|n| self.visit_node(n)).collect::<Vector<_, ArcK>>();
        let new_remainder = remainder.as_ref().map(|r| self.visit_node(r));
        let new_source = self.visit_node(source);
        let new_metadata = metadata.clone().unwrap_or_else(|| Arc::new(HashMap::new()));
        let mut data = new_metadata.as_ref().clone();
        let version = data.get("version").and_then(|v| v.downcast_ref::<usize>()).unwrap_or(&0) + 1;
        data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
        Arc::new(RholangNode::PeekBind {
            base: base.clone(),
            names: new_names,
            remainder: new_remainder,
            source: new_source,
            metadata: Some(Arc::new(data)),
        })
    }

    fn visit_comment(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        kind: &CommentKind,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_metadata = metadata.clone().unwrap_or_else(|| Arc::new(HashMap::new()));
        let mut data = new_metadata.as_ref().clone();
        let version = data.get("version").and_then(|v| v.downcast_ref::<usize>()).unwrap_or(&0) + 1;
        data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
        Arc::new(RholangNode::Comment {
            base: base.clone(),
            kind: kind.clone(),
            metadata: Some(Arc::new(data)),
        })
    }

    fn visit_receive_send_source(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        name: &Arc<RholangNode>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_name = self.visit_node(name);
        let new_metadata = metadata.clone().unwrap_or_else(|| Arc::new(HashMap::new()));
        let mut data = new_metadata.as_ref().clone();
        let version = data.get("version").and_then(|v| v.downcast_ref::<usize>()).unwrap_or(&0) + 1;
        data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
        Arc::new(RholangNode::ReceiveSendSource {
            base: base.clone(),
            name: new_name,
            metadata: Some(Arc::new(data)),
        })
    }

    fn visit_send_receive_source(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        name: &Arc<RholangNode>,
        inputs: &Vector<Arc<RholangNode>, ArcK>,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_name = self.visit_node(name);
        let new_inputs = inputs.iter().map(|i| self.visit_node(i)).collect::<Vector<_, ArcK>>();
        let new_metadata = metadata.clone().unwrap_or_else(|| Arc::new(HashMap::new()));
        let mut data = new_metadata.as_ref().clone();
        let version = data.get("version").and_then(|v| v.downcast_ref::<usize>()).unwrap_or(&0) + 1;
        data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
        Arc::new(RholangNode::SendReceiveSource {
            base: base.clone(),
            name: new_name,
            inputs: new_inputs,
            metadata: Some(Arc::new(data)),
        })
    }

    fn visit_wildcard(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_metadata = metadata.clone().unwrap_or_else(|| Arc::new(HashMap::new()));
        let mut data = new_metadata.as_ref().clone();
        let version = data.get("version").and_then(|v| v.downcast_ref::<usize>()).unwrap_or(&0) + 1;
        data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
        Arc::new(RholangNode::Wildcard {
            base: base.clone(),
            metadata: Some(Arc::new(data)),
        })
    }

    fn visit_simple_type(
        &self,
        _node: &Arc<RholangNode>,
        base: &NodeBase,
        value: &String,
        metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        let new_metadata = metadata.clone().unwrap_or_else(|| Arc::new(HashMap::new()));
        let mut data = new_metadata.as_ref().clone();
        let version = data.get("version").and_then(|v| v.downcast_ref::<usize>()).unwrap_or(&0) + 1;
        data.insert("version".to_string(), Arc::new(version) as Arc<dyn Any + Send + Sync>);
        Arc::new(RholangNode::SimpleType {
            base: base.clone(),
            value: value.clone(),
            metadata: Some(Arc::new(data)),
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
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree, &rope);
        let simplifier = SimplifyDoubleUnary;
        let transformed = simplifier.visit_node(&ir);
        assert!(matches!(*transformed, RholangNode::Var { ref name, .. } if name == "x"));
        assert!(!transformed.metadata().is_none(), "Transformed node should have metadata");
    }

    #[test]
    fn test_single_negation_unchanged() {
        let code = "-x";
        let tree = parse_code(code);
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree, &rope);
        let simplifier = SimplifyDoubleUnary;
        let transformed = simplifier.visit_node(&ir);
        assert!(matches!(*transformed, RholangNode::UnaryOp { op: UnaryOperator::Neg, ref operand, .. } if matches!(**operand, RholangNode::Var { ref name, .. } if name == "x")));
        assert!(!transformed.metadata().is_none(), "Transformed node should have metadata");
    }

    #[test]
    fn test_simplify_double_not() {
        let simplifier = SimplifyDoubleUnary;

        let code = "true";
        let tree_single = parse_code(code);
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree_single, &rope);
        let transformed_single = simplifier.visit_node(&ir);
        assert!(matches!(*transformed_single, RholangNode::BoolLiteral { value: true, .. }), "Non-negated should remain unchanged");
        assert!(!transformed_single.metadata().is_none(), "Transformed node should have metadata");

        let code = "not true";
        let tree_single = parse_code(code);
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree_single, &rope);
        let transformed_single = simplifier.visit_node(&ir);
        assert!(matches!(*transformed_single, RholangNode::BoolLiteral { value: false, .. }), "Single not be negated");
        assert!(!transformed_single.metadata().is_none(), "Transformed node should have metadata");

        let code = "not false";
        let tree_single = parse_code(code);
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree_single, &rope);
        let transformed_single = simplifier.visit_node(&ir);
        assert!(matches!(*transformed_single, RholangNode::BoolLiteral { value: true, .. }), "Single not be negated");
        assert!(!transformed_single.metadata().is_none(), "Transformed node should have metadata");

        let code = "not not true";
        let tree = parse_code(code);
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree, &rope);
        let transformed = simplifier.visit_node(&ir);
        assert!(matches!(*transformed, RholangNode::BoolLiteral { value: true, .. }), "Double not should simplify to original value");
        assert!(!transformed.metadata().is_none(), "Transformed node should have metadata");

        let code = "not not not true";
        let tree = parse_code(code);
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree, &rope);
        let transformed = simplifier.visit_node(&ir);
        assert!(matches!(*transformed, RholangNode::BoolLiteral { value: false, .. }), "Triple not should simplify to the negation of the original value");
        assert!(!transformed.metadata().is_none(), "Transformed node should have metadata");
    }

    #[test]
    fn test_simplify_within_par() {
        let code = "--42 | x";
        let tree = parse_code(code);
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree, &rope);
        let simplifier = SimplifyDoubleUnary;
        let transformed = simplifier.visit_node(&ir);
        if let RholangNode::Par { ref left, ref right, .. } = *transformed {
            assert!(matches!(**left, RholangNode::LongLiteral { value: 42, .. }), "Double negation in par should simplify");
            assert!(matches!(**right, RholangNode::Var { ref name, .. } if name == "x"));
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
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree, &rope);
        let transformed = pipeline.apply(&ir);
        assert!(matches!(*transformed, RholangNode::LongLiteral { value: 42, .. }), "Pipeline should simplify double negation");
        if let Some(metadata) = transformed.metadata() {
            let version = metadata.get("version")
                .and_then(|v| v.downcast_ref::<usize>())
                .copied()
                .unwrap_or(0);
            assert_eq!(version, 1, "Metadata version should be incremented");
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
            let rope = Rope::from_str(&code);
            let ir = parse_to_ir(&tree, &rope);
            let simplifier = SimplifyDoubleUnary;
            let transformed = simplifier.visit_node(&ir);

            let transformed_twice = simplifier.visit_node(&transformed);
            if transformed != transformed_twice {
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
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree, &rope);

        if let RholangNode::Par { left, right, .. } = &*ir {
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
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree, &rope);
        let simplifier = SimplifyDoubleUnary;
        let transformed = simplifier.visit_node(&ir);
        let original_start = ir.absolute_start(&ir);
        let transformed_start = transformed.absolute_start(&transformed);
        assert_eq!(original_start, transformed_start);
    }
}
