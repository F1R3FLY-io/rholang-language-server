# MeTTa LSP Integration Plan

**Status**: Planning Phase
**Created**: 2025-10-24
**Dependencies**: Tree-Sitter MeTTa parser, MeTTaTron compiler service
**Context**: Multi-language support architecture (see MULTI_LANGUAGE_DESIGN.md)

## Executive Summary

This plan details the integration of MeTTa language support into the rholang-language-server, enabling:
1. **Standalone `.metta` file support** - Full LSP features for pure MeTTa files
2. **Embedded MeTTa in Rholang** - MeTTa code regions within `.rho` files
3. **Cross-language navigation** - Navigate between Rholang and MeTTa symbols

The approach is **incremental**: Start with standalone MeTTa files, then add embedded region support.

## Current State Assessment

### ✅ What We Have

1. **Tree-Sitter MeTTa Parser** - Already in dependencies
   ```bash
   $ ls target/release/.fingerprint/tree-sitter-metta-*
   # Multiple build artifacts exist
   ```

2. **MettaNode IR** (`src/ir/metta_node.rs`) - 385 lines
   - 17 node types (SExpr, Atom, Variable, etc.)
   - SemanticNode implementation complete
   - children_count/child_at implemented
   - Ready for use

3. **UnifiedIR Support** - MeTTa → UnifiedIR conversion exists
   - `UnifiedIR::from_metta()` implemented
   - MettaExt variant for language-specific nodes

4. **Multi-Language Architecture** - Design complete
   - VirtualDocument pattern for embedded regions
   - Language region detection strategy
   - Cross-language symbol tables

### ❌ What We Need

1. **Tree-Sitter MeTTa Integration** - Parser not wired up to LSP
2. **MeTTaTron gRPC Client** - No validator implementation
3. **File Type Detection** - `.metta` files not recognized
4. **Document Lifecycle** - No didOpen/didChange for MeTTa
5. **LSP Feature Implementations** - No MeTTa-specific handlers

## Phase 1: Standalone MeTTa File Support (MVP)

**Goal**: Open `.metta` files in VSCode, get syntax highlighting and basic diagnostics
**Duration**: 1-2 weeks
**Prerequisites**: None

### 1.1 Tree-Sitter MeTTa Parser Integration

**File**: `src/parsers/metta_parser.rs` (new)

