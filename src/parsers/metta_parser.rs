//! MeTTa parser wrapper around MeTTaTron's TreeSitterMettaParser
//!
//! This module provides integration between MeTTaTron's SExpr representation
//! and our MettaNode IR, enabling LSP features for MeTTa files.

use std::sync::Arc;
use mettatron::TreeSitterMettaParser;
use mettatron::ir::SExpr;

use crate::ir::metta_node::{MettaNode, MettaVariableType};
use crate::ir::semantic_node::{NodeBase, Position};
use crate::ir::rholang_node::RelativePosition;

/// Wrapper around MeTTaTron's Tree-Sitter parser
pub struct MettaParser {
    parser: TreeSitterMettaParser,
}

impl MettaParser {
    /// Create a new MeTTa parser
    pub fn new() -> Result<Self, String> {
        let parser = TreeSitterMettaParser::new()?;
        Ok(Self { parser })
    }

    /// Parse MeTTa source code into SExpr AST
    pub fn parse(&mut self, source: &str) -> Result<Vec<SExpr>, String> {
        self.parser.parse(source)
    }

    /// Parse MeTTa source code into MettaNode IR
    pub fn parse_to_ir(&mut self, source: &str) -> Result<Vec<Arc<MettaNode>>, String> {
        let sexprs = self.parse(source)?;
        let mut nodes = Vec::new();

        for sexpr in sexprs {
            let node = self.convert_sexpr_to_node(&sexpr, &mut PositionTracker::new())?;
            nodes.push(node);
        }

        Ok(nodes)
    }

    /// Convert a single SExpr to MettaNode
    fn convert_sexpr_to_node(
        &self,
        expr: &SExpr,
        tracker: &mut PositionTracker,
    ) -> Result<Arc<MettaNode>, String> {
        match expr {
            SExpr::Atom(name) => self.convert_atom(name, tracker),
            SExpr::String(s) => Ok(Arc::new(MettaNode::String {
                base: tracker.next_base(s.len()),
                value: s.clone(),
                metadata: None,
            })),
            SExpr::Integer(i) => Ok(Arc::new(MettaNode::Integer {
                base: tracker.next_base(i.to_string().len()),
                value: *i,
                metadata: None,
            })),
            SExpr::Float(f) => Ok(Arc::new(MettaNode::Float {
                base: tracker.next_base(f.to_string().len()),
                value: *f,
                metadata: None,
            })),
            SExpr::List(items) => self.convert_list(items, tracker),
            SExpr::Quoted(inner) => {
                // For now, treat quoted expressions as atoms with ' prefix
                // TODO: Consider adding a Quoted variant to MettaNode
                let inner_node = self.convert_sexpr_to_node(inner, tracker)?;
                Ok(inner_node)
            }
        }
    }

    /// Convert an atom string to MettaNode (Atom or Variable)
    fn convert_atom(&self, name: &str, tracker: &mut PositionTracker) -> Result<Arc<MettaNode>, String> {
        // Check if this is a variable (starts with $, &, or ')
        if name.starts_with('$') {
            Ok(Arc::new(MettaNode::Variable {
                base: tracker.next_base(name.len()),
                name: name[1..].to_string(), // Strip prefix
                var_type: MettaVariableType::Regular,
                metadata: None,
            }))
        } else if name.starts_with('&') {
            Ok(Arc::new(MettaNode::Variable {
                base: tracker.next_base(name.len()),
                name: name[1..].to_string(), // Strip prefix
                var_type: MettaVariableType::Grounded,
                metadata: None,
            }))
        } else if name.starts_with('\'') {
            Ok(Arc::new(MettaNode::Variable {
                base: tracker.next_base(name.len()),
                name: name[1..].to_string(), // Strip prefix
                var_type: MettaVariableType::Quoted,
                metadata: None,
            }))
        } else if name == "True" || name == "true" {
            Ok(Arc::new(MettaNode::Bool {
                base: tracker.next_base(name.len()),
                value: true,
                metadata: None,
            }))
        } else if name == "False" || name == "false" {
            Ok(Arc::new(MettaNode::Bool {
                base: tracker.next_base(name.len()),
                value: false,
                metadata: None,
            }))
        } else if name == "Nil" || name == "()" {
            Ok(Arc::new(MettaNode::Nil {
                base: tracker.next_base(name.len()),
                metadata: None,
            }))
        } else {
            Ok(Arc::new(MettaNode::Atom {
                base: tracker.next_base(name.len()),
                name: name.to_string(),
                metadata: None,
            }))
        }
    }

