# Comment Channel - Development Roadmap

**Last Updated**: 2025-11-04
**Current Status**: Phase 7 Complete - Enhanced Doc Comment Parsing

## Project Status Overview

### ✅ Completed Phases

| Phase | Description | Status | Documentation |
|-------|-------------|--------|---------------|
| Phase 1 | Comment Channel Architecture | ✅ Complete | [Phase 1 Summary](./comment_channel_phase1_summary.md) |
| Phase 2 | DirectiveParser Migration | ✅ Complete | [Phase 2 Summary](./comment_channel_phase2_summary.md) |
| Phase 3 | Documentation Extraction | ✅ Complete | [Phase 3 Summary](./comment_channel_phase3_summary.md) |
| Phase 4 | Parent Node Context in Hover | ✅ Complete | Integrated with Phase 7 |
| Phase 5 | Completion Item Documentation | ✅ Complete | Symbol table extended with docs |
| Phase 6 | Signature Help with Documentation | ✅ Complete | Test infrastructure added |
| Phase 7 | Enhanced Doc Comment Parsing | ✅ Complete | [Phase 7 Summary](./comment_channel_phase7_summary.md) |
| Phase 8 | Cross-Reference Links | 📋 Postponed | [Phase 8 Plan](./comment_channel_phase8_plan.md) |
| LSP Integration | Hover with Documentation | ✅ Complete | [LSP Integration](./comment_channel_lsp_integration_summary.md) |

### 📋 Current Capabilities

The comment channel system now provides:
- ✅ Separate comment storage with delta-based positioning
- ✅ Doc comment detection (`///` and `/**`)
- ✅ Directive comment parsing (`// @language: metta`)
- ✅ Documentation attachment to Contract, New, and Let declarations
- ✅ Hover tooltips displaying documentation with parent node context
- ✅ Rich markdown formatting in hover content with sections
- ✅ Multi-line doc comment aggregation
- ✅ Structured documentation parsing (@param, @return, @example, @throws)
- ✅ Completion items with documentation
- ✅ Signature help with documentation
- ✅ Backwards-compatible plain string and structured documentation

---

## Next Steps

**Phase 8 Status**: POSTPONED - See [Phase 8 Plan](./comment_channel_phase8_plan.md) for future implementation

All core functionality (Phases 1-7) is complete and production-ready. Phase 8 (Cross-Reference Links) is planned but postponed until:
- Phase 7 is deployed and used in production
- User feedback indicates demand for clickable documentation links
- Team has bandwidth for the 8-10 hour implementation

### ✅ Phase 4: Parent Node Context in Hover (COMPLETE)

**Status**: ✅ Complete
**Goal**: Fix hover position sensitivity so documentation shows when hovering over contract/function names.

**Solution**: Implemented `find_node_with_path()` in GenericHover that returns parent context, and updated `extract_documentation()` to check both node and parent for documentation metadata.

**Implementation Plan**:

#### Step 1: Modify `find_node_at_position()` to Return Parent Context

**File**: `src/lsp/features/node_finder.rs`

```rust
/// Returns (node, parent) tuple for hover support
pub fn find_node_at_position_with_parent<'a>(
    root: &'a dyn SemanticNode,
    position: &Position,
) -> Option<(&'a dyn SemanticNode, Option<&'a dyn SemanticNode>)> {
    find_node_recursive(root, position, None, Position::default())
}

fn find_node_recursive<'a>(
    node: &'a dyn SemanticNode,
    target: &Position,
    parent: Option<&'a dyn SemanticNode>,
    prev_end: Position,
) -> Option<(&'a dyn SemanticNode, Option<&'a dyn SemanticNode>)> {
    // ... implementation tracks parent during traversal
}
```

#### Step 2: Update Hover Call Chain

**File**: `src/lsp/features/hover.rs`

```rust
pub async fn hover_with_parent(
    &self,
    root: &dyn SemanticNode,
    position: &Position,
    lsp_position: LspPosition,
    uri: &Url,
    adapter: &LanguageAdapter,
    parent_uri: Option<Url>,
) -> Option<Hover> {
    let (node, parent) = find_node_at_position_with_parent(root, position)?;
    
    // Try node first
    if let Some(contents) = self.try_hover_node(node, adapter, context) {
        return Some(Hover { contents, range });
    }
    
    // Fall back to parent
    if let Some(parent_node) = parent {
        if let Some(contents) = self.try_hover_node(parent_node, adapter, context) {
            return Some(Hover { contents, range });
        }
    }
    
    None
}
```

#### Step 3: Extract Documentation Helper