```rust
use tree_sitter::{Parser, Tree};
use tree_sitter_metta::language;

/// Tree-Sitter parser for MeTTa language
pub struct MettaParser {
    parser: Parser,
}

impl MettaParser {
    pub fn new() -> Result<Self, String> {
        let mut parser = Parser::new();
        parser.set_language(language())
            .map_err(|e| format!("Failed to load MeTTa grammar: {}", e))?;
        Ok(Self { parser })
    }

    /// Parses MeTTa source code to Tree-Sitter CST
    pub fn parse(&mut self, source: &str, old_tree: Option<&Tree>) -> Option<Tree> {
        self.parser.parse(source, old_tree)
    }
}

/// Converts Tree-Sitter CST to MettaNode IR
pub fn parse_metta_to_ir(source: &str) -> Result<Arc<MettaNode>, String> {
    let mut parser = MettaParser::new()?;
    let tree = parser.parse(source, None)
        .ok_or("Failed to parse MeTTa source")?;

    let root = tree.root_node();
    convert_ts_node_to_metta_ir(root, source)
}

/// Recursively converts Tree-Sitter node to MettaNode
fn convert_ts_node_to_metta_ir(node: Node, source: &str) -> Result<Arc<MettaNode>, String> {
    match node.kind() {
        "source_file" => {
            // Convert children
            let children: Vec<_> = (0..node.child_count())
                .filter_map(|i| node.child(i))
                .map(|child| convert_ts_node_to_metta_ir(child, source))
                .collect::<Result<Vec<_>, _>>()?;

            // Return Program node with all top-level expressions
            Ok(MettaNode::program(children))
        }

        "sexpr" => {
            // S-expression: (head arg1 arg2 ...)
            let children: Vec<_> = (0..node.child_count())
                .filter_map(|i| node.child(i))
                .filter(|c| !c.kind().starts_with("(") && !c.kind().starts_with(")"))
                .map(|child| convert_ts_node_to_metta_ir(child, source))
                .collect::<Result<Vec<_>, _>>()?;

            Ok(MettaNode::sexpr(children))
        }

        "atom" => {
            let text = node.utf8_text(source.as_bytes())
                .map_err(|e| format!("UTF-8 error: {}", e))?;
            Ok(MettaNode::atom(text.to_string()))
        }

        "variable" => {
            let text = node.utf8_text(source.as_bytes())
                .map_err(|e| format!("UTF-8 error: {}", e))?;

            // Detect variable type: $var, &grounded, 'quoted
            let var_type = if text.starts_with('$') {
                MettaVariableType::Regular
            } else if text.starts_with('&') {
                MettaVariableType::Grounded
            } else if text.starts_with('\'') {
                MettaVariableType::Quoted
            } else {
                MettaVariableType::Regular
            };

            Ok(MettaNode::variable(text[1..].to_string(), var_type))
        }

        "number" => {
            let text = node.utf8_text(source.as_bytes())
                .map_err(|e| format!("UTF-8 error: {}", e))?;

            if text.contains('.') {
                let value: f64 = text.parse()
                    .map_err(|e| format!("Invalid float: {}", e))?;
                Ok(MettaNode::float(value))
            } else {
                let value: i64 = text.parse()
                    .map_err(|e| format!("Invalid integer: {}", e))?;
                Ok(MettaNode::integer(value))
            }
        }

        "string" => {
            let text = node.utf8_text(source.as_bytes())
                .map_err(|e| format!("UTF-8 error: {}", e))?;
            // Remove quotes
            let unquoted = text.trim_matches('"');
            Ok(MettaNode::string(unquoted.to_string()))
        }

        "comment" => {
            let text = node.utf8_text(source.as_bytes())
                .map_err(|e| format!("UTF-8 error: {}", e))?;
            Ok(MettaNode::comment(text.to_string()))
        }

        _ => {
            Err(format!("Unknown MeTTa node kind: {}", node.kind()))
        }
    }
}
```

**Dependencies**:
```toml
# Cargo.toml
[dependencies]
tree-sitter-metta = { git = "https://github.com/trueagi-io/tree-sitter-metta" }
# Already present based on build artifacts
```

### 1.2 File Type Detection

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
    /// Detects language from file extension
    pub fn from_uri(uri: &Url) -> Self {
        let path = uri.path();

        if path.ends_with(".rho") {
            DocumentLanguage::Rholang
        } else if path.ends_with(".metta") || path.ends_with(".metta2") {
            DocumentLanguage::Metta
        } else {
            DocumentLanguage::Unknown
        }
    }

    /// Detects language from LSP language identifier
    pub fn from_language_id(id: &str) -> Self {
        match id {
            "rholang" => DocumentLanguage::Rholang,
            "metta" | "metta2" => DocumentLanguage::Metta,
            _ => DocumentLanguage::Unknown,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            DocumentLanguage::Rholang => "rholang",
            DocumentLanguage::Metta => "metta",
            DocumentLanguage::Unknown => "unknown",
        }
    }
}
```

### 1.3 MeTTa Document Lifecycle

**File**: `src/lsp/backend.rs` (enhance existing)

```rust
impl LanguageServer for RholangBackend {
    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let text = params.text_document.text;
        let language = DocumentLanguage::from_language_id(&params.text_document.language_id);

        match language {
            DocumentLanguage::Rholang => {
                // Existing Rholang logic
                self.open_rholang_document(uri, text).await;
            }

            DocumentLanguage::Metta => {
                // NEW: MeTTa document handling
                self.open_metta_document(uri, text).await;
            }

            DocumentLanguage::Unknown => {
                // Try Rholang by default
                self.open_rholang_document(uri, text).await;
            }
        }
    }
}

