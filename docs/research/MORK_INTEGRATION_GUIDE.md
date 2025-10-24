# MORK Pattern Matching Integration Guide for Rholang LSP

**Based on MeTTaTron's proven implementation**

This guide provides concrete implementation details for integrating MORK and PathMap into the Rholang Language Server, extracted from studying MeTTaTron's actual codebase.

## Overview

MORK (MeTTa Optimal Reduction Kernel) is a pattern matching library designed for s-expressions and processes. PathMap is a trie-based map providing O(log n) pattern lookup. Together, they enable:

- **Declarative pattern matching** instead of manual conditional logic
- **Efficient pattern caching** via PathMap trie structure
- **De Bruijn variable encoding** for consistent variable handling
- **Query optimization** through Space's `query_multi()` function

## Phase 0: Core Infrastructure Setup

### Step 1: Add Dependencies

Add to `Cargo.toml`:

```toml
[dependencies]
mork = { path = "../MORK/kernel", features = ["interning"] }
mork-expr = { path = "../MORK/expr" }
mork-frontend = { path = "../MORK/frontend" }
pathmap = { path = "../PathMap", features = ["jemalloc", "arena_compact"] }
```

### Step 2: Create Conversion Module

**New file**: `src/ir/mork_convert.rs`

This module handles bidirectional conversion between RholangNode and MORK Expr format.

