# Virtual Language Support Architecture

## System Overview

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         Rholang LSP Backend                              │
│                                                                           │
│  ┌────────────────────────────────────────────────────────────────────┐ │
│  │                    LanguageServer Trait Impl                        │ │
│  │                     (lsp_handlers.rs)                               │ │
│  │                                                                      │ │
│  │  ┌───────────────────┐         ┌────────────────────────────────┐  │ │
│  │  │ hover()           │────────▶│ Is virtual document?          │  │ │
│  │  │ goto_definition() │         │                                │  │ │
│  │  │ rename()          │         │ Yes: hover_virtual_document()  │  │ │
│  │  │ references()      │         │ No:  hover_rholang()           │  │ │
│  │  └───────────────────┘         └────────────────────────────────┘  │ │
│  └────────────────────────────────────────────────────────────────────┘ │
│                                                                           │
│  ┌────────────────────────────────────────────────────────────────────┐ │
│  │          Virtual Language Support (virtual_language_support.rs)     │ │
│  │                                                                      │ │
│  │  ┌──────────────────────────────────────────────────────────────┐  │ │
│  │  │ Generic LSP Handlers (Query-Driven)                          │  │ │
│  │  │                                                               │  │ │
│  │  │  • hover_virtual_document(virtual_doc, position)             │  │ │
│  │  │  • goto_definition_virtual_document(...)                     │  │ │
│  │  │  • rename_virtual_document(...)                              │  │ │
│  │  │  • references_virtual_document(...)                          │  │ │
│  │  │  • semantic_tokens_virtual_document(...)                     │  │ │
│  │  │                                                               │  │ │
│  │  │  All driven by Tree-Sitter queries ▼                         │  │ │
│  │  └──────────────────────────────────────────────────────────────┘  │ │
│  │                                                                      │ │
│  │  ┌──────────────────────────────────────────────────────────────┐  │ │
│  │  │ Query Engine                                                  │  │ │
│  │  │                                                               │  │ │
│  │  │  • load_language_query(language, "hover.scm")                │  │ │
│  │  │  • execute_query(query, tree, source)                        │  │ │
│  │  │  • find_captures_at_position(...)                            │  │ │
│  │  │  • query_cache: HashMap<(lang, file), Query>                 │  │ │
│  │  └──────────────────────────────────────────────────────────────┘  │ │
│  └────────────────────────────────────────────────────────────────────┘ │
│                                                                           │
│  ┌────────────────────────────────────────────────────────────────────┐ │
│  │         Virtual Document Registry (language_regions/)              │ │
│  │                                                                      │ │
│  │  ┌─────────────────────┐   ┌────────────────────────────────────┐  │ │
│  │  │ VirtualDocument     │   │ Detection Strategies:              │  │ │
│  │  │ ─────────────────   │   │                                    │  │ │
│  │  │ • uri               │   │  • DirectiveParser                 │  │ │
│  │  │ • language: "metta" │◀──│  • SemanticDetector                │  │ │
│  │  │ • content           │   │  • ChannelFlowAnalyzer             │  │ │
│  │  │ • tree (cached)     │   │                                    │  │ │
│  │  │ • start_pos, end_pos│   │  Result: LanguageRegion            │  │ │
│  │  └─────────────────────┘   └────────────────────────────────────┘  │ │
│  └────────────────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────────────┐
│                    Language Grammar Repository                           │
│                                                                           │
│  grammars/                                                                │
│  ├── metta/                                                               │
│  │   ├── grammar.js              ← Tree-Sitter grammar                   │
│  │   └── queries/                                                         │
│  │       ├── highlights.scm      ← Syntax highlighting                   │
│  │       ├── definitions.scm     ← Where symbols are defined             │
│  │       ├── references.scm      ← Where symbols are used                │
│  │       ├── hover.scm           ← Hover information                     │
│  │       └── symbols.scm         ← Document outline                      │
│  │                                                                         │
│  ├── sql/                        ← Just drop in to add SQL support       │
│  │   ├── grammar.js                                                       │
│  │   └── queries/                                                         │
│  │       ├── definitions.scm                                              │
│  │       ├── references.scm                                               │
│  │       └── ...                                                          │
│  │                                                                         │
│  └── javascript/                 ← Or JavaScript, or any TS grammar      │
│      ├── grammar.js                                                       │
│      └── queries/                                                         │
│          └── ...                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

## Data Flow: Hover Request Example

```
1. User hovers over MeTTa code in Rholang string
   ↓
2. LSP Client sends textDocument/hover(uri, position)
   ↓
3. lsp_handlers.rs::hover() receives request
   ↓
4. Check: Is position in virtual document?
   VirtualDocumentRegistry::get_document_at_position(uri, position)
   ↓
5. Yes → Delegate to virtual_language_support.rs
   hover_virtual_document(virtual_doc, virtual_position)
   ↓
6. Query Engine loads hover.scm for "metta"
   load_language_query("metta", "hover.scm")
   ↓
7. Execute query against VirtualDocument's cached Tree-Sitter tree
   execute_query(query, virtual_doc.tree, virtual_doc.content)
   ↓
8. Find captures at position
   find_captures_at_position(matches, virtual_position)
   ↓
9. Extract hover content from capture metadata
   generate_hover_content(capture, metadata)
   ↓
10. Return Hover response to LSP client
    ↓
11. VSCode displays hover tooltip
```