impl RholangBackend {
    /// Opens and parses a MeTTa document
    async fn open_metta_document(&self, uri: Url, text: String) {
        info!("Opening MeTTa document: {}", uri);

        // 1. Parse MeTTa source to IR
        let ir = match parse_metta_to_ir(&text) {
            Ok(ir) => ir,
            Err(e) => {
                error!("Failed to parse MeTTa document: {}", e);
                self.publish_parse_error(&uri, &e).await;
                return;
            }
        };

        // 2. Build symbol table for MeTTa
        let symbols = self.build_metta_symbols(&ir).await;

        // 3. Convert to UnifiedIR
        let unified_ir = UnifiedIR::from_metta(&ir);

        // 4. Store in document cache
        let doc = MettaDocument {
            uri: uri.clone(),
            text: Rope::from_str(&text),
            ir,
            unified_ir,
            symbols,
            version: 1,
        };

        self.metta_documents.write().await.insert(uri.clone(), doc);

        // 5. Run diagnostics
        self.validate_metta_document(&uri).await;

        info!("Successfully opened MeTTa document: {}", uri);
    }

    /// Builds symbol table for MeTTa document
    async fn build_metta_symbols(&self, ir: &Arc<MettaNode>)
        -> Arc<SymbolTable> {
        // Use GenericVisitor to collect symbols
        let mut collector = MettaSymbolCollector::new();
        collector.visit_node(&**ir);
        Arc::new(collector.into_symbol_table())
    }
}
```

### 1.4 MeTTa Document Storage

**File**: `src/lsp/metta_document.rs` (new)

```rust
use ropey::Rope;
use tower_lsp::lsp_types::Url;

/// Represents an open MeTTa document
pub struct MettaDocument {
    pub uri: Url,
    pub text: Rope,
    pub ir: Arc<MettaNode>,
    pub unified_ir: Arc<UnifiedIR>,
    pub symbols: Arc<SymbolTable>,
    pub version: i32,
}

impl MettaDocument {
    /// Updates document with new text (for didChange)
    pub fn update(&mut self, text: String) -> Result<(), String> {
        self.text = Rope::from_str(&text);
        self.version += 1;

        // Re-parse
        self.ir = parse_metta_to_ir(&text)?;
        self.unified_ir = UnifiedIR::from_metta(&self.ir);

        // Re-build symbols
        let mut collector = MettaSymbolCollector::new();
        collector.visit_node(&*self.ir);
        self.symbols = Arc::new(collector.into_symbol_table());

        Ok(())
    }

    /// Finds MettaNode at a specific position
    pub fn node_at_position(&self, position: Position) -> Option<&MettaNode> {
        // Use children_count/child_at to traverse tree
        find_node_at_position(&self.ir, position)
    }
}
```

### 1.5 MeTTa Symbol Collection

**File**: `src/lsp/metta_symbol_collector.rs` (new)

```rust
use crate::ir::semantic_node::{GenericVisitor, SemanticNode, SemanticCategory};

/// Collects symbols from MeTTa IR
pub struct MettaSymbolCollector {
    symbols: Vec<SymbolInfo>,
    current_scope: ScopeId,
}

impl MettaSymbolCollector {
    pub fn new() -> Self {
        Self {
            symbols: Vec::new(),
            current_scope: ScopeId::root(),
        }
    }

    pub fn into_symbol_table(self) -> SymbolTable {
        SymbolTable::from_symbols(self.symbols)
    }
}

impl GenericVisitor for MettaSymbolCollector {
    fn visit_binding(&mut self, node: &dyn SemanticNode) {
        // MeTTa definitions: (= (name params...) body)
        if let Some(metta) = node.as_any().downcast_ref::<MettaNode>() {
            match metta {
                MettaNode::Definition { pattern, body, .. } => {
                    // Extract function name from pattern
                    if let Some(name) = self.extract_definition_name(pattern) {
                        self.symbols.push(SymbolInfo {
                            name,
                            symbol_type: SymbolType::FunctionDef,
                            location: node.base().position(),
                            scope: self.current_scope,
                        });
                    }
                }

                MettaNode::Let { bindings, .. } => {
                    // (let (($x value) ($y value2)) body)
                    for binding in bindings {
                        if let Some(name) = self.extract_let_binding_name(binding) {
                            self.symbols.push(SymbolInfo {
                                name,
                                symbol_type: SymbolType::LetBind,
                                location: binding.base().position(),
                                scope: self.current_scope,
                            });
                        }
                    }
                }

                _ => {}
            }
        }

        // Continue traversing
        self.visit_children(node);
    }

