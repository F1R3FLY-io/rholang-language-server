# Unified LSP Architecture: Language-Agnostic Feature Implementation

**Date:** 2025-10-29
**Goal:** Eliminate duplication between Rholang/MeTTa handlers by leveraging unified IR (SemanticNode)

---

## Current State Analysis

### Duplication Problem

**Current Architecture**:
```
handlers.rs (1711 lines)           metta.rs (1018 lines)
├─ goto_definition()               ├─ goto_definition_metta()
├─ hover()                         ├─ hover_metta()
├─ references()                    ├─ references_metta()
├─ rename()                        ├─ rename_metta()
├─ document_symbols()              ├─ document_symbols_metta()
└─ ... (15+ more handlers)         └─ ... (duplicate logic)
```

**Duplication Estimate**: ~60-70% of LSP logic is duplicated
- Symbol resolution: 80% duplicate
- Position mapping: 95% duplicate
- Document lifecycle: 100% duplicate
- Hover formatting: 40% duplicate (some language-specific)

### What We Have Today

**Unified IR Foundation** (`SemanticNode` trait):
- ✅ Position tracking (relative → absolute)
- ✅ Semantic categories (Variable, Binding, Invocation, etc.)
- ✅ Metadata system (extensible, type-safe)
- ✅ Language-agnostic children traversal
- ✅ Symbol table integration (stored in metadata)

**Language-Specific** (Rholang/MeTTa):
- RholangNode / MettaNode enums
- Language-specific symbol resolvers
- Custom pattern matchers
- Virtual document detection

---

## Unified Architecture Design

### Core Principle

> **"Write LSP logic once using SemanticNode, specialize only where language semantics diverge"**

### Architecture Layers

```
┌──────────────────────────────────────────────────────────┐
│              LSP Protocol Layer (tower-lsp)              │
│  (initialize, didOpen, didChange, shutdown, etc.)        │
└──────────────────────────────────────────────────────────┘
                           ↓
┌──────────────────────────────────────────────────────────┐
│          Unified LSP Feature Layer (NEW)                 │
│  Generic implementations using SemanticNode trait        │
│  ├─ GenericGotoDefinition                                │
│  ├─ GenericHover                                         │
│  ├─ GenericReferences                                    │
│  ├─ GenericRename                                        │
│  ├─ GenericDocumentSymbols                               │
│  └─ GenericCompletion                                    │
└──────────────────────────────────────────────────────────┘
                           ↓
┌──────────────────────────────────────────────────────────┐
│       Language Adapter Layer (trait-based)               │
│  Language-specific behavior via traits                   │
│  ├─ SymbolResolver (already exists!)                     │
│  ├─ SymbolFilter (already exists!)                       │
│  ├─ HoverProvider (NEW)                                  │
│  ├─ CompletionProvider (NEW)                             │
│  └─ DocumentationProvider (NEW)                          │
└──────────────────────────────────────────────────────────┘
                           ↓
┌──────────────────────────────────────────────────────────┐
│         Unified IR Layer (SemanticNode)                  │
│  Language-agnostic node traversal and queries            │
│  ├─ Position tracking                                    │
│  ├─ Semantic categories                                  │
│  ├─ Metadata (symbol tables, types, etc.)                │
│  └─ Children traversal                                   │
└──────────────────────────────────────────────────────────┘
                           ↓
┌──────────────────────────────────────────────────────────┐
│      Language-Specific IR (RholangNode, MettaNode)       │
│  Implement SemanticNode trait                            │
└──────────────────────────────────────────────────────────┘
```

---

## Implementation Plan

### Phase 1: Generic LSP Feature Traits

Create trait-based contracts for language-specific behavior.

#### File: `src/lsp/features/traits.rs` (NEW)

