# LSP Backend Refactoring Plan

## Overview

Refactor `src/lsp/backend.rs` (3,495 lines) into a modular structure while preserving all functionality and handling the complexity of async trait implementations.

## Current Structure Analysis

### File Breakdown
- **Total**: 3,495 lines
- **Imports & Types**: ~100 lines
- **SemanticTokensBuilder**: ~60 lines
- **RholangBackend impl**: ~2,310 lines (mixed concerns)
- **LanguageServer trait impl**: ~1,022 lines

### Key Challenges

1. **Async Trait Implementation**: LanguageServer trait must be implemented in one location
2. **Shared Mutable State**: Heavy use of `Arc<RwLock<T>>`, `Arc<AsyncMutex<T>>`
3. **Cross-Cutting Concerns**: Validation, symbol lookup, etc. used across features
4. **Language-Specific Code**: MeTTa support (~850 lines) embedded in main impl

## Proposed Module Structure

```
src/lsp/backend/
├── mod.rs                      # Module coordinator with re-exports
├── state.rs                    # RholangBackend struct + state types
├── lifecycle.rs                # Initialization, spawners, shutdown
├── document_processing.rs      # Document parsing, indexing, validation
├── workspace.rs                # Workspace indexing, file watching
├── symbol_operations.rs        # Symbol queries, position lookups
├── metta_support.rs            # MeTTa language features
├── lsp_handlers.rs             # LanguageServer trait implementation
└── utils.rs                    # Utilities, SemanticTokensBuilder
```

## Module Responsibilities

### mod.rs (~50 lines)
**Purpose**: Module coordination and public API

- Re-export `RholangBackend`
- Re-export public helper functions
- Module declarations

### state.rs (~200 lines)
**Purpose**: State management and struct definitions

**Contents**:
- `RholangBackend` struct definition (lines 74-99)
- `DocumentChangeEvent` struct (lines 57-63)
- `IndexingTask` struct (lines 66-71)
- State initialization helpers

**Access**: `pub(super)` for cross-module access

### lifecycle.rs (~400 lines)
**Purpose**: Backend initialization and background task management

**Methods**:
- `pub async fn new()` (lines 167-256)
- `fn spawn_document_debouncer()` (lines 258-332)
- `fn spawn_progressive_indexer()` (lines 334-399)
- `fn spawn_file_watcher()` (lines 401-467)
- `fn next_document_id()` (lines 873-876)

**Dependencies**: Needs access to `RholangBackend` struct

### document_processing.rs (~600 lines)
**Purpose**: Core document workflow

**Methods**:
- `async fn process_document()` (lines 469-565)
- `async fn index_file()` (lines 567-695)
- `async fn index_metta_file()` (lines 697-782)
- `async fn validate()` (lines 878-1016)
- `async fn aggregate_with_virtual_diagnostics()` (lines 1018-1031)

**Pattern**: Takes `&self` and returns processed artifacts

### workspace.rs (~300 lines)
**Purpose**: Multi-file workspace operations

**Methods**:
- `async fn link_symbols()` (lines 784-821)
- `async fn handle_file_change()` (lines 823-842)
- `async fn index_directory()` (lines 844-871)

**Integration**: Works with document_processing for batch operations

### symbol_operations.rs (~500 lines)
**Purpose**: Symbol table queries and position-based operations

**Methods**:
- `pub async fn lookup_node_at_position()` (lines 1033-1051)
- `fn position_to_range()` (lines 1053-1065)
- `async fn get_symbol_at_position()` (lines 1067-1545)
- `async fn get_symbol_references()` (lines 1547-1595)
- `pub fn byte_offset_from_position()` (lines 1597-1614)

**Note**: `get_symbol_at_position()` is 478 lines - handles both Rholang and MeTTa

### metta_support.rs (~850 lines)
**Purpose**: All MeTTa-specific language features

**Methods**:
- `async fn hover_metta()` (lines 1616-1656)
- `fn create_metta_hover_content()` (lines 1658-1749)
- `fn extract_metta_name()` (lines 1751-1759)
- `async fn add_metta_semantic_tokens()` (lines 1761-1789)
- `fn visit_metta_node()` (lines 1791-1935)
- `async fn document_highlight_metta()` (lines 1937-2080)
- `async fn goto_definition_metta()` (lines 2082-2238)
- `fn find_metta_call_at_position()` (lines 2240-2291)
- `fn find_metta_call_in_node()` (lines 2293-2394)
- `fn position_in_range()` (lines 2396-2405)
- `async fn rename_metta()` (lines 2407-2471)

**Benefit**: Largest single extraction - isolates MeTTa complexity

### lsp_handlers.rs (~1,022 lines)
**Purpose**: LanguageServer trait implementation