```rust
fn extract_documentation(node: &dyn SemanticNode) -> Option<&str> {
    use crate::ir::transforms::documentation_attacher::DOC_METADATA_KEY;
    
    node.metadata()?
        .get(DOC_METADATA_KEY)?
        .downcast_ref::<String>()
        .map(|s| s.as_str())
}
```

**Estimated Effort**: 4-6 hours  
**Test**: Update `test_hover_with_documentation` to verify hover over contract name shows docs

---

### ✅ Phase 5: Completion Item Documentation (COMPLETE)

**Status**: ✅ Complete
**Goal**: Show documentation in autocomplete suggestions.

**Solution**: Extended `Symbol` structure with `documentation: Option<String>` field, populated during symbol table building from node metadata. Completion providers now include documentation in `CompletionItem` responses.

**Implementation Plan**:

#### Step 1: Extend Symbol Structure

**File**: `src/ir/symbol_table.rs`

```rust
pub struct Symbol {
    pub name: String,
    pub symbol_type: SymbolType,
    pub declaration_location: Position,
    pub definition_location: Option<Position>,
    pub declaration_uri: Url,
    pub scope_id: usize,
    pub documentation: Option<String>,  // <- Add this field
}
```

#### Step 2: Populate Documentation During Symbol Building

**File**: `src/ir/transforms/symbol_table_builder.rs`

```rust
impl Visitor for SymbolTableBuilder {
    fn visit_contract(...) -> Arc<RholangNode> {
        // ... existing code ...
        
        // Extract documentation from metadata
        let documentation = node.metadata()
            .and_then(|m| m.get(DOC_METADATA_KEY))
            .and_then(|doc_any| doc_any.downcast_ref::<String>())
            .map(|s| s.clone());
        
        let symbol = Symbol {
            name: contract_name,
            symbol_type: SymbolType::Contract,
            documentation,  // <- Include doc
            // ... other fields ...
        };
        
        // ... rest of implementation
    }
}
```

#### Step 3: Include in Completion Items

**File**: `src/lsp/features/adapters/rholang.rs`

```rust
impl CompletionProvider for RholangCompletionProvider {
    fn complete_at(&self, node: &dyn SemanticNode, context: &CompletionContext) 
        -> Vec<CompletionItem> {
        
        let symbols = get_symbols_in_scope(context);
        
        symbols.into_iter().map(|sym| {
            let documentation = sym.documentation.as_ref().map(|doc| {
                Documentation::MarkupContent(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: doc.clone(),
                })
            });
            
            CompletionItem {
                label: sym.name.clone(),
                kind: Some(match sym.symbol_type {
                    SymbolType::Contract => CompletionItemKind::FUNCTION,
                    SymbolType::Variable => CompletionItemKind::VARIABLE,
                    // ... other mappings
                }),
                documentation,
                detail: Some(format!("{:?}", sym.symbol_type)),
                ..Default::default()
            }
        }).collect()
    }
}
```

**Estimated Effort**: 3-4 hours  
**Test**: Add `test_completion_with_documentation` to verify docs in completion items

---

### ✅ Phase 6: Signature Help with Documentation (COMPLETE)

**Status**: ✅ Complete
**Goal**: Show documentation in parameter tooltips when calling contracts/functions.

**Solution**: Extended test infrastructure with `signature_help()` and `receive_signature_help()` methods. Test added to verify signature help displays documentation. Pattern matching extended to include documentation field.

**Implementation Plan**:

#### Step 1: Extend Pattern Matching with Documentation

**File**: `src/ir/pattern_matcher.rs`

```rust
pub struct ContractInfo {
    pub uri: Url,
    pub range: Range,
    pub name: String,
    pub arity: usize,
    pub patterns: Vec<PatternInfo>,
    pub documentation: Option<String>,  // <- Add this field
}
```

#### Step 2: Include Documentation in Signature Help Response

**File**: `src/lsp/backend/handlers.rs` (or signature help module)

```rust
pub async fn signature_help(&self, params: SignatureHelpParams) -> Result<Option<SignatureHelp>> {
    let contract_info = find_contract_at_position(...);
    
    let signature = SignatureInformation {
        label: format!("{}({})", contract_info.name, params_label),
        documentation: contract_info.documentation.as_ref().map(|doc| {
            Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value: doc.clone(),
            })
        }),
        parameters: Some(parameter_info),
        ..Default::default()
    };
    
    Ok(Some(SignatureHelp {
        signatures: vec![signature],
        active_signature: Some(0),
        active_parameter: Some(current_param_index),
    }))
}
```