    fn visit_variable(&mut self, node: &dyn SemanticNode) {
        // Track variable usage
        if let Some(metta) = node.as_any().downcast_ref::<MettaNode>() {
            if let MettaNode::Variable { name, var_type, .. } = metta {
                self.symbols.push(SymbolInfo {
                    name: name.clone(),
                    symbol_type: SymbolType::Variable,
                    location: node.base().position(),
                    scope: self.current_scope,
                });
            }
        }

        self.visit_children(node);
    }

    fn extract_definition_name(&self, pattern: &Arc<MettaNode>) -> Option<String> {
        // Pattern is typically (name params...)
        if let MettaNode::SExpr { elements, .. } = &**pattern {
            if let Some(first) = elements.first() {
                if let MettaNode::Atom { name, .. } = &***first {
                    return Some(name.clone());
                }
            }
        }
        None
    }
}
```

### 1.6 Basic MeTTa Diagnostics

**File**: `src/lsp/metta_diagnostics.rs` (new)

```rust
/// Provides basic syntax diagnostics for MeTTa (no MeTTaTron yet)
pub struct BasicMettaDiagnostics;

impl BasicMettaDiagnostics {
    pub async fn validate(ir: &Arc<MettaNode>) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();

        // Check for common syntax issues
        Self::check_unbalanced_parens(ir, &mut diagnostics);
        Self::check_undefined_symbols(ir, &mut diagnostics);
        Self::check_type_annotations(ir, &mut diagnostics);

        diagnostics
    }

    fn check_unbalanced_parens(ir: &Arc<MettaNode>, diagnostics: &mut Vec<Diagnostic>) {
        // Walk IR and verify all SExprs are well-formed
        // This should be caught by parser, but good sanity check
    }

    fn check_undefined_symbols(ir: &Arc<MettaNode>, diagnostics: &mut Vec<Diagnostic>) {
        // Build symbol table
        let mut collector = MettaSymbolCollector::new();
        collector.visit_node(&**ir);
        let symbols = collector.into_symbol_table();

        // Check for references to undefined symbols
        // (This is basic - MeTTaTron will do proper checking)
    }

    fn check_type_annotations(ir: &Arc<MettaNode>, diagnostics: &mut Vec<Diagnostic>) {
        // Warn if function definitions lack type annotations
        // (: func_name (-> Type1 Type2 RetType))
    }
}
```

### 1.7 Integration with LSP Backend

**File**: `src/lsp/backend.rs` (enhance)

```rust
pub struct RholangBackend {
    // Existing fields...

    /// Open MeTTa documents
    metta_documents: Arc<RwLock<HashMap<Url, MettaDocument>>>,
}

impl RholangBackend {
    async fn validate_metta_document(&self, uri: &Url) {
        let docs = self.metta_documents.read().await;
        let doc = match docs.get(uri) {
            Some(d) => d,
            None => return,
        };

        // Run basic diagnostics (MeTTaTron integration comes later)
        let diagnostics = BasicMettaDiagnostics::validate(&doc.ir).await;

        // Publish to client
        self.client.publish_diagnostics(
            uri.clone(),
            diagnostics,
            Some(doc.version),
        ).await;
    }
}
```

### 1.8 Testing

**File**: `tests/metta_integration_tests.rs` (new)

```rust
#[test]
fn test_parse_simple_metta() {
    let source = r#"
        (: add (-> Number Number Number))
        (= (add $x $y) (+ $x $y))
    "#;

    let ir = parse_metta_to_ir(source).unwrap();

    // Verify structure
    assert!(matches!(&*ir, MettaNode::Program { .. }));
}

