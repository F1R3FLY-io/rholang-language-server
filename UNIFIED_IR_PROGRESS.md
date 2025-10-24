# Unified IR Implementation Progress

**Project**: Language-Agnostic Intermediate Representation for rholang-language-server
**Goal**: Enable multi-language LSP support (Rholang + MeTTa) with shared infrastructure
**Status**: 12/14 Tasks Complete (85%)
**Last Updated**: 2025-10-24

## Overview

This document tracks the implementation of a language-agnostic IR system inspired by ASR (Abstract Semantic Representation) from LFortran. The system enables the rholang-language-server to support multiple languages with shared tooling and cross-language features.

## Architecture

```
┌──────────────────────────────────────────────────────┐
│           Language-Specific IRs                      │
│                                                      │
│  RholangNode (45 variants)    MettaNode (17 types)  │
│  • Par, Send, Contract...     • SExpr, Atom...      │
└────────────┬───────────────────────┬─────────────────┘
             │                       │
             │ implements SemanticNode trait
             ↓                       ↓
┌──────────────────────────────────────────────────────┐
│              SemanticNode Interface                  │
│  base(), metadata(), node_type(), children()         │
└────────────┬─────────────────────────────────────────┘
             │ converts to
             ↓
┌──────────────────────────────────────────────────────┐
│              UnifiedIR (12 types)                    │
│  Literal, Variable, Binding, Invocation...           │
└────────────┬─────────────────────────────────────────┘
             │ processed by
             ↓
┌──────────────────────────────────────────────────────┐
│  GenericVisitor / TransformVisitor                   │
│  • Language-agnostic traversal                       │
│  • Immutable transformations                         │
└────────────┬─────────────────────────────────────────┘
             │ used by
             ↓
┌──────────────────────────────────────────────────────┐
│  Symbol Tables, Transforms, LSP Features             │
│  • Cross-language go-to-definition                   │
│  • Unified symbol resolution                         │
│  • Multi-language workspaces                         │
└──────────────────────────────────────────────────────┘
```

## Module Structure

```
src/ir/
├── rholang_node.rs      # Rholang-specific IR (45 node variants)
├── metta_node.rs        # MeTTa-specific IR (17 node types)
├── unified_ir.rs        # Language-agnostic IR (12 construct types)
├── semantic_node.rs     # Core traits: SemanticNode, GenericVisitor, TransformVisitor
├── visitor.rs           # Rholang-specific visitor (legacy, still used)
├── formatter.rs         # IR formatting utilities
├── pipeline.rs          # Transform pipeline with dependency graph
├── symbol_table.rs      # Symbol resolution and scoping
└── transforms/          # IR transformation passes
    ├── symbol_table_builder.rs
    ├── document_symbol_visitor.rs
    └── pretty_printer.rs
```

## Implementation Phases

### ✅ Phase 1: Shared Infrastructure (COMPLETE)

**Goal**: Create language-agnostic foundation for IR systems

#### Completed Tasks

1. **✅ Create semantic_node.rs with SemanticNode trait and NodeType enum**
   - File: `src/ir/semantic_node.rs` (400 lines)
   - SemanticNode trait with universal interface
   - NodeType enum: 8 universal + 12 language-specific types
   - Metadata system: `HashMap<String, Arc<dyn Any + Send + Sync>>`
   - Thread-safe design (Send + Sync)

2. **✅ Extract and generalize NodeBase to work across languages**
   - NodeBase already language-agnostic (position tracking)
   - Used by all IR types: RholangNode, MettaNode, UnifiedIR
   - RelativePosition → Position computation

3. **✅ Create unified Metadata system with language-agnostic API**
   - Type: `HashMap<String, Arc<dyn Any + Send + Sync>>`
   - Helper functions: `empty_metadata()`, `metadata_with()`, `get_metadata()`, `insert_metadata()`
   - Extensible: transforms can attach arbitrary typed data

4. **✅ Implement SemanticNode for existing RholangNode enum**
   - File: `src/ir/rholang_node.rs` (added 160 lines)
   - All 45 variants implement SemanticNode
   - Methods: `base()`, `metadata()`, `node_type()`, `children()`, `as_any()`
   - Maps Rholang constructs to semantic types

5. **✅ Create generic Visitor trait that works with dyn SemanticNode**
   - File: `src/ir/semantic_node.rs` (added 150 lines)
   - `GenericVisitor` trait for language-agnostic traversal
   - `TransformVisitor` trait for immutable transformations
   - Type-specific handlers: `visit_literal()`, `visit_variable()`, etc.

### ✅ Phase 2: Common Semantic Layer (COMPLETE)

**Goal**: Create unified IR representation for cross-language analysis

