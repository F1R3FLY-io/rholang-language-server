# Complete Refactoring Roadmap

## Overview

This document provides a comprehensive roadmap for refactoring the Rholang Language Server codebase, breaking down large monolithic files into smaller, maintainable modules while building support for virtual languages (embedded MeTTa, SQL, etc.).

## Current Status

### Completed Refactorings ✅

1. **pretty_printer.rs** (2,279 lines → 3 modules)
   - ✅ Refactored into `mod.rs`, `json_formatters.rs`, `printer.rs`
   - ✅ All 41 tests passing
   - ✅ Commit: 009e53a

2. **rholang_node.rs** (5,494 lines → 5 modules)
   - ✅ Refactored into `node_types.rs`, `position_tracking.rs`, `node_operations.rs`, `node_impl.rs`, `mod.rs`
   - ✅ All 204 tests passing
   - ✅ Commit: 3eddb66

### Planned Refactorings 📋

3. **tree_sitter.rs** (1,493 lines → 9 modules)
   - 📋 Status: Plan complete, ready for implementation
   - 📋 Document: `docs/TREE_SITTER_REFACTORING_PLAN.md`

4. **visitor.rs** (1,601 lines → 8 modules)
   - 📋 Status: Plan complete, ready for implementation
   - 📋 Document: `docs/VISITOR_REFACTORING_PLAN.md`

5. **backend.rs** (3,495 lines → 10 modules + virtual language system)
   - 📋 Status: Architecture complete, ready for implementation
   - 📋 Documents:
     - `docs/BACKEND_REFACTORING_PLAN_REVISED.md` (main strategy)
     - `docs/VIRTUAL_LANGUAGE_ARCHITECTURE.md` (generic system)
     - `docs/VIRTUAL_LANGUAGE_EXTENSION_SYSTEM.md` (extension trait)
     - `docs/VIRTUAL_LANGUAGE_UNIFIED_IR_INTEGRATION.md` (IR integration)

---

## Refactoring Order and Dependencies

### Phase 1: Foundation Modules (COMPLETED ✅)

**Files:** `pretty_printer.rs`, `rholang_node.rs`

**Why first:**
- Lower risk (no async complexity)
- Clear boundaries
- Established patterns

**Results:**
- ✅ 15% reduction in largest file (pretty_printer)
- ✅ 90% reduction in largest file (rholang_node: 5,494 → ~500 lines)
- ✅ All tests passing

---

### Phase 2: Parsing Infrastructure (READY 📋)

**File:** `tree_sitter.rs` (1,493 lines)

**Module Structure:**
```
src/tree_sitter/
├── mod.rs                      # Public API (60 lines)
├── parsing.rs                  # Tree-Sitter interface (50 lines)
├── helpers.rs                  # Collection helpers (150 lines)
├── conversion/
│   ├── mod.rs                  # Dispatcher (100 lines)
│   ├── processes.rs            # (300 lines)
│   ├── control_flow.rs         # (180 lines)
│   ├── expressions.rs          # (200 lines)
│   ├── literals.rs             # (120 lines)
│   ├── collections.rs          # (180 lines)
│   ├── bindings.rs             # (250 lines)
│   └── patterns.rs             # (100 lines)
└── tests/
    ├── mod.rs                  # (20 lines)
    ├── parsing_tests.rs        # (150 lines)
    ├── position_tests.rs       # (120 lines)
    └── conversion_tests.rs     # (120 lines)
```

**Why now:**
- Critical for virtual language support (template pattern)
- No dependencies on backend refactoring
- Establishes conversion pattern for other languages

**Estimated effort:** 2 hours

**Benefits:**
- Template for MeTTa, SQL parsers
- Clear conversion boundaries
- Better testing per construct type

---

### Phase 3: Transform Infrastructure (READY 📋)

**File:** `visitor.rs` (1,601 lines)

**Module Structure:**
```
src/ir/visitor/
├── mod.rs                      # Trait definition (80 lines)
├── processes.rs                # ProcessVisitor (400 lines)
├── control_flow.rs             # ControlFlowVisitor (250 lines)
├── expressions.rs              # ExpressionVisitor (300 lines)
├── literals.rs                 # LiteralVisitor (200 lines)
├── collections.rs              # CollectionVisitor (250 lines)
├── bindings.rs                 # BindingVisitor (320 lines)
└── patterns.rs                 # PatternVisitor (200 lines)
```

**Why now:**
- Matches tree_sitter module structure (consistency)
- Used by backend transforms
- Template for MettaVisitor, SqlVisitor

**Estimated effort:** 2 hours

**Benefits:**
- Template for virtual language visitors
- Clear visitor boundaries
- Consistent with conversion modules

---

### Phase 4: Backend and Virtual Language System (READY 📋)

**File:** `backend.rs` (3,495 lines)

