use std::collections::HashMap;
use std::sync::Arc;

use rpds::Vector;
use archery::ArcK;


use super::node_types::*;

pub fn match_pat(pat: &Arc<RholangNode>, concrete: &Arc<RholangNode>, subst: &mut HashMap<String, Arc<RholangNode>>) -> bool {
    match (&**pat, &**concrete) {
        (RholangNode::Wildcard { .. }, _) => true,
        (RholangNode::Var { name: p_name, .. }, _) => {
            if let Some(bound) = subst.get(p_name) {
                **bound == **concrete
            } else {
                subst.insert(p_name.clone(), concrete.clone());
                true
            }
        }
        (
            RholangNode::Quote {
                quotable: p_q, ..
            },
            RholangNode::Quote {
                quotable: c_q, ..
            },
        ) => match_pat(p_q, c_q, subst),
        (RholangNode::Eval { name: p_n, .. }, RholangNode::Eval { name: c_n, .. }) => match_pat(p_n, c_n, subst),
        (
            RholangNode::VarRef {
                kind: p_k,
                var: p_v,
                ..
            },
            RholangNode::VarRef {
                kind: c_k,
                var: c_v,
                ..
            },
        ) => p_k == c_k && match_pat(p_v, c_v, subst),
        (
            RholangNode::List {
                elements: p_e,
                remainder: p_r,
                ..
            },
            RholangNode::List {
                elements: c_e,
                remainder: c_r,
                ..
            },
        ) => {
            if p_e.len() > c_e.len() {
                return false;
            }
            for (p, c) in p_e.iter().zip(c_e.iter()) {
                if !match_pat(p, c, subst) {
                    return false;
                }
            }
            let rem_c_elements = c_e.iter().skip(p_e.len()).cloned().collect::<Vector<_, ArcK>>();
            let rem_base = NodeBase::new(
                RelativePosition {
                    delta_lines: 0,
                    delta_columns: 0,
                    delta_bytes: 0,
                },
                0,
                0,
                0,
            );
            let rem_list = Arc::new(RholangNode::List {
                base: rem_base,
                elements: rem_c_elements,
                remainder: c_r.clone(),
                metadata: None,
            });
            if let Some(r) = p_r {
                match_pat(r, &rem_list, subst)
            } else if let RholangNode::List {
                elements,
                remainder,
                ..
            } = &*rem_list {
                elements.is_empty() && remainder.is_none()
            } else {
                false
            }
        }
        (RholangNode::Tuple { elements: p_e, .. }, RholangNode::Tuple { elements: c_e, .. }) => {
            if p_e.len() != c_e.len() {
                false
            } else {
                p_e.iter()
                    .zip(c_e.iter())
                    .all(|(p, c)| match_pat(p, c, subst))
            }
        }
        (
            RholangNode::Set {
                elements: p_e,
                remainder: p_r,
                ..
            },
            RholangNode::Set {
                elements: c_e,
                remainder: c_r,
                ..
            },
        ) => {
            let mut p_sorted: Vec<&Arc<RholangNode>> = p_e.iter().collect();
            p_sorted.sort_by(|a, b| RholangNode::node_cmp(a, b));
            let mut c_sorted: Vec<&Arc<RholangNode>> = c_e.iter().collect();
            c_sorted.sort_by(|a, b| RholangNode::node_cmp(a, b));
            if p_sorted.len() > c_sorted.len() {
                return false;
            }
            for (p, c) in p_sorted.iter().zip(c_sorted.iter()) {
                if !match_pat(p, c, subst) {
                    return false;
                }
            }
            let rem_c_elements = c_e.iter().skip(p_e.len()).cloned().collect::<Vector<_, ArcK>>();
            let rem_base = NodeBase::new(
                RelativePosition {
                    delta_lines: 0,
                    delta_columns: 0,
                    delta_bytes: 0,
                },
                0,
                0,
                0,
            );
            let rem_set = Arc::new(RholangNode::Set {
                base: rem_base,
                elements: rem_c_elements,
                remainder: c_r.clone(),
                metadata: None,
            });
            if let Some(r) = p_r {
                match_pat(r, &rem_set, subst)
            } else if let RholangNode::Set {
                elements,
                remainder,
                ..
            } = &*rem_set {
                elements.is_empty() && remainder.is_none()
            } else {
                false
            }
        }
        (
            RholangNode::Map {
                pairs: p_pairs,
                remainder: p_r,
                ..
            },
            RholangNode::Map {
                pairs: c_pairs,
                remainder: c_r,
                ..
            },
        ) => {
            let mut p_sorted: Vec<(&Arc<RholangNode>, &Arc<RholangNode>)> =
                p_pairs.iter().map(|(k, v)| (k, v)).collect();
            p_sorted.sort_by(|(ka, _), (kb, _)| RholangNode::node_cmp(ka, kb));
            let mut c_sorted: Vec<(&Arc<RholangNode>, &Arc<RholangNode>)> =
                c_pairs.iter().map(|(k, v)| (k, v)).collect();
            c_sorted.sort_by(|(ka, _), (kb, _)| RholangNode::node_cmp(ka, kb));
            if p_sorted.len() > c_sorted.len() {
                return false;
            }
            for ((p_k, p_v), (c_k, c_v)) in p_sorted.iter().zip(c_sorted.iter()) {
                if !match_pat(p_k, c_k, subst) || !match_pat(p_v, c_v, subst) {
                    return false;
                }
            }
            let rem_c_pairs = c_pairs.iter().skip(p_pairs.len()).cloned().collect::<Vector<_, ArcK>>();
            let rem_base = NodeBase::new(
                RelativePosition {
                    delta_lines: 0,
                    delta_columns: 0,
                    delta_bytes: 0,
                },
                0,
                0,
                0,
            );
            let rem_map = Arc::new(RholangNode::Map {
                base: rem_base,
                pairs: rem_c_pairs,
                remainder: c_r.clone(),
                metadata: None,
            });
            if let Some(r) = p_r {
                match_pat(r, &rem_map, subst)
            } else if let RholangNode::Map {
                pairs,
                remainder,
                ..
            } = &*rem_map {
                pairs.is_empty() && remainder.is_none()
            } else {
                false
            }
        }
        (RholangNode::BoolLiteral { value: p, .. }, RholangNode::BoolLiteral { value: c, .. }) => p == c,
        (RholangNode::LongLiteral { value: p, .. }, RholangNode::LongLiteral { value: c, .. }) => p == c,
        (RholangNode::StringLiteral { value: p, .. }, RholangNode::StringLiteral { value: c, .. }) => p == c,
        (RholangNode::UriLiteral { value: p, .. }, RholangNode::UriLiteral { value: c, .. }) => p == c,
        (RholangNode::SimpleType { value: p, .. }, RholangNode::SimpleType { value: c, .. }) => p == c,
        (RholangNode::Nil { .. }, RholangNode::Nil { .. }) => true,
        (RholangNode::Unit { .. }, RholangNode::Unit { .. }) => true,
        (RholangNode::Disjunction { left: p_l, right: p_r, .. }, RholangNode::Disjunction { left: c_l, right: c_r, .. }) => {
            match_pat(p_l, c_l, subst) && match_pat(p_r, c_r, subst)
        }
        (RholangNode::Conjunction { left: p_l, right: p_r, .. }, RholangNode::Conjunction { left: c_l, right: c_r, .. }) => {
            match_pat(p_l, c_l, subst) && match_pat(p_r, c_r, subst)
        }
        (RholangNode::Negation { operand: p_o, .. }, RholangNode::Negation { operand: c_o, .. }) => {
            match_pat(p_o, c_o, subst)
        }
        (RholangNode::Parenthesized { expr: p_e, .. }, RholangNode::Parenthesized { expr: c_e, .. }) => {
            match_pat(p_e, c_e, subst)
        }
        _ => false,
    }
}