    /// Convert a list to MettaNode, detecting special forms
    fn convert_list(&self, items: &[SExpr], tracker: &mut PositionTracker) -> Result<Arc<MettaNode>, String> {
        // Empty list
        if items.is_empty() {
            return Ok(Arc::new(MettaNode::Nil {
                base: tracker.next_base(2), // "()"
                metadata: None,
            }));
        }

        // Check for special forms
        if let Some(SExpr::Atom(op)) = items.first() {
            match op.as_str() {
                "=" if items.len() == 3 => {
                    return self.convert_definition(&items[1], &items[2], tracker);
                }
                ":" if items.len() == 3 => {
                    return self.convert_type_annotation(&items[1], &items[2], tracker);
                }
                "!" if items.len() == 2 => {
                    return self.convert_eval(&items[1], tracker);
                }
                "match" if items.len() >= 2 => {
                    return self.convert_match(&items[1..], tracker);
                }
                "let" if items.len() == 3 => {
                    return self.convert_let(&items[1], &items[2], tracker);
                }
                "lambda" | "λ" if items.len() == 3 => {
                    return self.convert_lambda(&items[1], &items[2], tracker);
                }
                "if" if items.len() >= 3 => {
                    return self.convert_if(&items[1..], tracker);
                }
                _ => {}
            }
        }

        // Default: convert as general s-expression
        let start_pos = tracker.current_position();
        tracker.advance(1); // '('

        let mut elements = Vec::new();
        for item in items {
            elements.push(self.convert_sexpr_to_node(item, tracker)?);
        }

        tracker.advance(1); // ')'

        Ok(Arc::new(MettaNode::SExpr {
            base: NodeBase::new(
                start_pos,
                tracker.current_byte() - start_pos.delta_bytes as usize,
                0,
                tracker.current_byte(),
            ),
            elements,
            metadata: None,
        }))
    }

    /// Convert (= pattern body) to Definition
    fn convert_definition(&self, pattern: &SExpr, body: &SExpr, tracker: &mut PositionTracker) -> Result<Arc<MettaNode>, String> {
        let start_pos = tracker.current_position();
        tracker.advance(1); // '('
        tracker.advance(1); // '='

        let pattern_node = self.convert_sexpr_to_node(pattern, tracker)?;
        let body_node = self.convert_sexpr_to_node(body, tracker)?;

        tracker.advance(1); // ')'

        Ok(Arc::new(MettaNode::Definition {
            base: NodeBase::new(
                start_pos,
                tracker.current_byte() - start_pos.delta_bytes as usize,
                0,
                tracker.current_byte(),
            ),
            pattern: pattern_node,
            body: body_node,
            metadata: None,
        }))
    }

    /// Convert (: expr type) to TypeAnnotation
    fn convert_type_annotation(&self, expr: &SExpr, type_expr: &SExpr, tracker: &mut PositionTracker) -> Result<Arc<MettaNode>, String> {
        let start_pos = tracker.current_position();
        tracker.advance(1); // '('
        tracker.advance(1); // ':'

        let expr_node = self.convert_sexpr_to_node(expr, tracker)?;
        let type_node = self.convert_sexpr_to_node(type_expr, tracker)?;

        tracker.advance(1); // ')'

        Ok(Arc::new(MettaNode::TypeAnnotation {
            base: NodeBase::new(
                start_pos,
                tracker.current_byte() - start_pos.delta_bytes as usize,
                0,
                tracker.current_byte(),
            ),
            expr: expr_node,
            type_expr: type_node,
            metadata: None,
        }))
    }

    /// Convert (! expr) to Eval
    fn convert_eval(&self, expr: &SExpr, tracker: &mut PositionTracker) -> Result<Arc<MettaNode>, String> {
        let start_pos = tracker.current_position();
        tracker.advance(1); // '('
        tracker.advance(1); // '!'

        let expr_node = self.convert_sexpr_to_node(expr, tracker)?;

        tracker.advance(1); // ')'

        Ok(Arc::new(MettaNode::Eval {
            base: NodeBase::new(
                start_pos,
                tracker.current_byte() - start_pos.delta_bytes as usize,
                0,
                tracker.current_byte(),
            ),
            expr: expr_node,
            metadata: None,
        }))
    }

