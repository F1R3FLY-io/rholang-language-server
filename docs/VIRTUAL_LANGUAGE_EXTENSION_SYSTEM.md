# Virtual Language Extension System
## Hybrid Architecture: Generic + Specialized Support

## Design Philosophy

### The Problem
While Tree-Sitter queries can handle many LSP features, some languages need specialized capabilities:

- **Type Inference** - Requires semantic analysis beyond syntax
- **Cross-File Resolution** - Module systems, imports, package management
- **Compiler Integration** - Error checking, warnings from actual compiler
- **Advanced Completion** - Context-aware suggestions, snippets
- **Semantic Diagnostics** - Type errors, undefined variables
- **Code Actions** - Refactorings, quick fixes
- **Inlay Hints** - Type annotations, parameter names

### The Solution
**Trait-based extension system** that allows languages to opt into specialized features while falling back to generic Tree-Sitter-driven defaults.

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────┐
│                    Virtual Language Handler                          │
│                                                                       │
│  ┌────────────────────────────────────────────────────────────────┐ │
│  │  LSP Request (e.g., hover)                                      │ │
│  └────────────────────────────────────────────────────────────────┘ │
│                           ↓                                          │
│  ┌────────────────────────────────────────────────────────────────┐ │
│  │  1. Check for specialized extension                            │ │
│  │     extension_registry.get(language).hover(...)                │ │
│  └────────────────────────────────────────────────────────────────┘ │
│                           ↓                                          │
│                    Has extension?                                    │
│                 Yes ↙            ↘ No                                │
│  ┌──────────────────────┐   ┌──────────────────────────────────┐   │
│  │ Specialized Handler  │   │ Generic Tree-Sitter Handler      │   │
│  │                      │   │                                  │   │
│  │ • Type inference     │   │ • Query-driven                   │   │
│  │ • Compiler API       │   │ • Works for all languages        │   │
│  │ • Custom logic       │   │ • Zero-config fallback           │   │
│  └──────────────────────┘   └──────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────┘
```

## Extension Trait: VirtualLanguageExtension

```rust
// src/lsp/backend/virtual_language_extension.rs

use async_trait::async_trait;
use tower_lsp::lsp_types::*;
use std::sync::Arc;

use crate::language_regions::VirtualDocument;

/// Extension trait for providing specialized LSP features for virtual languages
///
/// Languages can implement this trait to override default Tree-Sitter-driven
/// behavior with specialized logic (type checking, compiler integration, etc.)
#[async_trait]
pub trait VirtualLanguageExtension: Send + Sync {
    /// Language identifier (e.g., "metta", "sql")
    fn language(&self) -> &str;

    /// Display name (e.g., "MeTTa", "SQL")
    fn display_name(&self) -> &str {
        self.language()
    }

    // ============================================================================
    // LSP Feature Implementations (all optional - fallback to generic)
    // ============================================================================

    /// Provide hover information
    ///
    /// Return `None` to fallback to generic Tree-Sitter hover
    async fn hover(
        &self,
        _doc: &VirtualDocument,
        _position: Position,
    ) -> Option<Hover> {
        None // Fallback to generic
    }

    /// Go to definition
    ///
    /// Return `None` to fallback to generic Tree-Sitter definition finding
    async fn goto_definition(
        &self,
        _doc: &VirtualDocument,
        _position: Position,
    ) -> Option<GotoDefinitionResponse> {
        None
    }

    /// Find references
    async fn find_references(
        &self,
        _doc: &VirtualDocument,
        _position: Position,
        _include_declaration: bool,
    ) -> Option<Vec<Location>> {
        None
    }

    /// Rename symbol
    async fn rename(
        &self,
        _doc: &VirtualDocument,
        _position: Position,
        _new_name: &str,
    ) -> Option<WorkspaceEdit> {
        None
    }

    /// Document symbols (outline)
    async fn document_symbols(
        &self,
        _doc: &VirtualDocument,
    ) -> Option<DocumentSymbolResponse> {
        None
    }

    /// Completion suggestions
    async fn completion(
        &self,
        _doc: &VirtualDocument,
        _position: Position,
    ) -> Option<Vec<CompletionItem>> {
        None
    }

    /// Semantic diagnostics (errors, warnings)
    ///
    /// Unlike syntax errors from Tree-Sitter, these come from semantic analysis
    async fn diagnostics(
        &self,
        _doc: &VirtualDocument,
    ) -> Option<Vec<Diagnostic>> {
        None
    }

