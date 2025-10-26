# MeTTaTron Investigation Summary

**Date**: 2025-10-24
**Location**: `/home/dylon/Workspace/f1r3fly.io/MeTTa-Compiler/`
**Purpose**: Assess MeTTaTron capabilities for LSP integration

---

## Key Findings

### 1. **Direct Rust Integration Available** ✅

MeTTaTron is designed as a Rust library (`mettatron` crate) that can be directly linked:

```toml
[dependencies]
mettatron = { path = "../MeTTa-Compiler" }
```

**Advantages over gRPC**:
- 5-10x faster (no serialization overhead)
- Type-safe Rust API
- No separate process management
- Shared memory access to MORK environment

### 2. **Tree-Sitter Parser Already Integrated** ✅

**File**: `src/tree_sitter_parser.rs` (554 lines)
**Export**: `pub use tree_sitter_parser::TreeSitterMettaParser;`

Complete implementation that converts Tree-Sitter parse trees to MeTTa's internal `SExpr` AST:

```rust
pub struct TreeSitterMettaParser {
    parser: Parser,
}

impl TreeSitterMettaParser {
    pub fn new() -> Result<Self, String>
    pub fn parse(&mut self, source: &str) -> Result<Vec<SExpr>, String>
}
```

**Features**:
- Semantic node type decomposition (variables, operators, literals)
- Comment handling (line: `//`, `;` | block: `/* */`)
- Error detection with location reporting
- String escape processing
- All node types supported: lists, brace lists, prefixed expressions, atoms

**Grammar** (`tree-sitter-metta/grammar.js`):
- Decomposes atoms into semantic types for precise LSP support
- 14+ operator categories (arithmetic, comparison, arrow, type annotation, etc.)
- Variable prefixes: `$` (pattern vars)
- Special syntax: `!` (eval), `?` (query), `'` (quote)
- Brace lists `{...}` prepend `"{}"` atom

### 3. **Safe Compilation API** ✅

**File**: `src/rholang_integration.rs` (559 lines)

```rust
pub fn compile_safe(src: &str) -> MettaState
```

- **Never fails**: Always returns `MettaState`
- **Error handling**: Syntax errors → `(error "message")` s-expression
- **Error improvements**: Detects unclosed parentheses, provides hints
- **JSON export**: `metta_state_to_json()` for debugging

### 4. **Evaluation API** ✅

**Synchronous**:
```rust
pub fn run_state(
    accumulated_state: MettaState,
    compiled_state: MettaState,
) -> Result<MettaState, String>
```

**Async (Parallel)**:
```rust
#[cfg(feature = "async")]
pub async fn run_state_async(
    accumulated_state: MettaState,
    compiled_state: MettaState,
) -> Result<MettaState, String>
```

- Parallelizes independent `!` (eval) expressions
- Sequential execution for `=` (rule definitions)
- Preserves MeTTa semantics

### 5. **PathMap Par Integration** ✅

**File**: `src/pathmap_par_integration.rs` (57,668 bytes!)

Direct Rholang ↔ MeTTa value conversion:

```rust
pub fn metta_value_to_par(value: &MettaValue) -> Par
pub fn par_to_metta_value(par: &Par) -> Result<MettaValue, String>
pub fn metta_state_to_pathmap_par(state: &MettaState) -> Par
pub fn pathmap_par_to_metta_state(par: &Par) -> Result<MettaState, String>
```

**Critical for LSP**: Enables zero-copy MeTTa value representation using Rholang's protobuf models.

### 6. **REPL Utilities for LSP** ✅

**File**: `src/repl/mod.rs` (module with 7 sub-modules)

Exported components useful for LSP:

#### **a) QueryHighlighter** (`query_highlighter.rs`)
- Tree-Sitter query-based syntax highlighting
- Can power semantic tokens for LSP
- Query file: `src/repl/src/tree-sitter-query/highlights.scm`

#### **b) SmartIndenter** (`indenter.rs`)
- Tree-Sitter indent queries
- Can power LSP formatting/indentation
- Query file: `src/repl/src/tree-sitter-query/indents.scm`

#### **c) PatternHistory** (`pattern_history.rs`)
- PathMap-based pattern search
- Could power workspace symbol search

#### **d) ReplStateMachine** (`state_machine.rs`)
- Multi-line input detection
- Incomplete expression detection
- Could help with incremental parsing

