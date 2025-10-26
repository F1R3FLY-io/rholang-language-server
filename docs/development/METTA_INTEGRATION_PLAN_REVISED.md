# MeTTa LSP Integration Plan (REVISED)

**Status**: Planning Phase (Updated after MeTTaTron investigation)
**Created**: 2025-10-24
**Last Updated**: 2025-10-24
**Investigation**: See `docs/research/METTATRON_INVESTIGATION.md`

---

## Executive Summary

**MAJOR SIMPLIFICATION**: MeTTaTron investigation reveals that most infrastructure is already implemented!

### Key Discoveries

1. **TreeSitterMettaParser exists** - Complete parser ready to use (`src/tree_sitter_parser.rs`)
2. **No gRPC needed** - Direct Rust library linking is recommended (5-10x faster)
3. **REPL utilities available** - `QueryHighlighter`, `SmartIndenter`, `PatternHistory` ready for LSP
4. **PathMap Par integration** - Direct Rholang ↔ MeTTa value conversion
5. **Safe validation API** - `compile_safe()` never panics, returns error s-expressions

### Impact on Timeline

**Original estimate**: 6-10 weeks (180-240 hours)
**Revised estimate**: 3-4 weeks (60-80 hours)

**Why 60% reduction**:
- No gRPC protocol design/implementation needed
- Parser already built and tested
- Validation API ready to use
- Semantic highlighting infrastructure exists

---

## Architecture Overview

```
┌──────────────────────────────────────────────────┐
│  Rholang Language Server                         │
├──────────────────────────────────────────────────┤
│  File Type Detection                             │
│  ├─ .rho → RholangParser                         │
│  └─ .metta → use mettatron::TreeSitterMettaParser│
├──────────────────────────────────────────────────┤
│  Document Lifecycle                              │
│  ├─ didOpen(.rho) → RholangDocument              │
│  └─ didOpen(.metta) → MettaDocument              │
│     └─ parser.parse(source) → Vec<SExpr>         │
│     └─ SExpr → MettaNode IR                      │
├──────────────────────────────────────────────────┤
│  Validation (via mettatron crate)                │
│  ├─ use mettatron::compile_safe(source)          │
│  ├─ Check for (error ...) s-expressions          │
│  └─ Convert to LSP Diagnostics                   │
├──────────────────────────────────────────────────┤
│  LSP Features (via REPL utilities)               │
│  ├─ Semantic Tokens (QueryHighlighter)           │
│  ├─ Formatting (SmartIndenter)                   │
│  ├─ Hover (from MettaValue types)                │
│  └─ Symbols (traverse MettaNode IR)              │
└──────────────────────────────────────────────────┘
```

---

## Phase 1: Direct MeTTaTron Integration (MVP)

**Goal**: MeTTa support via direct Rust library linking
**Duration**: 1 week (20 hours)
**Dependencies**: MeTTaTron cloned at `../MeTTa-Compiler`

### 1.1 Add MeTTaTron Dependency

**File**: `Cargo.toml`

```toml
[dependencies]
# Direct Rust linking to MeTTaTron
mettatron = { path = "../MeTTa-Compiler" }

# MeTTaTron brings these dependencies:
# - tree-sitter = "0.25"
# - tree-sitter-metta (local path)
# - mork, mork-expr, mork-frontend
# - pathmap (with jemalloc)
# - models (Rholang protobuf)
```

**Verification**:
```bash
cd /home/dylon/Workspace/f1r3fly.io/rholang-language-server
cargo check
# Should compile without errors
```

### 1.2 Create MeTTa Parser Wrapper

**File**: `src/parsers/metta_parser.rs` (new)