**Module Structure:**
```
src/lsp/backend/
├── mod.rs                           # Coordinator (150 lines)
├── state.rs                         # RholangBackend struct (200 lines)
├── utils.rs                         # SemanticTokensBuilder (100 lines)
├── lifecycle.rs                     # Initialization (300 lines)
├── document_processing.rs           # Document workflow (400 lines)
├── workspace.rs                     # Multi-file operations (300 lines)
├── symbol_operations.rs             # Rholang symbols (350 lines)
├── virtual_language_extension.rs    # Extension trait (150 lines)
├── virtual_language_support.rs      # Generic + handlers (600 lines)
└── lsp_handlers.rs                  # LanguageServer impl (800 lines)
```

**Virtual Language Components:**
```
src/language_regions/               # (already exists)
├── directive_parser.rs             # Language region detection
├── semantic_detector.rs            # Semantic-based detection
├── channel_flow_analyzer.rs        # Channel flow analysis
└── virtual_document_registry.rs    # Document management

src/virtual_languages/              # (new)
├── mod.rs                          # Extension registry
├── extension_trait.rs              # VirtualLanguageExtension
└── metta/
    ├── mod.rs
    ├── extension.rs                # MettaExtension impl
    ├── hover.rs                    # MeTTa hover support
    ├── goto_definition.rs          # MeTTa navigation
    └── diagnostics.rs              # MeTTa validation

src/grammars/                       # (new)
├── metta/
│   ├── parser.so                   # Tree-Sitter grammar
│   └── queries/
│       ├── highlights.scm
│       ├── definitions.scm
│       ├── references.scm
│       └── diagnostics.scm
└── sql/
    └── ... (future)
```

**Why last:**
- Depends on tree_sitter refactoring (parsing template)
- Depends on visitor refactoring (transform template)
- Most complex integration
- Highest impact

**Estimated effort:** 4-6 hours

**Benefits:**
- Generalized virtual language support
- Extension system for specialized features
- Unified IR integration
- Template for adding new languages

---

## Consistent Module Structure

All refactorings follow the **same module categorization**:

| Category       | tree_sitter/conversion/ | visitor/         | Purpose                        |
|----------------|-------------------------|------------------|--------------------------------|
| Processes      | processes.rs            | processes.rs     | Send, New, Input, Block        |
| Control Flow   | control_flow.rs         | control_flow.rs  | IfElse, Match, Choice, Bundle  |
| Expressions    | expressions.rs          | expressions.rs   | BinOp, UnaryOp, Method, Quote  |
| Literals       | literals.rs             | literals.rs      | Bool, Long, String, Uri, Nil   |
| Collections    | collections.rs          | collections.rs   | List, Set, Map, Tuple          |
| Bindings       | bindings.rs             | bindings.rs      | Contract, Let, LinearBind      |
| Patterns       | patterns.rs             | patterns.rs      | Var, Wildcard, SimpleType      |

**This consistency means:**
- Easy to navigate: "Where's the Send logic?" → processes module in both places
- Template for new languages: MeTTa follows the same structure
- Clear mental model: conversion + visitor for each category

---

## Virtual Language Architecture

### Three-Tier System

**Tier 1: Tree-Sitter Only (Zero Config)**
- Add grammar + queries
- Instant LSP features (hover, goto-definition, etc.)
- No Rust code required
- Example: Adding SQL support

**Tier 2: Extension + Tree-Sitter (Enhanced)**
- Implement VirtualLanguageExtension trait
- Override specific LSP methods
- Fallback to Tree-Sitter for others
- Example: MeTTa with specialized hover

**Tier 3: Full Compiler Integration (Advanced)**
- Extension + Full compiler (e.g., Mettatron)
- Advanced semantic analysis
- Type checking, inference
- Example: MeTTa with Mettatron

### Key Components

**1. VirtualLanguageExtension Trait**
```rust
#[async_trait]
pub trait VirtualLanguageExtension: Send + Sync {
    fn language(&self) -> &str;

    // All optional - return None to fallback to Tree-Sitter
    async fn hover(...) -> Option<Hover> { None }
    async fn goto_definition(...) -> Option<GotoDefinitionResponse> { None }
    async fn diagnostics(...) -> Option<Vec<Diagnostic>> { None }

    // Unified IR translation
    async fn to_unified_ir(&self, doc: &VirtualDocument) -> Option<Arc<UnifiedIR>>;

    fn capabilities(&self) -> ExtensionCapabilities;
    fn ir_capabilities(&self) -> IRCapabilities;
}
```

**2. Hybrid Handler Pattern**
```rust
async fn hover_virtual_document(...) -> LspResult<Option<Hover>> {
    // Step 1: Try specialized extension
    if let Some(extension) = self.extension_registry.get(language) {
        if let Some(hover) = extension.hover(doc, position).await {
            return Ok(Some(hover));  // Use extension
        }
    }

    // Step 2: Fallback to generic Tree-Sitter
    self.hover_virtual_generic(doc, position).await
}
```

**3. Unified IR Integration**
- **Simple path:** Tree-Sitter CST → UnifiedIR (direct)
- **Complex path:** Tree-Sitter CST → LanguageIR → UnifiedIR (two-phase)