#### Completed Tasks

6. **✅ Define UnifiedIR enum with common semantic constructs**
   - File: `src/ir/unified_ir.rs` (600 lines)
   - 12 construct types:
     - Universal: Literal, Variable, Binding, Invocation, Match, Collection, Conditional, Block, Composition
     - Extensions: RholangExt, MettaExt, Error
   - Literal enum: Bool, Integer, Float, String, Uri, Nil
   - BindingKind enum: NewBind, LetBind, PatternBind, Parameter, InputBind
   - CollectionKind enum: List, Set, Map, Tuple

7. **✅ Implement conversion from RholangNode to UnifiedIR**
   - Method: `UnifiedIR::from_rholang(node: &Arc<Node>) -> Arc<UnifiedIR>`
   - Converts Rholang-specific constructs to universal types
   - Examples:
     - `BoolLiteral` → `UnifiedIR::Literal { value: Literal::Bool(...) }`
     - `Par` → `UnifiedIR::Composition { is_parallel: true, ... }`
     - `Match` → `UnifiedIR::Match { ... }`
   - Falls back to `RholangExt` for language-specific constructs

8. **✅ Create MettaNode enum for MeTTa language constructs**
   - File: `src/ir/metta_node.rs` (400 lines)
   - 17 node types:
     - Core: SExpr, Atom, Variable
     - Special: Definition, TypeAnnotation, Eval, Match, Let, Lambda, If
     - Literals: Bool, Integer, Float, String, Nil
     - Utility: Error, Comment
   - VariableType enum: Regular ($), Grounded (&), Quoted (')
   - Implements SemanticNode trait
   - Helper methods: `atom()`, `variable()`, `sexpr()`, `is_literal()`, `name()`

9. **✅ Implement conversion from MettaNode to UnifiedIR**
   - Method: `UnifiedIR::from_metta(node: &Arc<MettaNode>) -> Arc<UnifiedIR>`
   - Converts MeTTa constructs to universal types
   - Examples:
     - `SExpr` → `UnifiedIR::Invocation { target, args, ... }`
     - `Definition` → `UnifiedIR::Binding { kind: LetBind, ... }`
     - `Match` → `UnifiedIR::Match { ... }`
   - Falls back to `MettaExt` for language-specific constructs

### ✅ Phase 3: Refactoring & Consistency (COMPLETE)

10. **✅ Rename node.rs to rholang_node.rs for consistency**
    - Renamed: `src/ir/node.rs` → `src/ir/rholang_node.rs`
    - Updated 7 files with new import paths
    - Added compatibility re-export: `pub use rholang_node as node`
    - Consistent naming: `rholang_node.rs`, `metta_node.rs`, `unified_ir.rs`

11. **✅ Update all imports to use rholang_node instead of node**
    - Updated: semantic_node.rs, unified_ir.rs, metta_node.rs, visitor.rs, formatter.rs, pipeline.rs
    - Backward compatibility maintained via re-export

## Remaining Tasks

### ⏳ Phase 4: Integration & Testing (2 tasks remaining)

12. **⏳ Update symbol table to work with SemanticNode trait objects**
    - Goal: Make symbol tables language-agnostic
    - File: `src/ir/symbol_table.rs`
    - Tasks:
      - [ ] Update symbol storage to use `Arc<dyn SemanticNode>`
      - [ ] Modify symbol resolution to work with trait objects
      - [ ] Support cross-language symbol references
      - [ ] Update inverted index for trait objects
    - Difficulty: Medium
    - Estimated effort: 2-3 hours

13. **⏳ Update transform pipeline to work with UnifiedIR**
    - Goal: Make transforms work with language-agnostic IR
    - File: `src/ir/pipeline.rs`
    - Tasks:
      - [ ] Create `GenericTransform` trait using `GenericVisitor`
      - [ ] Update Pipeline to accept `Arc<dyn SemanticNode>`
      - [ ] Implement example transform using UnifiedIR
      - [ ] Update existing transforms to work with both Node and UnifiedIR
    - Difficulty: Medium
    - Estimated effort: 3-4 hours

14. **⏳ Add tests for cross-language IR transformations**
    - Goal: Verify bidirectional conversion and cross-language features
    - Files: `tests/unified_ir_tests.rs`, `tests/cross_language_tests.rs`
    - Tasks:
      - [ ] Test RholangNode → UnifiedIR → RholangNode roundtrip
      - [ ] Test MettaNode → UnifiedIR → MettaNode roundtrip
      - [ ] Test GenericVisitor on mixed IR trees
      - [ ] Test symbol resolution across languages
      - [ ] Test transform pipeline with UnifiedIR
    - Difficulty: Low-Medium
    - Estimated effort: 2-3 hours

## What This Enables

### Immediate Benefits

1. **Multi-Language LSP Server**
   - Single server supporting Rholang (.rho) + MeTTa (.metta)
   - File type detection and language-specific parsing
   - Unified workspace indexing

2. **Cross-Language IDE Features**
   - Go-to-definition works across .rho and .metta files
   - Find-all-references spans multiple languages
   - Workspace-wide symbol search

3. **Unified Analysis**
   - Single symbol table for mixed codebases
   - Cross-language dependency analysis
   - Unified semantic diagnostics

4. **Extensibility**
   - Add new languages by implementing SemanticNode
   - Reuse transforms, symbol tables, and LSP features
   - Language-agnostic refactoring tools

### Future Possibilities

1. **Regionalized MeTTa in Rholang**
   - Embed MeTTa expressions in Rholang contracts
   - Syntax: `metta { (+ 1 2) }` within Rholang code
   - Use MeTTa for metaprogramming Rholang

2. **Cross-Language Optimization**
   - Optimize across language boundaries
   - Inline MeTTa functions into Rholang
   - Whole-program analysis

3. **Language Interop**
   - Call MeTTa from Rholang
   - Share types and data structures
   - Unified module system

## Commits History

1. **d044f1e**: "Implement language-agnostic Unified IR (Phases 1 & 2)"
   - Initial semantic_node.rs and unified_ir.rs
   - SemanticNode implementation for RholangNode

2. **fb89fea**: "Complete Unified IR system with MeTTa support (10/12 tasks)"
   - Added metta_node.rs with full MeTTa IR
   - Implemented from_metta() conversion
   - Added GenericVisitor and TransformVisitor traits

3. **a0d3224**: "Refactor: Rename node.rs to rholang_node.rs for consistency"
   - Renamed for clarity and consistency
   - Updated all imports
   - Added compatibility re-export

## Testing Status

### Compilation
- ✅ All code compiles successfully
- ✅ No errors, only dependency warnings (MORK)
- ✅ Type system validates correctly

### Unit Tests
- ✅ semantic_node.rs: NodeType display, metadata helpers, GenericVisitor
- ✅ unified_ir.rs: Literal display, BindingKind equality
- ✅ metta_node.rs: Atom creation, variable types, literal checks, SemanticNode impl

### Integration Tests
- ⏳ Pending: Cross-language IR transformations
- ⏳ Pending: Symbol table with trait objects
- ⏳ Pending: Transform pipeline with UnifiedIR

## Next Steps

To complete the unified IR implementation:

1. **Update Symbol Tables** (2-3 hours)
   - Modify `src/ir/symbol_table.rs` to use `Arc<dyn SemanticNode>`
   - Update symbol resolution for trait objects
   - Test cross-language symbol references

2. **Update Transform Pipeline** (3-4 hours)
   - Create `GenericTransform` trait in `src/ir/pipeline.rs`
   - Update Pipeline to accept trait objects
   - Migrate one transform to use UnifiedIR as proof-of-concept

3. **Add Integration Tests** (2-3 hours)
   - Create `tests/unified_ir_tests.rs`
   - Test bidirectional conversions
   - Verify GenericVisitor functionality
   - Test cross-language features

4. **MeTTa Parser Integration** (Future work)
   - Integrate Tree-Sitter MeTTa parser
   - Add .metta file support to LSP backend
   - Implement language dispatcher (file extension → parser)

5. **LSP Feature Integration** (Future work)
   - Update go-to-definition to use UnifiedIR
   - Update find-references to work cross-language
   - Implement cross-language workspace symbols

## Code Statistics

- **New Files**: 3
  - `src/ir/semantic_node.rs` (400 lines)
  - `src/ir/unified_ir.rs` (600 lines)
  - `src/ir/metta_node.rs` (400 lines)

- **Modified Files**: 8
  - `src/ir/rholang_node.rs` (+170 lines for SemanticNode impl)
  - `src/ir/mod.rs` (updated exports)
  - `src/ir/visitor.rs`, `formatter.rs`, `pipeline.rs` (updated imports)

- **Total Lines Added**: ~2,000 lines of implementation + documentation

## References

- **ASR (Abstract Semantic Representation)**: https://github.com/lfortran/lfortran/tree/main/src/libasr
- **Tree-Sitter**: https://tree-sitter.github.io/
- **LSP Specification**: https://microsoft.github.io/language-server-protocol/

## Contributors

- Implementation: Claude Code + User collaboration
- Architecture: Based on ASR design principles
- Language Expertise: Rholang (RChain), MeTTa (TrueAGI)