/// Matches a contract against a call's channel and inputs.
/// Check if two nodes are equal for contract name matching (avoids pattern matching's Var unification)
pub fn contract_names_equal(a: &Arc<RholangNode>, b: &Arc<RholangNode>) -> bool {
    match (&**a, &**b) {
        // Fast path: pointer equality
        _ if Arc::ptr_eq(a, b) => true,
        // Var nodes: compare names by reference (cheap since names are strings in Arc)
        (RholangNode::Var { name: a_name, .. }, RholangNode::Var { name: b_name, .. }) => a_name == b_name,
        // StringLiteral nodes: compare values (for quoted contract names like @"myContract")
        (RholangNode::StringLiteral { value: a_val, .. }, RholangNode::StringLiteral { value: b_val, .. }) => a_val == b_val,
        // Quote nodes: recursively check quotable
        (RholangNode::Quote { quotable: a_q, .. }, RholangNode::Quote { quotable: b_q, .. }) => contract_names_equal(a_q, b_q),
        // Eval nodes: recursively check name
        (RholangNode::Eval { name: a_n, .. }, RholangNode::Eval { name: b_n, .. }) => contract_names_equal(a_n, b_n),
        // VarRef nodes: check kind and var
        (RholangNode::VarRef { kind: a_k, var: a_v, .. }, RholangNode::VarRef { kind: b_k, var: b_v, .. }) => {
            a_k == b_k && contract_names_equal(a_v, b_v)
        }
        // Different node types or other cases: not equal
        _ => false,
    }
}