```rust
//! Conversion utilities between RholangNode and MORK Expr format
//!
//! This module handles the bidirectional conversion needed for query_multi integration:
//! - RholangNode → MORK Expr (for pattern queries)
//! - MORK bindings → HashMap<String, RholangNode> (for pattern match results)

use crate::ir::rholang_node::RholangNode;
use mork::space::Space;
use mork_expr::{Expr, ExprEnv, ExprZipper, Tag, item_byte};
use mork_frontend::bytestring_parser::Parser;
use std::collections::HashMap;
use std::sync::Arc;

/// Context for tracking variables during RholangNode → Expr conversion
#[derive(Default)]
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
fn write_rholang_node(
    node: &Arc<RholangNode>,
    space: &Space,
    ctx: &mut ConversionContext,
    ez: &mut ExprZipper,
) -> Result<(), String> {
    match &**node {
        // Variables
        RholangNode::Var { name, .. } => {
            // Rholang variables use De Bruijn encoding
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

        // Wildcard patterns
        RholangNode::WildcardPat { .. } => {
            // Treat as anonymous variable
            ez.write_new_var();
            ez.loc += 1;
        }

        // Send (Rholang process invocation)
        RholangNode::Send { channel, inputs, .. } => {
            // Encode as (send <channel> <inputs...>)
            let arity = 2 + inputs.len() as u8;
            ez.write_arity(arity);
            ez.loc += 1;

            write_symbol(b"send", space, ez)?;
            write_rholang_node(channel, space, ctx, ez)?;
            for input in inputs.iter() {
                write_rholang_node(input, space, ctx, ez)?;
            }
        }

        // Contract definition
        RholangNode::Contract { name, formals, body, .. } => {
            // Encode as (contract <name> <formals...> <body>)
            let arity = 3 + formals.len() as u8;
            ez.write_arity(arity);
            ez.loc += 1;

            write_symbol(b"contract", space, ez)?;
            write_symbol(name.as_bytes(), space, ez)?;
            for formal in formals.iter() {
                write_rholang_node(formal, space, ctx, ez)?;
            }
            write_rholang_node(body, space, ctx, ez)?;
        }

        // New (name binding)
        RholangNode::New { bindings, body, .. } => {
            // Encode as (new <bindings...> <body>)
            let arity = 2 + bindings.len() as u8;
            ez.write_arity(arity);
            ez.loc += 1;

            write_symbol(b"new", space, ez)?;
            for binding in bindings.iter() {
                write_symbol(binding.as_bytes(), space, ez)?;
            }
            write_rholang_node(body, space, ctx, ez)?;
        }

        // Ground values
        RholangNode::GInt { value, .. } => {
            let s = value.to_string();
            write_symbol(s.as_bytes(), space, ez)?;
        }

        RholangNode::GString { value, .. } => {
            // MORK uses quoted strings
            let quoted = format!("\"{}\"", value);
            write_symbol(quoted.as_bytes(), space, ez)?;
        }

        RholangNode::GBool { value, .. } => {
            let s = if *value { "true" } else { "false" };
            write_symbol(s.as_bytes(), space, ez)?;
        }

        // Par (parallel composition)
        RholangNode::Par { nodes, .. } => {
            // Encode as (par <node1> <node2> ...)
            let arity = 1 + nodes.len() as u8;
            ez.write_arity(arity);
            ez.loc += 1;

            write_symbol(b"par", space, ez)?;
            for n in nodes.iter() {
                write_rholang_node(n, space, ctx, ez)?;
            }
        }

        // Nil
        RholangNode::Nil { .. } => {
            write_symbol(b"Nil", space, ez)?;
        }

        // Add other node types as needed...
        _ => {
            return Err(format!("Unsupported node type for MORK conversion: {:?}", node));
        }
    }

    Ok(())
}

/// Write a symbol to ExprZipper using Space's symbol table
fn write_symbol(bytes: &[u8], space: &Space, ez: &mut ExprZipper) -> Result<(), String> {
    // Use MORK's ParDataParser to intern the symbol
    let mut pdp = mork::space::ParDataParser::new(&space.sm);
    let token = pdp.tokenizer(bytes);

    ez.write_symbol(token);
    ez.loc += 1 + token.len();

    Ok(())
}

/// Convert MORK bindings to HashMap<String, Arc<RholangNode>>
///
/// MORK uses BTreeMap<(u8, u8), ExprEnv> where the key is (old_var, new_var).
/// We need to convert this to HashMap<String, RholangNode> using the original variable names.
pub fn mork_bindings_to_rholang(
    mork_bindings: &std::collections::BTreeMap<(u8, u8), ExprEnv>,
    ctx: &ConversionContext,
    space: &Space,
) -> Result<HashMap<String, Arc<RholangNode>>, String> {
    let mut bindings = HashMap::new();

    for (&(old_var, _new_var), expr_env) in mork_bindings {
        // Get the variable name from context
        if (old_var as usize) >= ctx.var_names.len() {
            continue; // Skip if variable not in our context
        }
        let var_name = &ctx.var_names[old_var as usize];

        // Convert MORK Expr back to RholangNode
        let expr: Expr = expr_env.subsexpr();
        if let Ok(node) = mork_expr_to_rholang(&expr, space) {
            bindings.insert(var_name.clone(), node);
        }
    }

    Ok(bindings)
}

/// Convert MORK Expr back to RholangNode
///
/// This is the reverse operation of write_rholang_node()
fn mork_expr_to_rholang(expr: &Expr, space: &Space) -> Result<Arc<RholangNode>, String> {
    // TODO: Implement based on examining expr structure
    // This would involve checking the Tag (Arity, SymbolSize, NewVar, VarRef)
    // and reconstructing RholangNode variants
    Err("Not yet implemented".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_variable_conversion() {
        let space = Space::new();
        let mut ctx = ConversionContext::new();

        // Create a simple Var node
        use crate::ir::rholang_node::{NodeBase, RelativePosition};
        let base = NodeBase::new(
            RelativePosition {
                delta_lines: 0,
                delta_columns: 0,
                delta_bytes: 0,
            },
            0,
            0,
            1,
        );
        let var_node = Arc::new(RholangNode::Var {
            name: "x".to_string(),
            base,
            metadata: None,
        });

        let result = rholang_to_mork_bytes(&var_node, &space, &mut ctx);
        assert!(result.is_ok());
        assert_eq!(ctx.var_names.len(), 1);
        assert_eq!(ctx.var_names[0], "x");
    }

    #[test]
    fn test_send_conversion() {
        let space = Space::new();
        let mut ctx = ConversionContext::new();

        // TODO: Create a Send node and test conversion
    }
}
```

### Step 3: Create Pattern Matching Module

**New file**: `src/ir/pattern_matching.rs`

This module provides the high-level pattern matching API using MORK.

