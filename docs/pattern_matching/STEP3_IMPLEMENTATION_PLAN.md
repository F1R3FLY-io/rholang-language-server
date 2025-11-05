# Step 3 Implementation Plan: LSP Backend Integration

**Status**: Ready to Implement
**Estimated Time**: 3 hours
**Priority**: Required for basic pattern matching functionality

---

## Architecture Understanding

### Current Implementation

The language server uses a **unified handler architecture** with language adapters:

```text
LSP Request (textDocument/definition)
       ↓
RholangBackend::goto_definition() [src/lsp/backend.rs]
       ↓
unified_goto_definition() [src/lsp/backend/unified_handlers.rs]
       ↓
Detect language → Get LanguageAdapter
       ↓
GenericGotoDefinition::goto_definition() [src/lsp/features/goto_definition.rs]
       ↓
Use adapter.resolver (SymbolResolver trait)
       ↓
Return definition locations
```

### Key Files

1. **`src/lsp/backend/unified_handlers.rs`**
   - `unified_goto_definition()` - Entry point for goto-definition
   - Detects language (Rholang vs MeTTa virtual documents)
   - Calls generic goto-definition with appropriate adapter

2. **`src/lsp/features/goto_definition.rs`**
   - `GenericGotoDefinition::goto_definition()` - Language-agnostic implementation
   - Finds node at position
   - Extracts symbol name
   - Uses `LanguageAdapter.resolver` to find definitions

3. **`src/ir/symbol_resolution/`**
   - Symbol resolver traits and implementations
   - `LexicalScopeResolver` - Default scope chain traversal
   - `ComposableSymbolResolver` - Combines base + filters + fallback

4. **`src/ir/transforms/symbol_index_builder.rs`**
   - Builds symbol tables during document indexing
   - Adds contract definitions to global index
   - **This is where we need to add pattern index integration**

5. **`src/ir/global_index.rs`**
   - `GlobalSymbolIndex` - Workspace-wide symbol storage
   - Already has `pattern_index: RholangPatternIndex` field (from Step 2D)
   - Already has wrapper methods: `add_contract_with_pattern_index()`, `query_contract_by_pattern()`

---

## Implementation Strategy

### Option A: Integrate at Symbol Resolution Level (RECOMMENDED)

**Approach**: Create a new `PatternAwareContractResolver` that queries the pattern index before falling back to lexical scope.

**Pros**:
- Clean separation of concerns
- Follows existing architecture (composable resolvers)
- Easy to test independently
- No changes to unified handlers needed

**Cons**:
- Requires understanding symbol resolution system

**Files to modify**:
1. `src/ir/symbol_resolution/pattern_aware_resolver.rs` (NEW)
2. `src/lsp/features/adapters/rholang.rs` (update to use new resolver)
3. `src/ir/transforms/symbol_index_builder.rs` (add pattern indexing)

### Option B: Integrate at Generic Goto-Definition Level

**Approach**: Modify `GenericGotoDefinition::goto_definition()` to check for contract invocations and query pattern index.

**Pros**:
- More direct integration
- Easier to understand flow

**Cons**:
- Couples generic feature to Rholang-specific logic
- Breaks language-agnostic design
- Harder to extend to other languages

**Files to modify**:
1. `src/lsp/features/goto_definition.rs`
2. `src/ir/transforms/symbol_index_builder.rs`

### Option C: Create Specialized Provider (ALTERNATIVE)

**Approach**: Similar to MeTTa's specialized `GotoDefinitionProvider`, create a Rholang-specific provider for contract invocations.

**Pros**:
- Keeps complex logic out of generic handlers
- Follows existing pattern (MeTTa uses this)
- Easy to test

**Cons**:
- More code to write
- Duplicates some logic

**Files to modify**:
1. `src/lsp/features/adapters/rholang.rs` (add `RholangGotoDefinitionProvider`)
2. `src/lsp/features/traits.rs` (ensure `GotoDefinitionProvider` trait exists)
3. `src/ir/transforms/symbol_index_builder.rs`

---

## Recommended Approach: Option A (Pattern-Aware Resolver)

This approach best fits the existing architecture and provides the cleanest integration.

### Step-by-Step Implementation

#### Step 3.1: Create Pattern-Aware Resolver

**File**: `src/ir/symbol_resolution/pattern_aware_resolver.rs` (NEW)

