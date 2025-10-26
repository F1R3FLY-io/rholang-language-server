# MeTTa Integration Implementation Checklist

**Date**: 2025-10-24
**Phase**: Pre-Implementation Planning
**Reference**: See `METTA_INTEGRATION_PLAN_REVISED.md` for full plan

---

## Phase 0: Pre-Implementation Verification

### 0.1 Environment Verification

- [ ] **Verify MeTTaTron location**
  ```bash
  ls -la /home/dylon/Workspace/f1r3fly.io/MeTTa-Compiler/
  # Expected: Cargo.toml, src/, tree-sitter-metta/
  ```

- [ ] **Verify MeTTaTron builds independently**
  ```bash
  cd /home/dylon/Workspace/f1r3fly.io/MeTTa-Compiler
  cargo build
  # Should complete without errors
  ```

- [ ] **Check MeTTaTron tests**
  ```bash
  cd /home/dylon/Workspace/f1r3fly.io/MeTTa-Compiler
  cargo test
  # Verify all tests pass
  ```

- [ ] **Verify tree-sitter-metta exists**
  ```bash
  ls /home/dylon/Workspace/f1r3fly.io/MeTTa-Compiler/tree-sitter-metta/
  # Expected: grammar.js, src/, package.json
  ```

- [ ] **Check current rholang-language-server builds**
  ```bash
  cd /home/dylon/Workspace/f1r3fly.io/rholang-language-server
  cargo build
  cargo test
  # Baseline: All existing tests pass
  ```

### 0.2 Dependency Analysis

- [ ] **Check for version conflicts**
  - MeTTaTron uses `tree-sitter = "0.25"`
  - Current LSP uses: `_______` (check Cargo.toml)
  - Potential conflict? YES / NO

- [ ] **Check MORK dependency paths**
  - MORK location: `../MORK/kernel`, `../MORK/expr`, `../MORK/frontend`
  - PathMap location: `../PathMap`
  - All paths exist? YES / NO

- [ ] **Check models dependency**
  - MeTTaTron depends on: `models = { path = "../f1r3node/models" }`
  - f1r3node location: `../f1r3node/`
  - Path exists? YES / NO

- [ ] **Review dependency closure**
  ```bash
  cd /home/dylon/Workspace/f1r3fly.io/MeTTa-Compiler
  cargo tree | grep -E "(tree-sitter|mork|pathmap|models)"
  # Document all transitive dependencies
  ```

### 0.3 Existing Code Review

- [ ] **Review existing IR structure**
  - File: `src/ir/metta_node.rs` (385 lines)
  - Verify MettaNode types match MeTTaTron's SExpr
  - Conversion strategy needed? YES / NO

- [ ] **Review existing UnifiedIR**
  - File: `src/ir/unified_ir.rs`
  - `UnifiedIR::from_metta()` implemented? YES / NO
  - MettaExt variant exists? YES / NO

- [ ] **Check existing parser infrastructure**
  - File: `src/parsers/` directory exists? YES / NO
  - Pattern to follow: `rholang_parser.rs`?

- [ ] **Review LSP backend structure**
  - File: `src/lsp/backend.rs`
  - `didOpen` handler location: line _____
  - Easy to extend for MeTTa? YES / NO

### 0.4 Risk Assessment

- [ ] **Identify breaking changes**
  - Will MeTTaTron change existing APIs? YES / NO
  - Will tree-sitter version bump break RholangParser? YES / NO
  - Will PathMap changes affect Rholang? YES / NO

- [ ] **Estimate build time impact**
  - Current clean build time: _____ seconds
  - Expected with MeTTaTron: _____ seconds (estimate)
  - Acceptable? YES / NO

- [ ] **Check disk space**
  ```bash
  df -h .
  # MeTTaTron build artifacts ~500MB
  # Sufficient space? YES / NO
  ```

---

## Phase 1: Dependency Integration (Day 1)

### 1.1 Backup Current State

- [ ] **Create git branch**
  ```bash
  git checkout -b dylon/metta-integration
  git push -u origin dylon/metta-integration
  ```

