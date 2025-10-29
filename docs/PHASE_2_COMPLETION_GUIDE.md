# Phase 2 Completion Guide

**Status**: Phase 2 ~60% Complete
**Date**: 2025-10-29

---

## ✅ Completed Work (Phases 0-2 Part 2)

### Phase 0: WorkspaceState Refactoring ✅
- Lock-free DashMap access
- Commit: `4930dae`

### Phase 1: Language Adapter Traits ✅
- 560 lines of trait definitions
- 4/4 tests passing
- Commit: `ae30aec`

### Phase 2 Part 1: node_finder + GenericGotoDefinition ✅
- 690 lines (node finding + goto-definition)
- 6/6 tests passing
- Commit: `797711a`

### Phase 2 Part 2: GenericHover ✅
- 397 lines (hover tooltips)
- 2/2 tests passing
- Commit: `9f67b69`

**Total So Far**:
- **1,647 lines** of production code
- **12/12 unit tests passing** ✅
- **Zero compilation errors**
- **4 commits** with comprehensive documentation

---

## 📋 Remaining Work to Complete Phase 2

### Part 3: GenericReferences
**Estimated**: 300-400 lines, 2-3 tests, 2-3 hours

**Implementation Pattern** (following GenericGotoDefinition):
```rust
pub struct GenericReferences;

impl GenericReferences {
    pub async fn find_references(
        &self,
        root: &dyn SemanticNode,
        position: &Position,
        uri: &Url,
        adapter: &LanguageAdapter,
        include_declaration: bool,
    ) -> Option<Vec<Location>> {
        // 1. Find node at position
        // 2. Extract symbol name
        // 3. Use adapter.resolver.resolve_symbol() to find definition
        // 4. Use inverted index to find all references
        // 5. Filter by include_declaration
        // 6. Return locations
    }
}
```

**Key Points**:
- Reuses same symbol extraction logic as goto-definition
- Uses `LanguageAdapter.resolver` for symbol resolution
- Returns Vec<Location> instead of GotoDefinitionResponse
- Must handle `include_declaration` parameter

### Part 4: GenericRename
**Estimated**: 350-450 lines, 2-3 tests, 3-4 hours

**Implementation Pattern**:
```rust
pub struct GenericRename;

impl GenericRename {
    pub async fn rename(
        &self,
        root: &dyn SemanticNode,
        position: &Position,
        new_name: &str,
        uri: &Url,
        adapter: &LanguageAdapter,
    ) -> Option<WorkspaceEdit> {
        // 1. Find node at position
        // 2. Extract old symbol name
        // 3. Find all references (reuse GenericReferences)
        // 4. Create TextEdit for each reference
        // 5. Validate rename (language-specific via adapter)
        // 6. Return WorkspaceEdit
    }
}
```

**Key Points**:
- Can reuse `GenericReferences` internally
- Need `RenameValidator` trait in LanguageAdapter (optional)
- Must compute text edits for all occurrences
- Cross-file rename support via WorkspaceEdit

### Part 5: Tests & Documentation
**Estimated**: 1-2 hours

- Write integration tests combining multiple features
- Update `UNIFIED_LSP_PROGRESS.md`
- Update module documentation
- Run full test suite

---

## 🎯 Phase 2 Completion Checklist

- [x] GenericGotoDefinition
- [x] GenericHover
- [ ] GenericReferences
- [ ] GenericRename
- [ ] Integration tests
- [ ] Documentation updates
- [ ] Final commit

**Estimated Time to Complete Phase 2**: 6-8 hours

---

## 🚀 Phase 3 Preview: Language Adapters

Once Phase 2 is complete, Phase 3 will extract language-specific logic:

### RholangAdapter Implementation

**File**: `src/lsp/features/adapters/rholang.rs`

```rust
pub struct RholangHoverProvider;

impl HoverProvider for RholangHoverProvider {
    fn hover_for_symbol(
        &self,
        symbol_name: &str,
        node: &dyn SemanticNode,
        context: &HoverContext,
    ) -> Option<HoverContents> {
        // Extract Rholang-specific symbol info
        let metadata = node.metadata()?;

        // Format Rholang-style hover
        let markdown = format!(
            "**{}** (Rholang channel)\n\n\
            Type: {}\n\
            Scope: {}\n\
            Declared at: {}",
            symbol_name,
            // ... extract from metadata
        );

        Some(HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: markdown,
        }))
    }
}

pub struct RholangCompletionProvider {
    keywords: Vec<&'static str>,
}

impl CompletionProvider for RholangCompletionProvider {
    fn complete_at(&self, node: &dyn SemanticNode, context: &CompletionContext) -> Vec<CompletionItem> {
        let mut items = vec![];

        // Add keywords
        for keyword in &self.keywords {
            items.push(CompletionItem {
                label: keyword.to_string(),
                kind: Some(CompletionItemKind::KEYWORD),
                ..Default::default()
            });
        }

        // Add context-sensitive completions based on node type
        match node.semantic_category() {
            SemanticCategory::Block => {
                // Add process constructors
            }
            _ => {}
        }

        items
    }

    fn keywords(&self) -> &[&str] {
        &["contract", "new", "for", "match", "if", "else"]
    }
}

pub fn create_rholang_adapter(
    symbol_table: Arc<SymbolTable>,
    workspace: Arc<WorkspaceState>,
) -> LanguageAdapter {
    let resolver = Arc::new(ComposableSymbolResolver::new(
        Box::new(LexicalScopeResolver::new(symbol_table, "rholang".to_string())),
        vec![],
        Some(Box::new(AsyncGlobalVirtualSymbolResolver::new(workspace))),
    ));

    LanguageAdapter::new(
        "rholang",
        resolver,
        Arc::new(RholangHoverProvider),
        Arc::new(RholangCompletionProvider::new()),
        Arc::new(RholangDocumentationProvider),
    )
}
```