```rust
//! Pattern-aware contract resolver using MORK+PathMap matching
//!
//! This resolver enhances contract goto-definition by matching call-site arguments
//! against contract parameter patterns. It enables overload resolution and parameter-
//! aware navigation.

use std::sync::Arc;
use crate::ir::global_index::GlobalSymbolIndex;
use crate::ir::rholang_node::RholangNode;
use crate::ir::semantic_node::SemanticNode;
use crate::ir::symbol_resolution::{
    ResolutionContext, SymbolLocation, SymbolResolver,
};

/// Pattern-aware resolver for contract invocations
///
/// This resolver:
/// 1. Detects if the symbol is a contract invocation (Send node)
/// 2. Extracts contract name and arguments from the call site
/// 3. Queries the pattern index using MORK serialization
/// 4. Falls back to name-only lookup if pattern matching fails
pub struct PatternAwareContractResolver {
    global_index: Arc<GlobalSymbolIndex>,
}

impl PatternAwareContractResolver {
    pub fn new(global_index: Arc<GlobalSymbolIndex>) -> Self {
        Self { global_index }
    }

    /// Extract contract name from a channel expression
    fn extract_contract_name(chan: &Arc<RholangNode>) -> Option<String> {
        match chan.as_ref() {
            RholangNode::Var { name, .. } => Some(name.clone()),
            RholangNode::Quote { proc, .. } => {
                // Handle @"contractName" pattern
                if let RholangNode::Ground { value, .. } = proc.as_ref() {
                    match value {
                        crate::ir::rholang_node::Ground::String(s) => Some(s.clone()),
                        _ => None,
                    }
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Extract arguments from a Send node
    fn extract_arguments(send_node: &RholangNode) -> Option<Vec<Arc<RholangNode>>> {
        match send_node {
            RholangNode::Send { data, .. } => Some(data.clone()),
            _ => None,
        }
    }
}

impl SymbolResolver for PatternAwareContractResolver {
    fn resolve_symbol(
        &self,
        symbol_name: &str,
        _position: &crate::ir::semantic_node::Position,
        context: &ResolutionContext,
    ) -> Vec<SymbolLocation> {
        // Check if we have an IR node in context (for pattern matching)
        if let Some(node_any) = &context.ir_node {
            // Try to downcast to RholangNode
            if let Some(rholang_node) = node_any.downcast_ref::<RholangNode>() {
                // Check if this is a Send node (contract invocation)
                if let RholangNode::Send { chan, .. } = rholang_node {
                    // Extract contract name
                    if let Some(contract_name) = Self::extract_contract_name(chan) {
                        // Only proceed if the contract name matches the symbol we're looking for
                        if contract_name == symbol_name {
                            // Extract arguments
                            if let Some(arguments) = Self::extract_arguments(rholang_node) {
                                // Query pattern index
                                let arg_refs: Vec<&RholangNode> =
                                    arguments.iter().map(|a| a.as_ref()).collect();

                                match self
                                    .global_index
                                    .query_contract_by_pattern(&contract_name, &arg_refs)
                                {
                                    Ok(locations) if !locations.is_empty() => {
                                        tracing::debug!(
                                            "PatternAwareContractResolver: Found {} matches via pattern index for contract '{}'",
                                            locations.len(),
                                            contract_name
                                        );
                                        return locations;
                                    }
                                    Ok(_) => {
                                        tracing::debug!(
                                            "PatternAwareContractResolver: No pattern matches for contract '{}', will fall back",
                                            contract_name
                                        );
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            "PatternAwareContractResolver: Pattern query failed: {}",
                                            e
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // If we reach here, either:
        // - Not a Send node
        // - Pattern matching failed
        // - No matches found
        // Return empty to let other resolvers handle it
        vec![]
    }

    fn supports_language(&self, language: &str) -> bool {
        language == "rholang"
    }
}
```

**Don't forget to add to `src/ir/symbol_resolution/mod.rs`**:
```rust
pub mod pattern_aware_resolver;
pub use pattern_aware_resolver::PatternAwareContractResolver;
```

#### Step 3.2: Update Rholang Adapter

**File**: `src/lsp/features/adapters/rholang.rs`

**Find the section where the symbol resolver is created** (likely in a function like `create_rholang_adapter()` or in the `RholangAdapter` struct).

**Modify to use ComposableSymbolResolver with PatternAwareContractResolver**:

```rust
use crate::ir::symbol_resolution::{
    ComposableSymbolResolver, LexicalScopeResolver, PatternAwareContractResolver,
};

// In the adapter creation function:
let pattern_resolver = Box::new(PatternAwareContractResolver::new(global_index.clone()));
let lexical_resolver = Box::new(LexicalScopeResolver::new(symbol_table.clone(), "rholang".to_string()));

let resolver = Box::new(ComposableSymbolResolver::new(
    pattern_resolver,  // Try pattern matching first
    vec![],            // No filters needed for now
    Some(lexical_resolver),  // Fall back to lexical scope
));
```

