# Code Refactoring Plan

## Overview

This document outlines a comprehensive refactoring strategy for breaking down the largest source files in the Rholang Language Server into smaller, more maintainable modules.

## Analysis Summary

### Current State

| File | Lines | Status |
|------|-------|--------|
| src/ir/rholang_node.rs | 5,494 | **Needs refactoring** |
| src/lsp/backend.rs | 3,495 | **Needs refactoring** |
| src/ir/transforms/pretty_printer.rs | 2,279 | **Needs refactoring** |
| src/ir/visitor.rs | 1,601 | Consider refactoring |
| src/tree_sitter.rs | 1,493 | Consider refactoring |

### Recommended Module Structure

## 1. Refactoring `src/ir/rholang_node.rs` (5,494 lines)

### Current Structure
- Lines 1-396: Enum definitions (RholangNode + supporting enums)
- Lines 398-761: Position computation functions
- Lines 763-1516: Pattern matching and collection functions
- Lines 1518-1979: Node finding functions
- Lines 1981-end: Trait implementations

### Proposed Structure

```
src/ir/rholang_node/
├── mod.rs                    # Re-exports all public items
├── node_types.rs             # ~400 lines - RholangNode enum + supporting enums
├── position_tracking.rs      # ~600 lines - compute_absolute_positions, find_node_at_position*
├── node_operations.rs        # ~750 lines - match_pat, match_contract, collect_*
└── node_impl.rs              # ~2200 lines - All trait implementations
```

### Migration Steps

1. **Create directory structure**
   ```bash
   mkdir -p src/ir/rholang_node
   ```

2. **Extract node_types.rs** (Lines 1-396)
   - Move: RholangNode enum
   - Move: RholangBundleType, RholangSendType, BinOperator, UnaryOperator, RholangVarRefKind, CommentKind
   - Move: Type aliases (RholangNodeVector, etc.)
   - Keep: All imports needed for these types

3. **Extract position_tracking.rs** (Lines 398-761 + 1518-1979)
   - Move: `compute_absolute_positions()`
   - Move: `compute_end_position()`
   - Move: `find_node_at_position_with_path()`
   - Move: `find_node_at_position()`
   - Add: `use super::node_types::*;`

4. **Extract node_operations.rs** (Lines 763-1516)
   - Move: `match_pat()`
   - Move: `match_contract()`
   - Move: `collect_contracts()`
   - Move: `collect_calls()`
   - Add: `use super::node_types::*;`

5. **Extract node_impl.rs** (Lines 1981-end)
   - Move: `impl RholangNode { ... }`
   - Move: `impl PartialEq for RholangNode`
   - Move: `impl Eq, PartialOrd, Ord for RholangNode`
   - Move: `impl SemanticNode for RholangNode`
   - Add: `use super::node_types::*;`

6. **Create mod.rs**
   ```rust
   mod node_types;
   mod position_tracking;
   mod node_operations;
   mod node_impl;

   // Re-export all public items
   pub use node_types::*;
   pub use position_tracking::*;
   pub use node_operations::*;
   ```

7. **Update imports in other files**
   - Change: `use crate::ir::rholang_node::{...}`
   - To: Same (mod.rs handles re-exports)

## 2. Refactoring `src/lsp/backend.rs` (3,495 lines)

### Current Structure
- Lines 1-100: Imports and type definitions
- Lines 102-160: Debug/utility impls
- Lines 161-2472: RholangBackend impl with initialization and helpers
- Lines 2473-end: LanguageServer trait implementation

### Proposed Structure

```
src/lsp/backend/
├── mod.rs                    # Main RholangBackend struct + initialization
├── models.rs                 # Move from lsp/models.rs - already extracted
├── document_handler.rs       # didOpen, didChange, didSave, didClose
├── workspace.rs              # Workspace indexing and management
├── features/
│   ├── mod.rs                # Feature module exports
│   ├── definition.rs         # textDocument/definition
│   ├── references.rs         # textDocument/references
│   ├── rename.rs             # textDocument/rename
│   ├── symbols.rs            # textDocument/documentSymbol, workspace/symbol
│   ├── highlight.rs          # textDocument/documentHighlight
│   ├── hover.rs              # textDocument/hover
│   └── semantic_tokens.rs    # textDocument/semanticTokens
└── utils.rs                  # Helper functions (byte_offset_from_position, etc.)
```

### Migration Steps

1. **Create directory structure**
   ```bash
   mkdir -p src/lsp/backend/features
   ```

2. **Extract features/definition.rs**
   - Move: `goto_definition` method
   - Move: `goto_declaration` method
   - Create trait: `pub trait DefinitionProvider`
   - Implement on `RholangBackend`

3. **Extract features/references.rs**
   - Move: `references` method
   - Create trait: `pub trait ReferencesProvider`

4. **Extract features/rename.rs**
   - Move: `rename` method
   - Create trait: `pub trait RenameProvider`

5. **Extract features/symbols.rs**
   - Move: `document_symbol` method
   - Move: `workspace_symbol` method
   - Create trait: `pub trait SymbolsProvider`

6. **Extract features/highlight.rs**
   - Move: `document_highlight` method
   - Create trait: `pub trait HighlightProvider`

7. **Extract features/hover.rs**
   - Move: `hover` method
   - Create trait: `pub trait HoverProvider`

8. **Extract features/semantic_tokens.rs**
   - Move: `semantic_tokens_full` method
   - Move: `SemanticTokensBuilder` struct
   - Create trait: `pub trait SemanticTokensProvider`

