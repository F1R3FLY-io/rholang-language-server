//! Conversion utilities between RholangNode and MORK Expr format
//!
//! This module handles the bidirectional conversion needed for query_multi integration:
//! - RholangNode → MORK Expr (for pattern queries)
//! - MORK bindings → HashMap<String, RholangNode> (for pattern match results)
//!
//! Based on MeTTaTron's implementation in `src/backend/mork_convert.rs`

use crate::ir::rholang_node::RholangNode;
use mork::space::Space;
use mork_expr::{Expr, ExprEnv, ExprZipper};
use mork_frontend::bytestring_parser::Parser;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use archery::ArcK;
use rpds::Vector;

/// Context for tracking variables during RholangNode → Expr conversion
///
/// Uses De Bruijn indices for consistent variable encoding across pattern matching.
/// First occurrence of a variable writes NewVar tag, subsequent uses write VarRef(index).
#[derive(Default, Debug)]
pub struct ConversionContext {
    /// Maps variable names to their De Bruijn indices
    pub var_map: HashMap<String, u8>,
    /// Reverse map: De Bruijn index → variable name
    pub var_names: Vec<String>,
}

impl ConversionContext {
    pub fn new() -> Self {
        ConversionContext {
            var_map: HashMap::new(),
            var_names: Vec::new(),
        }
    }

    /// Get or create a De Bruijn index for a variable
    ///
    /// Returns:
    /// - Ok(None): First occurrence - caller should write NewVar
    /// - Ok(Some(idx)): Subsequent occurrence - caller should write VarRef(idx)
    /// - Err(msg): Too many variables (max 64)
    pub fn get_or_create_var(&mut self, name: &str) -> Result<Option<u8>, String> {
        if let Some(&idx) = self.var_map.get(name) {
            // Variable already exists, return its index
            Ok(Some(idx))
        } else {
            // New variable
            if self.var_names.len() >= 64 {
                return Err("Too many variables (max 64)".to_string());
            }
            let idx = self.var_names.len() as u8;
            self.var_map.insert(name.to_string(), idx);
            self.var_names.push(name.to_string());
            Ok(None) // None means "write NewVar tag"
        }
    }
}

/// Convert RholangNode to MORK Expr bytes
///
/// This creates a MORK process expression that can be used with query_multi.
/// Variables are converted to De Bruijn indices.
///
/// # Example
/// ```ignore
/// let space = Space::new();
/// let mut ctx = ConversionContext::new();
/// let var_node = Arc::new(RholangNode::Var { name: "x".to_string(), .. });
/// let bytes = rholang_to_mork_bytes(&var_node, &space, &mut ctx)?;
/// ```
pub fn rholang_to_mork_bytes(
    node: &Arc<RholangNode>,
    space: &Space,
    ctx: &mut ConversionContext,
) -> Result<Vec<u8>, String> {
    let mut buffer = vec![0u8; 4096];
    let expr = Expr {
        ptr: buffer.as_mut_ptr(),
    };
    let mut ez = ExprZipper::new(expr);

    write_rholang_node(node, space, ctx, &mut ez)?;

    Ok(buffer[..ez.loc].to_vec())
}