```rust
use tower_lsp::lsp_types::*;
use crate::ir::semantic_node::SemanticNode;
use crate::ir::symbol_resolution::{SymbolResolver, ResolutionContext};

/// Language-specific hover information provider
pub trait HoverProvider: Send + Sync {
    /// Generate hover content for a symbol at position
    fn hover_for_symbol(
        &self,
        symbol_name: &str,
        node: &dyn SemanticNode,
        context: &ResolutionContext,
    ) -> Option<HoverContents>;

    /// Generate hover content for a literal
    fn hover_for_literal(
        &self,
        node: &dyn SemanticNode,
    ) -> Option<HoverContents>;
}

/// Language-specific completion provider
pub trait CompletionProvider: Send + Sync {
    /// Get completion items at position
    fn complete_at(
        &self,
        position: &Position,
        context: &CompletionContext,
        node: &dyn SemanticNode,
    ) -> Vec<CompletionItem>;

    /// Keywords for this language
    fn keywords(&self) -> &[&str];
}

/// Language-specific documentation provider
pub trait DocumentationProvider: Send + Sync {
    /// Get documentation for a symbol
    fn documentation_for(
        &self,
        symbol_name: &str,
        category: SemanticCategory,
    ) -> Option<Documentation>;
}

/// Unified language adapter
///
/// Combines all language-specific providers into a single interface
pub struct LanguageAdapter {
    pub name: String,
    pub resolver: Arc<dyn SymbolResolver>,
    pub hover: Arc<dyn HoverProvider>,
    pub completion: Arc<dyn CompletionProvider>,
    pub documentation: Arc<dyn DocumentationProvider>,
}
```

### Phase 2: Generic Feature Implementations

#### File: `src/lsp/features/goto_definition.rs` (NEW)

```rust
use tower_lsp::lsp_types::*;
use crate::ir::semantic_node::{SemanticNode, SemanticCategory};
use crate::ir::symbol_resolution::{SymbolResolver, ResolutionContext};
use crate::lsp::features::traits::LanguageAdapter;

/// Generic goto-definition implementation
///
/// Works with any language that:
/// 1. Implements SemanticNode
/// 2. Stores symbol tables in metadata
/// 3. Provides a SymbolResolver
pub struct GenericGotoDefinition {
    adapter: Arc<LanguageAdapter>,
}

impl GenericGotoDefinition {
    pub fn new(adapter: Arc<LanguageAdapter>) -> Self {
        Self { adapter }
    }

    pub async fn goto_definition(
        &self,
        root: &dyn SemanticNode,
        position: &Position,
        uri: &Url,
    ) -> LspResult<Option<GotoDefinitionResponse>> {
        // 1. Find node at position (language-agnostic)
        let node = find_node_at_position(root, position)?;

        // 2. Check semantic category
        match node.semantic_category() {
            SemanticCategory::Variable => {
                self.goto_definition_for_variable(node, position, uri).await
            }
            SemanticCategory::Invocation => {
                self.goto_definition_for_invocation(node, position, uri).await
            }
            _ => Ok(None),
        }
    }

    async fn goto_definition_for_variable(
        &self,
        node: &dyn SemanticNode,
        position: &Position,
        uri: &Url,
    ) -> LspResult<Option<GotoDefinitionResponse>> {
        // Get symbol name (language-agnostic via metadata)
        let symbol_name = self.get_symbol_name(node)?;

        // Resolve using language-specific resolver
        let context = ResolutionContext {
            uri: uri.clone(),
            position: *position,
            language: self.adapter.name.clone(),
            // ... populate from metadata
        };

        let locations = self.adapter.resolver.resolve_symbol(
            &symbol_name,
            position,
            &context,
        );

        if locations.is_empty() {
            return Ok(None);
        }

        // Convert to LSP response
        Ok(Some(GotoDefinitionResponse::Array(
            locations.into_iter().map(|loc| loc.to_lsp_location()).collect()
        )))
    }

    fn get_symbol_name(&self, node: &dyn SemanticNode) -> LspResult<String> {
        // Check metadata for symbol info (language-agnostic)
        if let Some(metadata) = node.metadata() {
            if let Some(symbol) = metadata.get("symbol") {
                if let Some(name) = symbol.downcast_ref::<String>() {
                    return Ok(name.clone());
                }
            }
        }

        // Fallback: use node text (requires text in metadata)
        Err(jsonrpc::Error::invalid_request())
    }
}
```

#### File: `src/lsp/features/hover.rs` (NEW)

```rust
/// Generic hover implementation
pub struct GenericHover {
    adapter: Arc<LanguageAdapter>,
}

impl GenericHover {
    pub async fn hover(
        &self,
        root: &dyn SemanticNode,
        position: &Position,
    ) -> LspResult<Option<Hover>> {
        let node = find_node_at_position(root, position)?;

        match node.semantic_category() {
            SemanticCategory::Variable | SemanticCategory::Binding => {
                // Use language-specific hover provider
                let symbol_name = self.get_symbol_name(node)?;
                let context = self.build_context(node, position);

                if let Some(contents) = self.adapter.hover.hover_for_symbol(
                    &symbol_name,
                    node,
                    &context,
                ) {
                    return Ok(Some(Hover {
                        contents,
                        range: Some(self.node_to_range(node)),
                    }));
                }
            }
            SemanticCategory::Literal => {
                // Generic hover for literals
                if let Some(contents) = self.adapter.hover.hover_for_literal(node) {
                    return Ok(Some(Hover {
                        contents,
                        range: Some(self.node_to_range(node)),
                    }));
                }
            }
            _ => {}
        }

        Ok(None)
    }
}
```