    /// Code actions (quick fixes, refactorings)
    async fn code_actions(
        &self,
        _doc: &VirtualDocument,
        _range: Range,
    ) -> Option<Vec<CodeActionOrCommand>> {
        None
    }

    /// Inlay hints (type annotations, parameter names)
    async fn inlay_hints(
        &self,
        _doc: &VirtualDocument,
        _range: Range,
    ) -> Option<Vec<InlayHint>> {
        None
    }

    /// Signature help (function parameter info)
    async fn signature_help(
        &self,
        _doc: &VirtualDocument,
        _position: Position,
    ) -> Option<SignatureHelp> {
        None
    }

    /// Document formatting
    async fn format_document(
        &self,
        _doc: &VirtualDocument,
    ) -> Option<Vec<TextEdit>> {
        None
    }

    // ============================================================================
    // Lifecycle Hooks
    // ============================================================================

    /// Called when a virtual document is created/opened
    async fn on_document_opened(&self, _doc: &VirtualDocument) {}

    /// Called when a virtual document is modified
    async fn on_document_changed(&self, _doc: &VirtualDocument) {}

    /// Called when a virtual document is closed
    async fn on_document_closed(&self, _doc: &VirtualDocument) {}

    // ============================================================================
    // Capabilities Declaration
    // ============================================================================

    /// Declare which features this extension provides
    ///
    /// Used to inform LSP clients of capabilities and to avoid calling
    /// extension methods that aren't implemented
    fn capabilities(&self) -> ExtensionCapabilities {
        ExtensionCapabilities::default()
    }
}

/// Capabilities that an extension can declare
#[derive(Debug, Clone, Default)]
pub struct ExtensionCapabilities {
    pub hover: bool,
    pub goto_definition: bool,
    pub find_references: bool,
    pub rename: bool,
    pub document_symbols: bool,
    pub completion: bool,
    pub diagnostics: bool,
    pub code_actions: bool,
    pub inlay_hints: bool,
    pub signature_help: bool,
    pub formatting: bool,
}
```

## Extension Registry

```rust
// src/lsp/backend/virtual_language_support.rs

use std::collections::HashMap;
use std::sync::Arc;

pub struct ExtensionRegistry {
    extensions: HashMap<String, Arc<dyn VirtualLanguageExtension>>,
}

impl ExtensionRegistry {
    pub fn new() -> Self {
        Self {
            extensions: HashMap::new(),
        }
    }

    /// Register a language extension
    pub fn register(&mut self, extension: Arc<dyn VirtualLanguageExtension>) {
        let language = extension.language().to_string();
        self.extensions.insert(language, extension);
    }

    /// Get extension for a language (if registered)
    pub fn get(&self, language: &str) -> Option<&Arc<dyn VirtualLanguageExtension>> {
        self.extensions.get(language)
    }

    /// Check if a language has a specialized extension
    pub fn has_extension(&self, language: &str) -> bool {
        self.extensions.contains_key(language)
    }
}
```

## Hybrid Handler Pattern

```rust
// src/lsp/backend/virtual_language_support.rs

impl RholangBackend {
    /// Hover with fallback: extension → Tree-Sitter → none
    pub(super) async fn hover_virtual_document(
        &self,
        virtual_doc: &Arc<VirtualDocument>,
        position: Position,
    ) -> LspResult<Option<Hover>> {
        let language = &virtual_doc.language;

        // Step 1: Try specialized extension
        if let Some(extension) = self.extension_registry.get(language) {
            if extension.capabilities().hover {
                if let Some(hover) = extension.hover(virtual_doc, position).await {
                    trace!("Hover from {} extension", language);
                    return Ok(Some(hover));
                }
            }
        }

        // Step 2: Fallback to generic Tree-Sitter hover
        trace!("Hover from generic Tree-Sitter for {}", language);
        self.hover_virtual_generic(virtual_doc, position).await
    }