```rust
//! MeTTa parser using MeTTaTron's TreeSitterMettaParser

use crate::ir::metta_node::MettaNode;
use mettatron::TreeSitterMettaParser;
use mettatron::ir::SExpr;
use std::sync::Arc;

/// Wrapper around MeTTaTron's TreeSitterMettaParser
pub struct MettaParser {
    parser: TreeSitterMettaParser,
}

impl MettaParser {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            parser: TreeSitterMettaParser::new()?,
        })
    }

    /// Parses MeTTa source to SExpr AST
    pub fn parse(&mut self, source: &str) -> Result<Vec<SExpr>, String> {
        self.parser.parse(source)
    }

    /// Parses MeTTa source to MettaNode IR
    pub fn parse_to_ir(&mut self, source: &str) -> Result<Arc<MettaNode>, String> {
        let sexprs = self.parse(source)?;
        sexpr_to_metta_node(&sexprs)
    }
}

/// Converts MeTTaTron's SExpr to our MettaNode IR
fn sexpr_to_metta_node(sexprs: &[SExpr]) -> Result<Arc<MettaNode>, String> {
    // Convert each SExpr to MettaNode
    let nodes: Vec<Arc<MettaNode>> = sexprs
        .iter()
        .map(convert_sexpr)
        .collect::<Result<_, _>>()?;

    // Return as Program node
    Ok(MettaNode::program(nodes))
}

fn convert_sexpr(sexpr: &SExpr) -> Result<Arc<MettaNode>, String> {
    match sexpr {
        SExpr::Atom(name) => Ok(MettaNode::atom(name.clone())),

        SExpr::Integer(n) => Ok(MettaNode::integer(*n)),

        SExpr::Float(f) => Ok(MettaNode::float(*f)),

        SExpr::String(s) => Ok(MettaNode::string(s.clone())),

        SExpr::List(items) => {
            let children: Vec<Arc<MettaNode>> = items
                .iter()
                .map(convert_sexpr)
                .collect::<Result<_, _>>()?;

            // Check for special forms
            if let Some(SExpr::Atom(op)) = items.first() {
                match op.as_str() {
                    "=" => {
                        // Definition: (= pattern body)
                        if items.len() == 3 {
                            return Ok(MettaNode::definition(
                                convert_sexpr(&items[1])?,
                                convert_sexpr(&items[2])?,
                            ));
                        }
                    }
                    "!" => {
                        // Evaluation: !(expr)
                        if items.len() == 2 {
                            return Ok(MettaNode::eval(convert_sexpr(&items[1])?));
                        }
                    }
                    ":" => {
                        // Type annotation: (: name type)
                        if items.len() == 3 {
                            return Ok(MettaNode::type_annotation(
                                convert_sexpr(&items[1])?,
                                convert_sexpr(&items[2])?,
                            ));
                        }
                    }
                    _ => {}
                }
            }

            // Generic S-expression
            Ok(MettaNode::sexpr(children))
        }
    }
}
```

### 1.3 Create MeTTa Validator

**File**: `src/validators/metta_validator.rs` (new)

```rust
//! MeTTa validation using mettatron::compile_safe

use mettatron::{compile_safe, MettaState, MettaValue};
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};

/// Validates MeTTa source using MeTTaTron
pub struct MettaValidator;

impl MettaValidator {
    pub fn validate(source: &str) -> Vec<Diagnostic> {
        let state = compile_safe(source);
        Self::extract_diagnostics(&state, source)
    }

    fn extract_diagnostics(state: &MettaState, source: &str) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();

        for expr in &state.source {
            if let MettaValue::SExpr(items) = expr {
                // Check for error s-expressions: (error "message" details)
                if let Some(MettaValue::Atom(op)) = items.first() {
                    if op == "error" {
                        if let Some(MettaValue::String(msg)) = items.get(1) {
                            diagnostics.push(Diagnostic {
                                range: Range {
                                    start: Position { line: 0, character: 0 },
                                    end: Position { line: 0, character: 0 },
                                },
                                severity: Some(DiagnosticSeverity::ERROR),
                                message: msg.clone(),
                                source: Some("mettatron".to_string()),
                                ..Default::default()
                            });
                        }
                    }
                }
            }
        }

        diagnostics
    }
}
```