This enables:
- Cross-language symbol resolution
- Unified type system
- Cross-language goto-definition
- Language interoperability

---

## Implementation Timeline

### Week 1: Parsing Infrastructure
- **Day 1-2:** tree_sitter.rs refactoring
  - Extract helpers and parsing
  - Extract conversion modules
  - Test after each module

### Week 2: Transform Infrastructure
- **Day 3-4:** visitor.rs refactoring
  - Extract trait modules
  - Test trait composition
  - Verify existing visitors work

### Week 3-4: Backend and Virtual Languages
- **Day 5-7:** backend.rs refactoring
  - Extract core modules
  - Extract LSP handlers
  - Test incrementally
- **Day 8-10:** Virtual language system
  - Implement extension trait
  - Create generic Tree-Sitter handlers
  - Integrate MeTTa extension
  - Test end-to-end

---

## Testing Strategy

### Per-Phase Testing
After each module extraction:
```bash
cargo check           # Quick validation
cargo test <module>   # Module-specific tests
cargo test            # Full test suite
```

### Critical Test Suites
- **IR tests:** Verify position tracking, node construction
- **Transform tests:** Symbol tables, document symbols
- **LSP tests:** All LSP features (hover, goto-def, etc.)
- **Integration tests:** End-to-end workflows
- **Virtual language tests:** MeTTa support, cross-file navigation

### Regression Prevention
- Run full test suite after each phase
- Test performance-critical paths
- Verify LSP protocol compliance

---

## Risk Management

### Low Risk Components ✅
- Helper function extraction
- Test reorganization
- Simple module splits

### Medium Risk Components ⚠️
- Trait composition (visitor)
- Main dispatcher functions
- Public API changes

### High Risk Components 🔴
- Backend async trait implementation
- Virtual language integration
- Cross-language symbol resolution

### Mitigation Strategies
1. **Incremental approach:** One module at a time
2. **Test after each step:** Catch regressions early
3. **Git commits:** Easy rollback if needed
4. **Backward compatibility:** Preserve public APIs
5. **Documentation:** Clear migration guides

---

## Success Metrics

### Code Quality Metrics
- ✅ All files < 500 lines (maintainability threshold)
- ✅ No duplicate code (DRY principle)
- ✅ Clear module boundaries (single responsibility)
- ✅ Consistent naming conventions

### Functional Metrics
- ✅ All 204+ tests passing
- ✅ No performance regression
- ✅ LSP features working correctly
- ✅ Virtual language support functional

### Developer Experience Metrics
- ✅ Easy to add new language constructs
- ✅ Clear where to find specific code
- ✅ Simple to add new languages (< 30 minutes for basic support)
- ✅ Good error messages and debugging

---

## Future Roadmap

### Phase 5: Additional Languages (Future)
- SQL support (Tier 1: Tree-Sitter only)
- Python support (Tier 1: Tree-Sitter only)
- JavaScript support (Tier 1: Tree-Sitter only)

### Phase 6: Advanced Features (Future)
- Cross-language type inference
- Unified symbol table across languages
- Cross-language refactoring support
- Language interop analysis

### Phase 7: Performance Optimization (Future)
- Incremental parsing for virtual documents
- Cached query results
- Parallel parsing for multiple virtual documents

---

## Documentation Updates

### User Documentation
- Update README.md with new architecture
- Add virtual language guide
- Document extension system

### Developer Documentation
- Module structure guide
- Contributing guide for new languages
- Architecture decision records (ADRs)

### API Documentation
- Update rustdoc comments
- Add code examples
- Document public APIs

---

## Conclusion

This refactoring roadmap provides:

1. **Clear Path:** Phased approach with defined dependencies
2. **Consistent Structure:** Same module organization across files
3. **Extensibility:** Easy to add new languages
4. **Quality:** Better testing, maintainability, and navigation
5. **Innovation:** Virtual language support with Unified IR

**Next Steps:**
1. ✅ Complete tree_sitter.rs refactoring (Phase 2)
2. ✅ Complete visitor.rs refactoring (Phase 3)
3. ✅ Complete backend.rs refactoring + virtual language system (Phase 4)

---

## References

- `docs/TREE_SITTER_REFACTORING_PLAN.md` - Tree-Sitter parsing refactoring
- `docs/VISITOR_REFACTORING_PLAN.md` - Visitor pattern refactoring
- `docs/BACKEND_REFACTORING_PLAN_REVISED.md` - Backend refactoring strategy
- `docs/VIRTUAL_LANGUAGE_ARCHITECTURE.md` - Generic virtual language system
- `docs/VIRTUAL_LANGUAGE_EXTENSION_SYSTEM.md` - Extension trait system
- `docs/VIRTUAL_LANGUAGE_UNIFIED_IR_INTEGRATION.md` - Unified IR integration
- `docs/REFACTORING_SUMMARY.md` - Completed refactorings