/// Recursively write RholangNode to ExprZipper
///
/// This is the core conversion logic that encodes Rholang processes as s-expressions.
fn write_rholang_node(
    node: &Arc<RholangNode>,
    space: &Space,
    ctx: &mut ConversionContext,
    ez: &mut ExprZipper,
) -> Result<(), String> {
    match &**node {
        // Variables: Use De Bruijn encoding
        RholangNode::Var { name, .. } => {
            match ctx.get_or_create_var(name)? {
                None => {
                    // First occurrence - write NewVar
                    ez.write_new_var();
                    ez.loc += 1;
                }
                Some(idx) => {
                    // Subsequent occurrence - write VarRef
                    ez.write_var_ref(idx);
                    ez.loc += 1;
                }
            }
        }

        // Nil: Special constant
        RholangNode::Nil { .. } => {
            write_symbol(b"Nil", space, ez)?;
        }

        // Ground values
        RholangNode::LongLiteral { value, .. } => {
            let s = value.to_string();
            write_symbol(s.as_bytes(), space, ez)?;
        }

        RholangNode::BoolLiteral { value, .. } => {
            let s = if *value { "true" } else { "false" };
            write_symbol(s.as_bytes(), space, ez)?;
        }

        RholangNode::StringLiteral { value, .. } => {
            // MORK uses quoted strings
            let quoted = format!("\"{}\"", value);
            write_symbol(quoted.as_bytes(), space, ez)?;
        }

        // Send: (send <channel> <inputs...>)
        RholangNode::Send { channel, inputs, .. } => {
            let arity = 2 + inputs.len() as u8;
            ez.write_arity(arity);
            ez.loc += 1;

            write_symbol(b"send", space, ez)?;
            write_rholang_node(channel, space, ctx, ez)?;
            for input in inputs.iter() {
                write_rholang_node(input, space, ctx, ez)?;
            }
        }

        // Contract: (contract <name> <formals...> <body>)
        RholangNode::Contract { name, formals, proc, .. } => {
            let arity = 3 + formals.len() as u8;
            ez.write_arity(arity);
            ez.loc += 1;

            write_symbol(b"contract", space, ez)?;
            write_rholang_node(name, space, ctx, ez)?;
            for formal in formals.iter() {
                write_rholang_node(formal, space, ctx, ez)?;
            }
            write_rholang_node(proc, space, ctx, ez)?;
        }

        // New: (new <decls...> <body>)
        RholangNode::New { decls, proc, .. } => {
            let arity = 2 + decls.len() as u8;
            ez.write_arity(arity);
            ez.loc += 1;

            write_symbol(b"new", space, ez)?;
            for decl in decls.iter() {
                write_rholang_node(decl, space, ctx, ez)?;
            }
            write_rholang_node(proc, space, ctx, ez)?;
        }

        // Par: (par <left> <right>)
        RholangNode::Par { left: Some(left), right: Some(right), .. } => {
            ez.write_arity(3);
            ez.loc += 1;

            write_symbol(b"par", space, ez)?;
            write_rholang_node(left, space, ctx, ez)?;
            write_rholang_node(right, space, ctx, ez)?;
        }

        // NameDecl: Used in New bindings
        RholangNode::NameDecl { var, .. } => {
            // For name declarations, just process the var
            write_rholang_node(var, space, ctx, ez)?;
        }

        // TODO: Add remaining node types as needed
        // - SendSync, Input, Match, Let, etc.
        // See MORK_INTEGRATION_GUIDE.md for complete examples
        _ => {
            return Err(format!(
                "Unsupported node type for MORK conversion: {:?}",
                std::mem::discriminant(&**node)
            ));
        }
    }

    Ok(())
}

/// Write a symbol to ExprZipper using Space's symbol table
///
/// This handles symbol interning via MORK's ParDataParser.
fn write_symbol(bytes: &[u8], space: &Space, ez: &mut ExprZipper) -> Result<(), String> {
    // Use MORK's ParDataParser to intern the symbol
    let mut pdp = mork::space::ParDataParser::new(&space.sm);
    let token = pdp.tokenizer(bytes);

    ez.write_symbol(token);
    ez.loc += 1 + token.len();

    Ok(())
}

/// Public wrapper for write_symbol
pub fn write_symbol_external(bytes: &[u8], space: &Space, ez: &mut ExprZipper) -> Result<(), String> {
    write_symbol(bytes, space, ez)
}