    /// Generic Tree-Sitter-driven hover
    async fn hover_virtual_generic(
        &self,
        virtual_doc: &Arc<VirtualDocument>,
        position: Position,
    ) -> LspResult<Option<Hover>> {
        // Use Tree-Sitter queries (as in previous plan)
        let query = self.load_language_query(&virtual_doc.language, "hover.scm")?;
        let tree = &virtual_doc.tree;
        let byte_offset = Self::position_to_byte_offset(&virtual_doc.content, position);

        // ... Tree-Sitter query execution ...

        Ok(None) // Simplified
    }
}
```

## Example: MeTTa Extension with Mettatron Integration

```rust
// src/lsp/extensions/metta_extension.rs

use async_trait::async_trait;
use tower_lsp::lsp_types::*;
use std::sync::Arc;

use crate::lsp::backend::virtual_language_extension::*;
use crate::language_regions::VirtualDocument;

pub struct MettaExtension {
    /// Optional: Mettatron compiler integration
    mettatron: Option<MettatronClient>,
}

impl MettaExtension {
    pub fn new() -> Self {
        Self {
            // Try to connect to Mettatron compiler
            mettatron: MettatronClient::try_connect().ok(),
        }
    }

    /// Use Mettatron for type inference
    async fn infer_type(&self, doc: &VirtualDocument, position: Position) -> Option<String> {
        let mettatron = self.mettatron.as_ref()?;

        // Parse MeTTa code with Mettatron
        let ast = mettatron.parse(&doc.content).ok()?;

        // Run type inference
        let types = mettatron.infer_types(&ast).ok()?;

        // Find type at position
        types.get_type_at_position(position)
    }
}

#[async_trait]
impl VirtualLanguageExtension for MettaExtension {
    fn language(&self) -> &str {
        "metta"
    }

    fn display_name(&self) -> &str {
        "MeTTa"
    }

    fn capabilities(&self) -> ExtensionCapabilities {
        ExtensionCapabilities {
            hover: true,
            goto_definition: true,
            diagnostics: self.mettatron.is_some(),  // Only if compiler available
            completion: true,
            inlay_hints: self.mettatron.is_some(),   // Type annotations
            ..Default::default()
        }
    }

    /// Specialized hover with type inference
    async fn hover(
        &self,
        doc: &VirtualDocument,
        position: Position,
    ) -> Option<Hover> {
        let tree = &doc.tree;
        let byte_offset = position_to_byte_offset(&doc.content, position)?;
        let node = tree.root_node().descendant_for_byte_range(byte_offset, byte_offset)?;
        let symbol_text = &doc.content[node.byte_range()];

        let mut hover_text = format!("**{}**", symbol_text);

        // Add type information if available
        if let Some(type_str) = self.infer_type(doc, position).await {
            hover_text.push_str(&format!("\n\n*Type:* `{}`", type_str));
        }

        // Add documentation from Mettatron
        if let Some(mettatron) = &self.mettatron {
            if let Ok(docs) = mettatron.get_documentation(symbol_text) {
                hover_text.push_str(&format!("\n\n---\n\n{}", docs));
            }
        }

        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: hover_text,
            }),
            range: Some(node_to_range(node)),
        })
    }

    /// Semantic diagnostics from Mettatron compiler
    async fn diagnostics(
        &self,
        doc: &VirtualDocument,
    ) -> Option<Vec<Diagnostic>> {
        let mettatron = self.mettatron.as_ref()?;

        // Compile MeTTa code
        let result = mettatron.compile(&doc.content).ok()?;

        // Convert compiler errors to LSP diagnostics
        let diagnostics = result.errors.iter().map(|err| {
            Diagnostic {
                range: Range {
                    start: Position {
                        line: err.line as u32,
                        character: err.column as u32,
                    },
                    end: Position {
                        line: err.line as u32,
                        character: err.column as u32 + err.length as u32,
                    },
                },
                severity: Some(DiagnosticSeverity::ERROR),
                message: err.message.clone(),
                source: Some("mettatron".to_string()),
                ..Default::default()
            }
        }).collect();

        Some(diagnostics)
    }

    /// Context-aware completion using Mettatron
    async fn completion(
        &self,
        doc: &VirtualDocument,
        position: Position,
    ) -> Option<Vec<CompletionItem>> {
        // Get context from Tree-Sitter
        let tree = &doc.tree;
        let byte_offset = position_to_byte_offset(&doc.content, position)?;
        let node = tree.root_node().descendant_for_byte_range(byte_offset, byte_offset)?;

        let mut completions = vec![
            // Built-in MeTTa functions
            CompletionItem {
                label: "import".to_string(),
                kind: Some(CompletionItemKind::KEYWORD),
                detail: Some("Import module".to_string()),
                ..Default::default()
            },
            CompletionItem {
                label: "bind".to_string(),
                kind: Some(CompletionItemKind::KEYWORD),
                detail: Some("Bind variable".to_string()),
                ..Default::default()
            },
        ];

        // Add symbols from current document (via Tree-Sitter)
        // ... query for definitions ...

        // Add symbols from Mettatron (if available)
        if let Some(mettatron) = &self.mettatron {
            if let Ok(symbols) = mettatron.get_available_symbols(&doc.content, position) {
                for symbol in symbols {
                    completions.push(CompletionItem {
                        label: symbol.name,
                        kind: Some(CompletionItemKind::VARIABLE),
                        detail: symbol.type_signature,
                        ..Default::default()
                    });
                }
            }
        }

        Some(completions)
    }

    /// Inlay hints showing inferred types
    async fn inlay_hints(
        &self,
        doc: &VirtualDocument,
        range: Range,
    ) -> Option<Vec<InlayHint>> {
        let mettatron = self.mettatron.as_ref()?;

        // Get type information
        let ast = mettatron.parse(&doc.content).ok()?;
        let types = mettatron.infer_types(&ast).ok()?;

        // Find all symbols in range
        let tree = &doc.tree;
        let mut hints = Vec::new();

        // Query for symbol definitions
        let query = load_query("metta", "definitions.scm")?;
        let mut cursor = QueryCursor::new();
        let matches = cursor.matches(&query, tree.root_node(), doc.content.as_bytes());

        for m in matches {
            for capture in m.captures {
                let node = capture.node;
                let position = node.start_position();

                // Check if in range
                if !range_contains(range, position) {
                    continue;
                }

                // Get inferred type
                if let Some(type_str) = types.get_type_for_node(node) {
                    hints.push(InlayHint {
                        position: Position {
                            line: position.row as u32,
                            character: position.column as u32,
                        },
                        label: InlayHintLabel::String(format!(": {}", type_str)),
                        kind: Some(InlayHintKind::TYPE),
                        ..Default::default()
                    });
                }
            }
        }

        Some(hints)
    }
}
```

## Registration in Backend

```rust
// src/lsp/backend/lifecycle.rs