### Phase 3: Language-Specific Adapters

#### File: `src/lsp/features/adapters/rholang.rs` (NEW)

```rust
use super::super::traits::*;

/// Rholang-specific hover provider
pub struct RholangHoverProvider;

impl HoverProvider for RholangHoverProvider {
    fn hover_for_symbol(
        &self,
        symbol_name: &str,
        node: &dyn SemanticNode,
        context: &ResolutionContext,
    ) -> Option<HoverContents> {
        // Rholang-specific: show channel types, process info
        let metadata = node.metadata()?;
        let symbol_info = metadata.get("symbol_info")?;

        // Format Rholang-specific hover
        let markdown = format!(
            "**{}** (Rholang)\n\n\
             Type: {}\n\
             Scope: {}\n\
             Declared at: {}",
            symbol_name,
            // ... extract from metadata
        );

        Some(HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: markdown,
        }))
    }

    fn hover_for_literal(&self, node: &dyn SemanticNode) -> Option<HoverContents> {
        // Rholang literals: show process notation info
        None // Default behavior OK for now
    }
}

/// Create Rholang language adapter
pub fn create_rholang_adapter(
    resolver: Arc<dyn SymbolResolver>,
) -> LanguageAdapter {
    LanguageAdapter {
        name: "rholang".to_string(),
        resolver,
        hover: Arc::new(RholangHoverProvider),
        completion: Arc::new(RholangCompletionProvider),
        documentation: Arc::new(RholangDocumentationProvider),
    }
}
```

#### File: `src/lsp/features/adapters/metta.rs` (NEW)

```rust
/// MeTTa-specific hover provider
pub struct MettaHoverProvider {
    pattern_matcher: Arc<PatternMatcher>,
}

impl HoverProvider for MettaHoverProvider {
    fn hover_for_symbol(
        &self,
        symbol_name: &str,
        node: &dyn SemanticNode,
        context: &ResolutionContext,
    ) -> Option<HoverContents> {
        // MeTTa-specific: show arity, type signature
        let arity = self.pattern_matcher.get_arity(symbol_name)?;

        let markdown = format!(
            "**{}** (MeTTa)\n\n\
             Arity: {}\n\
             Type: Function\n\
             Usage: ({} arg1 arg2 ...)",
            symbol_name,
            arity,
            symbol_name,
        );

        Some(HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: markdown,
        }))
    }

    fn hover_for_literal(&self, node: &dyn SemanticNode) -> Option<HoverContents> {
        // MeTTa literals: show atom type
        None
    }
}

/// Create MeTTa language adapter
pub fn create_metta_adapter(
    resolver: Arc<dyn SymbolResolver>,
    pattern_matcher: Arc<PatternMatcher>,
) -> LanguageAdapter {
    LanguageAdapter {
        name: "metta".to_string(),
        resolver,
        hover: Arc::new(MettaHoverProvider { pattern_matcher }),
        completion: Arc::new(MettaCompletionProvider),
        documentation: Arc::new(MettaDocumentationProvider),
    }
}
```

### Phase 4: Unified Handler Integration

#### File: `src/lsp/backend/unified_handlers.rs` (NEW)

