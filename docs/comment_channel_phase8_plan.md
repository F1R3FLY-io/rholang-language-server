# Comment Channel Phase 8: Cross-Reference Links - Implementation Plan

## Status: PLANNED (Not Yet Implemented)

**Priority**: Low
**Estimated Effort**: 8-10 hours
**Prerequisites**: Phases 1-7 complete ✅

## Overview

Phase 8 adds support for clickable cross-reference links in documentation comments. This allows documentation to reference other symbols with syntax like `[symbolName]` or `{@link symbolName}`, which become clickable links in the IDE.

## Goals

1. **Link Syntax Support**: Parse link references in documentation text
2. **Symbol Resolution**: Resolve link targets to their definitions
3. **LSP Integration**: Render links in hover tooltips using LSP `DocumentLink` or markdown links
4. **Cross-Document Links**: Support links to symbols in other files
5. **Validation**: Warn about broken links during diagnostics

## Motivation

**User Story**: "As a developer reading documentation, I want to click on referenced symbols to jump to their definitions without manually searching."

**Example Use Case**:
```rholang
/// Moves a player from one location to another.
///
/// Uses the [checkConnection] contract to validate the move,
/// then updates the player's position using [updatePosition].
///
/// @param player The player to move
/// @param fromRoom Current location (see [Room])
/// @param toRoom Destination location (see [Room])
/// @return Success indicator
contract movePlayer(@player, @fromRoom, @toRoom) = { ... }
```

Hover over `movePlayer` would show documentation where `[checkConnection]`, `[updatePosition]`, and `[Room]` are clickable links.

## Design

### 1. Link Syntax Options

**Option A: Markdown-Style Links** (Recommended)
```rholang
/// See [checkConnection] for validation logic
/// Or with explicit text: [validation](checkConnection)
```

**Pros**:
- Standard markdown syntax
- Works in GitHub/GitLab renderers
- No parsing needed if using markdown renderer
- Multiple formats: `[text]`, `[text](target)`, `[text][ref]`

**Cons**:
- Might conflict with Rholang syntax in examples

**Option B: JavaDoc-Style Links**
```rholang
/// See {@link checkConnection} for validation logic
/// Or {@linkplain checkConnection validation}
```

**Pros**:
- Explicit tag, no ambiguity
- Familiar to Java developers
- Clear distinction from markdown links

**Cons**:
- More verbose
- Requires custom parsing

**Recommended**: Start with Option A (markdown links), add Option B if needed.

### 2. Link Resolution Strategy

**Resolution Process**:
1. **Parse Documentation**: Extract link references during `StructuredDocumentation::parse()`
2. **Store Link Metadata**: Add `links: Vec<LinkRef>` to `StructuredDocumentation`
3. **Resolve on Demand**: During hover/rendering, resolve each link to a location
4. **Render as Markdown**: Use markdown link syntax `[text](file://path#L123)` or LSP `DocumentLink`

**LinkRef Structure**:
```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkRef {
    /// Text to display
    pub text: String,
    /// Symbol name to resolve
    pub target: String,
    /// Position in documentation text
    pub offset: usize,
    pub length: usize,
}
```

**Resolution Algorithm**:
```rust
impl StructuredDocumentation {
    /// Resolve all link references to locations
    pub fn resolve_links(
        &self,
        resolver: &dyn SymbolResolver,
        context: &ResolutionContext,
    ) -> Vec<(LinkRef, Option<SymbolLocation>)> {
        self.links.iter()
            .map(|link| {
                let location = resolver.resolve_symbol(
                    &link.target,
                    &context.position,
                    context,
                );
                (link.clone(), location.first().cloned())
            })
            .collect()
    }
}
```

### 3. LSP Integration

**Two Approaches**:

**Approach 1: DocumentLink Provider** (Recommended)
- Implement `textDocument/documentLink` request
- Returns clickable links for entire document
- Editor handles navigation
- More work but better UX