**Note**: This validator is MUCH simpler than the gRPC version in the original plan. No service, no protobuf, no connection management.

### 1.4 File Type Detection

**File**: `src/lsp/language_detection.rs` (new)

```rust
use tower_lsp::lsp_types::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentLanguage {
    Rholang,
    Metta,
    Unknown,
}

impl DocumentLanguage {
    pub fn from_uri(uri: &Url) -> Self {
        let path = uri.path();
        if path.ends_with(".rho") {
            Self::Rholang
        } else if path.ends_with(".metta") || path.ends_with(".metta2") {
            Self::Metta
        } else {
            Self::Unknown
        }
    }

    pub fn from_language_id(id: &str) -> Self {
        match id {
            "rholang" => Self::Rholang,
            "metta" | "metta2" => Self::Metta,
            _ => Self::Unknown,
        }
    }
}
```

### 1.5 MeTTa Document Management

**File**: `src/lsp/metta_document.rs` (new)

```rust
use mettatron::ir::SExpr;
use ropey::Rope;
use tower_lsp::lsp_types::{Diagnostic, Url};
use crate::ir::metta_node::MettaNode;
use crate::parsers::metta_parser::MettaParser;
use crate::validators::metta_validator::MettaValidator;
use std::sync::Arc;

pub struct MettaDocument {
    pub uri: Url,
    pub text: Rope,
    pub version: i32,
    pub sexprs: Vec<SExpr>,        // MeTTaTron's AST
    pub ir: Arc<MettaNode>,         // Our IR
    pub diagnostics: Vec<Diagnostic>,
}

impl MettaDocument {
    pub fn new(uri: Url, text: String, version: i32) -> Result<Self, String> {
        let rope = Rope::from_str(&text);

        // Parse with MeTTaTron
        let mut parser = MettaParser::new()?;
        let sexprs = parser.parse(&text)?;
        let ir = parser.parse_to_ir(&text)?;

        // Validate
        let diagnostics = MettaValidator::validate(&text);

        Ok(Self {
            uri,
            text: rope,
            version,
            sexprs,
            ir,
            diagnostics,
        })
    }

    pub fn update(&mut self, text: String) -> Result<(), String> {
        self.text = Rope::from_str(&text);
        self.version += 1;

        // Re-parse
        let mut parser = MettaParser::new()?;
        self.sexprs = parser.parse(&text)?;
        self.ir = parser.parse_to_ir(&text)?;

        // Re-validate
        self.diagnostics = MettaValidator::validate(&text);

        Ok(())
    }
}
```

### 1.6 Update LSP Backend

**File**: `src/lsp/backend.rs` (enhance)