pub fn match_contract(channel: &Arc<RholangNode>, inputs: &RholangNodeVector, contract: &Arc<RholangNode>) -> bool {
    if let RholangNode::Contract {
        name,
        formals,
        formals_remainder,
        ..
    } = &**contract
    {
        // For contract name matching, use exact equality instead of pattern matching
        // This avoids the issue where match_pat treats any Var as a pattern variable
        if !contract_names_equal(name, channel) {
            return false;
        }

        // For parameters, use pattern matching as before
        let mut subst = HashMap::new();
        let min_len = formals.len();
        if formals_remainder.is_none() && inputs.len() != min_len {
            return false;
        }
        if inputs.len() < min_len {
            return false;
        }
        for (f, a) in formals.iter().zip(inputs.iter()) {
            if !match_pat(f, a, &mut subst) {
                return false;
            }
        }
        if let Some(rem) = formals_remainder {
            let remaining_elements = inputs
                .iter()
                .skip(min_len)
                .cloned()
                .collect::<Vector<_, ArcK>>();
            let rem_base = NodeBase::new(
                RelativePosition {
                    delta_lines: 0,
                    delta_columns: 0,
                    delta_bytes: 0,
                },
                0,
                0,
                0,
            );
            let remaining_list = Arc::new(RholangNode::List {
                base: rem_base,
                elements: remaining_elements,
                remainder: None,
                metadata: None,
            });
            match_pat(rem, &remaining_list, &mut subst)
        } else {
            true
        }
    } else {
        false
    }
}

