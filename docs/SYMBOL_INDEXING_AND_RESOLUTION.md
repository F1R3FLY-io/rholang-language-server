# Symbol Indexing and Resolution Architecture

**Last Updated**: 2025-10-29
**Status**: In Progress - Generic strategy implemented, Rholang refactoring pending

---

## Overview

This document describes how symbols are indexed and resolved across the Rholang language server, covering both Rholang-specific and language-agnostic (generic) strategies.

## Table of Contents

1. [Architecture Overview](#architecture-overview)
2. [Symbol Indexing](#symbol-indexing)
3. [Symbol Resolution Strategies](#symbol-resolution-strategies)
4. [Cross-Document Linking](#cross-document-linking)
5. [Data Structures](#data-structures)
6. [Resolution Flow](#resolution-flow)
7. [Pending Refactoring](#pending-refactoring)

---

## Architecture Overview

The symbol indexing and resolution system supports **three different strategies**:

1. **Rholang-specific**: Hierarchical lexical scoping with symbol tables
2. **MeTTa-specific**: Composable resolution with pattern matching and global fallback
3. **Generic**: Flat global scope for language-agnostic embedded languages

Each strategy uses the **SymbolResolver** trait for a unified interface, but implements different resolution semantics based on language requirements.

---

## Symbol Indexing

### Rholang Symbol Indexing

**Current Implementation** (`src/lsp/backend/symbols.rs:link_symbols`):

Rholang symbols are indexed into multiple separate structures:

1. **`global_symbols`** (`WorkspaceState::global_symbols`):
   ```rust
   Arc<DashMap<String, (Url, IrPosition)>>
   ```
   - Maps symbol name → **ONE** location (URI + position)
   - **Limitation**: Only stores one location per symbol
   - Used for quick cross-file goto-definition lookups

2. **`global_table`** (`WorkspaceState::global_table`):
   ```rust
   Arc<tokio::sync::RwLock<SymbolTable>>
   ```
   - Hierarchical symbol table with parent chain
   - Stores full `Symbol` objects with metadata
   - Supports lexical scoping via parent links
   - Wrapped in RwLock (not lock-free)

3. **`global_inverted_index`** (`WorkspaceState::global_inverted_index`):
   ```rust
   Arc<DashMap<(Url, IrPosition), Vec<(Url, IrPosition)>>>
   ```
   - Maps definition location → list of usage locations
   - Used for find-references
   - Supports multiple usages per definition
   - Lock-free via DashMap

#### Indexing Process for Rholang

1. **Parse** all `.rho` files in workspace using Tree-Sitter
2. **Build IR** from Tree-Sitter CST
3. **Symbol Table Builder** transform:
   - Visits IR nodes
   - Collects symbols (contracts, variables, parameters)
   - Builds hierarchical symbol tables
   - Records declaration and definition locations
   - Builds inverted index (definition → usages)
4. **Link symbols** across files:
   - Merge symbol tables into `global_table`
   - Populate `global_symbols` map (one location per symbol)
   - Populate `global_inverted_index` (usages)

**File**: `src/ir/transforms/symbol_table_builder.rs`

### Virtual Document Symbol Indexing (MeTTa, Generic)

**Current Implementation** (`src/lsp/backend/symbols.rs:link_virtual_symbols`):

Virtual documents (embedded languages like MeTTa) use a **unified structure**:

**`global_virtual_symbols`** (`WorkspaceState::global_virtual_symbols`):
```rust
Arc<DashMap<
    String,  // Language name ("metta", "python", etc.)
    Arc<DashMap<
        String,  // Symbol name
        Vec<(Url, Range)>  // List of locations
    >>
>>
```

**Key Properties**:
- ✅ Supports **multiple locations per symbol**
- ✅ Lock-free concurrent access (nested DashMap)
- ✅ Language-agnostic structure
- ✅ Cross-document linking built-in

#### Indexing Process for Virtual Documents

1. **Detect virtual documents** in parent `.rho` files:
   - Scan string literals for language directives (`#!metta`)
   - Extract content and build `VirtualDocument`
2. **Parse virtual document** content:
   - MeTTa: Parse to `Vec<Arc<MettaNode>>`
   - Generic: Could use any parser
3. **Build symbol table**:
   - MeTTa: Uses `MettaSymbolTableBuilder`
   - Generic: No specific symbol table (relies on global index)
4. **Extract definitions**:
   - Find all symbols marked as `is_definition`
   - Record (uri, range) for each definition
5. **Link across documents**:
   - Group by language
   - Insert into `global_virtual_symbols[language][symbol_name]`
   - Supports multiple definitions per symbol

**File**: `src/lsp/backend/symbols.rs:link_virtual_symbols` (lines 198-277)

---

## Symbol Resolution Strategies

### 1. Rholang-Specific Resolution

**Implementation**: `RholangSymbolResolver` (`src/lsp/features/adapters/rholang.rs:102-199`)

**Resolution Model**:
- **Hierarchical lexical scoping**: local → document → global
- **Symbol constraints**:
  - Exactly **1 declaration** per symbol
  - At most **1 definition** per symbol
  - Declaration and definition may be at the same location

**Resolution Process**:
```rust
impl SymbolResolver for RholangSymbolResolver {
    fn resolve_symbol(&self, symbol_name: &str, position: &Position, context: &ResolutionContext)
        -> Vec<SymbolLocation>
    {
        // 1. Look up symbol in hierarchical symbol table
        if let Some(symbol) = self.symbol_table.lookup(symbol_name) {
            let mut locations = Vec::new();

            // 2. Always add declaration location
            locations.push(SymbolLocation {
                uri: symbol.declaration_uri,
                range: declaration_range,
                kind: symbol_kind,
                confidence: ResolutionConfidence::Exact,
            });

            // 3. Add definition location if different from declaration
            if let Some(def_location) = symbol.definition_location {
                if def_location != symbol.declaration_location {
                    locations.push(SymbolLocation {
                        uri: symbol.declaration_uri,
                        range: definition_range,
                        kind: symbol_kind,
                        confidence: ResolutionConfidence::Exact,
                    });
                }
            }

            locations  // Max 2 locations (declaration + definition)
        } else {
            Vec::new()  // Not found in scope chain
        }
    }
}
```

**Scope Chain Traversal**:
- Uses `SymbolTable::lookup()` which walks parent chain
- Starts from current scope's symbol table
- Traverses up to document's symbol table
- Finally checks `global_table` (workspace-wide)

**Return Value**:
- 1 location: Declaration only (or declaration == definition)
- 2 locations: Declaration + definition (when they differ)
- 0 locations: Symbol not found

### 2. MeTTa-Specific Resolution

**Implementation**: `ComposableSymbolResolver` (`src/lsp/features/adapters/metta.rs:130-159`)

**Resolution Model**:
- **Composed strategy**: Base resolver + filters + fallback
- **Pattern matching**: Arity-based refinement for function symbols
- **Cross-document**: Global fallback for symbols not in local scope

**Resolution Process**:
```rust
// MeTTa adapter uses composition:
ComposableSymbolResolver::new(
    Box::new(LexicalScopeResolver::new(symbol_table, "metta")),  // Base
    vec![Box::new(MettaPatternFilter::new(pattern_matcher))],    // Filters
    Some(Box::new(GlobalVirtualSymbolResolver::new(workspace))), // Fallback
)
```

**Resolution Steps**:
1. **Base resolution**: `LexicalScopeResolver` checks MeTTa symbol table (lexical scoping)
2. **Filter**: `MettaPatternFilter` refines by arity matching
   - If matches found: Return filtered results
   - If no matches: Return unfiltered (don't block valid symbols)
3. **Fallback**: If base returns empty, use `GlobalVirtualSymbolResolver`
   - Queries `global_virtual_symbols["metta"][symbol_name]`
   - Returns all cross-document matches

**Return Value**:
- Multiple locations possible (cross-document, multiple definitions)
- Ordered by confidence (Exact > Fuzzy)

### 3. Generic Resolution

**Implementation**: `GenericSymbolResolver` (`src/ir/symbol_resolution/generic.rs:17-83`)

**Resolution Model**:
- **Flat global scope**: No lexical hierarchy
- **Multiple declarations/definitions**: Unlimited locations per symbol
- **Cross-document**: Built-in via `global_virtual_symbols`

**Resolution Process**:
```rust
impl SymbolResolver for GenericSymbolResolver {
    fn resolve_symbol(&self, symbol_name: &str, position: &Position, context: &ResolutionContext)
        -> Vec<SymbolLocation>
    {
        // Query global_virtual_symbols[language][symbol_name]
        self.workspace
            .global_virtual_symbols
            .get(&self.language)
            .and_then(|lang_symbols| {
                lang_symbols.get(symbol_name).map(|locs| {
                    // Return ALL locations - no filtering
                    locs.iter().map(|(uri, range)| SymbolLocation {
                        uri: uri.clone(),
                        range: *range,
                        kind: SymbolKind::Variable,  // Generic - can't determine specific kind
                        confidence: ResolutionConfidence::Exact,
                        metadata: None,
                    }).collect()
                })
            })
            .unwrap_or_default()
    }
}
```

**Key Features**:
- No scoping logic - position doesn't matter
- Returns ALL matching locations across all documents
- Language-specific via `self.language` field
- Lock-free lookup (DashMap)

**Return Value**:
- 0 to N locations (unlimited)
- All have `ResolutionConfidence::Exact`

---

## Cross-Document Linking

### Rholang Cross-Document Linking

**Current Status**: ⚠️ **Partially Implemented**

**Current Approach**:
- Uses `global_symbols` for quick lookups (one location per symbol)
- Uses `global_inverted_index` for find-references
- **Problem**: Can't represent multiple declarations/definitions per symbol

**Pending Refactoring**:
- Replace `global_symbols: DashMap<String, (Url, IrPosition)>`
- With: Similar structure to `global_virtual_symbols` that supports multiple locations
- Maintain constraint: max 2 locations (1 declaration + 1 definition)

### Virtual Document Cross-Document Linking

**Current Status**: ✅ **Fully Implemented**

**Approach**:
- Uses `global_virtual_symbols[language][symbol_name] = Vec<(Url, Range)>`
- Supports unlimited locations per symbol
- Lock-free concurrent access
- Rebuilt on every document change (debounced)

**Link Process**:
1. Scan all parent documents for virtual documents
2. For each virtual document:
   - Get/build symbol table
   - Extract all definitions (`is_definition = true`)
   - Group by language
3. Merge into `global_virtual_symbols`
4. Lock-free update (clear + insert)

**File**: `src/lsp/backend/symbols.rs:link_virtual_symbols` (lines 198-277)

---

## Data Structures

### SymbolLocation

```rust
pub struct SymbolLocation {
    pub uri: Url,
    pub range: Range,
    pub kind: SymbolKind,
    pub confidence: ResolutionConfidence,
    pub metadata: Option<Arc<dyn Any + Send + Sync>>,
}
```

### ResolutionContext

```rust
pub struct ResolutionContext {
    pub uri: Url,
    pub scope_id: Option<usize>,
    pub ir_node: Option<Arc<dyn Any + Send + Sync>>,
    pub language: String,
    pub parent_uri: Option<Url>,
}
```

### Symbol (Rholang)

```rust
pub struct Symbol {
    pub name: String,
    pub symbol_type: SymbolType,  // Contract, Variable, Parameter
    pub declaration_uri: Url,
    pub declaration_location: Position,
    pub definition_location: Option<Position>,
    pub contract_pattern: Option<ContractPattern>,  // For pattern matching
}
```

**File**: `src/ir/symbol_table.rs:29-91`

### MettaSymbolOccurrence (MeTTa)

```rust
pub struct MettaSymbolOccurrence {
    pub name: String,
    pub range: Range,
    pub is_definition: bool,
    pub kind: MettaSymbolKind,
    pub scope_id: usize,
}
```

**File**: `src/ir/transforms/metta_symbol_table_builder.rs`

---

## Resolution Flow

### LSP Request → Symbol Resolution

```
1. LSP Request (goto_definition, references, rename)
   ↓
2. unified_goto_definition/unified_references/unified_rename
   ↓
3. detect_language(uri, position)
   → Returns LanguageContext (Rholang | MettaVirtual | Other)
   ↓
4. get_adapter(context)
   → Routes to appropriate adapter:
      - Rholang: create_rholang_adapter(symbol_table)
      - MeTTa: create_metta_adapter(symbol_table, workspace, parent_uri)
      - Other: create_generic_adapter(workspace, language)
   ↓
5. Generic LSP Feature (GenericGotoDefinition, GenericReferences, etc.)
   ↓
6. adapter.resolver.resolve_symbol(name, position, context)
   → Calls language-specific resolver
   ↓
7. Return Vec<SymbolLocation>
   ↓
8. Convert to LSP response format
   ↓
9. Return to client
```

**File**: `src/lsp/backend/unified_handlers.rs`

---

## Position Structure and Lookup Efficiency

### Position Triple Design

All IR nodes track their location using a `(row, col, byte)` triple:

```rust
pub struct Position {
    pub row: usize,    // Line number (0-based)
    pub column: usize, // Column number (0-based)
    pub byte: usize,   // Byte offset from start (metadata for random access)
}
```

**Key Design Decision**: Position equality is based **only** on `(row, column)`:

```rust
impl PartialEq for Position {
    fn eq(&self, other: &Self) -> bool {
        self.row == other.row && self.column == other.column
    }
}

impl std::hash::Hash for Position {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.row.hash(state);
        self.column.hash(state);
    }
}
```

### Rationale

1. **Lookup Efficiency**: `(row, column)` uniquely identifies a position in text
2. **Byte Offset Independence**: Different parsing passes may compute different byte offsets due to:
   - Incremental parsing
   - Different text encodings
   - Tree-sitter vs manual parsing
3. **Future Random Access**: Byte field enables O(1) seeking to a position **without re-scanning** the document
4. **No Manual Normalization**: Eliminates error-prone `byte: 0` hacks that were scattered throughout the codebase

### Phase 1 Cleanup (Completed 2025-10-30)

Before Phase 1, three places had to manually normalize byte to 0 for lookups to work:
- `src/lsp/backend/symbols.rs:140` - inverted index building
- `src/lsp/backend.rs:627` - global usage lookup
- `src/lsp/features/references.rs:83` - find-references

Phase 1 removed all manual normalization, leveraging the Position struct's built-in equality semantics.

---

## Pending Refactoring

### Rholang Symbol Indexing Refactoring

**Status**: ⚠️ **Planned but Not Implemented**

**Current Problems**:
1. **`global_symbols`** only stores one location per symbol
   - Can't represent both declaration and definition locations
   - Inconsistent with virtual document approach
2. **Multiple separate structures**:
   - `global_symbols`: Quick lookup (single location)
   - `global_table`: Hierarchical scoping
   - `global_inverted_index`: Find-references
   - Hard to maintain consistency

**Proposed Refactoring**:

Replace `global_symbols` with a structure similar to `global_virtual_symbols`:

```rust
// Proposed new structure
pub struct RholangGlobalSymbols {
    // Maps symbol name → list of (declaration, optional definition)
    symbols: Arc<DashMap<String, Vec<RholangSymbolLocation>>>,
}

pub struct RholangSymbolLocation {
    pub uri: Url,
    pub declaration: Range,
    pub definition: Option<Range>,  // Same as declaration if not separate
}
```

**Constraints**:
- Max 1 `RholangSymbolLocation` per symbol (Rholang's single-declaration rule)
- Can have both declaration and definition ranges in single object
- Validates constraint at insertion time

**Benefits**:
1. Unified structure similar to virtual documents
2. Supports both declaration and definition locations
3. Maintains Rholang's single-declaration semantics
4. Simpler to maintain

**Migration Path**:
1. Create new `RholangGlobalSymbols` structure
2. Update `link_symbols()` to populate new structure
3. Update `RholangSymbolResolver` to query new structure
4. Deprecate old `global_symbols`
5. Run tests to verify behavior unchanged

### Test Status

**Before Refactoring**: 17/23 tests passing
- 6 tests timing out (related to contracts with `new` bindings)

**After Refactoring**: TBD
- Need to fix timeouts first
- Then complete refactoring
- Re-run full test suite

---

## References

### Source Files

- **Symbol Resolution Trait**: `src/ir/symbol_resolution/mod.rs`
- **Rholang Resolver**: `src/lsp/features/adapters/rholang.rs`
- **MeTTa Resolver**: `src/lsp/features/adapters/metta.rs`
- **Generic Resolver**: `src/ir/symbol_resolution/generic.rs`
- **Symbol Indexing**: `src/lsp/backend/symbols.rs`
- **Symbol Table**: `src/ir/symbol_table.rs`
- **MeTTa Symbol Table**: `src/ir/transforms/metta_symbol_table_builder.rs`
- **Unified Handlers**: `src/lsp/backend/unified_handlers.rs`

### Related Documents

- `.claude/CLAUDE.md`: Architecture overview
- `docs/UNIFIED_LSP_ARCHITECTURE.md`: Unified LSP design
- `docs/EMBEDDED_LANGUAGES_GUIDE.md`: Virtual document system

---

## Revision History

- **2025-10-30**: Phase 2 Complete - RholangGlobalSymbols Structure Created
  - **Created** unified symbol storage structure (`src/lsp/rholang_global_symbols.rs`, 596 lines)
  - **Architecture**: Lock-free, single-source-of-truth replacing 3 redundant structures
  - **Features**:
    - SymbolDeclaration struct: name + type + declaration + optional definition + references
    - Enforces Rholang constraints: 1 declaration + 0-1 definition + N references
    - Lock-free via DashMap (zero contention)
    - 10 comprehensive tests covering all operations
  - **Integration**: Added `rholang_symbols` field to WorkspaceState
  - **Test Status**: Code compiles successfully, tests pass
  - **Implementation Plan**: Created `docs/REFACTORING_PHASES_3_6_PLAN.md` with detailed implementation steps for remaining phases
  - **Next Steps**:
    - Phase 3: Update SymbolTableBuilder for direct indexing (estimated 2-3 hours)
    - Phase 4: Remove old structures and link_symbols algorithm (estimated 1-2 hours)
    - Phase 5: Update RholangSymbolResolver (estimated 1 hour)
    - Phase 6: Test and verify all 23 tests pass (estimated 2-3 hours)

- **2025-10-30**: Phase 1 Complete - Position Simplification
  - **Removed manual byte normalization** from all lookups
  - Leveraged existing Position Hash/Eq implementations (row, column only)
  - Changes:
    - `src/lsp/backend/symbols.rs:140` - Removed `byte: 0` normalization
    - `src/lsp/backend.rs:627` - Removed `byte: 0` normalization
    - `src/lsp/features/references.rs:83` - Updated comment to clarify byte is metadata
  - **Test Status**: 17/23 passing (no regressions)
  - **Architecture Note**: Position struct already had custom Hash/Eq (lines 39-53 in semantic_node.rs) that only use (row, column). The byte field is metadata for future O(1) random access. This phase eliminated error-prone manual normalization and reduced code complexity.

- **2025-10-29**: Initial documentation created
  - Documented current Rholang, MeTTa, and Generic strategies
  - Identified pending refactoring work
  - Status: Generic strategy complete, Rholang refactoring pending