## Tree-Sitter Query Example: MeTTa Hover

```scheme
; grammars/metta/queries/hover.scm

; Built-in MeTTa functions with documentation
(expression
  (symbol) @hover.builtin
  (#match? @hover.builtin "^(import|include|bind|let|case)$")
  (#set! hover.kind "builtin")
  (#set! hover.doc "MeTTa built-in function"))

; User-defined functions (from definitions.scm match)
(expression
  .
  (symbol) @hover.function
  (#set! hover.kind "function"))

; Variables
(symbol) @hover.variable
(#set! hover.kind "variable")

; Type annotations
(type_annotation
  (symbol) @hover.type
  (#set! hover.kind "type"))
```

## Adding a New Language: Step-by-Step

### Example: Adding SQL Support

**Step 1**: Drop in Tree-Sitter grammar
```bash
cd grammars/
git clone https://github.com/tree-sitter/tree-sitter-sql.git sql
cd sql
tree-sitter generate
```

**Step 2**: Create query files
```bash
mkdir -p grammars/sql/queries
```

**Step 3**: Write `definitions.scm`
```scheme
; grammars/sql/queries/definitions.scm

; Table definitions
(create_table_statement
  table: (identifier) @definition.table)

; Column definitions
(column_definition
  name: (identifier) @definition.column)

; Function definitions
(create_function_statement
  name: (identifier) @definition.function)
```

**Step 4**: Write `references.scm`
```scheme
; grammars/sql/queries/references.scm

; Table references
(table_reference
  (identifier) @reference.table)

; Column references
(column_reference
  (identifier) @reference.column)
```

**Step 5**: Write `hover.scm`
```scheme
; grammars/sql/queries/hover.scm

; SQL keywords
(keyword) @hover.keyword
(#set! hover.kind "keyword")

; Tables
(identifier) @hover.table
(#set! hover.kind "table")
```

**Step 6**: Update configuration
```toml
# languages.toml

[languages.sql]
name = "SQL"
grammar = "tree-sitter-sql"
file_extensions = ["sql"]
detection_patterns = [
    "// @sql",
    "// @database"
]
```

**Step 7**: Use in Rholang
```rho
// @sql
new db in {
  contract query(@sql, return) = {
    // SQL code with full LSP support!
    @"
    SELECT users.name, orders.total
    FROM users
    JOIN orders ON users.id = orders.user_id
    WHERE orders.total > 100
    "@(sql) |

    // Now hover, goto-definition, rename all work!
    return!(result)
  }
}
```

**That's it!** No Rust code changes needed.

## Language Detection Methods

```
┌──────────────────────────────────────────────────────────────────────┐
│                       Rholang Source File                             │
│                                                                        │
│  new compiler in {                                                    │
│    // @metta                  ← DirectiveParser detects this         │
│    @"                                                                 │
│      (= (fib 0) 0)           ← This is MeTTa code                    │
│      (= (fib 1) 1)                                                    │
│    "@(metta_code) |                                                   │
│                                                                        │
│    compiler!(metta_code)     ← SemanticDetector sees "compiler"      │
│  }                                  pattern (e.g., "metta_compiler")  │
│                                                                        │
│  @"metta_compiler"!(          ← ChannelFlowAnalyzer tracks           │
│    @"(define foo 42)"@             channel flow to compiler          │
│  )                                                                    │
└──────────────────────────────────────────────────────────────────────┘
                                 ↓
                     Detection produces LanguageRegion
                                 ↓
              ┌──────────────────────────────────────────┐
              │ LanguageRegion {                         │
              │   language: "metta",                     │
              │   start_byte: 125,                       │
              │   end_byte: 167,                         │
              │   content: "(= (fib 0) 0)\n(= (fib 1) 1)",│
              │   source: CommentDirective               │
              │ }                                         │
              └──────────────────────────────────────────┘
                                 ↓
                    VirtualDocumentRegistry::create()
                                 ↓
              ┌──────────────────────────────────────────┐
              │ VirtualDocument {                        │
              │   uri: "file:///.../rho/metta/0",        │
              │   language: "metta",                     │
              │   content: "(= (fib 0) 0)...",           │
              │   tree: <cached Tree-Sitter tree>,       │
              │   parent_uri: "file://.../app.rho",      │
              │   start_pos: Position { line: 3, ... },  │
              │   end_pos: Position { line: 5, ... }     │
              │ }                                         │
              └──────────────────────────────────────────┘
```

## Query Execution Flow