```rust
use crate::lsp::metta_document::MettaDocument;
use crate::lsp::language_detection::DocumentLanguage;

pub struct RholangBackend {
    // Existing fields...

    /// Open MeTTa documents
    metta_documents: Arc<RwLock<HashMap<Url, MettaDocument>>>,
}

impl LanguageServer for RholangBackend {
    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let text = params.text_document.text;
        let language = DocumentLanguage::from_language_id(&params.text_document.language_id);

        match language {
            DocumentLanguage::Rholang => {
                self.open_rholang_document(uri, text).await;
            }
            DocumentLanguage::Metta => {
                self.open_metta_document(uri, text, 1).await;
            }
            DocumentLanguage::Unknown => {
                // Default to Rholang
                self.open_rholang_document(uri, text).await;
            }
        }
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        let language = DocumentLanguage::from_uri(&uri);

        match language {
            DocumentLanguage::Metta => {
                self.change_metta_document(uri, params.content_changes).await;
            }
            _ => {
                // Existing Rholang logic
                self.change_rholang_document(uri, params.content_changes).await;
            }
        }
    }
}

impl RholangBackend {
    async fn open_metta_document(&self, uri: Url, text: String, version: i32) {
        info!("Opening MeTTa document: {}", uri);

        let doc = match MettaDocument::new(uri.clone(), text, version) {
            Ok(doc) => doc,
            Err(e) => {
                error!("Failed to parse MeTTa document: {}", e);
                return;
            }
        };

        // Publish diagnostics
        self.client.publish_diagnostics(
            uri.clone(),
            doc.diagnostics.clone(),
            Some(doc.version),
        ).await;

        // Store document
        self.metta_documents.write().await.insert(uri, doc);
    }

    async fn change_metta_document(&self, uri: Url, changes: Vec<TextDocumentContentChangeEvent>) {
        let mut docs = self.metta_documents.write().await;
        let doc = match docs.get_mut(&uri) {
            Some(d) => d,
            None => return,
        };

        // Apply changes (full document sync for simplicity)
        for change in changes {
            if change.range.is_none() {
                if let Err(e) = doc.update(change.text) {
                    error!("Failed to update MeTTa document: {}", e);
                    return;
                }
            }
        }

        // Publish updated diagnostics
        self.client.publish_diagnostics(
            uri.clone(),
            doc.diagnostics.clone(),
            Some(doc.version),
        ).await;
    }
}
```

### 1.7 Testing

**File**: `tests/metta_basic_tests.rs` (new)

```rust
use mettatron::TreeSitterMettaParser;

#[test]
fn test_metta_parser_simple() {
    let mut parser = TreeSitterMettaParser::new().unwrap();

    let source = "(+ 1 2)";
    let result = parser.parse(source);

    assert!(result.is_ok());
    let sexprs = result.unwrap();
    assert_eq!(sexprs.len(), 1);
}

#[test]
fn test_metta_parser_definition() {
    let mut parser = TreeSitterMettaParser::new().unwrap();

    let source = r#"
        (: double (-> Number Number))
        (= (double $x) (* $x 2))
    "#;

    let result = parser.parse(source);
    assert!(result.is_ok());
    let sexprs = result.unwrap();
    assert_eq!(sexprs.len(), 2);
}

#[test]
fn test_metta_validator_valid() {
    use crate::validators::metta_validator::MettaValidator;

    let source = "(+ 1 2)";
    let diagnostics = MettaValidator::validate(source);

    assert_eq!(diagnostics.len(), 0);
}

#[test]
fn test_metta_validator_error() {
    use crate::validators::metta_validator::MettaValidator;

    let source = "(+ 1 2";  // Missing closing paren
    let diagnostics = MettaValidator::validate(source);

    assert!(diagnostics.len() > 0);
    assert!(diagnostics[0].message.contains("unclosed"));
}
```

**Deliverables for Phase 1**:
- ✅ MeTTaTron dependency integrated
- ✅ `.metta` files open and parse
- ✅ Syntax errors detected via `compile_safe()`
- ✅ Diagnostics published to client
- ✅ Tests passing

**Timeline**: 1 week (20 hours)

---

## Phase 2: LSP Features via REPL Utilities

**Goal**: Leverage MeTTaTron's REPL components for LSP features
**Duration**: 1 week (20 hours)
**Prerequisites**: Phase 1

### 2.1 Semantic Highlighting

**File**: `src/lsp/metta_semantic_tokens.rs` (new)