**Alternative** (if resolver is created inline):
```rust
// Replace existing resolver with composable version
let resolver: Arc<dyn SymbolResolver> = Arc::new(ComposableSymbolResolver::new(
    Box::new(PatternAwareContractResolver::new(workspace.global_index.clone())),
    vec![],
    Some(Box::new(LexicalScopeResolver::new(symbol_table, "rholang".to_string()))),
));
```

#### Step 3.3: Add Pattern Indexing During Document Processing

**File**: `src/ir/transforms/symbol_index_builder.rs`

**Find where contracts are visited** (likely in a `visit_contract()` method or similar).

**Add pattern indexing call**:

```rust
// When visiting a Contract node:
fn visit_contract(&mut self, node: &RholangNode) {
    // ... existing contract processing ...

    // Add to pattern index
    if let RholangNode::Contract { .. } = node {
        // Convert node metadata positions to IR positions
        let ir_location = crate::ir::symbol_resolution::SymbolLocation {
            uri: self.uri.clone(),
            start: crate::ir::semantic_node::Position {
                row: /* extract from node.base() */,
                column: /* extract from node.base() */,
                byte: 0, // Can use 0 if byte offset not available
            },
            end: crate::ir::semantic_node::Position {
                row: /* extract from node.base() */,
                column: /* extract from node.base() */,
                byte: 0,
            },
        };

        // Add to pattern index via global index
        if let Err(e) = self
            .global_index
            .add_contract_with_pattern_index(node, ir_location)
        {
            tracing::warn!("Failed to add contract to pattern index: {}", e);
        }
    }

    // ... continue with existing processing ...
}
```

**Note**: You'll need to:
1. Add `global_index: Arc<GlobalSymbolIndex>` field to `SymbolIndexBuilder`
2. Pass it during construction
3. Extract actual positions from `node.base().absolute_position()` or metadata

#### Step 3.4: Update Context to Include IR Node

**File**: `src/lsp/features/goto_definition.rs`

**In `GenericGotoDefinition::goto_definition()`**, ensure the `ResolutionContext` includes the IR node:

```rust
// When creating ResolutionContext:
let context = ResolutionContext {
    uri: uri.clone(),
    scope_id: /* extract from node metadata if available */,
    ir_node: Some(Arc::new(node.clone()) as Arc<dyn Any + Send + Sync>),  // Add this!
    language: adapter.language_name().to_string(),
    parent_uri: None,
};
```

This allows the `PatternAwareContractResolver` to access the actual RholangNode for pattern extraction.

---

## Testing Strategy

### Unit Tests

**File**: `src/ir/symbol_resolution/pattern_aware_resolver.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::rholang_node::*;
    use crate::ir::semantic_node::*;

    #[test]
    fn test_extract_contract_name_from_var() {
        let chan = Arc::new(RholangNode::Var {
            base: NodeBase::default(),
            name: "echo".to_string(),
        });

        let name = PatternAwareContractResolver::extract_contract_name(&chan);
        assert_eq!(name, Some("echo".to_string()));
    }

    #[test]
    fn test_extract_contract_name_from_quoted_string() {
        let proc = Arc::new(RholangNode::Ground {
            base: NodeBase::default(),
            value: Ground::String("myContract".to_string()),
        });

        let chan = Arc::new(RholangNode::Quote {
            base: NodeBase::default(),
            proc,
        });

        let name = PatternAwareContractResolver::extract_contract_name(&chan);
        assert_eq!(name, Some("myContract".to_string()));
    }

    // Add more tests for extract_arguments, full resolution, etc.
}
```

### Integration Tests

**File**: `tests/test_pattern_matching_goto_definition.rs` (NEW)