```
┌────────────────────────────────────────────────────────────────────┐
│ hover_virtual_document(virtual_doc, position)                      │
└────────────────────────────────────────────────────────────────────┘
          │
          ▼
┌────────────────────────────────────────────────────────────────────┐
│ load_language_query("metta", "hover.scm")                          │
│                                                                     │
│  1. Check cache: query_cache[("metta", "hover.scm")]              │
│  2. If miss: Load from disk: grammars/metta/queries/hover.scm     │
│  3. Parse query: Query::new(metta_language, hover_scm_text)       │
│  4. Store in cache                                                 │
│  5. Return Query object                                            │
└────────────────────────────────────────────────────────────────────┘
          │
          ▼
┌────────────────────────────────────────────────────────────────────┐
│ execute_query(query, tree, source)                                 │
│                                                                     │
│  let mut cursor = QueryCursor::new();                             │
│  let matches = cursor.matches(                                     │
│      &query,                                                       │
│      tree.root_node(),                                             │
│      source.as_bytes()                                             │
│  );                                                                │
│                                                                     │
│  Returns: Iterator<QueryMatch>                                     │
└────────────────────────────────────────────────────────────────────┘
          │
          ▼
┌────────────────────────────────────────────────────────────────────┐
│ find_captures_at_position(matches, byte_offset)                    │
│                                                                     │
│  for match in matches {                                            │
│      for capture in match.captures {                               │
│          if capture.node.byte_range().contains(byte_offset) {     │
│              return Some(capture);                                 │
│          }                                                         │
│      }                                                             │
│  }                                                                 │
└────────────────────────────────────────────────────────────────────┘
          │
          ▼
┌────────────────────────────────────────────────────────────────────┐
│ generate_hover_content(capture, metadata)                          │
│                                                                     │
│  let capture_name = query.capture_names()[capture.index];         │
│  // e.g., "hover.builtin"                                         │
│                                                                     │
│  let node_text = &source[capture.node.byte_range()];              │
│  // e.g., "import"                                                │
│                                                                     │
│  let kind = metadata.get("hover.kind");                            │
│  // e.g., "builtin"                                               │
│                                                                     │
│  let doc = metadata.get("hover.doc");                              │
│  // e.g., "MeTTa built-in function"                               │
│                                                                     │
│  format_hover_markdown(node_text, kind, doc)                       │
│  // Returns: "**import** (builtin)\n\nMeTTa built-in function"   │
└────────────────────────────────────────────────────────────────────┘
          │
          ▼
┌────────────────────────────────────────────────────────────────────┐
│ Return Hover {                                                      │
│   contents: MarkupContent {                                         │
│     kind: Markdown,                                                 │
│     value: "**import** (builtin)\n\nMeTTa built-in function"      │
│   },                                                                │
│   range: Some(Range { ... })                                       │
│ }                                                                   │
└────────────────────────────────────────────────────────────────────┘
```

## Performance Optimizations

### 1. Query Caching
```rust
query_cache: HashMap<(String, String), Query>
//                    ^^^^    ^^^^
//                    lang    file
```
Queries are parsed once and reused.

### 2. Tree Caching
```rust
VirtualDocument {
    tree: Tree,  // Cached Tree-Sitter parse tree
}
```
Virtual documents cache their parsed trees.

### 3. Lazy Loading
```rust
// Only load queries when needed
let query = self.load_language_query(language, "hover.scm")?;
```

### 4. Incremental Updates
```rust
// When virtual document content changes:
virtual_doc.tree.edit(&edit);
parser.parse(new_content, Some(&old_tree));
```

## Extension Points

### Custom Query Metadata
```scheme
(symbol) @hover.custom
(#set! hover.wiki_url "https://metta.org/symbol")
(#set! hover.signature "(symbol arg1 arg2)")
(#set! hover.return_type "Bool")
```

### Multi-Language Queries
```scheme
; Detect when MeTTa calls Rholang
(interop_call
  target: (identifier) @reference.rholang
  (#match? @reference.rholang "^rho:"))
```

### Query Composition
```scheme
; Base queries can be inherited/extended
(inherits "base_language/queries/definitions.scm")

; Override specific patterns
(function_declaration
  name: (identifier) @definition.function
  (#override! @definition.function "custom_handler"))
```

## Future: Language Server Protocol Extensions

### Embedded Diagnostics
```rust
async fn validate_virtual_document(
    &self,
    virtual_doc: &VirtualDocument,
) -> Vec<Diagnostic> {
    // Run language-specific validator
    match virtual_doc.language.as_str() {
        "metta" => self.metta_validator.validate(virtual_doc),
        "sql" => self.sql_validator.validate(virtual_doc),
        _ => vec![],
    }
}
```

### Cross-Language Navigation
```rust
// Jump from Rholang to MeTTa definition
@metta_compiler!(@"(define foo 42)"@) |  // Click "foo"
                                          //   ↓
for (@result <- metta_compiler) {         // Jumps here
  match result { ... }
}
```

### Semantic Analysis Integration
```rust
// MeTTa type checker integration
let metta_types = mettatron::type_check(virtual_doc.content)?;
// Use types for hover, completion, etc.
```

## Conclusion

This architecture transforms embedded language support from a **hard-coded feature** into a **declarative, extensible system**. The key innovations:

1. **Tree-Sitter Queries** drive LSP features
2. **VirtualDocumentRegistry** manages embedded documents
3. **Generic handlers** work with any language
4. **Zero Rust code** needed to add new languages

The result: A language server that grows with the community's needs, not just the maintainer's time.