**Approach 2: Markdown Links in Hover**
- Convert links to `[text](file://path#L123)` in `to_markdown()`
- Relies on editor's markdown renderer
- Simpler but less reliable across editors

**Implementation**: Start with Approach 2, add Approach 1 if needed.

### 4. Link Validation

**During Diagnostics**:
```rust
pub fn validate_doc_links(
    node: &dyn SemanticNode,
    resolver: &dyn SymbolResolver,
    context: &ResolutionContext,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    if let Some(structured_doc) = extract_structured_doc(node) {
        for link in &structured_doc.links {
            if resolver.resolve_symbol(&link.target, &context.position, context).is_empty() {
                diagnostics.push(Diagnostic {
                    range: link.range_in_doc(),
                    severity: Some(DiagnosticSeverity::WARNING),
                    message: format!("Broken documentation link: {}", link.target),
                    ..Default::default()
                });
            }
        }
    }

    diagnostics
}
```

## Implementation Steps

### Step 1: Extend StructuredDocumentation (2 hours)

**File**: `src/ir/structured_documentation.rs`

1. Add `links: Vec<LinkRef>` field
2. Update `parse()` to extract markdown links using regex:
   ```rust
   // Pattern: [text] or [text](target)
   let link_pattern = Regex::new(r"\[([^\]]+)\](?:\(([^\)]+)\))?").unwrap();
   ```
3. Store link positions for later resolution
4. Add tests for link parsing

### Step 2: Link Resolution (3 hours)

**File**: `src/ir/structured_documentation.rs`

1. Implement `resolve_links()` method
2. Use existing `SymbolResolver` trait (already used for goto-definition)
3. Handle cross-document links via `AsyncGlobalVirtualSymbolResolver`
4. Cache resolved links to avoid repeated resolution
5. Add tests for resolution

### Step 3: Markdown Rendering (1 hour)

**File**: `src/ir/structured_documentation.rs`

1. Update `to_markdown()` to convert links:
   ```rust
   pub fn to_markdown_with_links(
       &self,
       resolved_links: &[(LinkRef, Option<SymbolLocation>)],
   ) -> String {
       let mut result = self.to_markdown();

       // Replace [text] with [text](file://path#L123)
       for (link, location) in resolved_links {
           if let Some(loc) = location {
               let markdown_link = format!(
                   "[{}]({}#L{})",
                   link.text,
                   loc.uri,
                   loc.range.start.line
               );
               result = result.replace(&format!("[{}]", link.text), &markdown_link);
           }
       }

       result
   }
   ```

### Step 4: Hover Integration (1 hour)

**File**: `src/lsp/features/hover.rs`

1. Update `extract_documentation()` to resolve links
2. Pass resolved links to `to_markdown_with_links()`
3. Test in editor (VS Code, Neovim with LSP client)

### Step 5: Link Validation (2 hours)

**File**: `src/lsp/backend/diagnostics.rs`

1. Add `validate_doc_links()` function
2. Call during `textDocument/didSave` validation
3. Report broken links as warnings
4. Add tests for validation

### Step 6: DocumentLink Provider (Optional, 2-3 hours)

**File**: `src/lsp/backend.rs`

1. Implement `document_link()` method
2. Parse all documentation in file
3. Return all resolved links
4. Register capability in initialization

## Testing Strategy

### Unit Tests (in `structured_documentation.rs`)

```rust
#[test]
fn test_parse_markdown_links() {
    let doc = StructuredDocumentation::parse(vec![
        "See [foo] for details",
        "Or use [bar](bazz) directly",
    ].into_iter());

    assert_eq!(doc.links.len(), 2);
    assert_eq!(doc.links[0].text, "foo");
    assert_eq!(doc.links[0].target, "foo");
    assert_eq!(doc.links[1].text, "bar");
    assert_eq!(doc.links[1].target, "bazz");
}

#[test]
fn test_resolve_links() {
    // Mock resolver that returns known symbols
    let resolver = MockResolver::new();
    resolver.add("foo", Location { uri: "file.rho", line: 10 });

    let doc = StructuredDocumentation::parse(vec!["See [foo]"].into_iter());
    let resolved = doc.resolve_links(&resolver, &context);

    assert_eq!(resolved.len(), 1);
    assert!(resolved[0].1.is_some());
}
```