    /// Convert (match scrutinee (case1 result1) (case2 result2) ...) to Match
    fn convert_match(&self, items: &[SExpr], tracker: &mut PositionTracker) -> Result<Arc<MettaNode>, String> {
        if items.is_empty() {
            return Err("match requires at least a scrutinee".to_string());
        }

        let start_pos = tracker.current_position();
        tracker.advance(1); // '('
        tracker.advance(5); // 'match'

        let scrutinee = self.convert_sexpr_to_node(&items[0], tracker)?;

        let mut cases = Vec::new();
        for case_expr in &items[1..] {
            if let SExpr::List(case_items) = case_expr {
                if case_items.len() == 2 {
                    let pattern = self.convert_sexpr_to_node(&case_items[0], tracker)?;
                    let result = self.convert_sexpr_to_node(&case_items[1], tracker)?;
                    cases.push((pattern, result));
                } else {
                    return Err(format!("match case must have 2 elements, got {}", case_items.len()));
                }
            } else {
                return Err("match case must be a list".to_string());
            }
        }

        tracker.advance(1); // ')'

        Ok(Arc::new(MettaNode::Match {
            base: NodeBase::new(
                start_pos,
                tracker.current_byte() - start_pos.delta_bytes as usize,
                0,
                tracker.current_byte(),
            ),
            scrutinee,
            cases,
            metadata: None,
        }))
    }

    /// Convert (let ((var1 val1) (var2 val2)) body) to Let
    fn convert_let(&self, bindings_expr: &SExpr, body: &SExpr, tracker: &mut PositionTracker) -> Result<Arc<MettaNode>, String> {
        let start_pos = tracker.current_position();
        tracker.advance(1); // '('
        tracker.advance(3); // 'let'

        let mut bindings = Vec::new();
        if let SExpr::List(binding_list) = bindings_expr {
            for binding in binding_list {
                if let SExpr::List(pair) = binding {
                    if pair.len() == 2 {
                        let var = self.convert_sexpr_to_node(&pair[0], tracker)?;
                        let val = self.convert_sexpr_to_node(&pair[1], tracker)?;
                        bindings.push((var, val));
                    } else {
                        return Err(format!("let binding must have 2 elements, got {}", pair.len()));
                    }
                } else {
                    return Err("let binding must be a list".to_string());
                }
            }
        } else {
            return Err("let bindings must be a list".to_string());
        }

        let body_node = self.convert_sexpr_to_node(body, tracker)?;

        tracker.advance(1); // ')'

        Ok(Arc::new(MettaNode::Let {
            base: NodeBase::new(
                start_pos,
                tracker.current_byte() - start_pos.delta_bytes as usize,
                0,
                tracker.current_byte(),
            ),
            bindings,
            body: body_node,
            metadata: None,
        }))
    }

    /// Convert (lambda (params) body) to Lambda
    fn convert_lambda(&self, params_expr: &SExpr, body: &SExpr, tracker: &mut PositionTracker) -> Result<Arc<MettaNode>, String> {
        let start_pos = tracker.current_position();
        tracker.advance(1); // '('
        tracker.advance(6); // 'lambda' or 'λ'

        let mut params = Vec::new();
        if let SExpr::List(param_list) = params_expr {
            for param in param_list {
                params.push(self.convert_sexpr_to_node(param, tracker)?);
            }
        } else {
            return Err("lambda parameters must be a list".to_string());
        }

        let body_node = self.convert_sexpr_to_node(body, tracker)?;

        tracker.advance(1); // ')'

        Ok(Arc::new(MettaNode::Lambda {
            base: NodeBase::new(
                start_pos,
                tracker.current_byte() - start_pos.delta_bytes as usize,
                0,
                tracker.current_byte(),
            ),
            params,
            body: body_node,
            metadata: None,
        }))
    }