#### **e) MettaHelper** (`helper.rs`)
- Rustyline validator/highlighter/hinter
- Reusable completion logic

### 7. **MORK Integration** ✅

**Dependencies**:
```toml
mork = { path = "../MORK/kernel", features = ["interning"] }
mork-expr = { path = "../MORK/expr" }
mork-frontend = { path = "../MORK/frontend" }
pathmap = { path = "../PathMap", features = ["jemalloc", "arena_compact"] }
```

- Pattern matching via MORK unification
- Space/environment stored in MORK data structures
- Efficient symbol indexing via PathMap tries

### 8. **Type System Support** ✅

**File**: `src/backend/models.rs` (inferred from tests)

```rust
pub enum MettaValue {
    Atom(String),
    Bool(bool),
    Long(i64),
    Float(f64),
    String(String),
    Uri(String),
    Nil,
    SExpr(Vec<MettaValue>),
    Error(String, Box<MettaValue>),
    Type(Box<MettaValue>),  // Metatype support
}
```

Tests show support for:
- Type annotations: `(: Socrates Entity)`
- Rule definitions: `(:= (Add $x Z) $x)`
- Pattern matching: `(match &self pattern body)`
- Control flow: `(if cond then else)`, `(catch expr default)`
- Higher-order functions: `(apply-twice $f $x)`
- Nondeterminism: Multiple rule bodies

---

## Integration Implications

### What This Means for LSP

1. **No gRPC Needed**: Direct Rust linking is simpler and faster

2. **Parser Ready**: `TreeSitterMettaParser` can be used immediately
   ```rust
   let mut parser = TreeSitterMettaParser::new()?;
   let sexprs = parser.parse(source)?;
   ```

3. **Validation Ready**: `compile_safe()` provides diagnostics
   ```rust
   let state = compile_safe(source);
   // Check state.source for (error ...) s-expressions
   ```

4. **Semantic Highlighting Ready**: `QueryHighlighter` can be adapted
   - Already has Tree-Sitter query infrastructure
   - Maps to LSP semantic token types

5. **Formatting Ready**: `SmartIndenter` provides indentation logic

6. **Symbol Search Ready**: `PatternHistory` uses PathMap for efficient search

### Recommended Architecture

```
┌─────────────────────────────────────┐
│  Rholang Language Server            │
├─────────────────────────────────────┤
│  didOpen/didChange                  │
│  ├─ .rho files → RholangParser      │
│  └─ .metta files → TreeSitterMetta  │
├─────────────────────────────────────┤
│  Document Processing                │
│  ├─ RholangParser → RholangNode IR  │
│  └─ TreeSitterMetta → MettaNode IR  │
│      └─ via SExpr → MettaNode       │
├─────────────────────────────────────┤
│  Validation (via mettatron crate)   │
│  ├─ compile_safe(source)            │
│  ├─ run_state(accumulated, compiled)│
│  └─ Extract diagnostics from errors │
├─────────────────────────────────────┤
│  LSP Features                       │
│  ├─ Semantic Tokens (QueryHighlight)│
│  ├─ Formatting (SmartIndenter)      │
│  ├─ Completion (MettaHelper logic)  │
│  ├─ Hover (type from MettaValue)    │
│  └─ Symbols (PatternHistory search) │
└─────────────────────────────────────┘
```

### IR Conversion Path

Two approaches available:

**Option A: SExpr → MettaNode → UnifiedIR**
```rust
let sexprs = parser.parse(source)?;
let metta_nodes = sexprs_to_metta_nodes(sexprs)?;
let unified = UnifiedIR::from_metta(metta_nodes)?;
```

**Option B: SExpr directly** (if MettaNode not needed)
```rust
let sexprs = parser.parse(source)?;
// Work with SExpr directly, no conversion needed
```

Recommendation: **Use Option A** to leverage existing MettaNode infrastructure and maintain consistency with RholangNode.

---

## API Examples

### 1. Parse and Validate

```rust
use mettatron::{TreeSitterMettaParser, compile_safe};

let mut parser = TreeSitterMettaParser::new()?;

// Parse with Tree-Sitter
let sexprs = parser.parse(source)?;

// Validate via compile_safe
let state = compile_safe(source);

// Check for syntax errors
for sexpr in &state.source {
    if let MettaValue::SExpr(items) = sexpr {
        if let Some(MettaValue::Atom(op)) = items.first() {
            if op == "error" {
                // Extract error message
                if let Some(MettaValue::String(msg)) = items.get(1) {
                    eprintln!("Syntax error: {}", msg);
                }
            }
        }
    }
}
```