9. **Extract document_handler.rs**
   - Move: `did_open` method
   - Move: `did_change` method
   - Move: `did_save` method
   - Move: `did_close` method
   - Move: Document change event handling
   - Create trait: `pub trait DocumentHandler`

10. **Extract workspace.rs**
    - Move: Workspace indexing logic
    - Move: File watching setup
    - Move: `index_workspace_file` method
    - Move: `IndexingTask` struct
    - Create trait: `pub trait WorkspaceManager`

11. **Extract utils.rs**
    - Move: `byte_offset_from_position`
    - Move: Other utility functions

12. **Refactor mod.rs**
    ```rust
    mod document_handler;
    mod workspace;
    mod features;
    mod utils;

    pub use features::*;

    // Keep RholangBackend struct and initialization here
    // Implement all traits (DefinitionProvider, etc.) via delegation
    ```

13. **Implement LanguageServer trait via delegation**
    ```rust
    #[async_trait]
    impl LanguageServer for RholangBackend {
        async fn goto_definition(...) -> ... {
            DefinitionProvider::goto_definition(self, ...).await
        }
        // ... etc for all methods
    }
    ```

## 3. Refactoring `src/ir/transforms/pretty_printer.rs` (2,279 lines)

### Current Structure
- Lines 1-60: Imports, format() function, PrettyPrinter struct
- Lines 61-380: JsonStringFormatter trait + primitive/collection implementations
- Lines 381-end: PrettyPrinter impl and Visitor impl

### Proposed Structure

```
src/ir/transforms/pretty_printer/
├── mod.rs                    # format() function and re-exports
├── json_formatters.rs        # JsonStringFormatter trait + all impls
└── printer.rs                # PrettyPrinter struct and implementations
```

### Migration Steps

1. **Create directory structure**
   ```bash
   mkdir -p src/ir/transforms/pretty_printer
   ```

2. **Extract json_formatters.rs** (Lines 61-380)
   - Move: `JsonStringFormatter` trait
   - Move: All primitive type implementations (bool, i8-i128, u8-u128, f32, f64, char, String, ())
   - Move: Collection implementations (Vec<T>, HashMap<String, T>, BTreeMap<String, T>, Option<T>)
   - Move: `format_json_string` helper function
   - Keep minimal imports

3. **Extract printer.rs** (Lines 40-60 + 381-end)
   - Move: `PrettyPrinter` struct definition
   - Move: `impl PrettyPrinter { ... }`
   - Move: `impl Visitor for PrettyPrinter`
   - Add: `use super::json_formatters::*;`

4. **Create mod.rs** (Lines 1-39)
   - Keep: `format()` public function
   - Add: Module declarations and re-exports
   ```rust
   mod json_formatters;
   mod printer;

   pub use json_formatters::JsonStringFormatter;
   pub use printer::PrettyPrinter;

   // Keep format() function here
   pub fn format(tree: &Arc<RholangNode>, pretty_print: bool, rope: &Rope) -> Result<String, String> {
       // ... existing implementation
   }
   ```

## 4. Update Module Structure

### Files to Update

1. **src/lib.rs** - Update module declarations
2. **src/ir/mod.rs** - Update rholang_node module declaration
3. **src/lsp/mod.rs** - Update backend module declaration
4. **src/ir/transforms/mod.rs** - Update pretty_printer module declaration

### Example: src/ir/mod.rs

```rust
// Before
pub mod rholang_node;

// After
pub mod rholang_node;  // Now a directory with mod.rs
```

The re-exports in mod.rs ensure backward compatibility!

## Testing Strategy

### Phase 1: Compilation Test
After each refactoring step:
```bash
cargo check
```

### Phase 2: Unit Tests
```bash
cargo test --lib
```

### Phase 3: Integration Tests
```bash
cargo test --test ir_pipeline
cargo test --test lsp_features
cargo test --test performance_tests
```

### Phase 4: Full Test Suite
```bash
cargo nextest run
```

## Implementation Order

**Recommended sequence** (easiest to hardest):

1. ✅ **pretty_printer.rs** - Simplest, least dependencies
2. **rholang_node.rs** - More complex, but clear boundaries
3. **backend.rs** - Most complex, requires careful trait extraction

## Rollback Plan

If any refactoring causes issues:

1. **Git checkpoint before each file**
   ```bash
   git add -A
   git commit -m "checkpoint: before refactoring [filename]"
   ```

2. **Rollback if needed**
   ```bash
   git reset --hard HEAD~1
   ```

## Benefits

1. **Maintainability**: Smaller files are easier to understand and modify
2. **Compilation Speed**: Smaller modules = faster incremental compilation
3. **Code Navigation**: Clearer module boundaries improve IDE navigation
4. **Testing**: Easier to test isolated functionality
5. **Collaboration**: Reduces merge conflicts with smaller files
6. **Cognitive Load**: Developers can focus on one concern at a time

## Estimated Impact

| Metric | Before | After | Improvement |
|--------|--------|-------|-------------|
| Largest file | 5,494 lines | ~2,200 lines | **60% reduction** |
| Avg file size | ~1,500 lines | ~500 lines | **67% reduction** |
| Number of modules | 3 large files | 15+ focused modules | **5x more modular** |

## Next Steps

1. Review this plan with the team
2. Start with `pretty_printer.rs` refactoring (simplest)
3. Run full test suite to verify
4. Proceed to `rholang_node.rs`
5. Finally tackle `backend.rs`

## Notes

- All refactorings maintain **backward compatibility** via re-exports
- No changes to public API required
- Existing tests should pass without modification
- Can be done incrementally, one file at a time