### MettaAdapter Implementation

**File**: `src/lsp/features/adapters/metta.rs`

Similar structure to RholangAdapter, but with MeTTa-specific:
- Pattern matching in hover (show arity)
- S-expression aware completions
- MeTTa-specific documentation

---

## 📊 Phase 4 Preview: Backend Integration

**File**: `src/lsp/backend/unified_handlers.rs`

```rust
impl RholangBackend {
    /// Store language adapters
    fn new(...) -> Self {
        let rholang_adapter = create_rholang_adapter(symbol_table, workspace.clone());
        let metta_adapter = create_metta_adapter(symbol_table, workspace.clone());

        Self {
            rholang_adapter,
            metta_adapter,
            generic_goto_def: GenericGotoDefinition,
            generic_hover: GenericHover,
            // ...
        }
    }

    /// Unified goto-definition handler
    async fn goto_definition(&self, params: GotoDefinitionParams) -> LspResult<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        // Check if virtual document
        if uri.fragment().is_some() {
            return self.goto_definition_virtual(&uri, &position).await;
        }

        // Get cached document
        let doc = self.workspace.documents.get(&uri)?;

        // Determine language adapter
        let adapter = match doc.language() {
            "rholang" => &self.rholang_adapter,
            "metta" => &self.metta_adapter,
            _ => return Ok(None),
        };

        // Convert LSP position to IR position
        let ir_pos = lsp_to_ir_position(position);

        // Use generic goto-definition
        self.generic_goto_def.goto_definition(
            &doc.ir,
            &ir_pos,
            &uri,
            adapter,
        ).await
    }
}
```

---

## 🎯 Success Metrics

### Code Reduction (Target: 50%)
**Before**: ~2729 lines (handlers.rs: 1711 + metta.rs: 1018)
**After**: ~1200-1400 lines estimated
**Savings**: ~1400 lines (51%)

### Performance (Target: 0-5% overhead)
- Trait dispatch adds ~1-2% overhead
- Lock-free DashMap improvements offset this
- **Net result**: Similar or better performance

### Maintainability
- **Add new language**: 2-3 days (was 2-3 weeks)
- **Fix bug**: Single place instead of N places
- **Add feature**: Implement once, works for all languages

---

## 💡 Recommendations

### Immediate Next Steps
1. **Complete Phase 2** (6-8 hours):
   - Implement GenericReferences
   - Implement GenericRename
   - Write integration tests
   - Update documentation

2. **Proceed to Phase 3** (1-2 weeks):
   - Create RholangAdapter
   - Create MettaAdapter
   - Extract language-specific logic
   - Test adapters work with generic features

3. **Phase 4 Integration** (1-2 weeks):
   - Wire adapters into RholangBackend
   - Create unified dispatch logic
   - Gradually replace old handlers
   - Run comprehensive tests

### Alternative: Proof-of-Concept First
If you want to validate the architecture sooner:
1. Skip remaining Phase 2 features for now
2. Create minimal RholangAdapter (just goto-definition)
3. Integrate in backend and test with real code
4. If successful, complete Phase 2 + full adapters

**Pros**: Validates architecture early with real code
**Cons**: Will need to revisit when adding GenericReferences/Rename

---

## 📈 Progress Summary

```
Phase 0: ✅✅✅✅✅✅✅✅✅✅ 100% Complete
Phase 1: ✅✅✅✅✅✅✅✅✅✅ 100% Complete
Phase 2: ✅✅✅✅✅✅⬜⬜⬜⬜  60% Complete
Phase 3: ⬜⬜⬜⬜⬜⬜⬜⬜⬜⬜   0% Complete
Phase 4: ⬜⬜⬜⬜⬜⬜⬜⬜⬜⬜   0% Complete

Overall: ████████░░░░░░░░░░░░ 40% Complete
```

**Time Invested**: ~12-15 hours
**Time Remaining**: ~20-30 hours
**Expected Completion**: 3-4 weeks

---

## 🔗 Related Documentation

- [UNIFIED_LSP_ARCHITECTURE.md](./UNIFIED_LSP_ARCHITECTURE.md) - Original design
- [UNIFIED_LSP_PROGRESS.md](./UNIFIED_LSP_PROGRESS.md) - Detailed progress tracking
- [EMBEDDED_LANGUAGES_GUIDE.md](./EMBEDDED_LANGUAGES_GUIDE.md) - Language embedding
- [CLAUDE.md](../.claude/CLAUDE.md) - Project overview

---

## 🤝 Next Session Recommendations

When continuing this work:

1. **Start with GenericReferences**:
   - Follow the pattern from GenericGotoDefinition
   - Reuse symbol extraction logic
   - ~2-3 hours of work

2. **Then GenericRename**:
   - Builds on GenericReferences
   - More complex (WorkspaceEdit handling)
   - ~3-4 hours of work

3. **Complete Phase 2**:
   - Integration tests
   - Documentation updates
   - Final commit

4. **Move to Phase 3**:
   - Start with RholangAdapter
   - Test with existing code
   - Then MettaAdapter

**The foundation is solid and well-tested. The remaining work follows established patterns!** 🚀
