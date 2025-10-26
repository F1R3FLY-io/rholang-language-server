//! MeTTa parser wrapper around MeTTaTron's TreeSitterMettaParser
//!
//! This module provides integration between MeTTaTron's SExpr representation
//! and our MettaNode IR, enabling LSP features for MeTTa files.

use std::sync::Arc;
use mettatron::TreeSitterMettaParser;
use mettatron::ir::{SExpr, Span as MettaSpan, Position as MettaPosition};

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

        let mut prev_end = Position { row: 0, column: 0, byte: 0 };
        for sexpr in sexprs {
            let node = self.convert_sexpr_to_node(&sexpr, &mut prev_end)?;
            nodes.push(node);
        }

        Ok(nodes)
    }

    /// Convert a single SExpr to MettaNode
    fn convert_sexpr_to_node(
        &self,
        expr: &SExpr,
        prev_end: &mut Position,
    ) -> Result<Arc<MettaNode>, String> {
        match expr {
            SExpr::Atom(name, span) => self.convert_atom(name, span, prev_end),
            SExpr::String(s, span) => {
                let base = self.span_to_base(span.as_ref(), s.len(), prev_end);
                Ok(Arc::new(MettaNode::String {
                    base,
                    value: s.clone(),
                    metadata: None,
                }))
            }
            SExpr::Integer(i, span) => {
                let base = self.span_to_base(span.as_ref(), i.to_string().len(), prev_end);
                Ok(Arc::new(MettaNode::Integer {
                    base,
                    value: *i,
                    metadata: None,
                }))
            }
            SExpr::Float(f, span) => {
                let base = self.span_to_base(span.as_ref(), f.to_string().len(), prev_end);
                Ok(Arc::new(MettaNode::Float {
                    base,
                    value: *f,
                    metadata: None,
                }))
            }
            SExpr::List(items, span) => self.convert_list(items, span, prev_end),
            SExpr::Quoted(inner, _span) => {
                // For now, treat quoted expressions as atoms with ' prefix
                // TODO: Consider adding a Quoted variant to MettaNode
                let inner_node = self.convert_sexpr_to_node(inner, prev_end)?;
                Ok(inner_node)
            }
        }
    }

    /// Convert an atom string to MettaNode (Atom or Variable)
    fn convert_atom(&self, name: &str, span: &Option<MettaSpan>, prev_end: &mut Position) -> Result<Arc<MettaNode>, String> {
        // NOTE: The tree-sitter parser sometimes includes trailing text in atom strings,
        // so we need to extract just the actual atom characters (up to first whitespace/delimiter).
        let actual_name = name.split_whitespace()
            .next()
            .and_then(|s| s.split(|c| c == '(' || c == ')').next())
            .unwrap_or(name);
        let actual_len = actual_name.len();

        let base = self.span_to_base(span.as_ref(), actual_len, prev_end);

        // Check if this is a variable (starts with $, &, or ')
        if actual_name.starts_with('$') {
            Ok(Arc::new(MettaNode::Variable {
                base,
                name: actual_name[1..].to_string(), // Strip prefix
                var_type: MettaVariableType::Regular,
                metadata: None,
            }))
        } else if actual_name.starts_with('&') {
            Ok(Arc::new(MettaNode::Variable {
                base,
                name: actual_name[1..].to_string(), // Strip prefix
                var_type: MettaVariableType::Grounded,
                metadata: None,
            }))
        } else if actual_name.starts_with('\'') {
            Ok(Arc::new(MettaNode::Variable {
                base,
                name: actual_name[1..].to_string(), // Strip prefix
                var_type: MettaVariableType::Quoted,
                metadata: None,
            }))
        } else if actual_name == "True" || actual_name == "true" {
            Ok(Arc::new(MettaNode::Bool {
                base,
                value: true,
                metadata: None,
            }))
        } else if actual_name == "False" || actual_name == "false" {
            Ok(Arc::new(MettaNode::Bool {
                base,
                value: false,
                metadata: None,
            }))
        } else if actual_name == "Nil" || actual_name == "()" {
            Ok(Arc::new(MettaNode::Nil {
                base,
                metadata: None,
            }))
        } else {
            Ok(Arc::new(MettaNode::Atom {
                base,
                name: actual_name.to_string(),
                metadata: None,
            }))
        }
    }

    /// Convert a list to MettaNode, detecting special forms
    fn convert_list(&self, items: &[SExpr], span: &Option<MettaSpan>, prev_end: &mut Position) -> Result<Arc<MettaNode>, String> {
        // Empty list
        if items.is_empty() {
            let base = self.span_to_base(span.as_ref(), 2, prev_end);
            return Ok(Arc::new(MettaNode::Nil {
                base,
                metadata: None,
            }));
        }

        // Check for special forms
        if let Some(SExpr::Atom(op, _)) = items.first() {
            match op.as_str() {
                "=" if items.len() == 3 => {
                    return self.convert_definition(&items[1], &items[2], span, prev_end);
                }
                ":" if items.len() == 3 => {
                    return self.convert_type_annotation(&items[1], &items[2], span, prev_end);
                }
                "!" if items.len() == 2 => {
                    return self.convert_eval(&items[1], span, prev_end);
                }
                "match" if items.len() >= 2 => {
                    return self.convert_match(&items[1..], span, prev_end);
                }
                "let" if items.len() == 3 => {
                    return self.convert_let(&items[1], &items[2], span, prev_end);
                }
                "lambda" | "λ" if items.len() == 3 => {
                    return self.convert_lambda(&items[1], &items[2], span, prev_end);
                }
                "if" if items.len() >= 3 => {
                    return self.convert_if(&items[1..], span, prev_end);
                }
                _ => {}
            }
        }

        // Default: convert as general s-expression
        // Capture the start position for children before span_to_base updates prev_end
        let child_start = if let Some(s) = span.as_ref() {
            Position {
                row: s.start.row,
                column: s.start.column,
                byte: s.start_byte,
            }
        } else {
            *prev_end
        };

        let base = self.span_to_base(span.as_ref(), span.as_ref().map(|s| s.len()).unwrap_or(0), prev_end);

        let mut elements = Vec::new();
        let mut child_prev_end = child_start; // Start children from where list starts

        for item in items.iter() {
            let node = self.convert_sexpr_to_node(item, &mut child_prev_end)?;
            elements.push(node);
        }

        let result = Arc::new(MettaNode::SExpr {
            base,
            elements,
            metadata: None,
        });

        Ok(result)
    }

    /// Convert (= pattern body) to Definition
    fn convert_definition(&self, pattern: &SExpr, body: &SExpr, span: &Option<MettaSpan>, prev_end: &mut Position) -> Result<Arc<MettaNode>, String> {
        let child_start = if let Some(s) = span.as_ref() {
            Position { row: s.start.row, column: s.start.column, byte: s.start_byte }
        } else {
            *prev_end
        };

        let base = self.span_to_base(span.as_ref(), span.as_ref().map(|s| s.len()).unwrap_or(0), prev_end);

        let mut child_prev_end = child_start;
        let pattern_node = self.convert_sexpr_to_node(pattern, &mut child_prev_end)?;
        let body_node = self.convert_sexpr_to_node(body, &mut child_prev_end)?;

        Ok(Arc::new(MettaNode::Definition {
            base,
            pattern: pattern_node,
            body: body_node,
            metadata: None,
        }))
    }

    /// Convert (: expr type) to TypeAnnotation
    fn convert_type_annotation(&self, expr: &SExpr, type_expr: &SExpr, span: &Option<MettaSpan>, prev_end: &mut Position) -> Result<Arc<MettaNode>, String> {
        let child_start = if let Some(s) = span.as_ref() {
            Position { row: s.start.row, column: s.start.column, byte: s.start_byte }
        } else {
            *prev_end
        };

        let base = self.span_to_base(span.as_ref(), span.as_ref().map(|s| s.len()).unwrap_or(0), prev_end);

        let mut child_prev_end = child_start;
        let expr_node = self.convert_sexpr_to_node(expr, &mut child_prev_end)?;
        let type_node = self.convert_sexpr_to_node(type_expr, &mut child_prev_end)?;

        Ok(Arc::new(MettaNode::TypeAnnotation {
            base,
            expr: expr_node,
            type_expr: type_node,
            metadata: None,
        }))
    }

    /// Convert (! expr) to Eval
    fn convert_eval(&self, expr: &SExpr, span: &Option<MettaSpan>, prev_end: &mut Position) -> Result<Arc<MettaNode>, String> {
        let child_start = if let Some(s) = span.as_ref() {
            Position { row: s.start.row, column: s.start.column, byte: s.start_byte }
        } else {
            *prev_end
        };

        let base = self.span_to_base(span.as_ref(), span.as_ref().map(|s| s.len()).unwrap_or(0), prev_end);

        let mut child_prev_end = child_start;
        let expr_node = self.convert_sexpr_to_node(expr, &mut child_prev_end)?;

        Ok(Arc::new(MettaNode::Eval {
            base,
            expr: expr_node,
            metadata: None,
        }))
    }

    /// Convert (match scrutinee (case1 result1) (case2 result2) ...) to Match
    fn convert_match(&self, items: &[SExpr], span: &Option<MettaSpan>, prev_end: &mut Position) -> Result<Arc<MettaNode>, String> {
        if items.is_empty() {
            return Err("match requires at least a scrutinee".to_string());
        }

        let child_start = if let Some(s) = span.as_ref() {
            Position { row: s.start.row, column: s.start.column, byte: s.start_byte }
        } else {
            *prev_end
        };

        let base = self.span_to_base(span.as_ref(), span.as_ref().map(|s| s.len()).unwrap_or(0), prev_end);

        // Check if this is a grounded query: (match & self <pattern> <return>)
        // When parsed, `& self` becomes two separate atoms, so we have 4 items total
        let is_grounded_query = items.len() == 4
            && matches!(&items[0], SExpr::Atom(s, _) if s == "&");

        if is_grounded_query {
            // Grounded query form: (match & <space> <pattern> <return-value>)
            // items[0] = & (grounded operator)
            // items[1] = space reference (e.g., "self")
            // items[2] = pattern
            // items[3] = return value

            let mut child_prev_end = child_start;

            // Build scrutinee as (& space)
            let op = self.convert_sexpr_to_node(&items[0], &mut child_prev_end)?;
            let space = self.convert_sexpr_to_node(&items[1], &mut child_prev_end)?;

            // Compute scrutinee's span from child_start to child_prev_end
            let scrutinee_span_lines = child_prev_end.row - child_start.row;
            let scrutinee_span_columns = if scrutinee_span_lines == 0 {
                child_prev_end.column - child_start.column
            } else {
                child_prev_end.column
            };
            let scrutinee_length = child_prev_end.byte - child_start.byte;

            let scrutinee = Arc::new(MettaNode::SExpr {
                base: NodeBase::new(
                    RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 },
                    scrutinee_length,
                    scrutinee_span_lines,
                    scrutinee_span_columns,
                ),
                elements: vec![op, space],
                metadata: None,
            });

            let pattern = self.convert_sexpr_to_node(&items[2], &mut child_prev_end)?;

            let return_val = self.convert_sexpr_to_node(&items[3], &mut child_prev_end)?;

            // Represent as a single-case match
            Ok(Arc::new(MettaNode::Match {
                base,
                scrutinee,
                cases: vec![(pattern, return_val)],
                metadata: None,
            }))
        } else {
            // Standard case-based match
            let mut child_prev_end = child_start;
            let scrutinee = self.convert_sexpr_to_node(&items[0], &mut child_prev_end)?;

            // Standard case-based match: (match <scrutinee> (<pattern> <result>) ...)
            let mut cases = Vec::new();
            for case_expr in &items[1..] {
                if let SExpr::List(case_items, _) = case_expr {
                    if case_items.len() == 2 {
                        let pattern = self.convert_sexpr_to_node(&case_items[0], &mut child_prev_end)?;
                        let result = self.convert_sexpr_to_node(&case_items[1], &mut child_prev_end)?;
                        cases.push((pattern, result));
                    } else {
                        return Err(format!("match case must have 2 elements, got {}", case_items.len()));
                    }
                } else {
                    return Err("match case must be a list".to_string());
                }
            }

            Ok(Arc::new(MettaNode::Match {
                base,
                scrutinee,
                cases,
                metadata: None,
            }))
        }
    }

    /// Convert (let ((var1 val1) (var2 val2)) body) to Let
    fn convert_let(&self, bindings_expr: &SExpr, body: &SExpr, span: &Option<MettaSpan>, prev_end: &mut Position) -> Result<Arc<MettaNode>, String> {
        let child_start = if let Some(s) = span.as_ref() {
            Position { row: s.start.row, column: s.start.column, byte: s.start_byte }
        } else {
            *prev_end
        };

        let base = self.span_to_base(span.as_ref(), span.as_ref().map(|s| s.len()).unwrap_or(0), prev_end);

        let mut child_prev_end = child_start;
        let mut bindings = Vec::new();
        if let SExpr::List(binding_list, _) = bindings_expr {
            for binding in binding_list {
                if let SExpr::List(pair, _) = binding {
                    if pair.len() == 2 {
                        let var = self.convert_sexpr_to_node(&pair[0], &mut child_prev_end)?;
                        let val = self.convert_sexpr_to_node(&pair[1], &mut child_prev_end)?;
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

        let body_node = self.convert_sexpr_to_node(body, &mut child_prev_end)?;

        Ok(Arc::new(MettaNode::Let {
            base,
            bindings,
            body: body_node,
            metadata: None,
        }))
    }

    /// Convert (lambda (params) body) to Lambda
    fn convert_lambda(&self, params_expr: &SExpr, body: &SExpr, span: &Option<MettaSpan>, prev_end: &mut Position) -> Result<Arc<MettaNode>, String> {
        let child_start = if let Some(s) = span.as_ref() {
            Position { row: s.start.row, column: s.start.column, byte: s.start_byte }
        } else {
            *prev_end
        };

        let base = self.span_to_base(span.as_ref(), span.as_ref().map(|s| s.len()).unwrap_or(0), prev_end);

        let mut child_prev_end = child_start;
        let mut params = Vec::new();
        if let SExpr::List(param_list, _) = params_expr {
            for param in param_list {
                params.push(self.convert_sexpr_to_node(param, &mut child_prev_end)?);
            }
        } else {
            return Err("lambda parameters must be a list".to_string());
        }

        let body_node = self.convert_sexpr_to_node(body, &mut child_prev_end)?;

        Ok(Arc::new(MettaNode::Lambda {
            base,
            params,
            body: body_node,
            metadata: None,
        }))
    }

    /// Convert (if cond then [else]) to If
    fn convert_if(&self, items: &[SExpr], span: &Option<MettaSpan>, prev_end: &mut Position) -> Result<Arc<MettaNode>, String> {
        if items.len() < 2 {
            return Err("if requires at least condition and consequence".to_string());
        }

        let child_start = if let Some(s) = span.as_ref() {
            Position { row: s.start.row, column: s.start.column, byte: s.start_byte }
        } else {
            *prev_end
        };

        let base = self.span_to_base(span.as_ref(), span.as_ref().map(|s| s.len()).unwrap_or(0), prev_end);

        let mut child_prev_end = child_start;
        let condition = self.convert_sexpr_to_node(&items[0], &mut child_prev_end)?;
        let consequence = self.convert_sexpr_to_node(&items[1], &mut child_prev_end)?;
        let alternative = if items.len() > 2 {
            Some(self.convert_sexpr_to_node(&items[2], &mut child_prev_end)?)
        } else {
            None
        };

        Ok(Arc::new(MettaNode::If {
            base,
            condition,
            consequence,
            alternative,
            metadata: None,
        }))
    }

    /// Convert MeTTaTron's absolute Span to our RelativePosition-based NodeBase
    fn span_to_base(&self, span: Option<&MettaSpan>, length: usize, prev_end: &mut Position) -> NodeBase {
        if let Some(s) = span {
            // MeTTaTron's Span has absolute positions
            let start = Position {
                row: s.start.row,
                column: s.start.column,
                byte: s.start_byte,
            };
            let end = Position {
                row: s.end.row,
                column: s.end.column,
                byte: s.end_byte,
            };

            // Calculate relative position from previous end
            let delta_lines = (start.row as i32) - (prev_end.row as i32);
            let delta_columns = if delta_lines == 0 {
                (start.column as i32) - (prev_end.column as i32)
            } else {
                start.column as i32
            };
            let delta_bytes = start.byte - prev_end.byte;

            let relative_start = RelativePosition {
                delta_lines,
                delta_columns,
                delta_bytes,
            };

            // Calculate span metrics
            let span_lines = end.row - start.row;
            let span_columns = if span_lines > 0 {
                end.column
            } else {
                end.column - start.column
            };

            // Update prev_end for next sibling
            *prev_end = end;

            NodeBase::new(relative_start, length, span_lines, span_columns)
        } else {
            // No span information - create a minimal base
            NodeBase::new(
                RelativePosition { delta_lines: 0, delta_columns: 0, delta_bytes: 0 },
                length,
                0,
                length,
            )
        }
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
        assert!(matches!(&result[0], SExpr::Atom(s, _) if s == "foo"));
    }

    #[test]
    fn test_parse_integer() {
        let mut parser = MettaParser::new().unwrap();
        let result = parser.parse("42").unwrap();
        assert_eq!(result.len(), 1);
        assert!(matches!(&result[0], SExpr::Integer(42, _)));
    }

    #[test]
    fn test_parse_list() {
        let mut parser = MettaParser::new().unwrap();
        let result = parser.parse("(+ 1 2)").unwrap();
        assert_eq!(result.len(), 1);
        assert!(matches!(&result[0], SExpr::List(_, _)));
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