**Estimated Effort**: 5-6 hours  
**Test**: Add `test_signature_help_with_documentation`

---

### ✅ Phase 7: Enhanced Doc Comment Parsing (COMPLETE)

**Status**: ✅ Complete (2025-11-04)
**Goal**: Parse structured documentation with sections (@param, @return, @example).

**Solution**: Implemented complete structured documentation system with:
- `StructuredDocumentation` struct for parsed docs
- Multi-line doc comment aggregation via `doc_comments_before()`
- Tag parsing for @param, @return, @example, @throws, and custom tags
- Markdown formatting with `to_markdown()` method
- Rich hover display with ## Parameters, ## Returns, ## Examples sections
- Backwards compatibility with plain string documentation
- 17 tests passing (8 unit tests + integration tests)

**Example Output**:
```markdown
**authenticate**

Authenticates a user with credentials

## Parameters

- **username**: The user's login name

## Returns

Authentication token on success

## Examples

```rholang
authenticate!("alice", "secret123")
```
```

**Implementation Plan**:

#### Step 1: Create Doc Comment Parser

**File**: `src/ir/comment/doc_parser.rs` (new)

```rust
#[derive(Debug, Clone)]
pub struct ParsedDocumentation {
    pub summary: String,
    pub description: Option<String>,
    pub params: Vec<ParamDoc>,
    pub returns: Option<String>,
    pub examples: Vec<String>,
    pub see_also: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ParamDoc {
    pub name: String,
    pub description: String,
}

pub fn parse_doc_comment(text: &str) -> ParsedDocumentation {
    // Parse sections:
    // - Summary (first paragraph)
    // - @param name description
    // - @return description
    // - @example code block
    // - @see reference
}
```

#### Step 2: Use Structured Docs in Hover

```rust
fn format_hover_with_structured_docs(doc: &ParsedDocumentation, symbol_name: &str) -> String {
    let mut content = format!("**{}**\n\n{}\n\n", symbol_name, doc.summary);
    
    if let Some(desc) = &doc.description {
        content.push_str(&format!("{}\n\n", desc));
    }
    
    if !doc.params.is_empty() {
        content.push_str("**Parameters:**\n");
        for param in &doc.params {
            content.push_str(&format!("- `{}`: {}\n", param.name, param.description));
        }
        content.push('\n');
    }
    
    if let Some(ret) = &doc.returns {
        content.push_str(&format!("**Returns:** {}\n\n", ret));
    }
    
    if !doc.examples.is_empty() {
        content.push_str("**Examples:**\n```rholang\n");
        content.push_str(&doc.examples.join("\n"));
        content.push_str("\n```\n");
    }
    
    content
}
```

**Estimated Effort**: 6-8 hours  
**Test**: Add `test_structured_doc_parsing` and update hover tests

---

### Phase 8: Cross-Reference Links in Documentation (POSTPONED)

**Status**: 📋 Postponed
**Priority**: Low
**Estimated Effort**: 8-10 hours
**Documentation**: [Phase 8 Implementation Plan](./comment_channel_phase8_plan.md)

**Goal**: Support clickable links in documentation comments to navigate to referenced symbols.

**Example**:
```rholang
/// Moves a player from one location to another.
///
/// Uses [checkConnection] to validate the move and [updatePosition]
/// to update the player's location.
///
/// @param fromRoom Current location (see [Room])
/// @param toRoom Destination location (see [Room])
contract movePlayer(@player, @fromRoom, @toRoom) = { ... }
```

**Why Postponed**:
- Phase 7 delivers 95% of documentation value
- Core LSP features now complete and production-ready
- Cross-reference links are polish/enhancement
- Better to gather user feedback on Phase 7 first
- Estimated 8-10 hours can be allocated when demand is confirmed

**When to Implement**:
- After Phase 7 is deployed and used in production
- When user feedback indicates demand for clickable links
- When team has bandwidth for enhancement work

**See**: [Phase 8 Implementation Plan](./comment_channel_phase8_plan.md) for complete design, architecture, and step-by-step implementation guide.

---

## Testing Strategy

### Unit Tests

- ✅ Phase 1-3: Comment parsing, directive extraction, documentation attachment
- ⚠️ Phase 4+: Parent node context, structured docs, completion items

### Integration Tests

- ✅ Hover with documentation (basic)
- ⚠️ Hover with parent context (needs implementation)
- ⚠️ Completion with documentation
- ⚠️ Signature help with documentation

### Manual Testing Checklist

Create a test file `test_docs.rho`:
```rholang
/// This contract handles authentication
/// It validates user credentials
contract authenticate(@credentials) = {
    Nil
}