    /// Convert (if cond then [else]) to If
    fn convert_if(&self, items: &[SExpr], tracker: &mut PositionTracker) -> Result<Arc<MettaNode>, String> {
        if items.len() < 2 {
            return Err("if requires at least condition and consequence".to_string());
        }

        let start_pos = tracker.current_position();
        tracker.advance(1); // '('
        tracker.advance(2); // 'if'

        let condition = self.convert_sexpr_to_node(&items[0], tracker)?;
        let consequence = self.convert_sexpr_to_node(&items[1], tracker)?;
        let alternative = if items.len() > 2 {
            Some(self.convert_sexpr_to_node(&items[2], tracker)?)
        } else {
            None
        };

        tracker.advance(1); // ')'

        Ok(Arc::new(MettaNode::If {
            base: NodeBase::new(
                start_pos,
                tracker.current_byte() - start_pos.delta_bytes as usize,
                0,
                tracker.current_byte(),
            ),
            condition,
            consequence,
            alternative,
            metadata: None,
        }))
    }
}

/// Position tracker for building NodeBase during conversion
struct PositionTracker {
    line: usize,
    column: usize,
    byte: usize,
}

impl PositionTracker {
    fn new() -> Self {
        Self {
            line: 0,
            column: 0,
            byte: 0,
        }
    }

    fn current_position(&self) -> RelativePosition {
        RelativePosition {
            delta_lines: self.line as i32,
            delta_columns: self.column as i32,
            delta_bytes: self.byte,
        }
    }

    fn current_byte(&self) -> usize {
        self.byte
    }

    fn next_base(&mut self, len: usize) -> NodeBase {
        let pos = self.current_position();
        self.advance(len);
        NodeBase::new(pos, len, 0, self.byte)
    }

    fn advance(&mut self, len: usize) {
        self.byte += len;
        self.column += len;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_atom() {
        let mut parser = MettaParser::new().unwrap();
        let result = parser.parse("foo").unwrap();
        assert_eq!(result.len(), 1);
        assert!(matches!(&result[0], SExpr::Atom(s) if s == "foo"));
    }

    #[test]
    fn test_parse_integer() {
        let mut parser = MettaParser::new().unwrap();
        let result = parser.parse("42").unwrap();
        assert_eq!(result.len(), 1);
        assert!(matches!(&result[0], SExpr::Integer(42)));
    }

    #[test]
    fn test_parse_list() {
        let mut parser = MettaParser::new().unwrap();
        let result = parser.parse("(+ 1 2)").unwrap();
        assert_eq!(result.len(), 1);
        assert!(matches!(&result[0], SExpr::List(_)));
    }

    #[test]
    fn test_parse_to_ir_atom() {
        let mut parser = MettaParser::new().unwrap();
        let nodes = parser.parse_to_ir("foo").unwrap();
        assert_eq!(nodes.len(), 1);
        assert!(matches!(&*nodes[0], MettaNode::Atom { name, .. } if name == "foo"));
    }

    #[test]
    fn test_parse_to_ir_variable() {
        let mut parser = MettaParser::new().unwrap();
        let nodes = parser.parse_to_ir("$x").unwrap();
        assert_eq!(nodes.len(), 1);
        assert!(matches!(
            &*nodes[0],
            MettaNode::Variable { name, var_type, .. }
            if name == "x" && *var_type == MettaVariableType::Regular
        ));
    }

    #[test]
    fn test_parse_to_ir_integer() {
        let mut parser = MettaParser::new().unwrap();
        let nodes = parser.parse_to_ir("42").unwrap();
        assert_eq!(nodes.len(), 1);
        assert!(matches!(&*nodes[0], MettaNode::Integer { value: 42, .. }));
    }

    #[test]
    fn test_parse_to_ir_definition() {
        let mut parser = MettaParser::new().unwrap();
        let nodes = parser.parse_to_ir("(= foo bar)").unwrap();
        assert_eq!(nodes.len(), 1);
        assert!(matches!(&*nodes[0], MettaNode::Definition { .. }));
    }

    #[test]
    fn test_parse_to_ir_eval() {
        let mut parser = MettaParser::new().unwrap();
        let nodes = parser.parse_to_ir("!(foo)").unwrap();
        assert_eq!(nodes.len(), 1);
        println!("Parsed node: {:?}", nodes[0]);
        assert!(matches!(&*nodes[0], MettaNode::Eval { .. }));
    }

    #[test]
    fn test_parse_to_ir_if() {
        let mut parser = MettaParser::new().unwrap();
        let nodes = parser.parse_to_ir("(if true 1 2)").unwrap();
        assert_eq!(nodes.len(), 1);
        assert!(matches!(&*nodes[0], MettaNode::If { .. }));
    }
}