```rust
use mettatron::repl::QueryHighlighter;
use tower_lsp::lsp_types::{SemanticToken, SemanticTokenType};

/// Provides semantic tokens using MeTTaTron's QueryHighlighter
pub struct MettaSemanticTokens {
    highlighter: QueryHighlighter,
}

impl MettaSemanticTokens {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            highlighter: QueryHighlighter::new()
                .map_err(|e| format!("Failed to create highlighter: {}", e))?,
        })
    }

    pub fn tokenize(&self, source: &str) -> Vec<SemanticToken> {
        // Use MeTTaTron's highlighter
        let highlights = self.highlighter.highlight(source);

        // Convert to LSP semantic tokens
        highlights.into_iter()
            .map(|(range, token_type)| self.convert_token(range, token_type))
            .collect()
    }

    fn convert_token(&self, range: Range, token_type: &str) -> SemanticToken {
        // Map MeTTaTron token types to LSP token types
        let semantic_type = match token_type {
            "variable" => SemanticTokenType::VARIABLE,
            "function" => SemanticTokenType::FUNCTION,
            "keyword" => SemanticTokenType::KEYWORD,
            "operator" => SemanticTokenType::OPERATOR,
            "number" => SemanticTokenType::NUMBER,
            "string" => SemanticTokenType::STRING,
            "comment" => SemanticTokenType::COMMENT,
            _ => SemanticTokenType::TEXT,
        };

        SemanticToken {
            delta_line: range.start.line,
            delta_start: range.start.character,
            length: range.end.character - range.start.character,
            token_type: semantic_type as u32,
            token_modifiers_bitset: 0,
        }
    }
}
```

### 2.2 Formatting/Indentation

**File**: `src/lsp/metta_formatting.rs` (new)

```rust
use mettatron::repl::SmartIndenter;
use tower_lsp::lsp_types::{FormattingOptions, TextEdit};

pub struct MettaFormatter {
    indenter: SmartIndenter,
}

impl MettaFormatter {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            indenter: SmartIndenter::new()
                .map_err(|e| format!("Failed to create indenter: {}", e))?,
        })
    }

    pub fn format(&self, source: &str, options: FormattingOptions) -> Vec<TextEdit> {
        // Use MeTTaTron's smart indenter
        let formatted = self.indenter.indent(source);

        // Return as single replacement edit
        vec![TextEdit {
            range: Range {
                start: Position { line: 0, character: 0 },
                end: self.end_of_document(source),
            },
            new_text: formatted,
        }]
    }

    fn end_of_document(&self, source: &str) -> Position {
        let lines: Vec<&str> = source.lines().collect();
        Position {
            line: lines.len() as u32,
            character: lines.last().map(|l| l.len()).unwrap_or(0) as u32,
        }
    }
}
```

### 2.3 Hover (Type Information)

**File**: `src/lsp/metta_hover.rs` (new)

```rust
use mettatron::{compile_safe, MettaValue};
use tower_lsp::lsp_types::{Hover, HoverContents, MarkedString, Position};

pub struct MettaHover;

impl MettaHover {
    pub fn hover_at_position(source: &str, position: Position) -> Option<Hover> {
        // Compile source to get type information
        let state = compile_safe(source);

        // Extract type from MettaValue
        // (This is simplified - would need position mapping)
        for expr in &state.source {
            if let Some(type_info) = Self::extract_type(expr) {
                return Some(Hover {
                    contents: HoverContents::Scalar(
                        MarkedString::String(type_info)
                    ),
                    range: None,
                });
            }
        }

        None
    }

    fn extract_type(value: &MettaValue) -> Option<String> {
        match value {
            MettaValue::Type(t) => {
                Some(format!("{:?}", t))
            }
            MettaValue::SExpr(items) => {
                // Check for type annotation: (: name type)
                if let Some(MettaValue::Atom(op)) = items.first() {
                    if op == ":" {
                        if let Some(type_val) = items.get(2) {
                            return Some(format!("{:?}", type_val));
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }
}
```

**Deliverables for Phase 2**:
- ✅ Semantic highlighting works
- ✅ Document formatting works
- ✅ Hover shows type information
- ✅ REPL utilities successfully adapted

**Timeline**: 1 week (20 hours)

---

## Phase 3: Symbol Navigation

**Goal**: Goto definition, find references using MettaNode IR
**Duration**: 1 week (20 hours)
**Prerequisites**: Phases 1-2

### 3.1 Symbol Table Builder

**File**: `src/ir/transforms/metta_symbol_table_builder.rs` (new)

