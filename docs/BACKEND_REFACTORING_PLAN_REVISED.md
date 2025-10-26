# LSP Backend Refactoring Plan (Revised)
## Generalized Virtual Language Support Architecture

## Overview

Refactor `src/lsp/backend.rs` (3,495 lines) into a modular structure with **generalized virtual language support** that can handle multiple embedded languages (MeTTa, SQL, JavaScript, etc.) via Tree-Sitter parsers and query files alone.

## Key Insight

The current MeTTa-specific code (~850 lines) in backend.rs is actually the **first implementation of a general pattern** for embedded language support. Instead of creating "metta_support.rs", we should create a **generic virtual language handler** that works with any Tree-Sitter grammar.

## Architecture Philosophy

### Current State
- `language_regions/` - Infrastructure for detecting embedded languages (✅ Already generalized)
  - `DirectiveParser` - Detects `// @language` directives
  - `SemanticDetector` - Detects semantic patterns (e.g., compiler calls)
  - `ChannelFlowAnalyzer` - Detects channel flow patterns
  - `VirtualDocumentRegistry` - Manages virtual documents

- `backend.rs` - MeTTa-specific LSP handlers (❌ Needs generalization)
  - `hover_metta()`, `goto_definition_metta()`, `rename_metta()`, etc.
  - These should become **language-agnostic** handlers

### Target State
**Virtual languages should be plug-and-play**: Drop in a Tree-Sitter grammar + query files → Get LSP support automatically.

## Revised Module Structure

```
src/lsp/backend/
├── mod.rs                          # Module coordinator
├── state.rs                        # RholangBackend struct + types
├── lifecycle.rs                    # Initialization, spawners, shutdown
├── document_processing.rs          # Document parsing, indexing, validation
├── workspace.rs                    # Workspace indexing, file watching
├── symbol_operations.rs            # Symbol queries, position lookups
├── virtual_language_support.rs     # ⭐ GENERALIZED virtual language LSP handlers
├── lsp_handlers.rs                 # LanguageServer trait impl (delegates)
└── utils.rs                        # SemanticTokensBuilder, utilities
```

## Key Module: virtual_language_support.rs

This module replaces the MeTTa-specific code with a **generic virtual language handler**.

### Design Principles

1. **Tree-Sitter Driven**: All language understanding comes from Tree-Sitter grammar + queries
2. **Query-Based Features**: LSP features driven by Tree-Sitter query files
3. **Zero Hard-Coding**: No language-specific logic in Rust code
4. **Extensible**: Adding a new language = adding grammar + queries

### Tree-Sitter Query Files for LSP Features

For each embedded language, provide query files:

```
grammars/<language>/
├── grammar.js                  # Tree-Sitter grammar (required)
└── queries/
    ├── highlights.scm          # Syntax highlighting
    ├── symbols.scm             # Document symbols (for outline)
    ├── definitions.scm         # Go-to-definition targets
    ├── references.scm          # Find references
    ├── hover.scm               # Hover information
    └── rename.scm              # Rename targets
```

### Example: MeTTa Grammar Queries

**highlights.scm** (already exists for Tree-Sitter):
```scheme
(symbol) @variable
(atom) @function
(string) @string
```

**definitions.scm** (new - marks definition sites):
```scheme
; Function definitions
(expression
  (symbol) @definition.function
  (#match? @definition.function "^define"))

; Variable bindings
(let_binding
  (symbol) @definition.variable)
```

**references.scm** (new - marks reference sites):
```scheme
(symbol) @reference
```

**hover.scm** (new - provides hover content):
```scheme
; Built-in functions
(symbol) @hover.builtin
(#match? @hover.builtin "^(import|include|eval)")

; User definitions
(symbol) @hover.symbol
```

### Generic LSP Handler Pattern