impl RholangBackend {
    pub async fn new(client: Client, config: BackendConfig) -> Self {
        // ... existing initialization ...

        // Create extension registry
        let mut extension_registry = ExtensionRegistry::new();

        // Register MeTTa extension
        extension_registry.register(Arc::new(MettaExtension::new()));

        // Future: Register more extensions
        // extension_registry.register(Arc::new(SqlExtension::new()));
        // extension_registry.register(Arc::new(JavaScriptExtension::new()));

        Self {
            // ... existing fields ...
            extension_registry: Arc::new(extension_registry),
        }
    }
}
```

## Benefits of Hybrid Approach

### 1. **Progressive Enhancement**
```
Basic support:   Tree-Sitter only (syntax-based)
      ↓
Enhanced support: Tree-Sitter + Extension (semantics)
      ↓
Full support:     Extension + Compiler (advanced features)
```

### 2. **Graceful Degradation**
If Mettatron compiler isn't available:
- Extension returns `None` for diagnostics
- System falls back to Tree-Sitter
- Still get syntax highlighting, basic navigation

### 3. **Language-Specific Optimization**
```rust
// MeTTa can use specialized logic
async fn goto_definition(...) {
    // Use Mettatron's symbol resolution
}

// SQL falls back to Tree-Sitter
async fn goto_definition(...) {
    None  // Use generic
}
```

### 4. **Community Extensibility**
- **Easy**: Add language with just Tree-Sitter queries
- **Advanced**: Add extension for specialized features
- **Both work**: Extension can coexist with queries

## Extension Discovery

```rust
// src/lsp/extensions/mod.rs

pub mod metta_extension;

use std::sync::Arc;
use crate::lsp::backend::virtual_language_extension::VirtualLanguageExtension;

/// Discover and load all available extensions
pub fn load_extensions() -> Vec<Arc<dyn VirtualLanguageExtension>> {
    let mut extensions: Vec<Arc<dyn VirtualLanguageExtension>> = vec![];

    // Built-in extensions
    extensions.push(Arc::new(metta_extension::MettaExtension::new()));

    // Future: Plugin system
    // extensions.extend(discover_plugin_extensions());

    extensions
}
```

## Configuration Per Language

```toml
# languages.toml