```rust
use crate::lsp::features::*;
use crate::lsp::features::adapters::*;

impl RholangBackend {
    /// Unified goto-definition handler
    ///
    /// Automatically dispatches to correct language based on document type
    async fn goto_definition(&self, params: GotoDefinitionParams) -> LspResult<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        // Check if this is a virtual document (embedded language)
        if uri.fragment().is_some() {
            return self.goto_definition_virtual(&uri, &position).await;
        }

        // Regular document: determine language from extension
        let language = DocumentLanguage::from_uri(&uri);
        let adapter = self.get_language_adapter(&language)?;

        // Get cached document
        let cached_doc = self.workspace.documents.get(&uri)
            .ok_or_else(|| jsonrpc::Error::invalid_params("Document not found"))?;

        // Use generic goto-definition with language adapter
        let goto_def = GenericGotoDefinition::new(adapter);
        goto_def.goto_definition(
            cached_doc.unified_ir.as_ref(),
            &position,
            &uri,
        ).await
    }

    async fn goto_definition_virtual(
        &self,
        uri: &Url,
        position: &Position,
    ) -> LspResult<Option<GotoDefinitionResponse>> {
        // Get virtual document
        let virtual_docs = self.virtual_docs.read().await;
        let virtual_doc = virtual_docs.get(uri)
            .ok_or_else(|| jsonrpc::Error::invalid_params("Virtual document not found"))?;

        // Get adapter for virtual document's language
        let adapter = self.get_language_adapter_by_name(&virtual_doc.language)?;

        // Use same generic implementation!
        let goto_def = GenericGotoDefinition::new(adapter);

        // Map position from parent to virtual document
        let virtual_pos = virtual_doc.map_position_from_parent(position);

        goto_def.goto_definition(
            virtual_doc.unified_ir.as_ref(),
            &virtual_pos,
            uri,
        ).await
    }

    fn get_language_adapter(&self, language: &DocumentLanguage) -> LspResult<Arc<LanguageAdapter>> {
        match language {
            DocumentLanguage::Rholang => Ok(self.rholang_adapter.clone()),
            DocumentLanguage::Metta => Ok(self.metta_adapter.clone()),
            _ => Err(jsonrpc::Error::invalid_params("Unsupported language")),
        }
    }

    fn get_language_adapter_by_name(&self, name: &str) -> LspResult<Arc<LanguageAdapter>> {
        match name {
            "rholang" => Ok(self.rholang_adapter.clone()),
            "metta" => Ok(self.metta_adapter.clone()),
            _ => Err(jsonrpc::Error::invalid_params("Unsupported language")),
        }
    }
}
```

### Phase 5: Backend Refactoring

#### File: `src/lsp/backend/state.rs` (MODIFY)

```rust
pub struct RholangBackend {
    // ... existing fields ...

    // Phase 5: Language adapters (replaces language-specific handlers)
    rholang_adapter: Arc<LanguageAdapter>,
    metta_adapter: Arc<LanguageAdapter>,

    // Generic feature implementations (shared across all languages)
    generic_goto_def: Arc<GenericGotoDefinition>,
    generic_hover: Arc<GenericHover>,
    generic_references: Arc<GenericReferences>,
    generic_rename: Arc<GenericRename>,
    generic_symbols: Arc<GenericDocumentSymbols>,
}

impl RholangBackend {
    pub async fn new(...) -> Result<Self, Error> {
        // Create language adapters
        let rholang_adapter = Arc::new(create_rholang_adapter(
            // ... setup resolver, filters, etc.
        ));

        let metta_adapter = Arc::new(create_metta_adapter(
            // ... setup resolver, pattern matcher, etc.
        ));

        // Create generic features (instantiate once, use for all languages)
        let generic_goto_def = Arc::new(GenericGotoDefinition::new());
        let generic_hover = Arc::new(GenericHover::new());
        // ... etc.

        Ok(Self {
            // ... existing fields ...
            rholang_adapter,
            metta_adapter,
            generic_goto_def,
            generic_hover,
            // ...
        })
    }
}
```

---

## Benefits of Unified Architecture

### 1. Massive Code Reduction

**Before**:
- `handlers.rs`: 1711 lines (Rholang)
- `metta.rs`: 1018 lines (MeTTa)
- **Total**: ~2729 lines

**After**:
- `features/*.rs`: ~800 lines (generic implementations)
- `adapters/rholang.rs`: ~200 lines (language-specific)
- `adapters/metta.rs`: ~200 lines (language-specific)
- **Total**: ~1200 lines
- **Reduction**: **56% fewer lines** (1529 lines removed)

### 2. Consistent Behavior

All languages get the same high-quality LSP features:
- Position mapping logic: single implementation
- Symbol resolution flow: single implementation
- Error handling: consistent across languages
- Progress reporting: unified approach

### 3. Easy Language Addition

**Adding a new language (e.g., Python, Scheme)**:

1. Implement `SemanticNode` for new IR (~200 lines)
2. Create language adapter (~150 lines):
   ```rust
   pub fn create_python_adapter() -> LanguageAdapter {
       LanguageAdapter {
           name: "python".to_string(),
           resolver: Arc::new(PythonSymbolResolver::new()),
           hover: Arc::new(PythonHoverProvider),
           completion: Arc::new(PythonCompletionProvider),
           documentation: Arc::new(PythonDocumentationProvider),
       }
   }
   ```