```rust
// virtual_language_support.rs

impl RholangBackend {
    /// Generic hover for any virtual document
    pub(super) async fn hover_virtual_document(
        &self,
        virtual_doc: &Arc<VirtualDocument>,
        position: LspPosition,
    ) -> LspResult<Option<Hover>> {
        let language = &virtual_doc.language;

        // Get Tree-Sitter tree (cached in VirtualDocument)
        let tree = &virtual_doc.tree;
        let root = tree.root_node();

        // Convert position to byte offset
        let byte_offset = Self::position_to_byte_offset(&virtual_doc.content, position);

        // Find node at position
        let node = root.descendant_for_byte_range(byte_offset, byte_offset)?;

        // Load hover query for this language
        let query = self.load_language_query(language, "hover.scm")?;

        // Execute query
        let mut cursor = QueryCursor::new();
        let matches = cursor.matches(&query, root, virtual_doc.content.as_bytes());

        // Find match containing our position
        for m in matches {
            for capture in m.captures {
                if Self::node_contains_position(capture.node, byte_offset) {
                    // Extract hover content from query metadata
                    let hover_text = self.generate_hover_content(
                        language,
                        capture.node,
                        &virtual_doc.content,
                    );

                    return Ok(Some(Hover {
                        contents: HoverContents::Markup(MarkupContent {
                            kind: MarkupKind::Markdown,
                            value: hover_text,
                        }),
                        range: Some(Self::node_to_range(capture.node)),
                    }));
                }
            }
        }

        Ok(None)
    }

    /// Generic goto-definition for any virtual document
    pub(super) async fn goto_definition_virtual_document(
        &self,
        virtual_doc: &Arc<VirtualDocument>,
        position: LspPosition,
    ) -> LspResult<Option<GotoDefinitionResponse>> {
        let language = &virtual_doc.language;
        let tree = &virtual_doc.tree;
        let root = tree.root_node();
        let byte_offset = Self::position_to_byte_offset(&virtual_doc.content, position);

        // Step 1: Find symbol at cursor using references.scm
        let ref_query = self.load_language_query(language, "references.scm")?;
        let symbol_name = self.find_symbol_at_position(
            &ref_query,
            root,
            byte_offset,
            &virtual_doc.content,
        )?;

        // Step 2: Find definition using definitions.scm
        let def_query = self.load_language_query(language, "definitions.scm")?;
        let def_location = self.find_definition_by_name(
            &def_query,
            root,
            &symbol_name,
            &virtual_doc.content,
            &virtual_doc.uri,
        )?;

        Ok(def_location.map(GotoDefinitionResponse::Scalar))
    }

    /// Generic rename for any virtual document
    pub(super) async fn rename_virtual_document(
        &self,
        virtual_doc: &Arc<VirtualDocument>,
        position: LspPosition,
        new_name: &str,
    ) -> LspResult<Option<WorkspaceEdit>> {
        let language = &virtual_doc.language;
        let tree = &virtual_doc.tree;
        let root = tree.root_node();
        let byte_offset = Self::position_to_byte_offset(&virtual_doc.content, position);

        // Find symbol at position
        let ref_query = self.load_language_query(language, "references.scm")?;
        let symbol_name = self.find_symbol_at_position(
            &ref_query,
            root,
            byte_offset,
            &virtual_doc.content,
        )?;

        // Find all occurrences (both definitions and references)
        let mut locations = Vec::new();

        // Add definitions
        let def_query = self.load_language_query(language, "definitions.scm")?;
        locations.extend(self.find_all_by_name(&def_query, root, &symbol_name, &virtual_doc.content));

        // Add references
        locations.extend(self.find_all_by_name(&ref_query, root, &symbol_name, &virtual_doc.content));

        // Build WorkspaceEdit
        let edits = self.build_rename_edits(&virtual_doc.uri, locations, new_name);

        Ok(Some(edits))
    }

    /// Load a query file for a language
    fn load_language_query(&self, language: &str, query_file: &str) -> Result<Query, String> {
        // Load from grammars/<language>/queries/<query_file>
        // Cache queries for performance
        todo!()
    }

    /// Generate hover content using language-specific metadata
    fn generate_hover_content(
        &self,
        language: &str,
        node: TSNode,
        source: &str,
    ) -> String {
        // Could load from:
        // - Query metadata (#set! hover "...")
        // - Language server protocol files
        // - Documentation databases
        let node_text = &source[node.byte_range()];

        format!(
            "**{}** ({})\n\nNode type: `{}`",
            node_text,
            language,
            node.kind()
        )
    }
}
```

### Query-Driven Feature Matrix

| LSP Feature | Query File | Captures | Metadata |
|-------------|-----------|----------|----------|
| **Hover** | `hover.scm` | `@hover.builtin`, `@hover.symbol` | `#set! hover "text"` |
| **Goto Definition** | `definitions.scm` | `@definition.function`, `@definition.variable` | - |
| **Find References** | `references.scm` | `@reference` | - |
| **Document Symbols** | `symbols.scm` | `@symbol.function`, `@symbol.class` | - |
| **Rename** | `rename.scm` | `@rename.target` | - |
| **Semantic Tokens** | `highlights.scm` | `@variable`, `@function`, etc. | - |
| **Document Highlight** | `references.scm` | `@reference` | - |

## Implementation Strategy

