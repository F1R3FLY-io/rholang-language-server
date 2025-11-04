# Comment Channel - Known Issues and Debugging Guide

**Date**: 2025-11-04  
**Status**: Active Issues Requiring Investigation

## Overview

This document catalogs technical issues discovered during the comment channel implementation and LSP integration. Each issue includes reproduction steps, debugging information, and suggested approaches for resolution.

---

## Issue 1: Hover Position Sensitivity - Documentation Not Shown on Contract Names

**Severity**: Medium  
**Component**: LSP Hover, DocumentationAttacher  
**Status**: Reproducible, Root Cause Identified

### Problem Description

When hovering directly over a contract or function name, the documentation attached to the declaration node is not displayed. Hovering over other parts of the declaration (parameters, body, etc.) works correctly.

### Reproduction Steps

1. Create a file with a documented contract:
```rholang
/// This is a contract that does something important
/// It handles user requests
contract foo(@x) = {
    Nil
}
```

2. Hover over the name "foo" (line 2, characters 9-12)
3. **Expected**: Hover tooltip shows documentation
4. **Actual**: Hover tooltip shows basic symbol info without documentation

### Root Cause Analysis

**IR Node Structure**:
```rust
RholangNode::Contract {
    base: NodeBase,
    name: Arc<RholangNode::Var>,  // <- Child node
    formals: Vec<Arc<RholangNode>>,
    formals_remainder: Option<Arc<RholangNode>>,
    proc: Arc<RholangNode>,
    metadata: Some(HashMap {          // <- Documentation attached here
        "documentation": Arc<String>
    })
}
```