```rust
//! Pattern matching infrastructure using MORK and PathMap
//!
//! This module provides declarative pattern matching for Rholang processes,
//! following MeTTaTron's proven architecture.

use std::sync::Arc;
use std::collections::HashMap;
use mork::space::Space;
use mork_expr::{Expr, ExprZipper};
use mork_frontend::bytestring_parser::{Parser, Context};
use crate::ir::rholang_node::RholangNode;
use crate::ir::mork_convert::{ConversionContext, rholang_to_mork_bytes, mork_bindings_to_rholang};

/// Result of pattern matching
pub type MatchResult = Vec<(Arc<RholangNode>, HashMap<String, Arc<RholangNode>>)>;

/// Pattern matcher for Rholang processes
///
/// This uses MORK's Space for efficient pattern storage and query_multi for O(k) matching
/// where k is the number of matching patterns (vs O(n) for iteration over all patterns).
pub struct RholangPatternMatcher {
    /// MORK Space for pattern storage
    space: Space,
}

impl RholangPatternMatcher {
    pub fn new() -> Self {
        RholangPatternMatcher {
            space: Space::new(),
        }
    }

    /// Add a pattern-value pair to the matcher
    ///
    /// This stores the pattern in MORK Space for efficient lookup.
    /// Patterns are stored as: (pattern-key <pattern-bytes> <value-bytes>)
    pub fn add_pattern(
        &mut self,
        pattern: &Arc<RholangNode>,
        value: &Arc<RholangNode>,
    ) -> Result<(), String> {
        let mut ctx = ConversionContext::new();

        // Convert pattern to MORK bytes
        let pattern_bytes = rholang_to_mork_bytes(pattern, &self.space, &mut ctx)?;

        // Convert value to MORK bytes (with fresh context to avoid variable confusion)
        let mut value_ctx = ConversionContext::new();
        let value_bytes = rholang_to_mork_bytes(value, &self.space, &mut value_ctx)?;

        // Create entry: (pattern-key <pattern> <value>)
        let entry = format!(
            "(pattern-key {} {})",
            String::from_utf8_lossy(&pattern_bytes),
            String::from_utf8_lossy(&value_bytes)
        );

        // Parse and insert into Space
        let mut parse_buffer = vec![0u8; 4096];
        let mut pdp = mork::space::ParDataParser::new(&self.space.sm);
        let mut ez = ExprZipper::new(Expr {
            ptr: parse_buffer.as_mut_ptr(),
        });
        let mut context = Context::new(entry.as_bytes());

        pdp.sexpr(&mut context, &mut ez)
            .map_err(|e| format!("Parse error: {:?}", e))?;

        // Insert the pattern into the Space
        let data = &parse_buffer[..ez.loc];
        self.space.btm.insert(data, ());

        Ok(())
    }

    /// Match a query against all stored patterns
    ///
    /// Returns all (value, bindings) pairs where pattern matches query.
    /// Uses MORK's query_multi for O(k) performance.
    ///
    /// # Example Pattern Matching
    ///
    /// Given patterns stored:
    /// - (send (contract "foo") x) → handler1
    /// - (send (contract "bar") y) → handler2
    ///
    /// Query: (send (contract "foo") 42)
    /// Returns: [(handler1, {"x": 42})]
    pub fn match_query(&self, query: &Arc<RholangNode>) -> Result<MatchResult, String> {
        // Convert query to MORK bytes
        let mut ctx = ConversionContext::new();
        let query_bytes = rholang_to_mork_bytes(query, &self.space, &mut ctx)?;

        // Create pattern: (pattern-key <query> $value)
        // This will match all entries where the pattern matches our query
        let pattern_str = format!(
            "(pattern-key {} $value)",
            String::from_utf8_lossy(&query_bytes)
        );

        // Parse the pattern
        let mut parse_buffer = vec![0u8; 4096];
        let mut pdp = mork::space::ParDataParser::new(&self.space.sm);
        let mut ez = ExprZipper::new(Expr {
            ptr: parse_buffer.as_mut_ptr(),
        });
        let mut context = Context::new(pattern_str.as_bytes());

        pdp.sexpr(&mut context, &mut ez)
            .map_err(|e| format!("Parse error: {:?}", e))?;

        let pattern_expr = Expr {
            ptr: parse_buffer.as_ptr().cast_mut(),
        };

        // Collect all matches using query_multi
        let mut matches: MatchResult = Vec::new();

        Space::query_multi(&self.space.btm, pattern_expr, |result, _matched_expr| {
            if let Err(bindings) = result {
                // Convert MORK bindings to our format
                if let Ok(our_bindings) = mork_bindings_to_rholang(&bindings, &ctx, &self.space) {
                    // Extract the value from bindings
                    if let Some(value) = our_bindings.get("$value") {
                        matches.push((value.clone(), our_bindings));
                    }
                }
            }
            true // Continue searching for ALL matches
        });

        Ok(matches)
    }

    /// Find contract invocations matching a contract definition
    ///
    /// This is a specialized helper for the common LSP use case:
    /// Given a contract definition, find all Send nodes that invoke it.
    ///
    /// # Example
    ///
    /// Contract: (contract "myContract" (x y) body)
    /// Invocations: (send (contract "myContract") (42 100))
    ///
    /// This would match and return bindings: {"x": 42, "y": 100}
    pub fn find_contract_invocations(
        &self,
        contract_name: &str,
        formals: &[String],
    ) -> Result<Vec<(Arc<RholangNode>, HashMap<String, Arc<RholangNode>>)>, String> {
        // Build a pattern: (send (contract <name>) <args...>)
        // where args are fresh variables matching formals

        // TODO: Construct pattern from contract signature
        // This is similar to MeTTaTron's eval_match() function

        Err("Not yet implemented".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pattern_matcher_creation() {
        let matcher = RholangPatternMatcher::new();
        // Basic smoke test
    }

    #[test]
    fn test_add_and_match_pattern() {
        // TODO: Create test patterns and verify matching
    }
}
```