```rust
use crate::ir::metta_node::MettaNode;
use crate::ir::symbol_table::{SymbolTable, SymbolInfo, SymbolType};
use std::sync::Arc;

/// Builds symbol table from MettaNode IR
pub struct MettaSymbolTableBuilder {
    symbols: Vec<SymbolInfo>,
}

impl MettaSymbolTableBuilder {
    pub fn build(ir: &Arc<MettaNode>) -> SymbolTable {
        let mut builder = Self {
            symbols: Vec::new(),
        };
        builder.visit(ir);
        SymbolTable::from_symbols(builder.symbols)
    }

    fn visit(&mut self, node: &Arc<MettaNode>) {
        match &**node {
            MettaNode::Definition { pattern, body, base } => {
                // Extract function name from pattern
                if let Some(name) = self.extract_name(pattern) {
                    self.symbols.push(SymbolInfo {
                        name,
                        symbol_type: SymbolType::FunctionDef,
                        location: base.range.start,
                        scope: base.scope_id,
                    });
                }
                // Visit children
                self.visit(pattern);
                self.visit(body);
            }

            MettaNode::Variable { name, base, .. } => {
                self.symbols.push(SymbolInfo {
                    name: name.clone(),
                    symbol_type: SymbolType::Variable,
                    location: base.range.start,
                    scope: base.scope_id,
                });
            }

            MettaNode::SExpr { elements, .. } => {
                for elem in elements {
                    self.visit(elem);
                }
            }

            // ... other node types
            _ => {}
        }
    }

    fn extract_name(&self, node: &Arc<MettaNode>) -> Option<String> {
        match &**node {
            MettaNode::Atom { name, .. } => Some(name.clone()),
            MettaNode::SExpr { elements, .. } => {
                elements.first().and_then(|e| self.extract_name(e))
            }
            _ => None,
        }
    }
}
```

### 3.2 Goto Definition

**File**: `src/lsp/metta_goto.rs` (new)

```rust
pub async fn goto_definition(
    doc: &MettaDocument,
    position: Position
) -> Option<Location> {
    // Find symbol at position
    let symbol_name = find_symbol_at_position(&doc.ir, position)?;

    // Lookup in symbol table
    let symbols = MettaSymbolTableBuilder::build(&doc.ir);
    let definition = symbols.lookup(&symbol_name)?;

    Some(Location {
        uri: doc.uri.clone(),
        range: definition.location.into(),
    })
}
```

**Deliverables for Phase 3**:
- ✅ Goto definition works
- ✅ Find references works
- ✅ Document symbols (outline) works

**Timeline**: 1 week (20 hours)

---

## Phase 4: Embedded MeTTa in Rholang

**Goal**: Detect and validate MeTTa code within `.rho` files
**Duration**: 1 week (20 hours)
**Prerequisites**: Phases 1-3

See `MULTI_LANGUAGE_DESIGN.md` for architecture details.

**Key Components**:
1. Directive parser (`// @metta`)
2. Virtual document registry
3. Cross-language navigation

**Deliverables**:
- ✅ Embedded MeTTa detected
- ✅ Validation works across boundaries
- ✅ Navigation between languages

**Timeline**: 1 week (20 hours)

---

## Revised Timeline

| Phase | Duration | Effort | Key Benefit |
|-------|----------|--------|-------------|
| Phase 1: Direct Integration | 1 week | 20h | `.metta` files work |
| Phase 2: REPL Utilities | 1 week | 20h | Highlighting, formatting |
| Phase 3: Symbol Navigation | 1 week | 20h | Goto-def, references |
| Phase 4: Embedded Regions | 1 week | 20h | Multi-language support |
| **Total** | **4 weeks** | **80h** | Full MeTTa LSP |

**Comparison**:
- **Original estimate**: 6-10 weeks (180-240h)
- **Revised estimate**: 4 weeks (80h)
- **Reduction**: 60-67%

---

## Success Criteria