#[test]
fn test_metta_symbol_collection() {
    let source = r#"
        (= (factorial 0) 1)
        (= (factorial $n) (* $n (factorial (- $n 1))))
    "#;

    let ir = parse_metta_to_ir(source).unwrap();
    let mut collector = MettaSymbolCollector::new();
    collector.visit_node(&*ir);
    let symbols = collector.into_symbol_table();

    // Should find 'factorial' definition
    assert!(symbols.lookup("factorial").is_some());
}

with_lsp_client!(test_open_metta_file, CommType::Stdio, |client: &LspClient| {
    let metta_code = r#"
        (: greet (-> String String))
        (= (greet $name) (+ "Hello, " $name))
    "#;

    let doc = client.open_document("/tmp/test.metta", metta_code)
        .expect("Failed to open MeTTa file");

    // Should receive diagnostics
    let diags = client.await_diagnostics(&doc)
        .expect("No diagnostics received");

    // Basic syntax should be valid
    assert_eq!(diags.len(), 0, "Unexpected diagnostics: {:?}", diags);
});
```

**Deliverables for Phase 1**:
- ✅ Can open `.metta` files in VSCode
- ✅ Basic syntax highlighting (via Tree-Sitter grammar)
- ✅ Parse errors shown as diagnostics
- ✅ Symbol collection works (for document outline)
- ✅ File changes trigger re-parsing

**Not Included in Phase 1**:
- ❌ MeTTaTron validation (Phase 2)
- ❌ Embedded MeTTa in Rholang (Phase 3)
- ❌ Advanced LSP features (goto-def, hover, completion)

## Phase 2: MeTTaTron Validator Integration

**Goal**: Semantic validation via MeTTaTron compiler service
**Duration**: 1-2 weeks
**Prerequisites**: Phase 1, MeTTaTron service running

### 2.1 MeTTaTron gRPC Protocol

**File**: `proto/mettatron.proto` (new)

```protobuf
syntax = "proto3";

package mettatron;

service MettaTronCompiler {
    // Validates MeTTa source code
    rpc Validate(ValidateRequest) returns (ValidateResponse);

    // Compiles MeTTa to executable form
    rpc Compile(CompileRequest) returns (CompileResponse);

    // Evaluates MeTTa expression
    rpc Eval(EvalRequest) returns (EvalResponse);

    // Gets type information for symbol
    rpc GetType(GetTypeRequest) returns (GetTypeResponse);
}

message ValidateRequest {
    string source = 1;
    // Optional: Previous validation state for incremental checking
    bytes state = 2;
}

message ValidateResponse {
    repeated Diagnostic diagnostics = 1;
    // State for incremental validation
    bytes state = 2;
}

message Diagnostic {
    Range range = 1;
    DiagnosticSeverity severity = 2;
    string message = 3;
    string code = 4;  // Error code
    repeated string related_info = 5;
}

message Range {
    Position start = 1;
    Position end = 2;
}

message Position {
    uint32 line = 1;
    uint32 character = 2;
}

enum DiagnosticSeverity {
    ERROR = 0;
    WARNING = 1;
    INFORMATION = 2;
    HINT = 3;
}

message GetTypeRequest {
    string source = 1;
    Position position = 2;
}

message GetTypeResponse {
    string type_signature = 1;  // e.g., "(-> Number Number Number)"
    string documentation = 2;
}
```

### 2.2 MeTTaTron gRPC Client

**File**: `src/validators/mettatron_client.rs` (new)

```rust
use tonic::transport::Channel;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};

// Generated from proto
pub mod mettatron {
    tonic::include_proto!("mettatron");
}

use mettatron::metta_tron_compiler_client::MettaTronCompilerClient;

/// gRPC client for MeTTaTron compiler service
pub struct MettaTronValidator {
    client: MettaTronCompilerClient<Channel>,
    address: String,
}

impl MettaTronValidator {
    /// Creates new MeTTaTron validator
    pub async fn new(address: String) -> Result<Self, tonic::transport::Error> {
        let client = MettaTronCompilerClient::connect(address.clone()).await?;
        Ok(Self { client, address })
    }