### Phase 1: Extract Core Backend Modules (Same as before)
1. Create directory structure
2. Extract `utils.rs` - SemanticTokensBuilder
3. Extract `state.rs` - RholangBackend struct
4. Extract `symbol_operations.rs` - Rholang symbol operations
5. Extract `document_processing.rs` - Rholang document workflow
6. Extract `workspace.rs` - Workspace management

### Phase 2: Create Generic Virtual Language Support
7. **Create `virtual_language_support.rs`** with:
   - Generic Tree-Sitter query loader
   - Generic hover handler (query-driven)
   - Generic goto-definition handler (query-driven)
   - Generic references handler (query-driven)
   - Generic rename handler (query-driven)
   - Generic semantic tokens handler (query-driven)
   - Generic document highlight handler (query-driven)

### Phase 3: Refactor MeTTa-Specific Code
8. **Migrate MeTTa code** to use generic handlers:
   - Remove `hover_metta()` → Use `hover_virtual_document()`
   - Remove `goto_definition_metta()` → Use `goto_definition_virtual_document()`
   - Remove `rename_metta()` → Use `rename_virtual_document()`
   - etc.

### Phase 4: Create MeTTa Query Files
9. **Create Tree-Sitter queries** for MeTTa:
   - `grammars/metta/queries/definitions.scm`
   - `grammars/metta/queries/references.scm`
   - `grammars/metta/queries/hover.scm`
   - `grammars/metta/queries/symbols.scm`

### Phase 5: Integration
10. Extract `lifecycle.rs` - Initialization
11. Extract `lsp_handlers.rs` - LanguageServer trait impl
12. Create `mod.rs` - Coordinator

### Phase 6: Testing & Documentation
13. Test with MeTTa virtual documents
14. Verify all LSP features work
15. Document how to add new languages
16. Commit

## Benefits of This Approach

### 1. **Plug-and-Play Language Support**
Adding a new embedded language requires:
1. Drop in Tree-Sitter grammar (or use existing one)
2. Write query files (`.scm` files)
3. **No Rust code changes needed**

### 2. **Declarative LSP Features**
```scheme
; Example: JavaScript embedded in Rholang
; grammars/javascript/queries/definitions.scm
(function_declaration
  name: (identifier) @definition.function)

(variable_declarator
  name: (identifier) @definition.variable)
```

### 3. **Community-Driven Extensibility**
- Language support can be contributed without Rust knowledge
- Tree-Sitter queries are well-documented
- Existing Tree-Sitter query files can be reused

### 4. **Consistent Behavior**
All virtual languages get the same LSP features using the same generic code.

### 5. **Future Languages**
Examples that would "just work":
- **SQL** embedded in Rholang strings
- **JSON** configuration in contracts
- **GraphQL** queries
- **Regular expressions** (with Tree-Sitter-regex)
- **Custom DSLs** for specific contracts

## Query File Format Reference

### Definitions Query
```scheme
; Mark where symbols are defined
(function_declaration
  name: (identifier) @definition.function)

(class_declaration
  name: (identifier) @definition.class)

(variable_declarator
  name: (identifier) @definition.variable)

; Can use predicates for filtering
(identifier) @definition.constant
(#match? @definition.constant "^[A-Z_]+$")
```

### References Query
```scheme
; Mark where symbols are referenced
(identifier) @reference

; Exclude definitions from references
(identifier) @reference
(#not-match? @reference "definition")
```

### Hover Query
```scheme
; Built-in functions with documentation
(call_expression
  function: (identifier) @hover.builtin
  (#match? @hover.builtin "^(print|eval|import)$")
  (#set! hover "**print**: Outputs to console"))

; User symbols
(identifier) @hover.symbol
```

### Symbols Query (Document Outline)
```scheme
; Functions appear in outline
(function_declaration
  name: (identifier) @symbol.function
  (#set! symbol.kind "function"))

; Classes appear in outline
(class_declaration
  name: (identifier) @symbol.class
  (#set! symbol.kind "class"))
```

## Configuration Format

`languages.toml` - Language registry:
```toml
[languages.metta]
name = "MeTTa"
grammar = "tree-sitter-metta"
file_extensions = ["metta"]
detection_patterns = [
    "// @metta",
    "// @language: metta"
]

[languages.sql]
name = "SQL"
grammar = "tree-sitter-sql"
file_extensions = ["sql"]
detection_patterns = [
    "// @sql",
    "// @database"
]
```

## Migration Path for Existing MeTTa Code

Current MeTTa-specific implementations can guide query file creation:

1. **hover_metta()** → Extract symbol detection logic → Write `hover.scm`
2. **goto_definition_metta()** → Extract definition finding → Write `definitions.scm`
3. **rename_metta()** → Extract symbol finding → Write `references.scm`

The existing Rust implementations serve as **specifications** for what the queries should capture.

## Testing Strategy

### Unit Tests
- Test query loading/parsing
- Test generic handlers with mock VirtualDocuments
- Test each query file independently

### Integration Tests
- MeTTa: All LSP features work via queries
- Add a second language (e.g., JSON) to prove generalization
- Cross-language navigation (Rholang → MeTTa)

### Documentation Tests
- Tutorial: "Adding a new embedded language in 5 minutes"
- Example: Add Prolog support with just queries

## Success Criteria

- [ ] Generic `virtual_language_support.rs` module created
- [ ] All MeTTa LSP features work via Tree-Sitter queries
- [ ] No language names hard-coded in handlers (except config)
- [ ] Query loading system implemented
- [ ] At least 2 languages supported (MeTTa + one more)
- [ ] Documentation: "How to add a new language"
- [ ] All 204+ tests pass
- [ ] Backend modularized (9 focused files)

## Future Enhancements

### LSP 3.17 Features via Queries
- **Inlay Hints**: Show type information inline
- **Call Hierarchy**: Navigate call relationships
- **Type Hierarchy**: Navigate type relationships
- **Folding Ranges**: Code folding

### Language Server Protocol Extensions
- **Embedded Language Diagnostics**: Run language-specific validators
- **Cross-Language Navigation**: Jump from Rholang to embedded code
- **Multi-Language Renaming**: Rename symbols across languages

### Query Composition
- **Inheritance**: Languages can extend base queries
- **Overrides**: Customize behavior per language
- **Mixins**: Share common patterns (e.g., all C-like languages)

## Comparison: Before vs. After

### Before (Monolithic + Hard-Coded)
```
src/lsp/backend.rs (3,495 lines)
├── MeTTa-specific: hover_metta(), goto_definition_metta(), rename_metta()
├── Hard-coded logic: "if language == 'metta' then ..."
└── Adding new language: Write more Rust code
```

### After (Modular + Declarative)
```
src/lsp/backend/ (9 modules, ~388 lines each)
├── virtual_language_support.rs (generic handlers, query-driven)
├── grammars/metta/queries/*.scm (declarative LSP features)
└── Adding new language: Write *.scm query files
```

## Conclusion

This revised plan transforms the backend refactoring from **extracting MeTTa-specific code** into **building a generalized virtual language support framework**. The key insight is that Tree-Sitter queries can drive LSP features declaratively, making the system extensible without Rust code changes.

The MeTTa-specific code becomes the **first reference implementation** of this pattern, and future languages benefit from the same infrastructure.

## Addendum: Unified IR Integration

### Critical Addition: Tree-Sitter → Unified IR Translation

Virtual languages MUST integrate with the Unified IR system for:
- Cross-language symbol resolution  
- Semantic analysis
- Type checking
- Refactoring operations
- Language interoperability

See **VIRTUAL_LANGUAGE_UNIFIED_IR_INTEGRATION.md** for complete architecture.

### Extension Trait Addition

```rust
#[async_trait]
pub trait VirtualLanguageExtension: Send + Sync {
    // ... existing methods ...
    
    /// Translate virtual document to Unified IR
    async fn to_unified_ir(&self, doc: &VirtualDocument) -> Option<Arc<UnifiedIR>>;
    
    /// Declare IR translation capabilities
    fn ir_capabilities(&self) -> IRCapabilities;
}
```

### VirtualDocument Enhancement

```rust
pub struct VirtualDocument {
    // ... existing fields ...
    
    /// Optional: Language-specific IR (e.g., MettaNode)
    pub language_ir: Option<Arc<dyn Any + Send + Sync>>,
    
    /// Optional: Unified IR translation
    pub unified_ir: Option<Arc<UnifiedIR>>,
}
```

### Translation Paths

**Simple Languages** (SQL, JSON):
```
Tree-Sitter CST → UnifiedIR  (direct)
```

**Complex Languages** (MeTTa):
```
Tree-Sitter CST → MettaNode → UnifiedIR  (two-phase)
```

### Cross-Language Features Enabled

- ✅ Goto definition across Rholang ↔ MeTTa boundaries
- ✅ Find references in virtual documents
- ✅ Rename symbols across languages
- ✅ Unified symbol tables
- ✅ Type inference via UnifiedType
- ✅ Semantic diagnostics

This completes the virtual language architecture with full semantic integration.