```rust
//! Integration tests for pattern-aware goto-definition
//!
//! These tests verify that goto-definition correctly resolves contract calls
//! using MORK+PathMap pattern matching for overload resolution.

use std::fs;
use test_utils::with_lsp_client;
use test_utils::lsp::client::{CommType, LspClient};
use tower_lsp::lsp_types::Position;

/// Test that goto-definition disambiguates overloaded contracts by arity
///
/// Contract definitions:
/// - Line 3: contract echo(@x) = { stdout!(x) }
/// - Line 4: contract echo(@x, @y) = { stdout!((x, y)) }
///
/// Call sites:
/// - Line 7: echo!(42) - should match line 3 (1 parameter)
/// - Line 8: echo!(42, "hello") - should match line 4 (2 parameters)
with_lsp_client!(test_overload_resolution_by_arity, CommType::Stdio, |client: &LspClient| {
    println!("\n=== Testing overload resolution by parameter arity ===");

    let source = r#"
        new echo, stdout(`rho:io:stdout`) in {
            // Define two overloaded contracts
            contract echo(@x) = { stdout!(x) } |
            contract echo(@x, @y) = { stdout!((x, y)) } |

            // Call with one argument
            echo!(42) |
            // Call with two arguments
            echo!(42, "hello")
        }
    "#;

    let doc = client
        .open_document("/test/overload.rho", source)
        .expect("Failed to open document");

    let _diagnostics = client
        .await_diagnostics(&doc)
        .expect("Failed to receive diagnostics");

    // Test 1-argument call
    println!("\n--- Test 1: One-argument call should match one-parameter contract ---");
    let one_arg_call_pos = Position { line: 7, character: 12 }; // "echo" in echo!(42)

    let locations = client
        .definition_all(&doc.uri(), one_arg_call_pos)
        .expect("goto_definition failed");

    assert!(!locations.is_empty(), "Should find at least one definition");
    assert_eq!(
        locations[0].range.start.line, 3,
        "One-arg call should match one-param contract (line 3)"
    );

    println!("✓ One-argument call correctly matched to line 3");

    // Test 2-argument call
    println!("\n--- Test 2: Two-argument call should match two-parameter contract ---");
    let two_arg_call_pos = Position { line: 9, character: 12 }; // "echo" in echo!(42, "hello")

    let locations = client
        .definition_all(&doc.uri(), two_arg_call_pos)
        .expect("goto_definition failed");

    assert!(!locations.is_empty(), "Should find at least one definition");
    assert_eq!(
        locations[0].range.start.line, 4,
        "Two-arg call should match two-param contract (line 4)"
    );

    println!("✓ Two-argument call correctly matched to line 4");

    println!("\n=== TEST PASSED ===");
    client.close_document(&doc).expect("Failed to close document");
});

// Add more tests:
// - test_map_pattern_matching
// - test_list_pattern_matching
// - test_tuple_pattern_matching
// - test_fallback_to_name_lookup (when patterns don't match)
```

---

## Verification Checklist

Before considering Step 3 complete:

- [ ] `PatternAwareContractResolver` created and tested
- [ ] Rholang adapter updated to use pattern-aware resolver
- [ ] Contracts indexed with pattern information during workspace indexing
- [ ] `ResolutionContext` includes IR node for pattern extraction
- [ ] Unit tests for resolver pass
- [ ] Integration test `test_overload_resolution_by_arity` passes
- [ ] All existing goto-definition tests still pass (zero regressions)
- [ ] `cargo build` succeeds
- [ ] `cargo nextest run` shows 547+ tests passing

---

## Debugging Tips

### Enable Debug Logging

```bash
RUST_LOG=rholang_language_server::ir::symbol_resolution::pattern_aware_resolver=debug,rholang_language_server::lsp::features::goto_definition=debug cargo test test_overload_resolution_by_arity -- --nocapture
```

### Common Issues

1. **Pattern index not being populated**
   - Check that `add_contract_with_pattern_index()` is being called
   - Verify `SymbolIndexBuilder` has access to `global_index`
   - Add debug logging to see if contracts are being indexed

2. **Resolver not being called**
   - Verify `PatternAwareContractResolver` is added to composable resolver
   - Check `supports_language()` returns true for "rholang"
   - Ensure `ResolutionContext` has `ir_node` populated

3. **Pattern matching fails**
   - Check MORK serialization is working (`cargo test --lib mork_canonical`)
   - Verify arguments are being extracted correctly
   - Add logging to `query_contract_by_pattern()` to see query results

---

## Expected Results

After Step 3 is complete:

```bash
$ cargo test test_overload_resolution_by_arity
running 1 test
test test_overload_resolution_by_arity ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

```bash
$ cargo nextest run
Summary [~3s] 548+ tests run: 548+ passed, 9 skipped
```

---

## Next Steps After Step 3

Once Step 3 is complete and working:

1. **Document the implementation** in `docs/pattern_matching/implementation/05_lsp_integration.md`
2. **Update README.md** to mark Step 3 complete
3. **Consider Step 4** (optional advanced features):
   - MORK unification for variable/wildcard matching
   - Remainder pattern support
   - Map key navigation (uncomment `test_pathmap_pattern_goto_definition`)

---

**Status**: Implementation plan ready
**Estimated completion time**: 3 hours for experienced developer
**Dependencies**: All Step 1-2D work complete ✅