    /// Validates MeTTa source code
    pub async fn validate(&mut self, source: &str)
        -> Result<Vec<Diagnostic>, ValidatorError> {
        let request = mettatron::ValidateRequest {
            source: source.to_string(),
            state: vec![],
        };

        let response = self.client.validate(request).await
            .map_err(|e| ValidatorError::GrpcError(e.to_string()))?;

        let diagnostics = response.into_inner().diagnostics
            .into_iter()
            .map(|d| self.convert_diagnostic(d))
            .collect();

        Ok(diagnostics)
    }

    /// Converts MeTTaTron diagnostic to LSP diagnostic
    fn convert_diagnostic(&self, diag: mettatron::Diagnostic) -> Diagnostic {
        let range = diag.range.map(|r| Range {
            start: Position {
                line: r.start.map(|p| p.line).unwrap_or(0),
                character: r.start.map(|p| p.character).unwrap_or(0),
            },
            end: Position {
                line: r.end.map(|p| p.line).unwrap_or(0),
                character: r.end.map(|p| p.character).unwrap_or(0),
            },
        }).unwrap_or_else(|| Range::default());

        let severity = match diag.severity() {
            mettatron::DiagnosticSeverity::Error => DiagnosticSeverity::ERROR,
            mettatron::DiagnosticSeverity::Warning => DiagnosticSeverity::WARNING,
            mettatron::DiagnosticSeverity::Information => DiagnosticSeverity::INFORMATION,
            mettatron::DiagnosticSeverity::Hint => DiagnosticSeverity::HINT,
        };

        Diagnostic {
            range,
            severity: Some(severity),
            code: if !diag.code.is_empty() {
                Some(NumberOrString::String(diag.code))
            } else {
                None
            },
            message: diag.message,
            source: Some("mettatron".to_string()),
            ..Default::default()
        }
    }

    /// Gets type information at position
    pub async fn get_type(&mut self, source: &str, position: Position)
        -> Result<Option<String>, ValidatorError> {
        let request = mettatron::GetTypeRequest {
            source: source.to_string(),
            position: Some(mettatron::Position {
                line: position.line,
                character: position.character,
            }),
        };

        let response = self.client.get_type(request).await
            .map_err(|e| ValidatorError::GrpcError(e.to_string()))?;

        let type_info = response.into_inner();
        if type_info.type_signature.is_empty() {
            Ok(None)
        } else {
            Ok(Some(type_info.type_signature))
        }
    }
}

#[derive(Debug)]
pub enum ValidatorError {
    GrpcError(String),
    ConnectionError(String),
    ValidationFailed(String),
}
```

### 2.3 Configuration for MeTTaTron Service

**File**: `config.toml` (new or enhance existing)

```toml
[mettatron]
# MeTTaTron compiler service address
address = "http://localhost:50052"

# Enable/disable MeTTaTron validation
enabled = true

# Fallback to basic diagnostics if MeTTaTron unavailable
fallback_to_basic = true

# Timeout for validation requests (ms)
timeout_ms = 5000
```

### 2.4 Pluggable Validator Backend (Enhanced)

**File**: `src/validators/mod.rs` (new module)

```rust
pub mod mettatron_client;
pub mod basic_metta_diagnostics;

use async_trait::async_trait;

/// Trait for MeTTa validators
#[async_trait]
pub trait MettaValidator: Send + Sync {
    async fn validate(&mut self, source: &str) -> Result<Vec<Diagnostic>, ValidatorError>;
    async fn get_type(&mut self, source: &str, position: Position)
        -> Result<Option<String>, ValidatorError>;
}

/// Validator backend selection
pub enum MettaValidatorBackend {
    /// MeTTaTron gRPC service
    MettaTron(MettaTronValidator),
    /// Basic syntax checks only
    Basic(BasicMettaDiagnostics),
}