**Methods**: All LanguageServer trait methods (lines 2473-3495)
- `async fn initialize()`
- `async fn initialized()`
- `async fn shutdown()`
- `async fn did_open()`
- `async fn did_change()`
- `async fn did_save()`
- `async fn did_close()`
- `async fn rename()`
- `async fn goto_definition()`
- `async fn goto_declaration()`
- `async fn references()`
- `async fn document_symbol()`
- `async fn symbol()`
- `async fn symbol_resolve()`
- `async fn document_highlight()`
- `async fn hover()`
- `async fn semantic_tokens_full()`

**Important**: Must remain as single impl block for trait coherence

### utils.rs (~100 lines)
**Purpose**: Helper types and utility functions

**Contents**:
- `SemanticTokensBuilder` struct + impl (lines 101-159)
- Other utility functions that don't fit elsewhere

## Implementation Strategy

### Phase 1: Preparation
1. Create `src/lsp/backend/` directory
2. Backup original: `git mv src/lsp/backend.rs src/lsp/backend_backup.rs`
3. Create module structure

### Phase 2: Extract Independent Modules
**Order matters - extract least dependent first**

1. **utils.rs** - No dependencies on other modules
2. **state.rs** - Only type definitions
3. **metta_support.rs** - Relatively independent, uses state

### Phase 3: Extract Core Functionality
4. **symbol_operations.rs** - Used by handlers
5. **document_processing.rs** - Core workflow
6. **workspace.rs** - Uses document_processing

### Phase 4: Extract Lifecycle & Handlers
7. **lifecycle.rs** - Spawns background tasks
8. **lsp_handlers.rs** - Main trait impl, uses all modules

### Phase 5: Coordination
9. **mod.rs** - Tie everything together with re-exports

### Phase 6: Testing & Cleanup
10. Verify compilation
11. Run full test suite
12. Remove backup file
13. Commit

## Technical Considerations

### Module Visibility
- Use `pub(super)` for cross-module items within backend
- Keep RholangBackend methods that are called by handlers as `pub(in crate::lsp::backend)`
- Public API: Only what external code needs

### Async Trait Splitting Pattern
Since we can't split the LanguageServer trait impl, we use this pattern:

**lsp_handlers.rs**:
```rust
use super::symbol_operations::SymbolOperations;
use super::metta_support::MettaSupport;

#[async_trait]
impl LanguageServer for RholangBackend {
    async fn hover(&self, params: HoverParams) -> LspResult<Option<Hover>> {
        // Delegate to helper methods in other modules
        if is_metta_file(&uri) {
            self.hover_metta(params).await
        } else {
            self.hover_rholang(params).await
        }
    }
}
```

**metta_support.rs**:
```rust
impl RholangBackend {
    pub(super) async fn hover_metta(&self, params: HoverParams) -> LspResult<Option<Hover>> {
        // Implementation
    }
}
```

### Import Management
Each module imports only what it needs:
- `super::state::RholangBackend` - For `&self` methods
- External crates as needed
- Sibling modules for delegation

### State Access
All modules can access `RholangBackend` fields via `&self` since:
1. They implement methods on the same struct
2. Use `pub(super)` visibility for fields if needed
3. Maintain encapsulation through method APIs

## Benefits

1. **Modularity**: 3,495 lines → 8 focused modules (~300-1000 lines each)
2. **Clarity**: Each module has a single, clear purpose
3. **Maintainability**: Easier to locate and modify specific features
4. **Language Separation**: MeTTa support fully isolated
5. **Testing**: Each module can be tested independently
6. **Documentation**: Module-level docs provide clear navigation

## Risks & Mitigation

### Risk 1: Trait Coherence Issues
**Mitigation**: Keep LanguageServer impl in single file (lsp_handlers.rs)

### Risk 2: Circular Dependencies
**Mitigation**: Clear dependency hierarchy (utils → state → symbol_ops → document_processing → workspace → lifecycle → handlers)

### Risk 3: Compilation Errors
**Mitigation**: Incremental extraction with frequent compilation checks

### Risk 4: Async/Await Complexity
**Mitigation**: Preserve all async signatures and state access patterns

## Success Criteria

- [ ] All modules compile without errors
- [ ] All 204+ unit tests pass
- [ ] All integration tests pass
- [ ] No change in external API
- [ ] Git history preserved via `git mv`
- [ ] Clear module documentation

## Estimated Impact

- **Before**: 1 file, 3,495 lines
- **After**: 9 files, ~388 lines each (average)
- **Modularity increase**: 9x
- **Largest module reduction**: From 3,495 to ~1,022 lines (lsp_handlers.rs)
- **MeTTa isolation**: 850 lines moved to dedicated module

## Next Steps

This plan provides the roadmap. Implementation requires:
1. User approval
2. Systematic extraction following the phases
3. Continuous testing to ensure no regressions
4. Detailed commit message documenting the changes