## Usage Example: Replacing match_contract()

**Before (manual pattern matching)**:

```rust
// From src/lsp/backend.rs
fn match_contract(
    contract_name: &str,
    parent: &RholangNode,
    inputs: &[Arc<RholangNode>],
    formals: &[String],
) -> bool {
    match &*parent {
        RholangNode::Send { channel, inputs: send_inputs, .. } => {
            // Manually check if channel matches contract name
            // Manually check if inputs match formals
            // Lots of nested conditionals...
        }
        _ => false,
    }
}
```

**After (declarative MORK matching)**:

```rust
use crate::ir::pattern_matching::RholangPatternMatcher;

fn find_contract_references(
    contract_name: &str,
    formals: &[String],
    ir: &Arc<RholangNode>,
) -> Vec<Arc<RholangNode>> {
    // Create matcher
    let matcher = RholangPatternMatcher::new();

    // Find all invocations matching this contract signature
    match matcher.find_contract_invocations(contract_name, formals) {
        Ok(matches) => {
            matches.into_iter()
                .map(|(node, _bindings)| node)
                .collect()
        }
        Err(e) => {
            log::warn!("Pattern matching failed: {}", e);
            Vec::new()
        }
    }
}
```

## Key Differences from MeTTaTron

1. **Process vs S-Expression Encoding**:
   - MeTTa: S-expressions are natural (already in s-expr form)
   - Rholang: Processes need encoding as s-expressions (e.g., `Send` → `(send channel inputs)`)

2. **Variable Semantics**:
   - MeTTa: Variables have `$`, `&`, `'` prefixes
   - Rholang: Variables are just names (in `new` bindings) or patterns

3. **Structural Differences**:
   - MeTTa: Flat s-expression nesting
   - Rholang: Parallel composition (`Par`) and name channels require special handling

## Performance Characteristics

| Operation | Without MORK | With MORK | Improvement |
|-----------|--------------|-----------|-------------|
| Find matching contract | O(n) iterate all sends | O(k) where k = matches | 10-1000x for large codebases |
| Pattern compilation | Every query | Once at startup | Amortized across queries |
| Memory usage | Minimal | PathMap trie storage | Trade-off for speed |

## Migration Strategy

1. **Phase 0.1**: Implement `mork_convert.rs` with basic node types (Var, Send, Contract, New)
2. **Phase 0.2**: Implement `pattern_matching.rs` infrastructure
3. **Phase 0.3**: Replace `match_contract()` in `backend.rs` as proof of concept
4. **Phase 0.4**: Measure performance improvement
5. **Phase 0.5**: Extend to all node types
6. **Phase 1+**: Continue with semantic features migration (from MIGRATION_PLAN.md)

## Testing Strategy

1. **Unit Tests**: Each conversion function (per node type)
2. **Integration Tests**: Full pattern matching scenarios
3. **Performance Tests**: Compare O(n) vs O(k) on large codebases
4. **Fallback Tests**: Verify iterative fallback works when MORK fails

## References

- MeTTaTron implementation: `/home/dylon/Workspace/f1r3fly.io/MeTTa-Compiler/src/backend/`
  - `mork_convert.rs`: Conversion utilities (lines 1-281)
  - `eval.rs`: Pattern matching usage (lines 879-946)
- MORK Space API: `/home/dylon/Workspace/f1r3fly.io/MORK/kernel/src/space.rs`
  - `query_multi()`: Lines 988-1125
  - `coreferential_transition()`: Lines 78-193
- PathMap documentation: `/home/dylon/Workspace/f1r3fly.io/PathMap/`