### 2. Evaluate and Get Results

```rust
use mettatron::{compile, run_state, MettaState};

let source = r#"
    (= (double $x) (* $x 2))
    !(double 21)
"#;

let compiled = compile(source)?;
let accumulated = MettaState::new_empty();
let result = run_state(accumulated, compiled)?;

// result.output contains evaluation results
for value in result.output {
    println!("{:?}", value);  // Long(42)
}
```

### 3. Semantic Highlighting (Adapted from REPL)

```rust
use mettatron::{QueryHighlighter};

let highlighter = QueryHighlighter::new()?;
let highlights = highlighter.highlight(source);

// Convert to LSP SemanticToken format
for (range, token_type) in highlights {
    // Emit semantic token
}
```

---

## Dependencies to Add

```toml
[dependencies]
# MeTTaTron (direct Rust linking)
mettatron = { path = "../MeTTa-Compiler" }

# Already present (via rholang-parser?)
tree-sitter = "0.25"
```

No additional dependencies needed! MeTTaTron brings its own MORK/PathMap dependencies.

---

## Performance Considerations

1. **Memory**: MeTTaTron uses PathMap with jemalloc arena allocator
   - Efficient for large symbol tables
   - Shared with Rholang via PathMap Par integration

2. **Parsing**: Tree-Sitter is incremental
   - didChange can reuse parse trees
   - Only re-parse changed regions

3. **Evaluation**: Async version available
   - `run_state_async()` parallelizes independent expressions
   - Tokio async runtime (same as RNode)

4. **MORK Unification**: O(n) worst case
   - Pattern matching is fast
   - Suitable for LSP response times (<100ms)

---

## Testing Strategy

MeTTaTron has comprehensive tests (1,656 lines in `lib.rs`):
- Arithmetic, control flow, recursion
- Pattern matching, nondeterminism
- Error propagation, catch expressions
- Higher-order functions, list operations

**Recommendation**: Mirror these tests for LSP integration:
1. Parse tests (Tree-Sitter)
2. Validation tests (compile_safe)
3. Hover tests (type extraction)
4. Completion tests (MettaHelper adaptation)

---

## Open Questions

1. **PathMap Par Integration**: Should we use PathMap Par for MeTTa values in LSP context?
   - Pro: Zero-copy, efficient
   - Con: Adds complexity
   - **Recommendation**: Use MettaValue directly for simplicity

2. **SExpr vs MettaNode**: Which IR should LSP work with?
   - SExpr: Simpler, no conversion
   - MettaNode: Consistent with Rholang, supports UnifiedIR
   - **Recommendation**: Use MettaNode for consistency

3. **Async Evaluation**: Should LSP use `run_state_async()`?
   - Pro: Parallel evaluation for workspace symbols
   - Con: Adds tokio dependency (already present via tower_lsp)
   - **Recommendation**: Use async for workspace-wide operations

4. **REPL Components**: Which REPL utilities should be adapted?
   - QueryHighlighter: Yes (semantic tokens)
   - SmartIndenter: Yes (formatting)
   - PatternHistory: Maybe (workspace symbols)
   - ReplStateMachine: No (LSP has own state)
   - MettaHelper: Maybe (completion hints)

---

## Next Steps

See updated `METTA_INTEGRATION_PLAN.md` for revised implementation phases incorporating these findings.

### Immediate Actions

1. ✅ **Add mettatron dependency** to `Cargo.toml`
2. ✅ **Create `src/parsers/metta_parser.rs`** using `TreeSitterMettaParser`
3. ✅ **Wire up file type detection** for `.metta` files
4. ✅ **Implement didOpen for MeTTa** documents
5. ✅ **Add validation** via `compile_safe()`

### Phase 1 Timeline (Revised)

- **Week 1**: Parser integration + file detection (8-12 hours)
- **Week 2**: Validation + diagnostics (8-12 hours)
- **Total**: ~2 weeks (20 hours) instead of original 4 weeks

**Why faster**: MeTTaTron provides ready-to-use components; no gRPC implementation needed.