/// Collects all contract nodes from the IR tree.
pub fn collect_contracts(node: &Arc<RholangNode>, contracts: &mut Vec<Arc<RholangNode>>) {
    match &**node {
        RholangNode::Contract { .. } => contracts.push(node.clone()),
        RholangNode::Par { left: Some(left), right: Some(right), .. } => {
            collect_contracts(left, contracts);
            collect_contracts(right, contracts);
        }
        RholangNode::Par { processes: Some(procs), .. } => {
            for proc in procs.iter() {
                collect_contracts(proc, contracts);
            }
        }
        RholangNode::SendSync {
            channel, inputs, cont, ..
        } => {
            collect_contracts(channel, contracts);
            for input in inputs {
                collect_contracts(input, contracts);
            }
            collect_contracts(cont, contracts);
        }
        RholangNode::Send { channel, inputs, .. } => {
            collect_contracts(channel, contracts);
            for input in inputs {
                collect_contracts(input, contracts);
            }
        }
        RholangNode::New { decls, proc, .. } => {
            for decl in decls {
                collect_contracts(decl, contracts);
            }
            collect_contracts(proc, contracts);
        }
        RholangNode::IfElse {
            condition,
            consequence,
            alternative,
            ..
        } => {
            collect_contracts(condition, contracts);
            collect_contracts(consequence, contracts);
            if let Some(alt) = alternative {
                collect_contracts(alt, contracts);
            }
        }
        RholangNode::Let { decls, proc, .. } => {
            for decl in decls {
                collect_contracts(decl, contracts);
            }
            collect_contracts(proc, contracts);
        }
        RholangNode::Bundle { proc, .. } => collect_contracts(proc, contracts),
        RholangNode::Match {
            expression, cases, ..
        } => {
            collect_contracts(expression, contracts);
            for (pat, proc) in cases {
                collect_contracts(pat, contracts);
                collect_contracts(proc, contracts);
            }
        }
        RholangNode::Choice { branches, .. } => {
            for (inputs, proc) in branches {
                for input in inputs {
                    collect_contracts(input, contracts);
                }
                collect_contracts(proc, contracts);
            }
        }
        RholangNode::Input { receipts, proc, .. } => {
            for receipt in receipts {
                for bind in receipt {
                    collect_contracts(bind, contracts);
                }
            }
            collect_contracts(proc, contracts);
        }
        RholangNode::Block { proc, .. } => collect_contracts(proc, contracts),
        RholangNode::Parenthesized { expr, .. } => collect_contracts(expr, contracts),
        RholangNode::BinOp { left, right, .. } => {
            collect_contracts(left, contracts);
            collect_contracts(right, contracts);
        }
        RholangNode::UnaryOp { operand, .. } => collect_contracts(operand, contracts),
        RholangNode::Method {
            receiver, args, ..
        } => {
            collect_contracts(receiver, contracts);
            for arg in args {
                collect_contracts(arg, contracts);
            }
        }
        RholangNode::Eval { name, .. } => collect_contracts(name, contracts),
        RholangNode::Quote { quotable, .. } => collect_contracts(quotable, contracts),
        RholangNode::VarRef { var, .. } => collect_contracts(var, contracts),
        RholangNode::List {
            elements,
            remainder,
            ..
        } => {
            for elem in elements {
                collect_contracts(elem, contracts);
            }
            if let Some(rem) = remainder {
                collect_contracts(rem, contracts);
            }
        }
        RholangNode::Set {
            elements,
            remainder,
            ..
        } => {
            for elem in elements {
                collect_contracts(elem, contracts);
            }
            if let Some(rem) = remainder {
                collect_contracts(rem, contracts);
            }
        }
        RholangNode::Map {
            pairs, remainder, ..
        } => {
            for (key, value) in pairs {
                collect_contracts(key, contracts);
                collect_contracts(value, contracts);
            }
            if let Some(rem) = remainder {
                collect_contracts(rem, contracts);
            }
        }
        RholangNode::Tuple { elements, .. } => {
            for elem in elements {
                collect_contracts(elem, contracts);
            }
        }
        RholangNode::NameDecl { var, uri, .. } => {
            collect_contracts(var, contracts);
            if let Some(u) = uri {
                collect_contracts(u, contracts);
            }
        }
        RholangNode::Decl {
            names,
            names_remainder,
            procs,
            ..
        } => {
            for name in names {
                collect_contracts(name, contracts);
            }
            if let Some(rem) = names_remainder {
                collect_contracts(rem, contracts);
            }
            for proc in procs {
                collect_contracts(proc, contracts);
            }
        }
        RholangNode::LinearBind {
            names,
            remainder,
            source,
            ..
        } => {
            for name in names {
                collect_contracts(name, contracts);
            }
            if let Some(rem) = remainder {
                collect_contracts(rem, contracts);
            }
            collect_contracts(source, contracts);
        }
        RholangNode::RepeatedBind {
            names,
            remainder,
            source,
            ..
        } => {
            for name in names {
                collect_contracts(name, contracts);
            }
            if let Some(rem) = remainder {
                collect_contracts(rem, contracts);
            }
            collect_contracts(source, contracts);
        }
        RholangNode::PeekBind {
            names,
            remainder,
            source,
            ..
        } => {
            for name in names {
                collect_contracts(name, contracts);
            }
            if let Some(rem) = remainder {
                collect_contracts(rem, contracts);
            }
            collect_contracts(source, contracts);
        }
        RholangNode::ReceiveSendSource { name, .. } => collect_contracts(name, contracts),
        RholangNode::SendReceiveSource { name, inputs, .. } => {
            collect_contracts(name, contracts);
            for input in inputs {
                collect_contracts(input, contracts);
            }
        }
        RholangNode::Error { children, .. } => {
            for child in children {
                collect_contracts(child, contracts);
            }
        }
        RholangNode::Disjunction { left, right, .. } => {
            collect_contracts(left, contracts);
            collect_contracts(right, contracts);
        }
        RholangNode::Conjunction { left, right, .. } => {
            collect_contracts(left, contracts);
            collect_contracts(right, contracts);
        }
        RholangNode::Negation { operand, .. } => collect_contracts(operand, contracts),
        RholangNode::Unit { .. } => {}
        _ => {}
    }
}