impl MettaValidatorBackend {
    /// Creates appropriate validator based on config
    pub async fn from_config(config: &Config) -> Self {
        if config.mettatron.enabled {
            match MettaTronValidator::new(config.mettatron.address.clone()).await {
                Ok(validator) => {
                    info!("Connected to MeTTaTron service at {}", config.mettatron.address);
                    Self::MettaTron(validator)
                }
                Err(e) => {
                    warn!("Failed to connect to MeTTaTron: {}. Falling back to basic validation", e);
                    if config.mettatron.fallback_to_basic {
                        Self::Basic(BasicMettaDiagnostics)
                    } else {
                        Self::MettaTron(validator)  // Will error on use
                    }
                }
            }
        } else {
            Self::Basic(BasicMettaDiagnostics)
        }
    }
}

#[async_trait]
impl MettaValidator for MettaValidatorBackend {
    async fn validate(&mut self, source: &str) -> Result<Vec<Diagnostic>, ValidatorError> {
        match self {
            Self::MettaTron(v) => v.validate(source).await,
            Self::Basic(v) => Ok(v.validate_static(source)),
        }
    }

    async fn get_type(&mut self, source: &str, position: Position)
        -> Result<Option<String>, ValidatorError> {
        match self {
            Self::MettaTron(v) => v.get_type(source, position).await,
            Self::Basic(_) => Ok(None),  // Basic validator doesn't provide types
        }
    }
}
```

### 2.5 Integration with LSP Backend

**File**: `src/lsp/backend.rs` (enhance)

```rust
pub struct RholangBackend {
    // Existing...

    /// MeTTa validator backend
    metta_validator: Arc<RwLock<MettaValidatorBackend>>,
}

impl RholangBackend {
    pub async fn new(client: Client, config: Config) -> Self {
        // ...

        let metta_validator = Arc::new(RwLock::new(
            MettaValidatorBackend::from_config(&config).await
        ));

        Self {
            // ...
            metta_validator,
        }
    }

    async fn validate_metta_document(&self, uri: &Url) {
        let docs = self.metta_documents.read().await;
        let doc = match docs.get(uri) {
            Some(d) => d,
            None => return,
        };

        // Get source text
        let source = doc.text.to_string();

        // Validate with MeTTaTron (or fallback)
        let mut validator = self.metta_validator.write().await;
        let diagnostics = match validator.validate(&source).await {
            Ok(diags) => diags,
            Err(e) => {
                error!("MeTTa validation failed: {}", e);
                vec![Diagnostic {
                    range: Range::default(),
                    severity: Some(DiagnosticSeverity::ERROR),
                    message: format!("Validation error: {}", e),
                    source: Some("mettatron".to_string()),
                    ..Default::default()
                }]
            }
        };

        // Publish to client
        self.client.publish_diagnostics(
            uri.clone(),
            diagnostics,
            Some(doc.version),
        ).await;
    }
}
```

**Deliverables for Phase 2**:
- ✅ gRPC client for MeTTaTron
- ✅ Semantic validation via MeTTaTron
- ✅ Type information on hover
- ✅ Configurable validator backend
- ✅ Graceful fallback if MeTTaTron unavailable

## Phase 3: Embedded MeTTa in Rholang (Multi-Language Documents)

**Goal**: Detect and validate MeTTa code within Rholang `.rho` files
**Duration**: 2-3 weeks
**Prerequisites**: Phases 1 & 2

See `MULTI_LANGUAGE_DESIGN.md` for detailed architecture. Key components:

1. **Directive Parser** - Detect `// @metta` comments
2. **Virtual Document Registry** - Manage embedded regions as sub-documents
3. **Semantic Detector** - Find code sent to `rho:metta:compile`
4. **Cross-language navigation** - goto-def across language boundaries

## Phase 4: Advanced LSP Features

**Goal**: Full IDE experience for MeTTa
**Duration**: 2-3 weeks
**Prerequisites**: Phases 1-3

1. **Goto Definition** - Jump to MeTTa function definitions
2. **Find References** - Find all uses of MeTTa symbol
3. **Hover** - Type signatures and documentation
4. **Completion** - Suggest MeTTa built-ins and user symbols
5. **Rename** - Rename MeTTa symbols safely
6. **Document Symbols** - Outline view of MeTTa definitions

## Testing Strategy