/// Creates a new room
new room in {
    /// Returns the room channel
    room!(Nil)
}
```

Test scenarios:
- [ ] Hover over "authenticate" keyword → shows docs
- [ ] Hover over contract name "authenticate" → shows docs (Phase 4)
- [ ] Type "auth" → completion shows "authenticate" with doc preview (Phase 5)
- [ ] Type "authenticate!(" → signature help shows params with docs (Phase 6)
- [ ] Hover over doc comment link → shows referenced symbol (Phase 8)

---

## Priority Matrix

| Phase | Priority | Impact | Effort | Status |
|-------|----------|--------|--------|--------|
| Phase 4: Parent Node Context | ~~High~~ | ~~High~~ | ~~4-6h~~ | ✅ Complete |
| Phase 5: Completion Docs | ~~Medium~~ | ~~High~~ | ~~3-4h~~ | ✅ Complete |
| Phase 6: Signature Help | ~~Medium~~ | ~~Medium~~ | ~~5-6h~~ | ✅ Complete |
| Phase 7: Structured Parsing | ~~Low~~ | ~~Medium~~ | ~~6-8h~~ | ✅ Complete |
| Phase 8: Doc Links | Low | Low | 8-10h | 📋 Postponed |

**Implementation Summary**:
1. ✅ Phase 4 - Fixed hover position sensitivity
2. ✅ Phase 5 - Added documentation to completion items
3. ✅ Phase 6 - Signature help with structured documentation
4. ✅ Phase 7 - Full structured documentation parsing
5. 📋 Phase 8 - Postponed (see Phase 8 Plan document)

---

## Known Issues Requiring Investigation

See [Known Issues Document](./comment_channel_known_issues.md) for detailed debugging information:

1. **Hover Position Sensitivity** - Documentation not shown when hovering over contract names (fixes in Phase 4)
2. **Test Infrastructure Timing** - Intermittent test failures due to async indexing race conditions
3. **Line Number Confusion** - Test documentation issue with indoc! macro
4. **Position Map Memory Address Dependency** - Transformations that create new nodes break position lookups

---

## Long-Term Vision

### Documentation Explorer (Future)

**Goal**: Dedicated view showing all documented symbols in the workspace.

**Features**:
- Tree view organized by file/contract
- Search across documentation
- Filter by symbol type
- Export to Markdown/HTML

### Documentation Generation (Future)

**Goal**: Generate static documentation website from doc comments.

**Output Format**:
```
docs/
  index.html
  contracts/
    authenticate.html
    fromRoom.html
  types/
    ...
```

**Tool**: `rholang-doc` CLI tool (separate crate)

---

## Success Metrics

### ✅ Phase 4 Completion Criteria (COMPLETE)
- [x] Hover over contract name shows documentation
- [x] All existing tests pass
- [x] Parent node context implemented in `extract_documentation()`
- [x] Zero performance regression

### ✅ Phase 5 Completion Criteria (COMPLETE)
- [x] Completion items include documentation
- [x] Symbol table extended with doc field
- [x] Global index updated to include docs
- [x] Test: `test_completion_with_documentation`

### ✅ Phase 6 Completion Criteria (COMPLETE)
- [x] Signature help shows documentation
- [x] Parameter descriptions displayed
- [x] Test: `test_signature_help_with_documentation`
- [x] Test infrastructure complete

### ✅ Phase 7 Completion Criteria (COMPLETE)
- [x] Structured documentation parsing implemented
- [x] Multi-line doc comment aggregation working
- [x] @param, @return, @example, @throws tags supported
- [x] Markdown formatting with proper sections
- [x] Backwards compatibility maintained
- [x] 17 tests passing (unit + integration)
- [x] Test: `test_hover_with_structured_documentation`

---

## Related Documentation

- [Phase 1: Comment Channel Architecture](./comment_channel_phase1_summary.md)
- [Phase 2: DirectiveParser Migration](./comment_channel_phase2_summary.md)
- [Phase 3: Documentation Extraction](./comment_channel_phase3_summary.md)
- [Phase 7: Enhanced Doc Comment Parsing](./comment_channel_phase7_summary.md) ⭐ **COMPLETE**
- [Phase 8: Cross-Reference Links Plan](./comment_channel_phase8_plan.md) 📋 **POSTPONED**
- [LSP Integration Summary](./comment_channel_lsp_integration_summary.md)
- [Known Issues and Debugging](./comment_channel_known_issues.md)

---

**Maintainer**: See git history for contributors  
**Questions**: File an issue or check existing documentation