3. Register in backend (~5 lines)
4. **All LSP features work immediately** (goto-def, hover, references, rename, symbols, etc.)

**Estimated effort**: 2-3 days vs 2-3 weeks currently

### 4. Better Testing

**Generic Features**:
- Test once with mock language adapter
- Confidence all languages work correctly

**Language Adapters**:
- Small, focused unit tests
- Easy to mock and test in isolation

**Example**:
```rust
#[tokio::test]
async fn test_generic_goto_definition() {
    let mock_adapter = MockLanguageAdapter::new();
    let goto_def = GenericGotoDefinition::new(Arc::new(mock_adapter));

    // Test with synthetic SemanticNode
    let result = goto_def.goto_definition(...).await;

    assert!(result.is_ok());
}
```

### 5. Performance Optimization

**Single Optimization Point**:
- Optimize `find_node_at_position` once → all languages benefit
- Optimize symbol resolution caching once → all languages benefit
- Optimize position mapping once → all languages benefit

**Current**: Must optimize separately for Rholang and MeTTa

---

## Migration Strategy

### Step 1: Create Infrastructure (1 week)

1. Create `src/lsp/features/` module
2. Implement trait definitions
3. Create generic goto-definition (simplest feature)
4. Test with Rholang

### Step 2: Create Adapters (1 week)

1. Extract Rholang-specific logic into `RholangHoverProvider`, etc.
2. Extract MeTTa-specific logic into `MettaHoverProvider`, etc.
3. Test adapters in isolation

### Step 3: Migrate Features (2 weeks)

Migrate one feature at a time:
1. ✅ goto-definition
2. ✅ hover
3. ✅ references
4. ✅ rename
5. ✅ document_symbols
6. ... continue for remaining features

### Step 4: Remove Old Code (1 week)

1. Delete `goto_definition_rholang` (replaced by generic)
2. Delete `goto_definition_metta` (replaced by generic)
3. Delete duplicate position mapping code
4. Clean up imports

### Step 5: Documentation (1 week)

1. Update EMBEDDED_LANGUAGES_GUIDE.md
2. Create examples for adding new languages
3. Document adapter trait contracts

**Total Estimated Time**: 6 weeks
**ROI**: Every new language takes 2-3 days instead of 2-3 weeks

---

## Advanced: Composable Features

### Query Language for IR

Enable declarative queries on unified IR:

```rust
// Find all variables named "x" in scope
let vars = ir.query()
    .category(SemanticCategory::Variable)
    .name("x")
    .in_scope(current_scope)
    .collect();

// Find all function calls
let calls = ir.query()
    .category(SemanticCategory::Invocation)
    .descendant_of(current_node)
    .collect();
```

Implementation: ~500 lines, works for all languages

### Multi-Language Refactoring

Enable refactorings that work across languages:

```rust
// Rename symbol in Rholang, update references in embedded MeTTa
let rename = GenericRename::new();
rename.rename_symbol("foo", "bar", &workspace).await;
// Automatically handles:
// - Rholang files
// - Embedded MeTTa regions
// - Virtual documents
// - Cross-file references
```

---

## Next Steps

1. **Prototype**: Implement `GenericGotoDefinition` + `RholangAdapter`
2. **Validate**: Ensure it works for all Rholang goto-definition cases
3. **Benchmark**: Verify no performance regression
4. **Expand**: Add hover, references, rename
5. **Full Migration**: Complete all LSP features

---

## Success Metrics

- [ ] **Code Reduction**: 50%+ fewer lines
- [ ] **Feature Parity**: All existing features work
- [ ] **Performance**: No regression (0-5% overhead acceptable)
- [ ] **New Language**: Add test language in <3 days
- [ ] **Test Coverage**: 80%+ for generic features
- [ ] **Documentation**: Complete guide for adding languages

---

## Conclusion

The unified LSP architecture leverages the existing `SemanticNode` foundation to:

1. **Eliminate 56% of code duplication**
2. **Provide consistent, high-quality LSP features across all languages**
3. **Enable rapid addition of new languages** (2-3 days vs weeks)
4. **Simplify testing and maintenance**
5. **Create a foundation for advanced multi-language features**

The existing symbol resolution traits (`SymbolResolver`, `SymbolFilter`) already demonstrate this pattern works. We just need to extend it to all LSP features.

**Recommendation**: Start with `GenericGotoDefinition` prototype to validate the approach, then expand incrementally.