/// Convert RholangNode to MORK string representation (text s-expression)
///
/// This creates a text representation that can be parsed by MORK's ParDataParser.
/// Similar to MeTTaTron's `to_mork_string()`.
pub fn rholang_to_mork_string(node: &Arc<RholangNode>) -> String {
    match &**node {
        RholangNode::Nil { .. } => "nil".to_string(),
        RholangNode::Var { name, .. } => {
            // Variables become $ prefixed in MORK
            format!("${}", name)
        }
        RholangNode::LongLiteral { value, .. } => value.to_string(),
        RholangNode::BoolLiteral { value, .. } => value.to_string(),
        RholangNode::StringLiteral { value, .. } => format!("\"{}\"", value),
        RholangNode::Send { channel, inputs, .. } => {
            let channel_str = rholang_to_mork_string(channel);
            let inputs_str = inputs
                .iter()
                .map(|i| rholang_to_mork_string(i))
                .collect::<Vec<_>>()
                .join(" ");
            format!("(send {} {})", channel_str, inputs_str)
        }
        RholangNode::Contract { name, formals, proc, .. } => {
            let name_str = rholang_to_mork_string(name);
            let formals_str = formals
                .iter()
                .map(|f| rholang_to_mork_string(f))
                .collect::<Vec<_>>()
                .join(" ");
            let proc_str = rholang_to_mork_string(proc);
            format!("(contract {} {} {})", name_str, formals_str, proc_str)
        }
        RholangNode::New { decls, proc, .. } => {
            let decls_str = decls
                .iter()
                .map(|d| rholang_to_mork_string(d))
                .collect::<Vec<_>>()
                .join(" ");
            let proc_str = rholang_to_mork_string(proc);
            format!("(new {} {})", decls_str, proc_str)
        }
        RholangNode::Par { left: Some(left), right: Some(right), .. } => {
            let left_str = rholang_to_mork_string(left);
            let right_str = rholang_to_mork_string(right);
            format!("(par {} {})", left_str, right_str)
        }
        RholangNode::NameDecl { var, .. } => rholang_to_mork_string(var),
        _ => "(unsupported)".to_string(),
    }
}