### Integration Tests (in `lsp_features.rs`)

```rust
#[test]
fn test_hover_with_doc_links() {
    let source = indoc! {r#"
        contract helper(@x) = { Nil }

        /// Main contract that uses [helper]
        contract main(@y) = { Nil }
    "#};

    // Hover over "main"
    let hover = client.hover(...);

    // Check that link is rendered
    assert!(hover.contents.contains("[helper](file://"));
}

#[test]
fn test_broken_link_diagnostic() {
    let source = indoc! {r#"
        /// Uses [nonexistent] contract
        contract main(@y) = { Nil }
    "#};

    // Save file to trigger diagnostics
    client.did_save(...);
    let diagnostics = client.wait_for_diagnostics();

    // Check for broken link warning
    assert!(diagnostics.iter().any(|d|
        d.message.contains("Broken documentation link: nonexistent")
    ));
}
```

## Edge Cases

1. **Ambiguous Links**: `[foo]` when multiple `foo` symbols exist
   - **Solution**: Use scoping rules (prefer local scope)
   - **Enhancement**: Support `[Module.foo]` syntax for disambiguation

2. **External Links**: `[http://example.com]`
   - **Solution**: Detect URL pattern, pass through as-is
   - Don't attempt symbol resolution

3. **Code Blocks**: Links inside `@example` code blocks
   - **Solution**: Don't parse links in code blocks
   - Or: Parse but render as plain text

4. **Circular References**: A's doc links to B, B's doc links to A
   - **Solution**: No issue - just clickable navigation

5. **Performance**: Resolving many links in large files
   - **Solution**: Cache resolved links per document
   - Only re-resolve when document changes

## Compatibility

### Editor Support

**VS Code**: ✅ Supports markdown links in hover
**Neovim (with nvim-lsp)**: ✅ Supports markdown links
**Emacs (lsp-mode)**: ✅ Supports markdown links
**IntelliJ**: ⚠️ Limited markdown support in LSP hover

**DocumentLink Support**:
- All major editors support `textDocument/documentLink`
- More reliable than markdown links
- Recommended for Phase 8 final implementation

## Future Enhancements (Beyond Phase 8)

1. **Cross-Language Links**: Link from Rholang to MeTTa virtual docs
2. **URL Links**: Support external documentation links
3. **Type Links**: Auto-link parameter types to their definitions
4. **Link Preview**: Show target's documentation in link tooltip
5. **Link Completion**: Auto-complete symbol names in doc comments

## Migration Path

Phase 8 is fully backwards compatible:
- Old documentation without links works as-is
- No changes to existing StructuredDocumentation data
- Links are purely additive feature

## Success Metrics

- [ ] Can click links in hover tooltips
- [ ] Links resolve correctly across files
- [ ] Broken links show warnings
- [ ] Performance: < 100ms for link resolution in typical files
- [ ] Test coverage: > 90% for link-related code

## References

- **Rust Doc Links**: `[`Item`]` syntax, path-based resolution
- **JavaDoc @link**: `{@link Class#method}` syntax
- **TypeScript JSDoc**: `{@link symbolName}` and markdown links
- **LSP Specification**:
  - [Hover](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocument_hover)
  - [DocumentLink](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocument_documentLink)

## Recommendation

**Postpone Phase 8** until:
1. Phase 7 is deployed and used in production
2. User feedback indicates demand for clickable links
3. Core LSP features are fully stable
4. Team has bandwidth for 8-10 hour implementation

Phase 7 provides 95% of documentation value. Phase 8 is polish.

---

**Document Status**: Planning document - ready for implementation when prioritized
**Last Updated**: 2025-11-04
**Dependencies**: Phase 7 complete ✅