- [ ] **Document current state**
  ```bash
  cargo build --release
  cargo test
  # Save test output for comparison
  ```

### 1.2 Add MeTTaTron Dependency

- [ ] **Edit Cargo.toml**
  ```toml
  [dependencies]
  # Add after existing dependencies
  mettatron = { path = "../MeTTa-Compiler" }
  ```

- [ ] **Run cargo check**
  ```bash
  cargo check 2>&1 | tee metta-integration-check.log
  # Review for errors/warnings
  ```

- [ ] **Resolve dependency conflicts** (if any)
  - Document conflicts in: `METTA_INTEGRATION_CONFLICTS.md`
  - Resolution strategy: _____________________

- [ ] **Verify transitive dependencies**
  ```bash
  cargo tree | grep -i metta
  cargo tree | grep -i mork
  cargo tree | grep -i pathmap
  # All resolve correctly? YES / NO
  ```

### 1.3 Create Module Structure

- [ ] **Create parsers module** (if doesn't exist)
  ```bash
  mkdir -p src/parsers
  touch src/parsers/mod.rs
  touch src/parsers/metta_parser.rs
  ```

- [ ] **Create validators module** (if doesn't exist)
  ```bash
  mkdir -p src/validators
  touch src/validators/mod.rs
  touch src/validators/metta_validator.rs
  ```

- [ ] **Create language_detection module**
  ```bash
  touch src/lsp/language_detection.rs
  ```

- [ ] **Create metta_document module**
  ```bash
  touch src/lsp/metta_document.rs
  ```

- [ ] **Update module declarations**
  - [ ] Add to `src/parsers/mod.rs`
  - [ ] Add to `src/validators/mod.rs`
  - [ ] Add to `src/lsp/mod.rs`
  - [ ] Add to `src/lib.rs` (if needed)

---

## Phase 2: Parser Implementation (Day 2)

### 2.1 Implement MettaParser

**File**: `src/parsers/metta_parser.rs`

- [ ] **Import MeTTaTron types**
  ```rust
  use mettatron::TreeSitterMettaParser;
  use mettatron::ir::SExpr;
  ```

- [ ] **Create parser wrapper struct**
  ```rust
  pub struct MettaParser {
      parser: TreeSitterMettaParser,
  }
  ```

- [ ] **Implement constructor**
  - [ ] Handle initialization errors
  - [ ] Document error cases

- [ ] **Implement parse method**
  ```rust
  pub fn parse(&mut self, source: &str) -> Result<Vec<SExpr>, String>
  ```

- [ ] **Implement SExpr → MettaNode conversion**
  ```rust
  fn sexpr_to_metta_node(sexprs: &[SExpr]) -> Result<Arc<MettaNode>, String>
  ```
  - [ ] Handle Atom
  - [ ] Handle Integer
  - [ ] Handle Float
  - [ ] Handle String
  - [ ] Handle List
  - [ ] Handle special forms: `=`, `!`, `:`

- [ ] **Write unit tests**
  - [ ] Test simple atom parsing
  - [ ] Test number parsing
  - [ ] Test list parsing
  - [ ] Test nested lists
  - [ ] Test special forms
  - [ ] Test error cases

### 2.2 Implement MettaValidator

**File**: `src/validators/metta_validator.rs`

- [ ] **Import mettatron validation**
  ```rust
  use mettatron::{compile_safe, MettaState, MettaValue};
  ```

- [ ] **Create validator struct**
  ```rust
  pub struct MettaValidator;
  ```

- [ ] **Implement validate method**
  ```rust
  pub fn validate(source: &str) -> Vec<Diagnostic>
  ```

- [ ] **Extract error s-expressions**
  - [ ] Parse `(error "message" details)`
  - [ ] Extract line/column if available
  - [ ] Map to LSP Diagnostic structure

- [ ] **Write unit tests**
  - [ ] Test valid MeTTa code → no diagnostics
  - [ ] Test syntax error → diagnostic
  - [ ] Test unclosed paren → helpful hint
  - [ ] Test multiple errors

---

## Phase 3: LSP Integration (Day 3)

### 3.1 Implement Language Detection

**File**: `src/lsp/language_detection.rs`

- [ ] **Define DocumentLanguage enum**
  ```rust
  pub enum DocumentLanguage {
      Rholang,
      Metta,
      Unknown,
  }
  ```

- [ ] **Implement from_uri**
  - [ ] Detect `.rho` extension
  - [ ] Detect `.metta` extension
  - [ ] Detect `.metta2` extension
  - [ ] Handle unknown extensions

- [ ] **Implement from_language_id**
  - [ ] Map "rholang" language ID
  - [ ] Map "metta" language ID
  - [ ] Map "metta2" language ID

- [ ] **Write unit tests**
  - [ ] Test .rho detection
  - [ ] Test .metta detection
  - [ ] Test unknown extension

### 3.2 Implement MettaDocument

**File**: `src/lsp/metta_document.rs`

- [ ] **Define MettaDocument struct**
  ```rust
  pub struct MettaDocument {
      pub uri: Url,
      pub text: Rope,
      pub version: i32,
      pub sexprs: Vec<SExpr>,
      pub ir: Arc<MettaNode>,
      pub diagnostics: Vec<Diagnostic>,
  }
  ```

- [ ] **Implement constructor**
  - [ ] Parse source with MettaParser
  - [ ] Validate with MettaValidator
  - [ ] Store diagnostics

- [ ] **Implement update method**
  - [ ] Handle incremental changes
  - [ ] Re-parse efficiently
  - [ ] Re-validate

- [ ] **Write unit tests**
  - [ ] Test document creation
  - [ ] Test document update
  - [ ] Test error handling

### 3.3 Update LSP Backend

**File**: `src/lsp/backend.rs`

- [ ] **Add metta_documents field**
  ```rust
  metta_documents: Arc<RwLock<HashMap<Url, MettaDocument>>>,
  ```

- [ ] **Update didOpen handler**
  - [ ] Add language detection
  - [ ] Route to open_metta_document for .metta files
  - [ ] Keep existing Rholang logic

- [ ] **Implement open_metta_document**
  ```rust
  async fn open_metta_document(&self, uri: Url, text: String, version: i32)
  ```
  - [ ] Create MettaDocument
  - [ ] Store in metta_documents map
  - [ ] Publish diagnostics

- [ ] **Update didChange handler**
  - [ ] Detect MeTTa documents
  - [ ] Route to change_metta_document

- [ ] **Implement change_metta_document**
  - [ ] Update document
  - [ ] Publish new diagnostics

- [ ] **Update didClose handler**
  - [ ] Remove from metta_documents map

---

## Phase 4: Testing (Day 4)

### 4.1 Unit Tests

- [ ] **Parser tests** (`tests/metta_parser_tests.rs`)
  - [ ] Test all SExpr types
  - [ ] Test conversion to MettaNode
  - [ ] Test error cases

- [ ] **Validator tests** (`tests/metta_validator_tests.rs`)
  - [ ] Test valid code
  - [ ] Test syntax errors
  - [ ] Test semantic errors (if applicable)

- [ ] **Language detection tests**
  - [ ] Test file extension detection
  - [ ] Test language ID mapping

### 4.2 Integration Tests

- [ ] **LSP integration test** (`tests/metta_lsp_tests.rs`)
  - [ ] Test opening .metta file
  - [ ] Test receiving diagnostics
  - [ ] Test editing .metta file
  - [ ] Test closing .metta file

- [ ] **Create test fixtures**
  ```bash
  mkdir -p tests/fixtures/metta
  # Add sample .metta files
  ```

### 4.3 Manual Testing

- [ ] **Test in VSCode**
  - [ ] Install language server
  - [ ] Open .metta file
  - [ ] Verify syntax highlighting
  - [ ] Verify error detection
  - [ ] Verify diagnostics appear

- [ ] **Create test cases**
  - [ ] Valid MeTTa code
  - [ ] Syntax error (unclosed paren)
  - [ ] Multiple expressions
  - [ ] Comments
  - [ ] Large file (1000+ lines)

---

## Phase 5: Documentation & Cleanup

### 5.1 Update Documentation

- [ ] **Update README.md**
  - [ ] Add MeTTa support to feature list
  - [ ] Update dependencies section
  - [ ] Add MeTTa usage examples

- [ ] **Update CLAUDE.md**
  - [ ] Document MeTTa integration
  - [ ] Add mettatron dependency note
  - [ ] Update architecture section

- [ ] **Create METTA_LSP_FEATURES.md**
  - [ ] List supported features
  - [ ] List planned features
  - [ ] Known limitations

### 5.2 Code Cleanup

- [ ] **Run clippy**
  ```bash
  cargo clippy -- -D warnings
  ```

- [ ] **Run rustfmt**
  ```bash
  cargo fmt
  ```

- [ ] **Review all TODOs/FIXMEs**
  ```bash
  grep -r "TODO\|FIXME" src/
  ```

- [ ] **Remove debug print statements**

### 5.3 Performance Testing

- [ ] **Measure parse time**
  - [ ] Small file (< 100 lines): _____ ms
  - [ ] Medium file (500 lines): _____ ms
  - [ ] Large file (2000 lines): _____ ms

- [ ] **Measure validation time**
  - [ ] Similar benchmarks as above

- [ ] **Check memory usage**
  ```bash
  /usr/bin/time -v cargo test
  # Maximum resident set size: _____ KB
  ```

---

## Rollback Plan

### If Integration Fails

- [ ] **Revert git branch**
  ```bash
  git checkout main
  git branch -D dylon/metta-integration
  ```

- [ ] **Document blockers**
  - Create: `METTA_INTEGRATION_BLOCKERS.md`
  - List all issues encountered
  - Propose alternative approaches

- [ ] **Consider alternative approaches**
  - [ ] Option A: Fork MeTTaTron with modifications
  - [ ] Option B: Custom parser without MeTTaTron
  - [ ] Option C: Minimal integration (syntax only)

---

## Success Criteria

### Phase 1 Complete When:

- [ ] ✅ `cargo build` succeeds with mettatron dependency
- [ ] ✅ No version conflicts
- [ ] ✅ All existing tests still pass
- [ ] ✅ MettaParser compiles
- [ ] ✅ MettaValidator compiles

### Full Integration Complete When:

- [ ] ✅ Can open `.metta` files in editor
- [ ] ✅ Syntax errors show as diagnostics
- [ ] ✅ Editing updates diagnostics in real-time
- [ ] ✅ All tests pass
- [ ] ✅ No performance regression for .rho files
- [ ] ✅ Documentation updated

---

## Open Questions (Must Answer Before Starting)

### Technical Questions

1. **SExpr vs MettaNode**:
   - Do we convert immediately, or keep both representations?
   - Performance implications?

2. **Error handling**:
   - How to map MeTTaTron errors to LSP positions?
   - Does TreeSitterMettaParser provide position info?

3. **Incremental parsing**:
   - Can we reuse Tree-Sitter trees for didChange?
   - Or full re-parse each time?

4. **Memory management**:
   - How much memory does MettaDocument consume?
   - Limit on number of open .metta files?

### Integration Questions

1. **Version compatibility**:
   - Is MeTTaTron API stable?
   - Will updates break our integration?

2. **Testing strategy**:
   - Should we vendor test files from MeTTaTron?
   - Or create our own test corpus?

3. **Error recovery**:
   - What if mettatron panics (despite compile_safe)?
   - Fallback strategy?

---

## Timeline Estimate

| Phase | Task | Estimated Time |
|-------|------|----------------|
| 0 | Pre-verification | 2 hours |
| 1 | Dependency integration | 4 hours |
| 2 | Parser implementation | 8 hours |
| 3 | LSP integration | 6 hours |
| 4 | Testing | 6 hours |
| 5 | Documentation & cleanup | 4 hours |
| **Total** | | **30 hours** |

**Buffer**: +10 hours for unexpected issues = **40 hours total**

---

## Next Action

After completing this checklist:

1. Review with stakeholder
2. Address open questions
3. Get approval to proceed
4. Start Phase 0 verification

**Do NOT proceed with implementation until all Phase 0 items are verified.**