/// Collects all call nodes (Send and SendSync) from the IR tree.
pub fn collect_calls(node: &Arc<RholangNode>, calls: &mut Vec<Arc<RholangNode>>) {
    match &**node {
        RholangNode::Send { .. } | RholangNode::SendSync { .. } => calls.push(node.clone()),
        RholangNode::Par { left: Some(left), right: Some(right), .. } => {
            collect_calls(left, calls);
            collect_calls(right, calls);
        }
        RholangNode::Par { processes: Some(procs), .. } => {
            for proc in procs.iter() {
                collect_calls(proc, calls);
            }
        }
        RholangNode::New { decls, proc, .. } => {
            for decl in decls {
                collect_calls(decl, calls);
            }
            collect_calls(proc, calls);
        }
        RholangNode::IfElse {
            condition,
            consequence,
            alternative,
            ..
        } => {
            collect_calls(condition, calls);
            collect_calls(consequence, calls);
            if let Some(alt) = alternative {
                collect_calls(alt, calls);
            }
        }
        RholangNode::Let { decls, proc, .. } => {
            for decl in decls {
                collect_calls(decl, calls);
            }
            collect_calls(proc, calls);
        }
        RholangNode::Bundle { proc, .. } => collect_calls(proc, calls),
        RholangNode::Match {
            expression, cases, ..
        } => {
            collect_calls(expression, calls);
            for (pat, proc) in cases {
                collect_calls(pat, calls);
                collect_calls(proc, calls);
            }
        }
        RholangNode::Choice { branches, .. } => {
            for (inputs, proc) in branches {
                for input in inputs {
                    collect_calls(input, calls);
                }
                collect_calls(proc, calls);
            }
        }
        RholangNode::Contract {
            name,
            formals,
            formals_remainder,
            proc,
            ..
        } => {
            collect_calls(name, calls);
            for formal in formals {
                collect_calls(formal, calls);
            }
            if let Some(rem) = formals_remainder {
                collect_calls(rem, calls);
            }
            collect_calls(proc, calls);
        }
        RholangNode::Input { receipts, proc, .. } => {
            for receipt in receipts {
                for bind in receipt {
                    collect_calls(bind, calls);
                }
            }
            collect_calls(proc, calls);
        }
        RholangNode::Block { proc, .. } => collect_calls(proc, calls),
        RholangNode::Parenthesized { expr, .. } => collect_calls(expr, calls),
        RholangNode::BinOp { left, right, .. } => {
            collect_calls(left, calls);
            collect_calls(right, calls);
        }
        RholangNode::UnaryOp { operand, .. } => collect_calls(operand, calls),
        RholangNode::Method {
            receiver, args, ..
        } => {
            collect_calls(receiver, calls);
            for arg in args {
                collect_calls(arg, calls);
            }
        }
        RholangNode::Eval { name, .. } => collect_calls(name, calls),
        RholangNode::Quote { quotable, .. } => collect_calls(quotable, calls),
        RholangNode::VarRef { var, .. } => collect_calls(var, calls),
        RholangNode::List {
            elements,
            remainder,
            ..
        } => {
            for elem in elements {
                collect_calls(elem, calls);
            }
            if let Some(rem) = remainder {
                collect_calls(rem, calls);
            }
        }
        RholangNode::Set {
            elements,
            remainder,
            ..
        } => {
            for elem in elements {
                collect_calls(elem, calls);
            }
            if let Some(rem) = remainder {
                collect_calls(rem, calls);
            }
        }
        RholangNode::Map {
            pairs, remainder, ..
        } => {
            for (key, value) in pairs {
                collect_calls(key, calls);
                collect_calls(value, calls);
            }
            if let Some(rem) = remainder {
                collect_calls(rem, calls);
            }
        }
        RholangNode::Tuple { elements, .. } => {
            for elem in elements {
                collect_calls(elem, calls);
            }
        }
        RholangNode::NameDecl { var, uri, .. } => {
            collect_calls(var, calls);
            if let Some(u) = uri {
                collect_calls(u, calls);
            }
        }
        RholangNode::Decl {
            names,
            names_remainder,
            procs,
            ..
        } => {
            for name in names {
                collect_calls(name, calls);
            }
            if let Some(rem) = names_remainder {
                collect_calls(rem, calls);
            }
            for proc in procs {
                collect_calls(proc, calls);
            }
        }
        RholangNode::LinearBind {
            names,
            remainder,
            source,
            ..
        } => {
            for name in names {
                collect_calls(name, calls);
            }
            if let Some(rem) = remainder {
                collect_calls(rem, calls);
            }
            collect_calls(source, calls);
        }
        RholangNode::RepeatedBind {
            names,
            remainder,
            source,
            ..
        } => {
            for name in names {
                collect_calls(name, calls);
            }
            if let Some(rem) = remainder {
                collect_calls(rem, calls);
            }
            collect_calls(source, calls);
        }
        RholangNode::PeekBind {
            names,
            remainder,
            source,
            ..
        } => {
            for name in names {
                collect_calls(name, calls);
            }
            if let Some(rem) = remainder {
                collect_calls(rem, calls);
            }
            collect_calls(source, calls);
        }
        RholangNode::ReceiveSendSource { name, .. } => collect_calls(name, calls),
        RholangNode::SendReceiveSource { name, inputs, .. } => {
            collect_calls(name, calls);
            for input in inputs {
                collect_calls(input, calls);
            }
        }
        RholangNode::Error { children, .. } => {
            for child in children {
                collect_calls(child, calls);
            }
        }
        RholangNode::Disjunction { left, right, .. } => {
            collect_calls(left, calls);
            collect_calls(right, calls);
        }
        RholangNode::Conjunction { left, right, .. } => {
            collect_calls(left, calls);
            collect_calls(right, calls);
        }
        RholangNode::Negation { operand, .. } => collect_calls(operand, calls),
        RholangNode::Unit { .. } => {},
        _ => {},
    }
}