[languages.metta]
name = "MeTTa"
grammar = "tree-sitter-metta"
extension = "builtin:metta"  # Use built-in MettaExtension
compiler = "mettatron"        # Optional compiler integration

[languages.metta.capabilities]
hover = "extension"           # Use extension (not Tree-Sitter)
diagnostics = "compiler"      # Requires Mettatron
completion = "hybrid"         # Merge extension + Tree-Sitter
goto_definition = "tree-sitter"  # Use generic

[languages.sql]
name = "SQL"
grammar = "tree-sitter-sql"
# No extension - all features via Tree-Sitter

[languages.javascript]
name = "JavaScript"
grammar = "tree-sitter-javascript"
extension = "plugin:/path/to/js-extension.wasm"  # Future: WASM plugins
```

## Testing Strategy

### Unit Tests: Extension Trait
```rust
#[tokio::test]
async fn test_extension_hover_fallback() {
    let extension = MockExtension::new();
    let doc = create_test_document();

    // Extension returns None → should fallback
    assert_eq!(extension.hover(&doc, Position::default()).await, None);
}
```

### Integration Tests: Hybrid Handler
```rust
#[tokio::test]
async fn test_hover_uses_extension_when_available() {
    let backend = create_backend_with_metta_extension();
    let doc = create_metta_virtual_document();

    let hover = backend.hover_virtual_document(&doc, Position::default()).await;

    // Should use MettaExtension, not generic
    assert!(hover.unwrap().contents.to_string().contains("Type:"));
}

#[tokio::test]
async fn test_hover_fallback_to_generic() {
    let backend = create_backend_without_extensions();
    let doc = create_sql_virtual_document();

    let hover = backend.hover_virtual_document(&doc, Position::default()).await;

    // Should use generic Tree-Sitter hover
    assert!(hover.is_some());
}
```

## Future: WASM Plugin System

```rust
// Future enhancement: Load extensions from WASM
pub struct WasmExtension {
    module: wasmer::Module,
    instance: wasmer::Instance,
}

#[async_trait]
impl VirtualLanguageExtension for WasmExtension {
    fn language(&self) -> &str {
        // Call WASM function
        self.call_wasm_fn("language")
    }

    async fn hover(&self, doc: &VirtualDocument, position: Position) -> Option<Hover> {
        // Marshal to WASM, call function, unmarshal result
        self.call_wasm_fn_async("hover", (doc, position)).await
    }
}
```

## Summary: Three Tiers of Support

### Tier 1: Tree-Sitter Only (Zero Config)
- Drop in grammar + queries
- Get syntax highlighting, basic navigation
- **Example**: Custom DSLs, JSON, YAML

### Tier 2: Extension + Tree-Sitter (Enhanced)
- Implement `VirtualLanguageExtension` trait
- Provide specialized features
- Fallback to Tree-Sitter for others
- **Example**: SQL with query validation, Python with type checking

### Tier 3: Full Compiler Integration (Advanced)
- Extension + Compiler API
- Semantic analysis, diagnostics, inlay hints
- Tree-Sitter for syntax, compiler for semantics
- **Example**: MeTTa + Mettatron, Rust + rust-analyzer

## Decision Matrix

| Feature | Tree-Sitter | Extension | Compiler |
|---------|------------|-----------|----------|
| Syntax Highlighting | ✅ | - | - |
| Basic Navigation | ✅ | - | - |
| Hover (basic) | ✅ | - | - |
| Hover (with types) | ❌ | ✅ | ✅ |
| Goto Definition (local) | ✅ | - | - |
| Goto Definition (cross-file) | ❌ | ✅ | ✅ |
| Diagnostics (syntax) | ✅ | - | - |
| Diagnostics (semantic) | ❌ | ⚠️ | ✅ |
| Code Completion | ❌ | ✅ | ✅ |
| Inlay Hints | ❌ | ⚠️ | ✅ |
| Code Actions | ❌ | ✅ | ✅ |

✅ = Supported, ⚠️ = Partial, ❌ = Not supported

This hybrid architecture gives us the best of both worlds: easy extensibility through Tree-Sitter queries, plus the ability to provide world-class LSP support for languages that need it.