**Phase 1 Complete When**:
- ✅ `cargo check` passes with mettatron dependency
- ✅ `.metta` files open without error
- ✅ Syntax errors shown in editor
- ✅ Tests passing

**Phase 2 Complete When**:
- ✅ Semantic highlighting works
- ✅ Document formatting works
- ✅ Hover shows types

**Phase 3 Complete When**:
- ✅ Goto definition jumps to symbol
- ✅ Find references lists all uses
- ✅ Document symbols shows outline

**Phase 4 Complete When**:
- ✅ Embedded MeTTa in `.rho` files validated
- ✅ Cross-language navigation works

---

## Next Immediate Steps

1. **Add MeTTaTron Dependency**
   ```bash
   cd /home/dylon/Workspace/f1r3fly.io/rholang-language-server
   # Edit Cargo.toml
   cargo check
   ```

2. **Create Parser Wrapper**
   ```rust
   // src/parsers/metta_parser.rs
   use mettatron::TreeSitterMettaParser;
   ```

3. **Create Validator**
   ```rust
   // src/validators/metta_validator.rs
   use mettatron::compile_safe;
   ```

4. **Wire Up didOpen**
   ```rust
   // src/lsp/backend.rs
   async fn open_metta_document(...)
   ```

5. **First Test**
   ```bash
   cargo test test_metta_parser_simple
   ```

---

## Questions (REVISED)

1. ~~**MeTTaTron Deployment**~~: **NOT NEEDED** - Direct Rust linking
2. ~~**MeTTaTron API**~~: **READY** - `compile_safe()` API exists
3. ~~**gRPC Protocol**~~: **NOT NEEDED** - No gRPC required
4. **Priority**: Start with Phase 1 (standalone .metta files)?
5. **Testing**: Sample MeTTa files available?
6. **REPL Utilities**: Which query files exist for highlighting/indentation?

---

## Risk Mitigation

### Original Risks (Now Resolved)

1. ~~**MeTTaTron Performance**~~ → Direct linking is faster than gRPC
2. ~~**gRPC Connection Issues**~~ → No gRPC needed
3. ~~**Service Deployment**~~ → No separate service

### Remaining Risks

1. **MeTTaTron Build Complexity**
   - MeTTaTron depends on MORK, PathMap, models crates
   - *Mitigation*: All paths are local, should build cleanly

2. **Tree-Sitter Grammar Gaps**
   - Grammar may not cover all MeTTa syntax
   - *Mitigation*: MeTTaTron has comprehensive tests (1,656 lines)

3. **IR Conversion**
   - SExpr → MettaNode conversion may lose information
   - *Mitigation*: Keep both representations (SExpr + MettaNode)

---

## Summary of Changes from Original Plan

### Removed Components
- ❌ gRPC protocol definition (`proto/mettatron.proto`)
- ❌ gRPC client implementation
- ❌ Service deployment/management
- ❌ Connection pooling/retry logic
- ❌ Custom Tree-Sitter parser wrapper (use MeTTaTron's)

### Added Components
- ✅ Direct mettatron crate dependency
- ✅ MettaValidator using `compile_safe()`
- ✅ REPL utility adapters (QueryHighlighter, SmartIndenter)
- ✅ SExpr → MettaNode conversion

### Simplified Components
- Validation: `compile_safe()` instead of gRPC `Validate()`
- Parsing: `TreeSitterMettaParser` instead of custom wrapper
- Diagnostics: Extract from error s-expressions instead of proto conversion

---

## Conclusion

**MeTTaTron investigation reveals a major simplification opportunity**. By using direct Rust linking instead of gRPC, we eliminate 100+ hours of protocol design, service implementation, and connection management work.

The revised 4-week timeline is realistic because:
1. Parser is already built and tested
2. Validation API is ready to use
3. REPL utilities provide LSP infrastructure
4. No service deployment needed

**Recommendation**: Proceed with revised Phase 1 immediately.
