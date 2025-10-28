use std::any::Any;
use std::cmp::Ordering;
use std::sync::Arc;

use ropey::{Rope, RopeSlice};

use tracing::{debug, warn};

use super::node_types::*;
use super::position_tracking::compute_absolute_positions;

#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use rpds::Vector;
#[cfg(test)]
use super::node_operations::{match_pat, match_contract};

impl RholangNode {
    /// Returns the processes in a Par node, handling both binary and n-ary forms.
    ///
    /// This helper provides uniform access during the migration from binary to n-ary Par.
    /// Returns an empty vector if called on a non-Par node.
    pub fn par_processes(&self) -> Vec<Arc<RholangNode>> {
        match self {
            RholangNode::Par { processes: Some(procs), .. } => {
                // N-ary form - return all processes
                procs.iter().cloned().collect()
            }
            RholangNode::Par { left: Some(left), right: Some(right), .. } => {
                // Binary form - return left and right
                vec![left.clone(), right.clone()]
            }
            _ => vec![],
        }
    }

    /// Returns the starting line number of the node within the source code.
    ///
    /// # Arguments
    /// * root - The root node of the IR tree, used for position computation.
    pub fn start_line(&self, root: &Arc<RholangNode>) -> usize {
        let positions = compute_absolute_positions(root);
        let key = self as *const RholangNode as usize;
        positions.get(&key).expect("RholangNode not found").0.row
    }

    /// Returns the starting column number of the node within the source code.
    ///
    /// # Arguments
    /// * root - The root node of the IR tree, used for position computation.
    pub fn start_column(&self, root: &Arc<RholangNode>) -> usize {
        let positions = compute_absolute_positions(root);
        let key = self as *const RholangNode as usize;
        positions.get(&key).expect("RholangNode not found").0.column
    }

    /// Returns the ending line number of the node within the source code.
    ///
    /// # Arguments
    /// * root - The root node of the IR tree, used for position computation.
    pub fn end_line(&self, root: &Arc<RholangNode>) -> usize {
        let positions = compute_absolute_positions(root);
        let key = self as *const RholangNode as usize;
        positions.get(&key).expect("RholangNode not found").1.row
    }

    /// Returns the ending column number of the node within the source code.
    ///
    /// # Arguments
    /// * root - The root node of the IR tree, used for position computation.
    pub fn end_column(&self, root: &Arc<RholangNode>) -> usize {
        let positions = compute_absolute_positions(root);
        let key = self as *const RholangNode as usize;
        positions.get(&key).expect("RholangNode not found").1.column
    }

    /// Returns the byte offset of the node’s start position in the source code.
    ///
    /// # Arguments
    /// * root - The root node of the IR tree, used for position computation.
    pub fn position(&self, root: &Arc<RholangNode>) -> usize {
        let positions = compute_absolute_positions(root);
        let key = self as *const RholangNode as usize;
        positions.get(&key).expect("RholangNode not found").0.byte
    }

    /// Returns the length of the node's text in bytes.
    pub fn length(&self) -> usize {
        self.base().length()
    }

    /// Returns the absolute start position of the node in the source code.
    ///
    /// # Arguments
    /// * root - The root node of the IR tree, used for position computation.
    pub fn absolute_start(&self, root: &Arc<RholangNode>) -> Position {
        let positions = compute_absolute_positions(root);
        let key = self as *const RholangNode as usize;
        positions.get(&key).expect("RholangNode not found").0
    }

    /// Returns the absolute end position of the node in the source code.
    ///
    /// # Arguments
    /// * root - The root node of the IR tree, used for position computation.
    pub fn absolute_end(&self, root: &Arc<RholangNode>) -> Position {
        let positions = compute_absolute_positions(root);
        let key = self as *const RholangNode as usize;
        positions.get(&key).expect("RholangNode not found").1
    }