**What Happens**:
1. User hovers over "foo" at position (2, 9)
2. `find_node_at_position()` traverses IR tree
3. Finds the innermost node: `RholangNode::Var { name: "foo" }`
4. Var node has NO documentation metadata (it's on the parent Contract)
5. Hover provider checks Var node metadata → None found
6. Returns basic hover without documentation

**Debug Output**:
```
[DEBUG] find_node_at_position: Looking for node at position (2, 9)
[DEBUG] >>> Par node contains target! type=Rholang::Par, start=(0, 0), children=2
[DEBUG]   Child[0] type=Rholang::Contract, start=(1, 0), end=(3, 1), in_range=true
CONTRACT FOUND: start=(1, 0), end=(3, 1), target=(2, 9)
[DEBUG] === POSITION TRACKING DEBUG for Rholang::Contract ===
[DEBUG]   Child[0] type=Rholang::Var, start=(1, 9), end=(1, 12), in_range=true
[DEBUG] find_node_at_position: FOUND node type=Rholang::Var, start=(0, 9)
[DEBUG] Found node at position: type=Rholang::Var, category=Variable
[DEBUG] extract_symbol_name: node has metadata with 3 keys
[DEBUG] Extracted symbol name from RholangNode::Var: foo
[DEBUG] Returning hover for Rholang::Var at Range { ... }
```

The Var node metadata has 3 keys (likely symbol table info) but NO "documentation" key.

### Attempted Solution

**Approach**: Attach documentation to child name node during transformation.

**Code**:
```rust
// In DocumentationAttacher::visit_contract()
let new_name = if let Some(doc_text) = &should_attach_doc {
    if let RholangNode::Var { base, name, metadata } = name.as_ref() {
        // Create new Var node with documentation metadata
        Arc::new(RholangNode::Var {
            base: base.clone(),
            name: name.clone(),
            metadata: Some(Arc::new(doc_metadata)),
        })
    } else {
        self.visit_node(name)
    }
} else {
    self.visit_node(name)
};
```

**Result**: ❌ FAILED with panic "RholangNode not found"

**Why It Failed**:
- Creating a new Var node assigns it a new memory address
- Position computation (`compute_absolute_positions()`) creates a HashMap keyed by node addresses
- Later code calls `node.position(&root)` → recomputes positions → looks up new node address
- New address not in position map → panic

**Relevant Code** (`src/ir/rholang_node/node_impl.rs:82-86`):
```rust
pub fn position(&self, root: &Arc<RholangNode>) -> usize {
    let positions = compute_absolute_positions(root);
    let key = self as *const RholangNode as usize;  // Memory address as key
    positions.get(&key).expect("RholangNode not found").0.byte  // ← Panics here
}
```

### Recommended Solution

**Option 1: Parent Node Context in Hover (Preferred)**

Modify hover system to pass parent node context:

```rust
// In GenericHover::hover_with_node()
pub async fn hover_with_node(
    &self,
    pre_found_node: Option<&dyn SemanticNode>,
    parent_node: Option<&dyn SemanticNode>,  // <- Add parent parameter
    root: &dyn SemanticNode,
    // ... other params
) -> Option<Hover> {
    // Check node first
    if let Some(doc) = extract_documentation(node) {
        return Some(format_hover(doc));
    }
    
    // Fall back to parent node
    if let Some(parent) = parent_node {
        if let Some(doc) = extract_documentation(parent) {
            return Some(format_hover(doc));
        }
    }
    
    None
}
```

**Changes Required**:
1. Update `find_node_at_position()` to return `(node, parent)` tuple
2. Pass parent through hover call chain
3. Update HoverProvider trait to accept parent context
4. Check parent metadata when child has no documentation

**Benefits**:
- No IR transformation needed
- Works with existing position computation
- General solution for any parent→child metadata lookup

**Option 2: Use Stable Node IDs Instead of Memory Addresses**

Add UUID-based node IDs to NodeBase:

```rust
pub struct NodeBase {
    pub position: RelativePosition,
    pub length: usize,
    pub span_lines: usize,
    pub span_columns: usize,
    pub node_id: uuid::Uuid,  // <- Add stable ID
}
```

**Benefits**:
- Node transformations preserve IDs
- Position maps work across transformations
- Enables node tracking across pipeline stages

**Drawbacks**:
- Large refactoring required
- Breaks existing serialization
- Adds memory overhead (16 bytes per node)

### Test Case

**File**: `tests/lsp_features.rs` - `test_hover_with_documentation`

**Current Status**: Test fails because hovering over contract name doesn't find documentation.

**Expected Behavior After Fix**:
```rust
let hover_pos = Position { line: 2, character: 10 }; // On "foo"
let hover = client.hover(&doc.uri(), hover_pos).unwrap();

assert!(hover.is_some());
let content = hover.unwrap().contents;
assert!(content.contains("This is a contract that does something important"));
```

### Priority

**Medium** - Feature works for most hover positions (contract keyword, parameters, body), just not the specific name position. Workaround exists (hover elsewhere).

---

## Issue 2: Test Infrastructure - Intermittent Document Indexing Failures

**Severity**: Low  
**Component**: LSP Test Client, Background Indexing  
**Status**: Intermittent, Needs Investigation

### Problem Description

LSP integration tests occasionally fail with "No language context found" errors during document indexing. The error is intermittent - same test may pass or fail on different runs.

### Reproduction

**Test**: `test_hover_with_documentation`

**Failure Rate**: ~30-40% of test runs

**Error Log**:
```
[ERROR] Failed to index file: Failed to spawn blocking task: task 22 panicked with message "RholangNode not found"
[DEBUG] unified_hover: uri=file:///path/to/documented.rho, position=Position { line: 2, character: 10 }
[WARN] detect_language: No language context found for Url { ... path: "/path/to/documented.rho" ... }
[DEBUG] Received null hover response for URI: file:///path/to/documented.rho
```

### Analysis

**Timing Issue**:
1. Test opens document with `client.open_document()`
2. Test waits for diagnostics with `client.await_diagnostics()`
3. Diagnostics arrive, indicating document was parsed
4. Test immediately sends hover request
5. **Problem**: Document IR may not be fully indexed in workspace yet

**Evidence**:
- "No language context found" suggests document not in workspace maps
- Error message: "Failed to index file" during background processing
- Works when test adds artificial delay before hover request

**Relevant Code** (`src/lsp/backend/unified_handlers.rs`):
```rust
fn detect_language(&self, uri: &Url, position: LspPosition) -> Option<LanguageContext> {
    // Check documents_by_uri first (open documents)
    if let Some(lsp_doc) = self.documents_by_uri.get(uri) {
        // ... return context
    }
    
    // Fall back to workspace.documents (indexed files)
    if let Some(cached) = self.workspace.documents.get(uri) {
        // ... return context
    }
    
    warn!("detect_language: No language context found for {}", uri);
    None  // ← Returns here when not found
}
```

### Potential Root Causes

**Theory 1: Race Condition in Async Indexing**

```rust
// In RholangBackend::did_open()
let cached_doc = self.index_file(&uri, &text, version, None).await?;

// Indexing happens on blocking thread pool
tokio::task::spawn_blocking(move || {
    Self::process_document_blocking(...)  // ← Async, may not complete before hover
})
```

**Theory 2: DashMap Visibility Delay**

Uses DashMap for lock-free concurrent access, but reads may not immediately see writes from other threads.

**Theory 3: Test Framework Timing**

`await_diagnostics()` may return before workspace state is fully committed.

### Recommended Investigation Steps

1. **Add Synchronization Point**:
```rust
// In test
client.await_diagnostics(&doc)?;
tokio::time::sleep(Duration::from_millis(100)).await;  // Allow indexing to complete
let hover = client.hover(&doc.uri(), hover_pos)?;
```

If this fixes it → timing issue confirmed.

2. **Add Workspace Ready Check**:
```rust
impl RholangBackend {
    pub async fn wait_for_document_indexed(&self, uri: &Url) -> Result<()> {
        for _ in 0..50 {  // 5 second timeout
            if self.workspace.documents.contains_key(uri) {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err("Timeout waiting for document indexing")
    }
}
```

3. **Enable Debug Logging in Test**:
```rust
let _ = tracing_subscriber::fmt()
    .with_max_level(tracing::Level::DEBUG)
    .with_test_writer()
    .try_init();
```

Then inspect timing between:
- `did_open` start
- `process_document_blocking` start/end
- Workspace update
- Hover request arrival

### Priority

**Low** - Doesn't affect production usage, only test reliability. Can work around with retry logic in tests.

---

## Issue 3: Line Number Confusion in LSP Tests

**Severity**: Low  
**Component**: Test Infrastructure  
**Status**: Understood, Documentation Issue

### Problem Description

When writing tests, line numbers in hover/goto-definition requests don't match intuition because indented heredocs include leading whitespace as line 0.

### Example

**Source**:
```rust
let source = indoc! {r#"
    /// This is a contract
    contract foo(@x) = {
        Nil
    }"#};
```

**Actual Line Numbering**:
```
Line 0: /// This is a contract
Line 1: contract foo(@x) = {
Line 2:     Nil
Line 3: }
```

**Developer Expected**:
```
Line 0: [empty line after opening r#"]
Line 1: /// This is a contract
Line 2: contract foo(@x) = {
Line 3:     Nil
Line 4: }
```

### Root Cause

The `indoc!` macro removes common leading whitespace but doesn't skip the first line. The opening `r#"` is on the same line as the first content line.

### Solution

**Best Practice for Tests**:
```rust
// Always print line numbers in tests
let text = doc.text().expect("Failed to get document text");
println!("=== DOCUMENT SOURCE ===");
for (i, line) in text.lines().enumerate() {
    println!("Line {}: {}", i, line);
}
println!("======================");
```

Then use actual line numbers from output, not expected ones.

### Priority

**Trivial** - Just a documentation/best practices issue.

---

## Issue 4: Documentation in Completion Items (Future Work)

**Severity**: Enhancement  
**Component**: Completion Provider  
**Status**: Not Yet Implemented

### Feature Request

Show documentation in autocomplete suggestions:

```
contract foo  ← Shows "This is a contract that does something important"
contract bar  ← Shows "This is another contract"
```

### Implementation Approach

1. **Extract Documentation in Symbol Table Builder**:
```rust
// When processing contract declarations
if let Some(doc) = document_ir.doc_comment_before(&contract_pos) {
    symbol.documentation = Some(doc.doc_text());
}
```

2. **Store in Global Symbol Table**:
```rust
pub struct Symbol {
    pub name: String,
    pub symbol_type: SymbolType,
    pub documentation: Option<String>,  // <- Add field
    // ...
}
```

3. **Include in Completion Items**:
```rust
impl CompletionProvider for RholangCompletionProvider {
    fn complete_at(&self, node: &dyn SemanticNode, context: &CompletionContext) 
        -> Vec<CompletionItem> {
        
        // Look up symbols in scope
        let symbols = get_symbols_in_scope(context);
        
        symbols.into_iter().map(|sym| CompletionItem {
            label: sym.name.clone(),
            documentation: sym.documentation.as_ref().map(|doc| {
                Documentation::MarkupContent(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: doc.clone(),
                })
            }),
            // ...
        }).collect()
    }
}
```

### Priority

**Low** - Enhancement, not a bug. Current completion works, just without docs.

---

## Summary of Action Items

| Issue | Priority | Recommended Action | Estimated Effort |
|-------|----------|-------------------|------------------|
| Hover Position Sensitivity | Medium | Implement parent node context in hover | 4-6 hours |
| Test Infrastructure Timing | Low | Add synchronization primitives to tests | 2-3 hours |
| Line Number Confusion | Trivial | Document best practices | 30 minutes |
| Completion Documentation | Low | Extend symbol table with docs | 3-4 hours |

## Related Documentation

- [LSP Integration Summary](./comment_channel_lsp_integration_summary.md)
- [Phase 3: Documentation Extraction](./comment_channel_phase3_summary.md)
- [Unified LSP Architecture](./UNIFIED_LSP_ARCHITECTURE.md)