/// Convert MORK bindings to HashMap<String, Arc<RholangNode>>
///
/// MORK uses BTreeMap<(u8, u8), ExprEnv> where the key is (old_var, new_var).
/// We need to convert this to HashMap<String, RholangNode> using the original variable names.
pub fn mork_bindings_to_rholang(
    _mork_bindings: &BTreeMap<(u8, u8), ExprEnv>,
    _ctx: &ConversionContext,
    _space: &Space,
) -> Result<HashMap<String, Arc<RholangNode>>, String> {
    // TODO: Implement based on MeTTaTron's mork_bindings_to_metta()
    // See MORK_INTEGRATION_GUIDE.md lines 184-214
    Err("Not yet implemented".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::rholang_node::{NodeBase, RelativePosition};

    fn create_base() -> NodeBase {
        NodeBase::new(
            RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            0,
            0,
            1,
        )
    }

    #[test]
    fn test_conversion_context_new_variable() {
        let mut ctx = ConversionContext::new();

        // First occurrence should return None (write NewVar)
        let result = ctx.get_or_create_var("x");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), None);

        // Context should now have the variable
        assert_eq!(ctx.var_names.len(), 1);
        assert_eq!(ctx.var_names[0], "x");
        assert_eq!(ctx.var_map.get("x"), Some(&0));
    }

    #[test]
    fn test_conversion_context_existing_variable() {
        let mut ctx = ConversionContext::new();

        // First occurrence
        ctx.get_or_create_var("x").unwrap();

        // Second occurrence should return Some(0) (write VarRef(0))
        let result = ctx.get_or_create_var("x");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Some(0));

        // Context should still have only one variable
        assert_eq!(ctx.var_names.len(), 1);
    }

    #[test]
    fn test_conversion_context_multiple_variables() {
        let mut ctx = ConversionContext::new();

        // Add multiple variables
        assert_eq!(ctx.get_or_create_var("x").unwrap(), None); // First x → NewVar
        assert_eq!(ctx.get_or_create_var("y").unwrap(), None); // First y → NewVar
        assert_eq!(ctx.get_or_create_var("x").unwrap(), Some(0)); // Second x → VarRef(0)
        assert_eq!(ctx.get_or_create_var("y").unwrap(), Some(1)); // Second y → VarRef(1)

        assert_eq!(ctx.var_names.len(), 2);
        assert_eq!(ctx.var_names[0], "x");
        assert_eq!(ctx.var_names[1], "y");
    }

    #[test]
    fn test_simple_var_conversion() {
        let space = Space::new();
        let mut ctx = ConversionContext::new();

        let var_node = Arc::new(RholangNode::Var {
            name: "x".to_string(),
            base: create_base(),
            metadata: None,
        });

        let result = rholang_to_mork_bytes(&var_node, &space, &mut ctx);
        assert!(result.is_ok());
        assert_eq!(ctx.var_names.len(), 1);
        assert_eq!(ctx.var_names[0], "x");
    }

    #[test]
    fn test_nil_conversion() {
        let space = Space::new();
        let mut ctx = ConversionContext::new();

        let nil_node = Arc::new(RholangNode::Nil {
            base: create_base(),
            metadata: None,
        });

        let result = rholang_to_mork_bytes(&nil_node, &space, &mut ctx);
        assert!(result.is_ok());
        // Nil doesn't create variables
        assert_eq!(ctx.var_names.len(), 0);
    }

    #[test]
    fn test_long_literal_conversion() {
        let space = Space::new();
        let mut ctx = ConversionContext::new();

        let int_node = Arc::new(RholangNode::LongLiteral {
            value: 42,
            base: create_base(),
            metadata: None,
        });

        let result = rholang_to_mork_bytes(&int_node, &space, &mut ctx);
        assert!(result.is_ok());
        assert_eq!(ctx.var_names.len(), 0);
    }

    #[test]
    fn test_bool_literal_conversion() {
        let space = Space::new();
        let mut ctx = ConversionContext::new();

        let bool_node = Arc::new(RholangNode::BoolLiteral {
            value: true,
            base: create_base(),
            metadata: None,
        });

        let result = rholang_to_mork_bytes(&bool_node, &space, &mut ctx);
        assert!(result.is_ok());
        assert_eq!(ctx.var_names.len(), 0);
    }

    #[test]
    fn test_string_literal_conversion() {
        let space = Space::new();
        let mut ctx = ConversionContext::new();

        let string_node = Arc::new(RholangNode::StringLiteral {
            value: "hello".to_string(),
            base: create_base(),
            metadata: None,
        });

        let result = rholang_to_mork_bytes(&string_node, &space, &mut ctx);
        assert!(result.is_ok());
        assert_eq!(ctx.var_names.len(), 0);
    }

    #[test]
    fn test_send_conversion() {
        use rpds::Vector;

        let space = Space::new();
        let mut ctx = ConversionContext::new();

        // Create: x!(42)
        let channel = Arc::new(RholangNode::Var {
            name: "x".to_string(),
            base: create_base(),
            metadata: None,
        });

        let input = Arc::new(RholangNode::LongLiteral {
            value: 42,
            base: create_base(),
            metadata: None,
        });

        let inputs = Vector::<Arc<RholangNode>, ArcK>::new_with_ptr_kind().push_back(input);

        let send_node = Arc::new(RholangNode::Send {
            channel,
            send_type: crate::ir::rholang_node::RholangSendType::Single,
            send_type_delta: RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            inputs,
            base: create_base(),
            metadata: None,
        });

        let result = rholang_to_mork_bytes(&send_node, &space, &mut ctx);
        assert!(result.is_ok(), "Send conversion should succeed");
        assert_eq!(ctx.var_names.len(), 1, "Should have one variable (x)");
        assert_eq!(ctx.var_names[0], "x");
    }

    #[test]
    fn test_par_conversion() {
        let space = Space::new();
        let mut ctx = ConversionContext::new();

        // Create: Nil | Nil
        let left = Arc::new(RholangNode::Nil {
            base: create_base(),
            metadata: None,
        });

        let right = Arc::new(RholangNode::Nil {
            base: create_base(),
            metadata: None,
        });

        let par_node = Arc::new(RholangNode::Par {
                processes: None,
            left: Some(left),
            right: Some(right),
            base: create_base(),
            metadata: None,
        });

        let result = rholang_to_mork_bytes(&par_node, &space, &mut ctx);
        assert!(result.is_ok(), "Par conversion should succeed");
        assert_eq!(ctx.var_names.len(), 0, "No variables in Nil | Nil");
    }

    #[test]
    fn test_new_conversion() {
        use rpds::Vector;

        let space = Space::new();
        let mut ctx = ConversionContext::new();

        // Create: new x in { Nil }
        let var_decl = Arc::new(RholangNode::NameDecl {
            var: Arc::new(RholangNode::Var {
                name: "x".to_string(),
                base: create_base(),
                metadata: None,
            }),
            uri: None,
            base: create_base(),
            metadata: None,
        });

        let body = Arc::new(RholangNode::Nil {
            base: create_base(),
            metadata: None,
        });

        let decls = Vector::<Arc<RholangNode>, ArcK>::new_with_ptr_kind().push_back(var_decl);

        let new_node = Arc::new(RholangNode::New {
            decls,
            proc: body,
            base: create_base(),
            metadata: None,
        });

        let result = rholang_to_mork_bytes(&new_node, &space, &mut ctx);
        assert!(result.is_ok(), "New conversion should succeed");
        assert_eq!(ctx.var_names.len(), 1, "Should have one variable (x)");
        assert_eq!(ctx.var_names[0], "x");
    }

    #[test]
    fn test_contract_conversion() {
        use rpds::Vector;

        let space = Space::new();
        let mut ctx = ConversionContext::new();

        // Create: contract foo(x, y) = { Nil }
        let name = Arc::new(RholangNode::Var {
            name: "foo".to_string(),
            base: create_base(),
            metadata: None,
        });

        let formal1 = Arc::new(RholangNode::Var {
            name: "x".to_string(),
            base: create_base(),
            metadata: None,
        });

        let formal2 = Arc::new(RholangNode::Var {
            name: "y".to_string(),
            base: create_base(),
            metadata: None,
        });

        let formals = Vector::<Arc<RholangNode>, ArcK>::new_with_ptr_kind()
            .push_back(formal1)
            .push_back(formal2);

        let body = Arc::new(RholangNode::Nil {
            base: create_base(),
            metadata: None,
        });

        let contract_node = Arc::new(RholangNode::Contract {
            name,
            formals,
            formals_remainder: None,
            proc: body,
            base: create_base(),
            metadata: None,
        });

        let result = rholang_to_mork_bytes(&contract_node, &space, &mut ctx);
        assert!(result.is_ok(), "Contract conversion should succeed");
        assert_eq!(ctx.var_names.len(), 3, "Should have 3 variables (foo, x, y)");
        assert!(ctx.var_names.contains(&"foo".to_string()));
        assert!(ctx.var_names.contains(&"x".to_string()));
        assert!(ctx.var_names.contains(&"y".to_string()));
    }

    #[test]
    fn test_complex_nested_structure() {
        use rpds::Vector;

        let space = Space::new();
        let mut ctx = ConversionContext::new();

        // Create: new x in { x!(42) }
        let var_x = Arc::new(RholangNode::Var {
            name: "x".to_string(),
            base: create_base(),
            metadata: None,
        });

        let decl = Arc::new(RholangNode::NameDecl {
            var: var_x.clone(),
            uri: None,
            base: create_base(),
            metadata: None,
        });

        let input = Arc::new(RholangNode::LongLiteral {
            value: 42,
            base: create_base(),
            metadata: None,
        });

        let send = Arc::new(RholangNode::Send {
            channel: var_x,
            send_type: crate::ir::rholang_node::RholangSendType::Single,
            send_type_delta: RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            inputs: Vector::<Arc<RholangNode>, ArcK>::new_with_ptr_kind().push_back(input),
            base: create_base(),
            metadata: None,
        });

        let new_node = Arc::new(RholangNode::New {
            decls: Vector::<Arc<RholangNode>, ArcK>::new_with_ptr_kind().push_back(decl),
            proc: send,
            base: create_base(),
            metadata: None,
        });

        let result = rholang_to_mork_bytes(&new_node, &space, &mut ctx);
        assert!(result.is_ok(), "Complex nested structure should convert");

        // First x in "new x" creates NewVar, second x in "x!" references it
        assert_eq!(ctx.var_names.len(), 1, "Should have one variable (x)");
        assert_eq!(ctx.var_names[0], "x");
    }

    #[test]
    fn test_variable_reuse_in_different_contexts() {
        use rpds::Vector;

        let space = Space::new();
        let mut ctx = ConversionContext::new();

        // Create: x | x (same variable used twice)
        let var1 = Arc::new(RholangNode::Var {
            name: "x".to_string(),
            base: create_base(),
            metadata: None,
        });

        let var2 = Arc::new(RholangNode::Var {
            name: "x".to_string(),
            base: create_base(),
            metadata: None,
        });

        let par = Arc::new(RholangNode::Par {
                processes: None,
            left: Some(var1),
            right: Some(var2),
            base: create_base(),
            metadata: None,
        });

        let result = rholang_to_mork_bytes(&par, &space, &mut ctx);
        assert!(result.is_ok(), "Par with repeated variable should convert");
        assert_eq!(ctx.var_names.len(), 1, "Should have one variable despite two uses");
        assert_eq!(ctx.var_names[0], "x");
    }
}