### Unit Tests
- Tree-Sitter parsing for all MeTTa constructs
- Symbol collection accuracy
- Diagnostic conversion (MeTTaTron → LSP)

### Integration Tests
- Open `.metta` file → receive diagnostics
- Edit MeTTa file → incremental update works
- MeTTaTron connection failure → fallback to basic

### End-to-End Tests
- VSCode with real MeTTaTron service
- Embedded MeTTa in Rholang files
- Cross-language navigation scenarios

## Timeline Estimate

| Phase | Duration | Effort | Dependencies |
|-------|----------|--------|--------------|
| Phase 1: Standalone MeTTa | 1-2 weeks | 40-50h | None |
| Phase 2: MeTTaTron Integration | 1-2 weeks | 30-40h | Phase 1, MeTTaTron API |
| Phase 3: Embedded Regions | 2-3 weeks | 60-80h | Phases 1-2 |
| Phase 4: Advanced Features | 2-3 weeks | 50-70h | Phases 1-3 |
| **Total** | **6-10 weeks** | **180-240h** | |

## Dependencies and Risks

### External Dependencies
1. **Tree-Sitter MeTTa Grammar** ✅ Already integrated
2. **MeTTaTron Compiler Service** ⚠️ Requires deployment
3. **MeTTaTron gRPC API** ⚠️ Must be documented

### Technical Risks
1. **MeTTaTron Performance** - Validation may be slow for large files
   - *Mitigation*: Incremental validation, caching, timeouts
2. **Tree-Sitter Grammar Coverage** - May not parse all MeTTa syntax
   - *Mitigation*: Contribute improvements to grammar repo
3. **gRPC Connection Issues** - Service may be unavailable
   - *Mitigation*: Graceful fallback to basic validation

### Resource Constraints
1. **MeTTaTron Service Deployment** - Needs infrastructure
2. **Testing Coverage** - Comprehensive MeTTa test corpus needed
3. **Documentation** - MeTTa language features need documenting

## Success Criteria

**Phase 1 Complete When**:
- ✅ Can open `.metta` files in VSCode
- ✅ Syntax errors highlighted
- ✅ Document outline shows MeTTa definitions
- ✅ All unit tests passing

**Phase 2 Complete When**:
- ✅ Semantic errors from MeTTaTron shown
- ✅ Type information available on hover
- ✅ Graceful handling of MeTTaTron unavailability

**Phase 3 Complete When**:
- ✅ Embedded MeTTa detected in Rholang files
- ✅ Validation works across language boundaries
- ✅ Navigation between Rholang and MeTTa works

**Full Integration Complete When**:
- ✅ All LSP features work for both languages
- ✅ Performance acceptable (< 200ms for most operations)
- ✅ Comprehensive test coverage (> 80%)
- ✅ Documentation complete

## Next Immediate Steps

1. **Verify Tree-Sitter MeTTa Integration**
   ```bash
   cd tree-sitter-metta
   tree-sitter generate
   tree-sitter test
   ```

2. **Implement `MettaParser`** (`src/parsers/metta_parser.rs`)
   - Wire up Tree-Sitter grammar
   - Test with simple MeTTa files

3. **Add File Type Detection** (`src/lsp/language_detection.rs`)
   - `.metta` extension recognition
   - LSP language ID mapping

4. **Update `didOpen` Handler**
   - Dispatch to `open_metta_document()` for `.metta` files
   - Basic IR construction and storage

5. **Create First Integration Test**
   ```rust
   #[test]
   fn test_open_metta_file() {
       // Open .metta file
       // Verify no parse errors
   }
   ```

## Questions for User

1. **MeTTaTron Deployment**: Where will the MeTTaTron service run? (localhost, remote, docker?)
2. **MeTTaTron API**: Is the gRPC API spec available? (Need to design `proto/mettatron.proto`)
3. **MeTTa Grammar**: Any known limitations in tree-sitter-metta we should be aware of?
4. **Priority**: Should we start with Phase 1 (standalone) or skip to Phase 3 (embedded)?
5. **Testing**: Do you have sample MeTTa files we can use for testing?