    /// Creates a new node with the same fields but a different NodeBase.
    ///
    /// # Arguments
    /// * new_base - The new NodeBase to apply to the node.
    ///
    /// # Returns
    /// A new Arc<RholangNode> with the updated base.
    pub fn with_base(&self, new_base: NodeBase) -> Arc<RholangNode> {
        match self {
            RholangNode::Par {
                processes: None,
                metadata,
                left,
                right,
                ..
            } => Arc::new(RholangNode::Par {
                processes: None,
                base: new_base,
                left: left.clone(),
                right: right.clone(),
                metadata: metadata.clone(),
            }),
            // N-ary Par case (currently not used but needed for exhaustiveness)
            RholangNode::Par {
                processes: Some(procs),
                metadata,
                ..
            } => Arc::new(RholangNode::Par {
                base: new_base,
                left: None,
                right: None,
                processes: Some(procs.clone()),
                metadata: metadata.clone(),
            }),
            RholangNode::SendSync {
                metadata,
                channel,
                inputs,
                cont,
                ..
            } => Arc::new(RholangNode::SendSync {
                base: new_base,
                channel: channel.clone(),
                inputs: inputs.clone(),
                cont: cont.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Send {
                metadata,
                channel,
                send_type,
                send_type_delta,
                inputs,
                ..
            } => Arc::new(RholangNode::Send {
                base: new_base,
                channel: channel.clone(),
                send_type: send_type.clone(),
                send_type_delta: *send_type_delta,
                inputs: inputs.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::New { decls, proc, metadata, .. } => Arc::new(RholangNode::New {
                base: new_base,
                decls: decls.clone(),
                proc: proc.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::IfElse {
                condition,
                consequence,
                alternative,
                metadata,
                ..
            } => Arc::new(RholangNode::IfElse {
                base: new_base,
                condition: condition.clone(),
                consequence: consequence.clone(),
                alternative: alternative.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Let { decls, proc, metadata, .. } => Arc::new(RholangNode::Let {
                base: new_base,
                decls: decls.clone(),
                proc: proc.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Bundle {
                bundle_type,
                proc,
                metadata,
                ..
            } => Arc::new(RholangNode::Bundle {
                base: new_base,
                bundle_type: bundle_type.clone(),
                proc: proc.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Match {
                expression,
                cases,
                metadata,
                ..
            } => Arc::new(RholangNode::Match {
                base: new_base,
                expression: expression.clone(),
                cases: cases.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Choice {
                branches, metadata, ..
            } => Arc::new(RholangNode::Choice {
                base: new_base,
                branches: branches.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Contract {
                name,
                formals,
                formals_remainder,
                proc,
                metadata,
                ..
            } => Arc::new(RholangNode::Contract {
                base: new_base,
                name: name.clone(),
                formals: formals.clone(),
                formals_remainder: formals_remainder.clone(),
                proc: proc.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Input {
                receipts, proc, metadata, ..
            } => Arc::new(RholangNode::Input {
                base: new_base,
                receipts: receipts.clone(),
                proc: proc.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Block { proc, metadata, .. } => Arc::new(RholangNode::Block {
                base: new_base,
                proc: proc.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Parenthesized { expr, metadata, .. } => Arc::new(RholangNode::Parenthesized {
                base: new_base,
                expr: expr.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::BinOp {
                op,
                left,
                right,
                metadata,
                ..
            } => Arc::new(RholangNode::BinOp {
                base: new_base,
                op: op.clone(),
                left: left.clone(),
                right: right.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::UnaryOp {
                op, operand, metadata, ..
            } => Arc::new(RholangNode::UnaryOp {
                base: new_base,
                op: op.clone(),
                operand: operand.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Method {
                receiver,
                name,
                args,
                metadata,
                ..
            } => Arc::new(RholangNode::Method {
                base: new_base,
                receiver: receiver.clone(),
                name: name.clone(),
                args: args.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Eval { name, metadata, .. } => Arc::new(RholangNode::Eval {
                base: new_base,
                name: name.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Quote {
                quotable, metadata, ..
            } => Arc::new(RholangNode::Quote {
                base: new_base,
                quotable: quotable.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::VarRef {
                kind, var, metadata, ..
            } => Arc::new(RholangNode::VarRef {
                base: new_base,
                kind: kind.clone(),
                var: var.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::BoolLiteral { value, metadata, .. } => Arc::new(RholangNode::BoolLiteral {
                base: new_base,
                value: *value,
                metadata: metadata.clone(),
            }),
            RholangNode::LongLiteral { value, metadata, .. } => Arc::new(RholangNode::LongLiteral {
                base: new_base,
                value: *value,
                metadata: metadata.clone(),
            }),
            RholangNode::StringLiteral { value, metadata, .. } => Arc::new(RholangNode::StringLiteral {
                base: new_base,
                value: value.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::UriLiteral { value, metadata, .. } => Arc::new(RholangNode::UriLiteral {
                base: new_base,
                value: value.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Nil { metadata, .. } => Arc::new(RholangNode::Nil {
                base: new_base,
                metadata: metadata.clone(),
            }),
            RholangNode::List {
                elements,
                remainder,
                metadata,
                ..
            } => Arc::new(RholangNode::List {
                base: new_base,
                elements: elements.clone(),
                remainder: remainder.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Set {
                elements,
                remainder,
                metadata,
                ..
            } => Arc::new(RholangNode::Set {
                base: new_base,
                elements: elements.clone(),
                remainder: remainder.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Pathmap {
                elements,
                remainder,
                metadata,
                ..
            } => Arc::new(RholangNode::Pathmap {
                base: new_base,
                elements: elements.clone(),
                remainder: remainder.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Map {
                pairs,
                remainder,
                metadata,
                ..
            } => Arc::new(RholangNode::Map {
                base: new_base,
                pairs: pairs.clone(),
                remainder: remainder.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Tuple {
                elements, metadata, ..
            } => Arc::new(RholangNode::Tuple {
                base: new_base,
                elements: elements.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Var { name, metadata, .. } => Arc::new(RholangNode::Var {
                base: new_base,
                name: name.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::NameDecl {
                var, uri, metadata, ..
            } => Arc::new(RholangNode::NameDecl {
                base: new_base,
                var: var.clone(),
                uri: uri.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Decl {
                names,
                names_remainder,
                procs,
                metadata,
                ..
            } => Arc::new(RholangNode::Decl {
                base: new_base,
                names: names.clone(),
                names_remainder: names_remainder.clone(),
                procs: procs.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::LinearBind {
                names,
                remainder,
                source,
                metadata,
                ..
            } => Arc::new(RholangNode::LinearBind {
                base: new_base,
                names: names.clone(),
                remainder: remainder.clone(),
                source: source.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::RepeatedBind {
                names,
                remainder,
                source,
                metadata,
                ..
            } => Arc::new(RholangNode::RepeatedBind {
                base: new_base,
                names: names.clone(),
                remainder: remainder.clone(),
                source: source.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::PeekBind {
                names,
                remainder,
                source,
                metadata,
                ..
            } => Arc::new(RholangNode::PeekBind {
                base: new_base,
                names: names.clone(),
                remainder: remainder.clone(),
                source: source.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Comment { kind, metadata, .. } => Arc::new(RholangNode::Comment {
                base: new_base,
                kind: kind.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Wildcard { metadata, .. } => Arc::new(RholangNode::Wildcard {
                base: new_base,
                metadata: metadata.clone(),
            }),
            RholangNode::SimpleType { value, metadata, .. } => Arc::new(RholangNode::SimpleType {
                base: new_base,
                value: value.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::ReceiveSendSource { name, metadata, .. } => Arc::new(RholangNode::ReceiveSendSource {
                base: new_base,
                name: name.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::SendReceiveSource {
                name,
                inputs,
                metadata,
                ..
            } => Arc::new(RholangNode::SendReceiveSource {
                base: new_base,
                name: name.clone(),
                inputs: inputs.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Error {
                children, metadata, ..
            } => Arc::new(RholangNode::Error {
                base: new_base,
                children: children.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Disjunction {
                left, right, metadata, ..
            } => Arc::new(RholangNode::Disjunction {
                base: new_base,
                left: left.clone(),
                right: right.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Conjunction {
                left, right, metadata, ..
            } => Arc::new(RholangNode::Conjunction {
                base: new_base,
                left: left.clone(),
                right: right.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Negation {
                operand, metadata, ..
            } => Arc::new(RholangNode::Negation {
                base: new_base,
                operand: operand.clone(),
                metadata: metadata.clone(),
            }),
            RholangNode::Unit { metadata, .. } => Arc::new(RholangNode::Unit {
                base: new_base,
                metadata: metadata.clone(),
            }),
        }
    }

    /// Validates the node by checking for reserved keyword usage in variable names.
    ///
    /// # Returns
    /// * Ok(()) if validation passes.
    /// * Err(String) with an error message if a reserved keyword is misused.
    pub fn validate(&self) -> Result<(), String> {
        const RESERVED_KEYWORDS: &[&str] = &[
            "if", "else", "new", "in", "match", "contract", "select", "for", "let",
            "bundle", "bundle+", "bundle-", "bundle0", "true", "false", "Nil",
            "or", "and", "not", "matches",
        ];
        match self {
            RholangNode::Send { channel, .. } | RholangNode::SendSync { channel, .. } => {
                if let RholangNode::Var { name, .. } = &**channel {
                    if RESERVED_KEYWORDS.contains(&name.as_str()) {
                        return Err(format!("Channel name '{name}' is a reserved keyword"));
                    }
                }
            }
            RholangNode::Par { left: Some(left), right: Some(right), .. } => {
                left.validate()?;
                right.validate()?;
            }
            RholangNode::Par { processes: Some(procs), .. } => {
                for proc in procs.iter() {
                    proc.validate()?;
                }
            }
            RholangNode::New { decls, proc, .. } => {
                for decl in decls {
                    decl.validate()?;
                }
                proc.validate()?;
            }
            RholangNode::IfElse {
                condition,
                consequence,
                alternative,
                ..
            } => {
                condition.validate()?;
                consequence.validate()?;
                if let Some(alt) = alternative {
                    alt.validate()?;
                }
            }
            RholangNode::Let { decls, proc, .. } => {
                for decl in decls {
                    decl.validate()?;
                }
                proc.validate()?;
            }
            RholangNode::Bundle { proc, .. } => proc.validate()?,
            RholangNode::Match {
                expression, cases, ..
            } => {
                expression.validate()?;
                for (pattern, proc) in cases {
                    if let RholangNode::Var { name, .. } = &**pattern {
                        if RESERVED_KEYWORDS.contains(&name.as_str()) {
                            let pos = pattern.absolute_start(&Arc::new(self.clone()));
                            return Err(format!(
                                "Match pattern '{}' uses reserved keyword at {}:{}",
                                name, pos.row, pos.column
                            ));
                        }
                    }
                    pattern.validate()?;
                    proc.validate()?;
                }
            }
            RholangNode::Choice { branches, .. } => {
                for (inputs, proc) in branches {
                    for input in inputs {
                        if let RholangNode::LinearBind {
                            names, remainder, ..
                        } = &**input
                        {
                            for name in names {
                                if let RholangNode::Var { name: var_name, .. } = &**name {
                                    if RESERVED_KEYWORDS.contains(&var_name.as_str()) {
                                        let pos = name.absolute_start(&Arc::new(self.clone()));
                                        return Err(format!(
                                            "Select variable '{}' uses reserved keyword at {}:{}",
                                            var_name, pos.row, pos.column
                                        ));
                                    }
                                }
                            }
                            if let Some(rem) = remainder {
                                rem.validate()?;
                            }
                        }
                        input.validate()?;
                    }
                    proc.validate()?;
                }
            }
            RholangNode::Contract {
                name,
                formals,
                formals_remainder,
                proc,
                ..
            } => {
                name.validate()?;
                for formal in formals {
                    formal.validate()?;
                }
                if let Some(rem) = formals_remainder {
                    rem.validate()?;
                }
                proc.validate()?;
            }
            RholangNode::Input { receipts, proc, .. } => {
                for receipt in receipts {
                    for bind in receipt {
                        bind.validate()?;
                    }
                }
                proc.validate()?;
            }
            RholangNode::Block { proc, .. } => proc.validate()?,
            RholangNode::Parenthesized { expr, .. } => expr.validate()?,
            RholangNode::BinOp { left, right, .. } => {
                left.validate()?;
                right.validate()?;
            }
            RholangNode::UnaryOp { operand, .. } => operand.validate()?,
            RholangNode::Method { receiver, args, .. } => {
                receiver.validate()?;
                for arg in args {
                    arg.validate()?;
                }
            }
            RholangNode::Eval { name, .. } => name.validate()?,
            RholangNode::Quote { quotable, .. } => quotable.validate()?,
            RholangNode::VarRef { var, .. } => var.validate()?,
            RholangNode::List {
                elements,
                remainder,
                ..
            } => {
                for elem in elements {
                    elem.validate()?;
                }
                if let Some(rem) = remainder {
                    rem.validate()?;
                }
            }
            RholangNode::Set {
                elements,
                remainder,
                ..
            } => {
                for elem in elements {
                    elem.validate()?;
                }
                if let Some(rem) = remainder {
                    rem.validate()?;
                }
            }
            RholangNode::Map { pairs, remainder, .. } => {
                for (key, value) in pairs {
                    key.validate()?;
                    value.validate()?;
                }
                if let Some(rem) = remainder {
                    rem.validate()?;
                }
            }
            RholangNode::Tuple { elements, .. } => {
                for elem in elements {
                    elem.validate()?;
                }
            }
            RholangNode::NameDecl { var, uri, .. } => {
                var.validate()?;
                if let Some(u) = uri {
                    u.validate()?;
                }
            }
            RholangNode::Decl {
                names,
                names_remainder,
                procs,
                ..
            } => {
                for name in names {
                    name.validate()?;
                }
                if let Some(rem) = names_remainder {
                    rem.validate()?;
                }
                for proc in procs {
                    proc.validate()?;
                }
            }
            RholangNode::LinearBind {
                names,
                remainder,
                source,
                ..
            } => {
                for name in names {
                    name.validate()?;
                }
                if let Some(rem) = remainder {
                    rem.validate()?;
                }
                source.validate()?;
            }
            RholangNode::RepeatedBind {
                names,
                remainder,
                source,
                ..
            } => {
                for name in names {
                    name.validate()?;
                }
                if let Some(rem) = remainder {
                    rem.validate()?;
                }
                source.validate()?;
            }
            RholangNode::PeekBind {
                names,
                remainder,
                source,
                ..
            } => {
                for name in names {
                    name.validate()?;
                }
                if let Some(rem) = remainder {
                    rem.validate()?;
                }
                source.validate()?;
            }
            RholangNode::ReceiveSendSource { name, .. } => name.validate()?,
            RholangNode::SendReceiveSource { name, inputs, .. } => {
                name.validate()?;
                for input in inputs {
                    input.validate()?;
                }
            }
            RholangNode::Error { children, .. } => {
                for child in children {
                    child.validate()?;
                }
            }
            RholangNode::Disjunction { left, right, .. } => {
                left.validate()?;
                right.validate()?;
            }
            RholangNode::Conjunction { left, right, .. } => {
                left.validate()?;
                right.validate()?;
            }
            RholangNode::Negation { operand, .. } => operand.validate()?,
            RholangNode::Unit { .. } => {},
            _ => {},
        }
        Ok(())
    }

    /// Updates the node's metadata with a new value.
    ///
    /// # Arguments
    /// * new_metadata - The new metadata to apply to the node.
    ///
    /// # Returns
    /// A new Arc<RholangNode> with the updated metadata.
    pub fn with_metadata(&self, new_metadata: Option<Arc<Metadata>>) -> Arc<RholangNode> {
        match self {
            RholangNode::Par { base, left, right, processes, .. } => Arc::new(RholangNode::Par {
                base: base.clone(),
                left: left.clone(),
                right: right.clone(),
                processes: processes.clone(),
                metadata: new_metadata,
            }),
            RholangNode::SendSync {
                base,
                channel,
                inputs,
                cont,
                ..
            } => Arc::new(RholangNode::SendSync {
                base: base.clone(),
                channel: channel.clone(),
                inputs: inputs.clone(),
                cont: cont.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Send {
                base,
                channel,
                send_type,
                send_type_delta,
                inputs,
                ..
            } => Arc::new(RholangNode::Send {
                base: base.clone(),
                channel: channel.clone(),
                send_type: send_type.clone(),
                send_type_delta: *send_type_delta,
                inputs: inputs.clone(),
                metadata: new_metadata,
            }),
            RholangNode::New { base, decls, proc, .. } => Arc::new(RholangNode::New {
                base: base.clone(),
                decls: decls.clone(),
                proc: proc.clone(),
                metadata: new_metadata,
            }),
            RholangNode::IfElse {
                base,
                condition,
                consequence,
                alternative,
                ..
            } => Arc::new(RholangNode::IfElse {
                base: base.clone(),
                condition: condition.clone(),
                consequence: consequence.clone(),
                alternative: alternative.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Let { base, decls, proc, .. } => Arc::new(RholangNode::Let {
                base: base.clone(),
                decls: decls.clone(),
                proc: proc.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Bundle {
                base, bundle_type, proc, ..
            } => Arc::new(RholangNode::Bundle {
                base: base.clone(),
                bundle_type: bundle_type.clone(),
                proc: proc.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Match {
                base, expression, cases, ..
            } => Arc::new(RholangNode::Match {
                base: base.clone(),
                expression: expression.clone(),
                cases: cases.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Choice { base, branches, .. } => Arc::new(RholangNode::Choice {
                base: base.clone(),
                branches: branches.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Contract {
                base,
                name,
                formals,
                formals_remainder,
                proc,
                ..
            } => Arc::new(RholangNode::Contract {
                base: base.clone(),
                name: name.clone(),
                formals: formals.clone(),
                formals_remainder: formals_remainder.clone(),
                proc: proc.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Input {
                base, receipts, proc, ..
            } => Arc::new(RholangNode::Input {
                base: base.clone(),
                receipts: receipts.clone(),
                proc: proc.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Block { base, proc, .. } => Arc::new(RholangNode::Block {
                base: base.clone(),
                proc: proc.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Parenthesized { base, expr, .. } => Arc::new(RholangNode::Parenthesized {
                base: base.clone(),
                expr: expr.clone(),
                metadata: new_metadata,
            }),
            RholangNode::BinOp {
                base,
                op,
                left,
                right,
                ..
            } => Arc::new(RholangNode::BinOp {
                base: base.clone(),
                op: op.clone(),
                left: left.clone(),
                right: right.clone(),
                metadata: new_metadata,
            }),
            RholangNode::UnaryOp {
                base, op, operand, ..
            } => Arc::new(RholangNode::UnaryOp {
                base: base.clone(),
                op: op.clone(),
                operand: operand.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Method {
                base,
                receiver,
                name,
                args,
                ..
            } => Arc::new(RholangNode::Method {
                base: base.clone(),
                receiver: receiver.clone(),
                name: name.clone(),
                args: args.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Eval { base, name, .. } => Arc::new(RholangNode::Eval {
                base: base.clone(),
                name: name.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Quote { base, quotable, .. } => Arc::new(RholangNode::Quote {
                base: base.clone(),
                quotable: quotable.clone(),
                metadata: new_metadata,
            }),
            RholangNode::VarRef {
                base, kind, var, ..
            } => Arc::new(RholangNode::VarRef {
                base: base.clone(),
                kind: kind.clone(),
                var: var.clone(),
                metadata: new_metadata,
            }),
            RholangNode::BoolLiteral { base, value, .. } => Arc::new(RholangNode::BoolLiteral {
                base: base.clone(),
                value: *value,
                metadata: new_metadata,
            }),
            RholangNode::LongLiteral { base, value, .. } => Arc::new(RholangNode::LongLiteral {
                base: base.clone(),
                value: *value,
                metadata: new_metadata,
            }),
            RholangNode::StringLiteral { base, value, .. } => Arc::new(RholangNode::StringLiteral {
                base: base.clone(),
                value: value.clone(),
                metadata: new_metadata,
            }),
            RholangNode::UriLiteral { base, value, .. } => Arc::new(RholangNode::UriLiteral {
                base: base.clone(),
                value: value.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Nil { base, .. } => Arc::new(RholangNode::Nil {
                base: base.clone(),
                metadata: new_metadata,
            }),
            RholangNode::List {
                base,
                elements,
                remainder,
                ..
            } => Arc::new(RholangNode::List {
                base: base.clone(),
                elements: elements.clone(),
                remainder: remainder.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Set {
                base,
                elements,
                remainder,
                ..
            } => Arc::new(RholangNode::Set {
                base: base.clone(),
                elements: elements.clone(),
                remainder: remainder.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Pathmap {
                base,
                elements,
                remainder,
                ..
            } => Arc::new(RholangNode::Pathmap {
                base: base.clone(),
                elements: elements.clone(),
                remainder: remainder.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Map {
                base,
                pairs,
                remainder,
                ..
            } => Arc::new(RholangNode::Map {
                base: base.clone(),
                pairs: pairs.clone(),
                remainder: remainder.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Tuple { base, elements, .. } => Arc::new(RholangNode::Tuple {
                base: base.clone(),
                elements: elements.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Var { base, name, .. } => Arc::new(RholangNode::Var {
                base: base.clone(),
                name: name.clone(),
                metadata: new_metadata,
            }),
            RholangNode::NameDecl {
                base, var, uri, ..
            } => Arc::new(RholangNode::NameDecl {
                base: base.clone(),
                var: var.clone(),
                uri: uri.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Decl {
                base,
                names,
                names_remainder,
                procs,
                ..
            } => Arc::new(RholangNode::Decl {
                base: base.clone(),
                names: names.clone(),
                names_remainder: names_remainder.clone(),
                procs: procs.clone(),
                metadata: new_metadata,
            }),
            RholangNode::LinearBind {
                base,
                names,
                remainder,
                source,
                ..
            } => Arc::new(RholangNode::LinearBind {
                base: base.clone(),
                names: names.clone(),
                remainder: remainder.clone(),
                source: source.clone(),
                metadata: new_metadata,
            }),
            RholangNode::RepeatedBind {
                base,
                names,
                remainder,
                source,
                ..
            } => Arc::new(RholangNode::RepeatedBind {
                base: base.clone(),
                names: names.clone(),
                remainder: remainder.clone(),
                source: source.clone(),
                metadata: new_metadata,
            }),
            RholangNode::PeekBind {
                base,
                names,
                remainder,
                source,
                ..
            } => Arc::new(RholangNode::PeekBind {
                base: base.clone(),
                names: names.clone(),
                remainder: remainder.clone(),
                source: source.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Comment { base, kind, .. } => Arc::new(RholangNode::Comment {
                base: base.clone(),
                kind: kind.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Wildcard { base, .. } => Arc::new(RholangNode::Wildcard {
                base: base.clone(),
                metadata: new_metadata,
            }),
            RholangNode::SimpleType { base, value, .. } => Arc::new(RholangNode::SimpleType {
                base: base.clone(),
                value: value.clone(),
                metadata: new_metadata,
            }),
            RholangNode::ReceiveSendSource { base, name, .. } => Arc::new(RholangNode::ReceiveSendSource {
                base: base.clone(),
                name: name.clone(),
                metadata: new_metadata,
            }),
            RholangNode::SendReceiveSource {
                base, name, inputs, ..
            } => Arc::new(RholangNode::SendReceiveSource {
                base: base.clone(),
                name: name.clone(),
                inputs: inputs.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Error {
                base, children, ..
            } => Arc::new(RholangNode::Error {
                base: base.clone(),
                children: children.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Disjunction {
                base, left, right, ..
            } => Arc::new(RholangNode::Disjunction {
                base: base.clone(),
                left: left.clone(),
                right: right.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Conjunction {
                base, left, right, ..
            } => Arc::new(RholangNode::Conjunction {
                base: base.clone(),
                left: left.clone(),
                right: right.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Negation { base, operand, .. } => Arc::new(RholangNode::Negation {
                base: base.clone(),
                operand: operand.clone(),
                metadata: new_metadata,
            }),
            RholangNode::Unit { base, .. } => Arc::new(RholangNode::Unit {
                base: base.clone(),
                metadata: new_metadata,
            }),
        }
    }

    /// Returns the textual representation of the node by slicing the Rope.
    /// The slice is based on the node's absolute start and end byte offsets in the source.
    pub fn text<'a>(&self, rope: &'a Rope, root: &Arc<RholangNode>) -> RopeSlice<'a> {
        let start = self.absolute_start(root).byte;
        let end = self.absolute_end(root).byte;

        // Comprehensive bounds check to prevent panic
        let rope_len = rope.len_bytes();

        // Check basic invariants
        if start > end {
            warn!("Invalid text slice: start {} > end {} (rope len={}). Returning empty slice.", start, end, rope_len);
            return rope.slice(0..0);
        }

        // Check bounds
        if start > rope_len || end > rope_len {
            warn!("Text slice out of bounds: {}-{} (rope len={}). Clamping to rope length.", start, end, rope_len);
            let safe_start = start.min(rope_len);
            let safe_end = end.min(rope_len);
            if safe_start == safe_end {
                return rope.slice(0..0);
            }
            return rope.slice(safe_start..safe_end);
        }

        // Ropey requires char boundary alignment. Use char-based slicing to be safe.
        // Catch any potential panics from byte_to_char (e.g., invalid UTF-8 boundaries)
        let start_char = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            rope.byte_to_char(start)
        })).unwrap_or_else(|_| {
            warn!("byte_to_char panicked for start={}, using 0", start);
            0
        });

        let end_char = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            rope.byte_to_char(end)
        })).unwrap_or_else(|_| {
            warn!("byte_to_char panicked for end={}, using len_chars", end);
            rope.len_chars()
        });

        if start_char >= end_char {
            debug!("Empty slice after char conversion: start_char={}, end_char={}", start_char, end_char);
            return rope.slice(0..0);
        }

        // Use char-based slicing which is always safe
        let text = rope.slice(start_char..end_char);
        debug!(r#"rope.slice(char {}..{}) = "{text}""#, start_char, end_char);
        text
    }

    /// Returns a reference to the node's NodeBase.
    pub fn base(&self) -> &NodeBase {
        match self {
            RholangNode::Par { base, .. } => base,
            RholangNode::SendSync { base, .. } => base,
            RholangNode::Send { base, .. } => base,
            RholangNode::New { base, .. } => base,
            RholangNode::IfElse { base, .. } => base,
            RholangNode::Let { base, .. } => base,
            RholangNode::Bundle { base, .. } => base,
            RholangNode::Match { base, .. } => base,
            RholangNode::Choice { base, .. } => base,
            RholangNode::Contract { base, .. } => base,
            RholangNode::Input { base, .. } => base,
            RholangNode::Block { base, .. } => base,
            RholangNode::Parenthesized { base, .. } => base,
            RholangNode::BinOp { base, .. } => base,
            RholangNode::UnaryOp { base, .. } => base,
            RholangNode::Method { base, .. } => base,
            RholangNode::Eval { base, .. } => base,
            RholangNode::Quote { base, .. } => base,
            RholangNode::VarRef { base, .. } => base,
            RholangNode::BoolLiteral { base, .. } => base,
            RholangNode::LongLiteral { base, .. } => base,
            RholangNode::StringLiteral { base, .. } => base,
            RholangNode::UriLiteral { base, .. } => base,
            RholangNode::Nil { base, .. } => base,
            RholangNode::List { base, .. } => base,
            RholangNode::Set { base, .. } => base,
            RholangNode::Pathmap { base, .. } => base,
            RholangNode::Map { base, .. } => base,
            RholangNode::Tuple { base, .. } => base,
            RholangNode::Var { base, .. } => base,
            RholangNode::NameDecl { base, .. } => base,
            RholangNode::Decl { base, .. } => base,
            RholangNode::LinearBind { base, .. } => base,
            RholangNode::RepeatedBind { base, .. } => base,
            RholangNode::PeekBind { base, .. } => base,
            RholangNode::Comment { base, .. } => base,
            RholangNode::Wildcard { base, .. } => base,
            RholangNode::SimpleType { base, .. } => base,
            RholangNode::ReceiveSendSource { base, .. } => base,
            RholangNode::SendReceiveSource { base, .. } => base,
            RholangNode::Error { base, .. } => base,
            RholangNode::Disjunction { base, .. } => base,
            RholangNode::Conjunction { base, .. } => base,
            RholangNode::Negation { base, .. } => base,
            RholangNode::Unit { base, .. } => base,
        }
    }

    /// Returns an optional reference to the node's metadata.
    pub fn metadata(&self) -> Option<&Arc<Metadata>> {
        match self {
            RholangNode::Par { metadata, .. } => metadata.as_ref(),
            RholangNode::SendSync { metadata, .. } => metadata.as_ref(),
            RholangNode::Send { metadata, .. } => metadata.as_ref(),
            RholangNode::New { metadata, .. } => metadata.as_ref(),
            RholangNode::IfElse { metadata, .. } => metadata.as_ref(),
            RholangNode::Let { metadata, .. } => metadata.as_ref(),
            RholangNode::Bundle { metadata, .. } => metadata.as_ref(),
            RholangNode::Match { metadata, .. } => metadata.as_ref(),
            RholangNode::Choice { metadata, .. } => metadata.as_ref(),
            RholangNode::Contract { metadata, .. } => metadata.as_ref(),
            RholangNode::Input { metadata, .. } => metadata.as_ref(),
            RholangNode::Block { metadata, .. } => metadata.as_ref(),
            RholangNode::Parenthesized { metadata, .. } => metadata.as_ref(),
            RholangNode::BinOp { metadata, .. } => metadata.as_ref(),
            RholangNode::UnaryOp { metadata, .. } => metadata.as_ref(),
            RholangNode::Method { metadata, .. } => metadata.as_ref(),
            RholangNode::Eval { metadata, .. } => metadata.as_ref(),
            RholangNode::Quote { metadata, .. } => metadata.as_ref(),
            RholangNode::VarRef { metadata, .. } => metadata.as_ref(),
            RholangNode::BoolLiteral { metadata, .. } => metadata.as_ref(),
            RholangNode::LongLiteral { metadata, .. } => metadata.as_ref(),
            RholangNode::StringLiteral { metadata, .. } => metadata.as_ref(),
            RholangNode::UriLiteral { metadata, .. } => metadata.as_ref(),
            RholangNode::Nil { metadata, .. } => metadata.as_ref(),
            RholangNode::List { metadata, .. } => metadata.as_ref(),
            RholangNode::Set { metadata, .. } => metadata.as_ref(),
            RholangNode::Pathmap { metadata, .. } => metadata.as_ref(),
            RholangNode::Map { metadata, .. } => metadata.as_ref(),
            RholangNode::Tuple { metadata, .. } => metadata.as_ref(),
            RholangNode::Var { metadata, .. } => metadata.as_ref(),
            RholangNode::NameDecl { metadata, .. } => metadata.as_ref(),
            RholangNode::Decl { metadata, .. } => metadata.as_ref(),
            RholangNode::LinearBind { metadata, .. } => metadata.as_ref(),
            RholangNode::RepeatedBind { metadata, .. } => metadata.as_ref(),
            RholangNode::PeekBind { metadata, .. } => metadata.as_ref(),
            RholangNode::Comment { metadata, .. } => metadata.as_ref(),
            RholangNode::Wildcard { metadata, .. } => metadata.as_ref(),
            RholangNode::SimpleType { metadata, .. } => metadata.as_ref(),
            RholangNode::ReceiveSendSource { metadata, .. } => metadata.as_ref(),
            RholangNode::SendReceiveSource { metadata, .. } => metadata.as_ref(),
            RholangNode::Error { metadata, .. } => metadata.as_ref(),
            RholangNode::Disjunction { metadata, .. } => metadata.as_ref(),
            RholangNode::Conjunction { metadata, .. } => metadata.as_ref(),
            RholangNode::Negation { metadata, .. } => metadata.as_ref(),
            RholangNode::Unit { metadata, .. } => metadata.as_ref(),
        }
    }

    pub fn node_cmp(a: &RholangNode, b: &RholangNode) -> Ordering {
        let tag_a = a.tag();
        let tag_b = b.tag();
        if tag_a != tag_b {
            return tag_a.cmp(&tag_b);
        }
        match (a, b) {
            (RholangNode::Var { name: na, .. }, RholangNode::Var { name: nb, .. }) => na.cmp(nb),
            (RholangNode::BoolLiteral { value: va, .. }, RholangNode::BoolLiteral { value: vb, .. }) => va.cmp(vb),
            (RholangNode::LongLiteral { value: va, .. }, RholangNode::LongLiteral { value: vb, .. }) => va.cmp(vb),
            (RholangNode::StringLiteral { value: va, .. }, RholangNode::StringLiteral { value: vb, .. }) => va.cmp(vb),
            (RholangNode::UriLiteral { value: va, .. }, RholangNode::UriLiteral { value: vb, .. }) => va.cmp(vb),
            (RholangNode::SimpleType { value: va, .. }, RholangNode::SimpleType { value: vb, .. }) => va.cmp(vb),
            (RholangNode::Nil { .. }, RholangNode::Nil { .. }) => Ordering::Equal,
            (RholangNode::Unit { .. }, RholangNode::Unit { .. }) => Ordering::Equal,
            (RholangNode::Quote { quotable: qa, .. }, RholangNode::Quote { quotable: qb, .. }) => {
                RholangNode::node_cmp(&*qa, &*qb)
            }
            (RholangNode::Eval { name: na, .. }, RholangNode::Eval { name: nb, .. }) => RholangNode::node_cmp(&*na, &*nb),
            (
                RholangNode::VarRef {
                    kind: ka,
                    var: va,
                    ..
                },
                RholangNode::VarRef {
                    kind: kb,
                    var: vb,
                    ..
                },
            ) => ka.cmp(kb).then_with(|| RholangNode::node_cmp(&*va, &*vb)),
            (RholangNode::Disjunction { left: p_l, right: p_r, .. }, RholangNode::Disjunction { left: c_l, right: c_r, .. }) => {
                RholangNode::node_cmp(p_l, c_l).then_with(|| RholangNode::node_cmp(p_r, c_r))
            }
            (RholangNode::Conjunction { left: p_l, right: p_r, .. }, RholangNode::Conjunction { left: c_l, right: c_r, .. }) => {
                RholangNode::node_cmp(p_l, c_l).then_with(|| RholangNode::node_cmp(p_r, c_r))
            }
            (RholangNode::Negation { operand: p_o, .. }, RholangNode::Negation { operand: c_o, .. }) => {
                RholangNode::node_cmp(p_o, c_o)
            }
            (RholangNode::Parenthesized { expr: p_e, .. }, RholangNode::Parenthesized { expr: c_e, .. }) => {
                RholangNode::node_cmp(p_e, c_e)
            }
            (
                RholangNode::List {
                    elements: ea,
                    remainder: ra,
                    ..
                },
                RholangNode::List {
                    elements: eb,
                    remainder: rb,
                    ..
                },
            ) => {
                let mut ea_sorted: Vec<&Arc<RholangNode>> = ea.iter().collect();
                ea_sorted.sort_by(|a, b| RholangNode::node_cmp(a, b));
                let mut eb_sorted: Vec<&Arc<RholangNode>> = eb.iter().collect();
                eb_sorted.sort_by(|a, b| RholangNode::node_cmp(a, b));
                ea_sorted.cmp(&eb_sorted).then_with(|| ra.cmp(rb))
            }
            (RholangNode::Tuple { elements: ea, .. }, RholangNode::Tuple { elements: eb, .. }) => ea.cmp(eb),
            (
                RholangNode::Set {
                    elements: ea,
                    remainder: ra,
                    ..
                },
                RholangNode::Set {
                    elements: eb,
                    remainder: rb,
                    ..
                },
            ) => {
                let mut ea_sorted: Vec<&Arc<RholangNode>> = ea.iter().collect();
                ea_sorted.sort_by(|a, b| RholangNode::node_cmp(a, b));
                let mut eb_sorted: Vec<&Arc<RholangNode>> = eb.iter().collect();
                eb_sorted.sort_by(|a, b| RholangNode::node_cmp(a, b));
                ea_sorted.cmp(&eb_sorted).then_with(|| ra.cmp(rb))
            }
            (
                RholangNode::Map {
                    pairs: pa,
                    remainder: ra,
                    ..
                },
                RholangNode::Map {
                    pairs: pb,
                    remainder: rb,
                    ..
                },
            ) => {
                let mut pa_sorted: Vec<(&Arc<RholangNode>, &Arc<RholangNode>)> =
                    pa.iter().map(|(k, v)| (k, v)).collect();
                pa_sorted.sort_by(|(ka, _), (kb, _)| RholangNode::node_cmp(ka, kb));
                let mut pb_sorted: Vec<(&Arc<RholangNode>, &Arc<RholangNode>)> =
                    pb.iter().map(|(k, v)| (k, v)).collect();
                pb_sorted.sort_by(|(ka, _), (kb, _)| RholangNode::node_cmp(ka, kb));
                pa_sorted.cmp(&pb_sorted).then_with(|| ra.cmp(rb))
            }
            _ => Ordering::Equal, // For unmatched or leaf variants without comparable fields
        }
    }

    pub fn tag(&self) -> u32 {
        match self {
            RholangNode::Par { .. } => 0,
            RholangNode::SendSync { .. } => 1,
            RholangNode::Send { .. } => 2,
            RholangNode::New { .. } => 3,
            RholangNode::IfElse { .. } => 4,
            RholangNode::Let { .. } => 5,
            RholangNode::Bundle { .. } => 6,
            RholangNode::Match { .. } => 7,
            RholangNode::Choice { .. } => 8,
            RholangNode::Contract { .. } => 9,
            RholangNode::Input { .. } => 10,
            RholangNode::Block { .. } => 11,
            RholangNode::Parenthesized { .. } => 12,
            RholangNode::BinOp { .. } => 13,
            RholangNode::UnaryOp { .. } => 14,
            RholangNode::Method { .. } => 15,
            RholangNode::Eval { .. } => 16,
            RholangNode::Quote { .. } => 17,
            RholangNode::VarRef { .. } => 18,
            RholangNode::BoolLiteral { .. } => 19,
            RholangNode::LongLiteral { .. } => 20,
            RholangNode::StringLiteral { .. } => 21,
            RholangNode::UriLiteral { .. } => 22,
            RholangNode::Nil { .. } => 23,
            RholangNode::List { .. } => 24,
            RholangNode::Set { .. } => 25,
            RholangNode::Pathmap { .. } => 26,
            RholangNode::Map { .. } => 27,
            RholangNode::Tuple { .. } => 28,
            RholangNode::Var { .. } => 29,
            RholangNode::NameDecl { .. } => 30,
            RholangNode::Decl { .. } => 31,
            RholangNode::LinearBind { .. } => 32,
            RholangNode::RepeatedBind { .. } => 33,
            RholangNode::PeekBind { .. } => 34,
            RholangNode::Comment { .. } => 35,
            RholangNode::Wildcard { .. } => 36,
            RholangNode::SimpleType { .. } => 37,
            RholangNode::ReceiveSendSource { .. } => 38,
            RholangNode::SendReceiveSource { .. } => 39,
            RholangNode::Error { .. } => 40,
            RholangNode::Disjunction { .. } => 41,
            RholangNode::Conjunction { .. } => 42,
            RholangNode::Negation { .. } => 43,
            RholangNode::Unit { .. } => 44,
        }
    }

    /// Constructs a new Par node with the given attributes.
    pub fn new_par(
        left: Arc<RholangNode>,
        right: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Par {
                processes: None,
            base,
            left: Some(left),
            right: Some(right),
            metadata,
        }
    }

    /// Constructs a new SendSync node with the given attributes.
    pub fn new_send_sync(
        channel: Arc<RholangNode>,
        inputs: RholangNodeVector,
        cont: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::SendSync {
            base,
            channel,
            inputs,
            cont,
            metadata,
        }
    }

    /// Constructs a new Send node with the given attributes.
    pub fn new_send(
        channel: Arc<RholangNode>,
        send_type: RholangSendType,
        send_type_delta: RelativePosition,
        inputs: RholangNodeVector,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Send {
            base,
            channel,
            send_type,
            send_type_delta,
            inputs,
            metadata,
        }
    }

    /// Constructs a new New node with the given attributes.
    pub fn new_new(
        decls: RholangNodeVector,
        proc: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::New {
            base,
            decls,
            proc,
            metadata,
        }
    }

    /// Constructs a new IfElse node with the given attributes.
    pub fn new_if_else(
        condition: Arc<RholangNode>,
        consequence: Arc<RholangNode>,
        alternative: Option<Arc<RholangNode>>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::IfElse {
            base,
            condition,
            consequence,
            alternative,
            metadata,
        }
    }

    /// Constructs a new Let node with the given attributes.
    pub fn new_let(
        decls: RholangNodeVector,
        proc: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Let {
            base,
            decls,
            proc,
            metadata,
        }
    }

    /// Constructs a new Bundle node with the given attributes.
    pub fn new_bundle(
        bundle_type: RholangBundleType,
        proc: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Bundle {
            base,
            bundle_type,
            proc,
            metadata,
        }
    }

    /// Constructs a new Match node with the given attributes.
    pub fn new_match(
        expression: Arc<RholangNode>,
        cases: RholangNodePairVector,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Match {
            base,
            expression,
            cases,
            metadata,
        }
    }

    /// Constructs a new Choice node with the given attributes.
    pub fn new_choice(
        branches: RholangBranchVector,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Choice {
            base,
            branches,
            metadata,
        }
    }

    /// Constructs a new Contract node with the given attributes.
    pub fn new_contract(
        name: Arc<RholangNode>,
        formals: RholangNodeVector,
        formals_remainder: Option<Arc<RholangNode>>,
        proc: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Contract {
            base,
            name,
            formals,
            formals_remainder,
            proc,
            metadata,
        }
    }

    /// Constructs a new Input node with the given attributes.
    pub fn new_input(
        receipts: RholangReceiptVector,
        proc: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Input {
            base,
            receipts,
            proc,
            metadata,
        }
    }

    /// Constructs a new Block node with the given attributes.
    pub fn new_block(
        proc: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Block {
            base,
            proc,
            metadata,
        }
    }

    /// Constructs a new Parenthesized node with the given attributes.
    pub fn new_parenthesized(
        expr: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Parenthesized {
            base,
            expr,
            metadata,
        }
    }

    /// Constructs a new BinOp node with the given attributes.
    pub fn new_bin_op(
        op: BinOperator,
        left: Arc<RholangNode>,
        right: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::BinOp {
            base,
            op,
            left,
            right,
            metadata,
        }
    }

    /// Constructs a new UnaryOp node with the given attributes.
    pub fn new_unary_op(
        op: UnaryOperator,
        operand: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::UnaryOp {
            base,
            op,
            operand,
            metadata,
        }
    }

    /// Constructs a new Method node with the given attributes.
    pub fn new_method(
        receiver: Arc<RholangNode>,
        name: String,
        args: RholangNodeVector,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Method {
            base,
            receiver,
            name,
            args,
            metadata,
        }
    }

    /// Constructs a new Eval node with the given attributes.
    pub fn new_eval(
        name: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Eval {
            base,
            name,
            metadata,
        }
    }

    /// Constructs a new Quote node with the given attributes.
    pub fn new_quote(
        quotable: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Quote {
            base,
            quotable,
            metadata,
        }
    }

    /// Constructs a new VarRef node with the given attributes.
    pub fn new_var_ref(
        kind: RholangVarRefKind,
        var: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::VarRef {
            base,
            kind,
            var,
            metadata,
        }
    }

    /// Constructs a new BoolLiteral node with the given attributes.
    pub fn new_bool_literal(
        value: bool,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::BoolLiteral {
            base,
            value,
            metadata,
        }
    }

    /// Constructs a new LongLiteral node with the given attributes.
    pub fn new_long_literal(
        value: i64,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::LongLiteral {
            base,
            value,
            metadata,
        }
    }

    /// Constructs a new StringLiteral node with the given attributes.
    pub fn new_string_literal(
        value: String,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::StringLiteral {
            base,
            value,
            metadata,
        }
    }

    /// Constructs a new UriLiteral node with the given attributes.
    pub fn new_uri_literal(
        value: String,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::UriLiteral {
            base,
            value,
            metadata,
        }
    }

    /// Constructs a new Nil node with the given attributes.
    pub fn new_nil(
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Nil { base, metadata }
    }

    /// Constructs a new List node with the given attributes.
    pub fn new_list(
        elements: RholangNodeVector,
        remainder: Option<Arc<RholangNode>>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::List {
            base,
            elements,
            remainder,
            metadata,
        }
    }

    /// Constructs a new Set node with the given attributes.
    pub fn new_set(
        elements: RholangNodeVector,
        remainder: Option<Arc<RholangNode>>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Set {
            base,
            elements,
            remainder,
            metadata,
        }
    }

    /// Constructs a new Map node with the given attributes.
    pub fn new_map(
        pairs: RholangNodePairVector,
        remainder: Option<Arc<RholangNode>>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Map {
            base,
            pairs,
            remainder,
            metadata,
        }
    }

    /// Constructs a new Tuple node with the given attributes.
    pub fn new_tuple(
        elements: RholangNodeVector,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Tuple {
            base,
            elements,
            metadata,
        }
    }

    /// Constructs a new Var node with the given attributes.
    pub fn new_var(
        name: String,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Var { base, name, metadata }
    }

    /// Constructs a new NameDecl node with the given attributes.
    pub fn new_name_decl(
        var: Arc<RholangNode>,
        uri: Option<Arc<RholangNode>>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::NameDecl {
            base,
            var,
            uri,
            metadata,
        }
    }

    /// Constructs a new Decl node with the given attributes.
    pub fn new_decl(
        names: RholangNodeVector,
        names_remainder: Option<Arc<RholangNode>>,
        procs: RholangNodeVector,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Decl {
            base,
            names,
            names_remainder,
            procs,
            metadata,
        }
    }

    /// Constructs a new LinearBind node with the given attributes.
    pub fn new_linear_bind(
        names: RholangNodeVector,
        remainder: Option<Arc<RholangNode>>,
        source: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::LinearBind {
            base,
            names,
            remainder,
            source,
            metadata,
        }
    }

    /// Constructs a new RepeatedBind node with the given attributes.
    pub fn new_repeated_bind(
        names: RholangNodeVector,
        remainder: Option<Arc<RholangNode>>,
        source: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::RepeatedBind {
            base,
            names,
            remainder,
            source,
            metadata,
        }
    }

    /// Constructs a new PeekBind node with the given attributes.
    pub fn new_peek_bind(
        names: RholangNodeVector,
        remainder: Option<Arc<RholangNode>>,
        source: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::PeekBind {
            base,
            names,
            remainder,
            source,
            metadata,
        }
    }

    /// Constructs a new Comment node with the given attributes.
    pub fn new_comment(
        kind: CommentKind,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Comment {
            base,
            kind,
            metadata,
        }
    }

    /// Constructs a new Wildcard node with the given attributes.
    pub fn new_wildcard(
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Wildcard { base, metadata }
    }

    /// Constructs a new SimpleType node with the given attributes.
    pub fn new_simple_type(
        value: String,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::SimpleType {
            base,
            value,
            metadata,
        }
    }

    /// Constructs a new ReceiveSendSource node with the given attributes.
    pub fn new_receive_send_source(
        name: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::ReceiveSendSource {
            base,
            name,
            metadata,
        }
    }

    /// Constructs a new SendReceiveSource node with the given attributes.
    pub fn new_send_receive_source(
        name: Arc<RholangNode>,
        inputs: RholangNodeVector,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::SendReceiveSource {
            base,
            name,
            inputs,
            metadata,
        }
    }

    /// Constructs a new Error node with the given attributes.
    pub fn new_error(
        children: RholangNodeVector,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Error {
            base,
            children,
            metadata,
        }
    }

    /// Constructs a new Disjunction node with the given attributes.
    pub fn new_disjunction(
        left: Arc<RholangNode>,
        right: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Disjunction {
            base,
            left,
            right,
            metadata,
        }
    }

    /// Constructs a new Conjunction node with the given attributes.
    pub fn new_conjunction(
        left: Arc<RholangNode>,
        right: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Conjunction {
            base,
            left,
            right,
            metadata,
        }
    }

    /// Constructs a new Negation node with the given attributes.
    pub fn new_negation(
        operand: Arc<RholangNode>,
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Negation {
            base,
            operand,
            metadata,
        }
    }

    /// Constructs a new Unit node with the given attributes.
    pub fn new_unit(
        metadata: Option<Arc<Metadata>>,
        relative_start: RelativePosition,
        length: usize,
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        let base = NodeBase::new(relative_start, length, span_lines, span_columns);
        RholangNode::Unit { base, metadata }
    }
}

impl PartialEq for RholangNode {
    fn eq(&self, other: &Self) -> bool {
        RholangNode::node_cmp(self, other) == Ordering::Equal
    }
}

impl Eq for RholangNode {}

impl PartialOrd for RholangNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(RholangNode::node_cmp(self, other))
    }
}

impl Ord for RholangNode {
    fn cmp(&self, other: &Self) -> Ordering {
        RholangNode::node_cmp(self, other)
    }
}

// Implementation of SemanticNode trait for language-agnostic IR operations
impl super::super::semantic_node::SemanticNode for RholangNode {
    fn base(&self) -> &NodeBase {
        match self {
            RholangNode::Par { base, .. } => base,
            RholangNode::SendSync { base, .. } => base,
            RholangNode::Send { base, .. } => base,
            RholangNode::New { base, .. } => base,
            RholangNode::IfElse { base, .. } => base,
            RholangNode::Let { base, .. } => base,
            RholangNode::Bundle { base, .. } => base,
            RholangNode::Match { base, .. } => base,
            RholangNode::Choice { base, .. } => base,
            RholangNode::Contract { base, .. } => base,
            RholangNode::Input { base, .. } => base,
            RholangNode::Block { base, .. } => base,
            RholangNode::Parenthesized { base, .. } => base,
            RholangNode::BinOp { base, .. } => base,
            RholangNode::UnaryOp { base, .. } => base,
            RholangNode::Method { base, .. } => base,
            RholangNode::Eval { base, .. } => base,
            RholangNode::Quote { base, .. } => base,
            RholangNode::VarRef { base, .. } => base,
            RholangNode::BoolLiteral { base, .. } => base,
            RholangNode::LongLiteral { base, .. } => base,
            RholangNode::StringLiteral { base, .. } => base,
            RholangNode::UriLiteral { base, .. } => base,
            RholangNode::Nil { base, .. } => base,
            RholangNode::List { base, .. } => base,
            RholangNode::Set { base, .. } => base,
            RholangNode::Pathmap { base, .. } => base,
            RholangNode::Map { base, .. } => base,
            RholangNode::Tuple { base, .. } => base,
            RholangNode::Var { base, .. } => base,
            RholangNode::NameDecl { base, .. } => base,
            RholangNode::Decl { base, .. } => base,
            RholangNode::LinearBind { base, .. } => base,
            RholangNode::RepeatedBind { base, .. } => base,
            RholangNode::PeekBind { base, .. } => base,
            RholangNode::Comment { base, .. } => base,
            RholangNode::Wildcard { base, .. } => base,
            RholangNode::SimpleType { base, .. } => base,
            RholangNode::ReceiveSendSource { base, .. } => base,
            RholangNode::SendReceiveSource { base, .. } => base,
            RholangNode::Error { base, .. } => base,
            RholangNode::Disjunction { base, .. } => base,
            RholangNode::Conjunction { base, .. } => base,
            RholangNode::Negation { base, .. } => base,
            RholangNode::Unit { base, .. } => base,
        }
    }

    fn metadata(&self) -> Option<&Metadata> {
        match self {
            RholangNode::Par { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::SendSync { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Send { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::New { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::IfElse { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Let { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Bundle { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Match { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Choice { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Contract { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Input { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Block { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Parenthesized { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::BinOp { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::UnaryOp { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Method { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Eval { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Quote { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::VarRef { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::BoolLiteral { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::LongLiteral { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::StringLiteral { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::UriLiteral { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Nil { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::List { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Set { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Pathmap { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Map { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Tuple { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Var { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::NameDecl { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Decl { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::LinearBind { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::RepeatedBind { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::PeekBind { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Comment { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Wildcard { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::SimpleType { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::ReceiveSendSource { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::SendReceiveSource { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Error { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Disjunction { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Conjunction { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Negation { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
            RholangNode::Unit { metadata, .. } => metadata.as_ref().map(|m| m.as_ref()),
        }
    }

    fn metadata_mut(&mut self) -> Option<&mut Metadata> {
        // Metadata is currently Option<Arc<Metadata>> which is immutable
        // This would require refactoring to support mutable access
        // For now, return None to indicate unsupported
        None
    }

    fn semantic_category(&self) -> super::super::semantic_node::SemanticCategory {
        use super::super::semantic_node::SemanticCategory;
        match self {
            // Rholang-specific process calculus constructs
            RholangNode::Par { .. } => SemanticCategory::LanguageSpecific,
            RholangNode::SendSync { .. } | RholangNode::Send { .. } => SemanticCategory::LanguageSpecific,
            RholangNode::New { .. } => SemanticCategory::Binding,
            RholangNode::IfElse { .. } | RholangNode::Choice { .. } => SemanticCategory::Conditional,
            RholangNode::Let { .. } => SemanticCategory::Binding,
            RholangNode::Bundle { .. } => SemanticCategory::LanguageSpecific,
            RholangNode::Match { .. } => SemanticCategory::Match,
            RholangNode::Contract { .. } => SemanticCategory::Binding,
            RholangNode::Input { .. } => SemanticCategory::LanguageSpecific,
            RholangNode::Block { .. } | RholangNode::Parenthesized { .. } => SemanticCategory::Block,
            RholangNode::BinOp { .. } | RholangNode::UnaryOp { .. } | RholangNode::Method { .. } => SemanticCategory::Invocation,
            RholangNode::Eval { .. } => SemanticCategory::LanguageSpecific,
            RholangNode::Quote { .. } => SemanticCategory::LanguageSpecific,
            RholangNode::VarRef { .. } | RholangNode::Var { .. } => SemanticCategory::Variable,
            RholangNode::BoolLiteral { .. } | RholangNode::LongLiteral { .. }
            | RholangNode::StringLiteral { .. } | RholangNode::UriLiteral { .. } => SemanticCategory::Literal,
            RholangNode::Nil { .. } | RholangNode::Wildcard { .. } | RholangNode::Unit { .. } => SemanticCategory::Literal,
            RholangNode::List { .. } | RholangNode::Set { .. } | RholangNode::Pathmap { .. } | RholangNode::Map { .. } | RholangNode::Tuple { .. } => SemanticCategory::Collection,
            RholangNode::NameDecl { .. } | RholangNode::Decl { .. } => SemanticCategory::Binding,
            RholangNode::LinearBind { .. } | RholangNode::RepeatedBind { .. } | RholangNode::PeekBind { .. } => SemanticCategory::Binding,
            RholangNode::Comment { .. } => SemanticCategory::Unknown,
            RholangNode::SimpleType { .. } => SemanticCategory::Literal,
            RholangNode::ReceiveSendSource { .. } | RholangNode::SendReceiveSource { .. } => SemanticCategory::LanguageSpecific,
            RholangNode::Error { .. } => SemanticCategory::Unknown,
            RholangNode::Disjunction { .. } | RholangNode::Conjunction { .. } | RholangNode::Negation { .. } => SemanticCategory::Match,
        }
    }

    fn type_name(&self) -> &'static str {
        match self {
            RholangNode::Par { .. } => "Rholang::Par",
            RholangNode::SendSync { .. } => "Rholang::SendSync",
            RholangNode::Send { .. } => "Rholang::Send",
            RholangNode::New { .. } => "Rholang::New",
            RholangNode::IfElse { .. } => "Rholang::IfElse",
            RholangNode::Let { .. } => "Rholang::Let",
            RholangNode::Bundle { .. } => "Rholang::Bundle",
            RholangNode::Match { .. } => "Rholang::Match",
            RholangNode::Choice { .. } => "Rholang::Choice",
            RholangNode::Contract { .. } => "Rholang::Contract",
            RholangNode::Input { .. } => "Rholang::Input",
            RholangNode::Block { .. } => "Rholang::Block",
            RholangNode::Parenthesized { .. } => "Rholang::Parenthesized",
            RholangNode::BinOp { .. } => "Rholang::BinOp",
            RholangNode::UnaryOp { .. } => "Rholang::UnaryOp",
            RholangNode::Method { .. } => "Rholang::Method",
            RholangNode::Eval { .. } => "Rholang::Eval",
            RholangNode::Quote { .. } => "Rholang::Quote",
            RholangNode::VarRef { .. } => "Rholang::VarRef",
            RholangNode::Var { .. } => "Rholang::Var",
            RholangNode::BoolLiteral { .. } => "Rholang::BoolLiteral",
            RholangNode::LongLiteral { .. } => "Rholang::LongLiteral",
            RholangNode::StringLiteral { .. } => "Rholang::StringLiteral",
            RholangNode::UriLiteral { .. } => "Rholang::UriLiteral",
            RholangNode::Nil { .. } => "Rholang::Nil",
            RholangNode::Wildcard { .. } => "Rholang::Wildcard",
            RholangNode::Unit { .. } => "Rholang::Unit",
            RholangNode::List { .. } => "Rholang::List",
            RholangNode::Set { .. } => "Rholang::Set",
            RholangNode::Pathmap { .. } => "Rholang::Pathmap",
            RholangNode::Map { .. } => "Rholang::Map",
            RholangNode::Tuple { .. } => "Rholang::Tuple",
            RholangNode::NameDecl { .. } => "Rholang::NameDecl",
            RholangNode::Decl { .. } => "Rholang::Decl",
            RholangNode::LinearBind { .. } => "Rholang::LinearBind",
            RholangNode::RepeatedBind { .. } => "Rholang::RepeatedBind",
            RholangNode::PeekBind { .. } => "Rholang::PeekBind",
            RholangNode::Comment { .. } => "Rholang::Comment",
            RholangNode::SimpleType { .. } => "Rholang::SimpleType",
            RholangNode::ReceiveSendSource { .. } => "Rholang::ReceiveSendSource",
            RholangNode::SendReceiveSource { .. } => "Rholang::SendReceiveSource",
            RholangNode::Error { .. } => "Rholang::Error",
            RholangNode::Disjunction { .. } => "Rholang::Disjunction",
            RholangNode::Conjunction { .. } => "Rholang::Conjunction",
            RholangNode::Negation { .. } => "Rholang::Negation",
        }
    }

    fn children_count(&self) -> usize {
        match self {
            // N-ary nodes (variable children)
            RholangNode::Par { processes: Some(procs), .. } => procs.len(),
            // Binary nodes (2 children)
            RholangNode::Par { left: Some(left), right: Some(right), .. } => {
                let _ = (left, right);
                2
            }
            RholangNode::BinOp { left, right, .. } => {
                let _ = (left, right);
                2
            }
            RholangNode::Disjunction { left, right, .. } => {
                let _ = (left, right);
                2
            }
            RholangNode::Conjunction { left, right, .. } => {
                let _ = (left, right);
                2
            }

            // Unary nodes (1 child)
            RholangNode::UnaryOp { operand, .. } => {
                let _ = operand;
                1
            }
            RholangNode::Bundle { proc, .. } => {
                let _ = proc;
                1
            }
            RholangNode::Eval { name, .. } => {
                let _ = name;
                1
            }
            RholangNode::Quote { quotable, .. } => {
                let _ = quotable;
                1
            }
            RholangNode::Block { proc, .. } => {
                let _ = proc;
                1
            }
            RholangNode::Parenthesized { expr, .. } => {
                let _ = expr;
                1
            }
            RholangNode::Negation { operand, .. } => {
                let _ = operand;
                1
            }

            // Nodes with vector children
            RholangNode::SendSync { channel, inputs, cont, .. } => {
                let _ = (channel, cont);
                1 + inputs.len() + 1 // channel + inputs + cont
            }
            RholangNode::Send { channel, inputs, .. } => {
                let _ = channel;
                1 + inputs.len() // channel + inputs
            }
            RholangNode::New { decls, proc, .. } => {
                let _ = (decls, proc);
                decls.len() + 1 // decls + proc
            }
            RholangNode::Let { decls, proc, .. } => {
                let _ = (decls, proc);
                decls.len() + 1 // decls + proc
            }
            RholangNode::Match { expression, cases, .. } => {
                let _ = expression;
                1 + cases.len() * 2 // target + (pattern, body) for each case
            }
            RholangNode::Choice { branches, .. } => {
                branches.len() * 2 // (patterns, body) for each branch
            }
            RholangNode::Input { receipts, proc, .. } => {
                let _ = proc;
                receipts.len() + 1 // receipts + proc
            }
            RholangNode::Contract { name, formals, proc, .. } => {
                let _ = (name, proc);
                1 + formals.len() + 1 // name + formals + proc
            }
            RholangNode::Method { receiver, args, .. } => {
                let _ = receiver;
                1 + args.len() // receiver + args
            }

            // Conditional nodes
            RholangNode::IfElse { condition, consequence, alternative, .. } => {
                let _ = (condition, consequence);
                if alternative.is_some() { 3 } else { 2 }
            }

            // Collection nodes
            RholangNode::List { elements, remainder, .. } => {
                if remainder.is_some() {
                    elements.len() + 1
                } else {
                    elements.len()
                }
            }
            RholangNode::Set { elements, .. } => elements.len(),
            RholangNode::Pathmap { elements, .. } => elements.len(),
            RholangNode::Map { pairs, .. } => pairs.len() * 2, // key + value for each pair
            RholangNode::Tuple { elements, .. } => elements.len(),

            // Leaf nodes and nodes we'll skip for now
            _ => 0,
        }
    }

    fn child_at(&self, index: usize) -> Option<&dyn super::super::semantic_node::SemanticNode> {
        match self {
            // N-ary nodes
            RholangNode::Par { processes: Some(procs), .. } => {
                procs.get(index).map(|p| &**p as &dyn super::super::semantic_node::SemanticNode)
            }
            // Binary nodes
            RholangNode::Par { left: Some(left), right: Some(right), .. } => match index {
                0 => Some(&**left),
                1 => Some(&**right),
                _ => None,
            },
            RholangNode::BinOp { left, right, .. } => match index {
                0 => Some(&**left),
                1 => Some(&**right),
                _ => None,
            },
            RholangNode::Disjunction { left, right, .. } => match index {
                0 => Some(&**left),
                1 => Some(&**right),
                _ => None,
            },
            RholangNode::Conjunction { left, right, .. } => match index {
                0 => Some(&**left),
                1 => Some(&**right),
                _ => None,
            },

            // Unary nodes
            RholangNode::UnaryOp { operand, .. } if index == 0 => Some(&**operand),
            RholangNode::Bundle { proc, .. } if index == 0 => Some(&**proc),
            RholangNode::Eval { name, .. } if index == 0 => Some(&**name),
            RholangNode::Quote { quotable, .. } if index == 0 => Some(&**quotable),
            RholangNode::Block { proc, .. } if index == 0 => Some(&**proc),
            RholangNode::Parenthesized { expr, .. } if index == 0 => Some(&**expr),
            RholangNode::Negation { operand, .. } if index == 0 => Some(&**operand),

            // Nodes with vector children
            RholangNode::SendSync { channel, inputs, cont, .. } => {
                if index == 0 {
                    Some(&**channel)
                } else if index <= inputs.len() {
                    Some(&**inputs.get(index - 1)?)
                } else if index == inputs.len() + 1 {
                    Some(&**cont)
                } else {
                    None
                }
            }
            RholangNode::Send { channel, inputs, .. } => {
                if index == 0 {
                    Some(&**channel)
                } else if index <= inputs.len() {
                    Some(&**inputs.get(index - 1)?)
                } else {
                    None
                }
            }
            RholangNode::New { decls, proc, .. } => {
                if index < decls.len() {
                    Some(&**decls.get(index)?)
                } else if index == decls.len() {
                    Some(&**proc)
                } else {
                    None
                }
            }
            RholangNode::Let { decls, proc, .. } => {
                if index < decls.len() {
                    decls.get(index).map(|d| &**d as &dyn super::super::semantic_node::SemanticNode)
                } else if index == decls.len() {
                    Some(&**proc)
                } else {
                    None
                }
            }
            RholangNode::Match { expression, cases, .. } => {
                if index == 0 {
                    Some(&**expression)
                } else {
                    let case_index = (index - 1) / 2;
                    if case_index < cases.len() {
                        let (pattern, body) = cases.get(case_index)?;
                        if (index - 1) % 2 == 0 {
                            Some(&**pattern)
                        } else {
                            Some(&**body)
                        }
                    } else {
                        None
                    }
                }
            }
            RholangNode::Choice { branches, .. } => {
                let branch_index = index / 2;
                if branch_index < branches.len() {
                    let (patterns, body) = branches.get(branch_index)?;
                    if index % 2 == 0 {
                        // Return first pattern as representative
                        patterns.get(0).map(|p| &**p as &dyn super::super::semantic_node::SemanticNode)
                    } else {
                        Some(&**body)
                    }
                } else {
                    None
                }
            }
            RholangNode::Input { receipts, proc, .. } => {
                if index < receipts.len() {
                    // Access first receipt pattern as representative
                    receipts.get(index).and_then(|r| r.get(0).map(|p| &**p as &dyn super::super::semantic_node::SemanticNode))
                } else if index == receipts.len() {
                    Some(&**proc)
                } else {
                    None
                }
            }
            RholangNode::Contract { name, formals, proc, .. } => {
                if index == 0 {
                    Some(&**name)
                } else if index <= formals.len() {
                    Some(&**formals.get(index - 1)?)
                } else if index == formals.len() + 1 {
                    Some(&**proc)
                } else {
                    None
                }
            }
            RholangNode::Method { receiver, args, .. } => {
                if index == 0 {
                    Some(&**receiver)
                } else if index <= args.len() {
                    Some(&**args.get(index - 1)?)
                } else {
                    None
                }
            }

            // Conditional nodes
            RholangNode::IfElse { condition, consequence, alternative, .. } => match index {
                0 => Some(&**condition),
                1 => Some(&**consequence),
                2 => alternative.as_ref().map(|alt| &**alt as &dyn super::super::semantic_node::SemanticNode),
                _ => None,
            },

            // Collection nodes
            RholangNode::List { elements, remainder, .. } => {
                if index < elements.len() {
                    Some(&**elements.get(index)?)
                } else if index == elements.len() && remainder.is_some() {
                    remainder.as_ref().map(|r| &**r as &dyn super::super::semantic_node::SemanticNode)
                } else {
                    None
                }
            }
            RholangNode::Set { elements, .. } => {
                elements.get(index).map(|e| &**e as &dyn super::super::semantic_node::SemanticNode)
            }
            RholangNode::Pathmap { elements, .. } => {
                elements.get(index).map(|e| &**e as &dyn super::super::semantic_node::SemanticNode)
            }
            RholangNode::Map { pairs, .. } => {
                let pair_index = index / 2;
                if pair_index < pairs.len() {
                    let (key, value) = pairs.get(pair_index)?;
                    if index % 2 == 0 {
                        Some(&**key)
                    } else {
                        Some(&**value)
                    }
                } else {
                    None
                }
            }
            RholangNode::Tuple { elements, .. } => {
                elements.get(index).map(|e| &**e as &dyn super::super::semantic_node::SemanticNode)
            }

            // Leaf nodes and unhandled return None
            _ => None,
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quickcheck::{QuickCheck, TestResult};
    use test_utils::ir::generator::RholangProc;
    use crate::tree_sitter::{parse_code, parse_to_ir};

    #[test]
    fn test_position_computation() {
        let _ = crate::logging::init_logger(false, Some("warn"), false);
        let code = "ch!(\"msg\")\nNil";
        let tree = parse_code(code);
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree, &rope);
        let root = Arc::new(ir.clone());
        if let RholangNode::Par { left: Some(left), right: Some(right), .. } = &*ir {
            let left_start = left.absolute_start(&root);
            assert_eq!(left_start.row, 0);
            assert_eq!(left_start.column, 0);
            assert_eq!(left_start.byte, 0);
            let left_end = left.absolute_end(&root);
            assert_eq!(left_end.row, 0);
            assert_eq!(left_end.column, 10);
            assert_eq!(left_end.byte, 10);
            let right_start = right.absolute_start(&root);
            assert_eq!(right_start.row, 1);
            assert_eq!(right_start.column, 0);
            assert_eq!(right_start.byte, 11);
            let right_end = right.absolute_end(&root);
            assert_eq!(right_end.row, 1);
            assert_eq!(right_end.column, 3);
            assert_eq!(right_end.byte, 14);
        } else {
            panic!("Expected Par node");
        }
    }

    #[test]
    fn test_nested_position() {
        let _ = crate::logging::init_logger(false, Some("warn"), false);
        let code = r#"new x in { x!("msg") }"#;
        let tree = parse_code(code);
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree, &rope);
        let root = Arc::new(ir.clone());
        if let RholangNode::New { decls, proc, .. } = &*ir {
            let decl_start = decls[0].absolute_start(&root);
            assert_eq!(decl_start.row, 0);
            assert_eq!(decl_start.column, 4);
            assert_eq!(decl_start.byte, 4);
            if let RholangNode::Block { proc: inner, .. } = &**proc {
                if let RholangNode::Send { channel, .. } = &**inner {
                    let chan_start = channel.absolute_start(&root);
                    assert_eq!(chan_start.row, 0);
                    assert_eq!(chan_start.column, 11);
                    assert_eq!(chan_start.byte, 11);
                } else {
                    panic!("Expected Send node");
                }
            } else {
                panic!("Expected Block node");
            }
        } else {
            panic!("Expected New node");
        }
    }

    #[test]
    fn test_prop_position_consistency() {
        fn prop(proc: RholangProc) -> TestResult {
            let code = proc.to_code();
            let tree = parse_code(&code);
            if tree.root_node().has_error() {
                return TestResult::discard();
            }
            let rope = Rope::from_str(&code);
            let ir = parse_to_ir(&tree, &rope);
            let root = Arc::new(ir.clone());
            let start = ir.absolute_start(&root);
            let end = ir.absolute_end(&root);
            assert!(start.byte <= end.byte);
            assert!(start.row <= end.row);
            if start.row == end.row {
                assert!(start.column <= end.column);
            }
            TestResult::passed()
        }
        QuickCheck::new().tests(100).max_tests(1000).quickcheck(prop as fn(RholangProc) -> TestResult);
    }

    #[test]
    fn test_multi_line_positions() {
        let _ = crate::logging::init_logger(false, Some("warn"), false);
        let code = "ch!(\n\"msg\"\n)";
        let tree = parse_code(code);
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree, &rope);
        let root = Arc::new(ir.clone());
        if let RholangNode::Send { inputs, .. } = &*ir {
            let input_start = inputs[0].absolute_start(&root);
            assert_eq!(input_start.row, 1);
            assert_eq!(input_start.column, 0);
        } else {
            panic!("Expected Send node");
        }
    }

    #[test]
    fn test_match_positioning() {
        let _ = crate::logging::init_logger(false, Some("warn"), false);
        let code = r#"match "target" { "pat" => Nil }"#;
        let tree = parse_code(code);
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree, &rope);
        let root = Arc::new(ir.clone());
        if let RholangNode::Match { expression, cases, .. } = &*ir {
            let expr_start = expression.absolute_start(&root);
            assert_eq!(expr_start.row, 0);
            assert_eq!(expr_start.column, 6);
            assert_eq!(expr_start.byte, 6);
            let (pattern, proc) = &cases[0];
            let pat_start = pattern.absolute_start(&root);
            assert_eq!(pat_start.row, 0);
            assert_eq!(pat_start.column, 17);
            assert_eq!(pat_start.byte, 17);
            let proc_start = proc.absolute_start(&root);
            assert_eq!(proc_start.row, 0);
            assert_eq!(proc_start.column, 26);
            assert_eq!(proc_start.byte, 26);
        } else {
            panic!("Expected Match node");
        }
    }

    #[test]
    fn test_metadata_dynamic() {
        let mut data = HashMap::new();
        data.insert(
            "version".to_string(),
            Arc::new(1_usize) as Arc<dyn Any + Send + Sync>,
        );
        data.insert(
            "custom".to_string(),
            Arc::new("test".to_string()) as Arc<dyn Any + Send + Sync>,
        );
        let metadata = Arc::new(data);
        let base = NodeBase::new(
            RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            0,
            0,
            0,
        );
        let node = RholangNode::Nil {
            base,
            metadata: Some(metadata.clone()),
        };
        assert_eq!(
            node.metadata()
                .unwrap()
                .get("version")
                .unwrap()
                .downcast_ref::<usize>(),
            Some(&1)
        );
        assert_eq!(
            node.metadata()
                .unwrap()
                .get("custom")
                .unwrap()
                .downcast_ref::<String>(),
            Some(&"test".to_string())
        );
    }

    #[test]
    fn test_error_node_with_children() {
        let code = r#"new x { x!("") }"#;
        let tree = parse_code(code);
        let rope = Rope::from_str(code);
        let ir = parse_to_ir(&tree, &rope);
        if let RholangNode::Par { left: Some(left), .. } = &*ir {
            if let RholangNode::Error { children, .. } = &**left {
                assert!(!children.is_empty(), "Error node should have children");
            }
        }
    }

    #[test]
    fn test_match_pat_simple() {
        let wild = Arc::new(RholangNode::new_wildcard(
            None,
            RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            1,
            0,
            1,
        ));
        let var_pat = Arc::new(RholangNode::new_var(
            "x".to_string(),
            None,
            RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            1,
            0,
            1,
        ));
        let var_con = Arc::new(RholangNode::new_var(
            "y".to_string(),
            None,
            RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            1,
            0,
            1,
        ));
        let string_pat = Arc::new(RholangNode::new_string_literal(
            "foo".to_string(),
            None,
            RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            5,
            0,
            5,
        ));
        let string_con = Arc::new(RholangNode::new_string_literal(
            "foo".to_string(),
            None,
            RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            5,
            0,
            5,
        ));
        let string_con_diff = Arc::new(RholangNode::new_string_literal(
            "bar".to_string(),
            None,
            RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            5,
            0,
            5,
        ));
        let mut subst = HashMap::new();
        assert!(match_pat(&wild, &var_con, &mut subst));
        assert!(match_pat(&var_pat, &var_con, &mut subst));
        assert_eq!(subst.get("x"), Some(&var_con));
        assert!(match_pat(&string_pat, &string_con, &mut subst));
        assert!(!match_pat(&string_pat, &string_con_diff, &mut subst));
    }

    #[test]
    fn test_match_pat_repeat() {
        let var_pat = Arc::new(RholangNode::new_var(
            "x".to_string(),
            None,
            RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            1,
            0,
            1,
        ));
        let con1 = Arc::new(RholangNode::new_long_literal(
            1,
            None,
            RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            1,
            0,
            1,
        ));
        let con2 = Arc::new(RholangNode::new_long_literal(
            1,
            None,
            RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            1,
            0,
            1,
        ));
        let con_diff = Arc::new(RholangNode::new_long_literal(
            2,
            None,
            RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            1,
            0,
            1,
        ));
        let mut subst = HashMap::new();
        assert!(match_pat(&var_pat, &con1, &mut subst));
        assert!(match_pat(&var_pat, &con2, &mut subst));
        assert!(!match_pat(&var_pat, &con_diff, &mut subst));
    }

    #[test]
    fn test_match_contract_basic() {
        let channel = Arc::new(RholangNode::new_var(
            "foo".to_string(),
            None,
            RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            3,
            0,
            3,
        ));
        let inputs = Vector::new_with_ptr_kind().push_back(Arc::new(RholangNode::new_long_literal(
            1,
            None,
            RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            1,
            0,
            1,
        )));
        let contract_name = Arc::new(RholangNode::new_var(
            "foo".to_string(),
            None,
            RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            3,
            0,
            3,
        ));
        let contract_formals = Vector::new_with_ptr_kind().push_back(Arc::new(RholangNode::new_var(
            "x".to_string(),
            None,
            RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            1,
            0,
            1,
        )));
        let contract = Arc::new(RholangNode::new_contract(
            contract_name,
            contract_formals,
            None,
            Arc::new(RholangNode::new_nil(
                None,
                RelativePosition {
                    delta_lines: 0,
                    delta_columns: 0,
                    delta_bytes: 0,
                },
                3,
                0,
                3,
            )),
            None,
            RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            0,
            0,
            0,
        ));
        assert!(match_contract(&channel, &inputs, &contract));
        let bad_inputs = Vector::new_with_ptr_kind();
        assert!(!match_contract(&channel, &bad_inputs, &contract));
    }

    #[test]
    fn test_match_pat_set() {
        let p_e = Vector::new_with_ptr_kind()
            .push_back(Arc::new(RholangNode::LongLiteral {
                base: NodeBase::new(
                    RelativePosition {
                        delta_lines: 0,
                        delta_columns: 0,
                        delta_bytes: 0,
                    },
                    1,
                    0,
                    1,
                ),
                value: 1,
                metadata: None,
            }))
            .push_back(Arc::new(RholangNode::LongLiteral {
                base: NodeBase::new(
                    RelativePosition {
                        delta_lines: 0,
                        delta_columns: 0,
                        delta_bytes: 0,
                    },
                    1,
                    0,
                    1,
                ),
                value: 2,
                metadata: None,
            }));
        let pat = Arc::new(RholangNode::Set {
            base: NodeBase::new(
                RelativePosition {
                    delta_lines: 0,
                    delta_columns: 0,
                    delta_bytes: 0,
                },
                0,
                0,
                0,
            ),
            elements: p_e,
            remainder: None,
            metadata: None,
        });
        let c_e = Vector::new_with_ptr_kind()
            .push_back(Arc::new(RholangNode::LongLiteral {
                base: NodeBase::new(
                    RelativePosition {
                        delta_lines: 0,
                        delta_columns: 0,
                        delta_bytes: 0,
                    },
                    1,
                    0,
                    1,
                ),
                value: 2,
                metadata: None,
            }))
            .push_back(Arc::new(RholangNode::LongLiteral {
                base: NodeBase::new(
                    RelativePosition {
                        delta_lines: 0,
                        delta_columns: 0,
                        delta_bytes: 0,
                    },
                    1,
                    0,
                    1,
                ),
                value: 1,
                metadata: None,
            }));
        let concrete = Arc::new(RholangNode::Set {
            base: NodeBase::new(
                RelativePosition {
                    delta_lines: 0,
                    delta_columns: 0,
                    delta_bytes: 0,
                },
                0,
                0,
                0,
            ),
            elements: c_e,
            remainder: None,
            metadata: None,
        });
        let mut subst = HashMap::new();
        assert!(crate::ir::rholang_node::match_pat(&pat, &concrete, &mut subst));
    }

    #[test]
    fn test_match_pat_map() {
        let p_pair1 = (
            Arc::new(RholangNode::StringLiteral {
                base: NodeBase::new(
                    RelativePosition {
                        delta_lines: 0,
                        delta_columns: 0,
                        delta_bytes: 0,
                    },
                    3,
                    0,
                    3,
                ),
                value: "a".to_string(),
                metadata: None,
            }),
            Arc::new(RholangNode::LongLiteral {
                base: NodeBase::new(
                    RelativePosition {
                        delta_lines: 0,
                        delta_columns: 0,
                        delta_bytes: 0,
                    },
                    1,
                    0,
                    1,
                ),
                value: 1,
                metadata: None,
            }),
        );
        let p_pair2 = (
            Arc::new(RholangNode::StringLiteral {
                base: NodeBase::new(
                    RelativePosition {
                        delta_lines: 0,
                        delta_columns: 0,
                        delta_bytes: 0,
                    },
                    3,
                    0,
                    3,
                ),
                value: "b".to_string(),
                metadata: None,
            }),
            Arc::new(RholangNode::LongLiteral {
                base: NodeBase::new(
                    RelativePosition {
                        delta_lines: 0,
                        delta_columns: 0,
                        delta_bytes: 0,
                    },
                    1,
                    0,
                    1,
                ),
                value: 2,
                metadata: None,
            }),
        );
        let p_pairs = Vector::new_with_ptr_kind().push_back(p_pair1).push_back(p_pair2);
        let pat = Arc::new(RholangNode::Map {
            base: NodeBase::new(
                RelativePosition {
                    delta_lines: 0,
                    delta_columns: 0,
                    delta_bytes: 0,
                },
                0,
                0,
                0,
            ),
            pairs: p_pairs,
            remainder: None,
            metadata: None,
        });
        let c_pair1 = (
            Arc::new(RholangNode::StringLiteral {
                base: NodeBase::new(
                    RelativePosition {
                        delta_lines: 0,
                        delta_columns: 0,
                        delta_bytes: 0,
                    },
                    3,
                    0,
                    3,
                ),
                value: "b".to_string(),
                metadata: None,
            }),
            Arc::new(RholangNode::LongLiteral {
                base: NodeBase::new(
                    RelativePosition {
                        delta_lines: 0,
                        delta_columns: 0,
                        delta_bytes: 0,
                    },
                    1,
                    0,
                    1,
                ),
                value: 2,
                metadata: None,
            }),
        );
        let c_pair2 = (
            Arc::new(RholangNode::StringLiteral {
                base: NodeBase::new(
                    RelativePosition {
                        delta_lines: 0,
                        delta_columns: 0,
                        delta_bytes: 0,
                    },
                    3,
                    0,
                    3,
                ),
                value: "a".to_string(),
                metadata: None,
            }),
            Arc::new(RholangNode::LongLiteral {
                base: NodeBase::new(
                    RelativePosition {
                        delta_lines: 0,
                        delta_columns: 0,
                        delta_bytes: 0,
                    },
                    1,
                    0,
                    1,
                ),
                value: 1,
                metadata: None,
            }),
        );
        let c_pairs = Vector::new_with_ptr_kind().push_back(c_pair1).push_back(c_pair2);
        let concrete = Arc::new(RholangNode::Map {
            base: NodeBase::new(
                RelativePosition {
                    delta_lines: 0,
                    delta_columns: 0,
                    delta_bytes: 0,
                },
                0,
                0,
                0,
            ),
            pairs: c_pairs,
            remainder: None,
            metadata: None,
        });
        let mut subst = HashMap::new();
        assert!(crate::ir::rholang_node::match_pat(&pat, &concrete, &mut subst));
    }

    #[test]
    fn test_match_pat_disjunction() {
        let p_left = Arc::new(RholangNode::LongLiteral {
            base: NodeBase::new(
                RelativePosition {
                    delta_lines: 0,
                    delta_columns: 0,
                    delta_bytes: 0,
                },
                1,
                0,
                1,
            ),
            value: 1,
            metadata: None,
        });
        let p_right = Arc::new(RholangNode::LongLiteral {
            base: NodeBase::new(
                RelativePosition {
                    delta_lines: 0,
                    delta_columns: 0,
                    delta_bytes: 0,
                },
                1,
                0,
                1,
            ),
            value: 2,
            metadata: None,
        });
        let pat = Arc::new(RholangNode::Disjunction {
            base: NodeBase::new(
                RelativePosition {
                    delta_lines: 0,
                    delta_columns: 0,
                    delta_bytes: 0,
                },
                0,
                0,
                0,
            ),
            left: p_left,
            right: p_right,
            metadata: None,
        });
        let c_left = Arc::new(RholangNode::LongLiteral {
            base: NodeBase::new(
                RelativePosition {
                    delta_lines: 0,
                    delta_columns: 0,
                    delta_bytes: 0,
                },
                1,
                0,
                1,
            ),
            value: 1,
            metadata: None,
        });
        let c_right = Arc::new(RholangNode::LongLiteral {
            base: NodeBase::new(
                RelativePosition {
                    delta_lines: 0,
                    delta_columns: 0,
                    delta_bytes: 0,
                },
                1,
                0,
                1,
            ),
            value: 2,
            metadata: None,
        });
        let concrete = Arc::new(RholangNode::Disjunction {
            base: NodeBase::new(
                RelativePosition {
                    delta_lines: 0,
                    delta_columns: 0,
                    delta_bytes: 0,
                },
                0,
                0,
                0,
            ),
            left: c_left,
            right: c_right,
            metadata: None,
        });
        let mut subst = HashMap::new();
        assert!(crate::ir::rholang_node::match_pat(&pat, &concrete, &mut subst));
    }

    #[test]
    fn test_prop_match_pat_no_crash() {
        fn prop(pat: RholangProc, concrete: RholangProc) -> TestResult {
            let pat_code = pat.to_code();
            let concrete_code = concrete.to_code();
            let pat_tree = parse_code(&pat_code);
            let concrete_tree = parse_code(&concrete_code);
            if pat_tree.root_node().has_error() || concrete_tree.root_node().has_error() {
                return TestResult::discard();
            }
            let pat_rope = Rope::from_str(&pat_code);
            let concrete_rope = Rope::from_str(&concrete_code);
            let pat_ir = parse_to_ir(&pat_tree, &pat_rope);
            let concrete_ir = parse_to_ir(&concrete_tree, &concrete_rope);
            let mut subst = HashMap::new();
            let _ = match_pat(&pat_ir, &concrete_ir, &mut subst);
            TestResult::passed()
        }
        QuickCheck::new().tests(50).max_tests(500).quickcheck(prop as fn(RholangProc, RholangProc) -> TestResult);
    }
}
