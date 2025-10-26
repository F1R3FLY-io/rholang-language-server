# Virtual Language → Unified IR Integration
## Tree-Sitter to Semantic IR Translation for Virtual Languages

## Overview

Virtual languages embedded in Rholang documents must integrate with the **Unified IR** system to enable:
- Cross-language semantic analysis
- Shared symbol resolution
- Unified type system
- Common refactoring operations
- Language interoperability

## Architecture

```
┌─────────────────────────────────────────────────────────────────────────┐
│                        Virtual Document                                  │
│  ┌────────────────────────────────────────────────────────────────────┐ │
│  │ Source Text (e.g., MeTTa code in Rholang string)                   │ │
│  │ "(= (fib 0) 0)\n(= (fib 1) 1)"                                     │ │
│  └────────────────────────────────────────────────────────────────────┘ │
│                               ↓                                          │
│  ┌────────────────────────────────────────────────────────────────────┐ │
│  │ Tree-Sitter Parse (Language-Specific CST)                          │ │
│  │ parser.parse(source)  → tree_sitter::Tree                          │ │
│  └────────────────────────────────────────────────────────────────────┘ │
│                               ↓                                          │
│  ┌────────────────────────────────────────────────────────────────────┐ │
│  │ Language-Specific IR (Optional, for complex languages)             │ │
│  │ MettaNode, SqlNode, JavaScriptNode, etc.                           │ │
│  └────────────────────────────────────────────────────────────────────┘ │
│                               ↓                                          │
│  ┌────────────────────────────────────────────────────────────────────┐ │
│  │ Unified IR (Common Semantic Representation)                        │ │
│  │ UnifiedIR::Literal, ::Variable, ::Invocation, etc.                 │ │
│  └────────────────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────────────┘
                               ↓
┌─────────────────────────────────────────────────────────────────────────┐
│                    Shared LSP Features                                   │
│                                                                           │
│  • Symbol tables                                                         │
│  • Cross-language references                                             │
│  • Unified type system                                                   │
│  • Refactoring operations                                                │
│  • Semantic validation                                                   │
└─────────────────────────────────────────────────────────────────────────┘
```

## Translation Strategy: Two Paths

### Path 1: Direct Tree-Sitter → Unified IR (Simple Languages)

For simple languages where Tree-Sitter is sufficient:

```rust
// Direct translation using Tree-Sitter queries
impl VirtualLanguageExtension for SqlExtension {
    async fn to_unified_ir(&self, doc: &VirtualDocument) -> Option<Arc<UnifiedIR>> {
        let tree = &doc.tree;
        let root = tree.root_node();

        // Translate directly from Tree-Sitter CST
        Some(self.translate_ts_to_unified(root, &doc.content))
    }

    fn translate_ts_to_unified(&self, node: TSNode, source: &str) -> Arc<UnifiedIR> {
        match node.kind() {
            "select_statement" => self.translate_select(node, source),
            "identifier" => UnifiedIR::Variable {
                name: source[node.byte_range()].to_string(),
                // ...
            },
            // ... more translations
        }
    }
}
```

### Path 2: Tree-Sitter → Language IR → Unified IR (Complex Languages)

For complex languages needing intermediate representation:

```rust
// Two-phase translation for MeTTa
impl VirtualLanguageExtension for MettaExtension {
    async fn to_unified_ir(&self, doc: &VirtualDocument) -> Option<Arc<UnifiedIR>> {
        // Phase 1: Tree-Sitter → MettaNode (language-specific IR)
        let metta_ir = MettaNode::from_tree_sitter(&doc.tree, &doc.content)?;

        // Phase 2: MettaNode → UnifiedIR (semantic translation)
        Some(UnifiedIR::from_metta(&metta_ir))
    }
}
```

## Extension Trait: Translation Methods

```rust
// src/lsp/backend/virtual_language_extension.rs

#[async_trait]
pub trait VirtualLanguageExtension: Send + Sync {
    // ... existing methods ...

    /// Translate virtual document to Unified IR
    ///
    /// This enables cross-language semantic analysis and symbol resolution.
    /// Return None if translation is not supported/needed for this language.
    async fn to_unified_ir(
        &self,
        doc: &VirtualDocument,
    ) -> Option<Arc<UnifiedIR>> {
        None  // Default: no translation
    }

    /// Declare IR translation capabilities
    fn ir_capabilities(&self) -> IRCapabilities {
        IRCapabilities::default()
    }
}

/// IR translation capabilities
#[derive(Debug, Clone, Default)]
pub struct IRCapabilities {
    /// Can translate to UnifiedIR
    pub supports_unified_ir: bool,

    /// Has language-specific IR (e.g., MettaNode)
    pub has_language_ir: bool,

    /// Can translate back from UnifiedIR (for code generation)
    pub supports_from_unified_ir: bool,
}
```

## Virtual Document Enhancement

```rust
// src/language_regions/virtual_document.rs

pub struct VirtualDocument {
    // ... existing fields ...

    /// Cached Tree-Sitter tree
    pub tree: Tree,

    /// Optional: Language-specific IR (e.g., MettaNode for MeTTa)
    pub language_ir: Option<Arc<dyn Any + Send + Sync>>,

    /// Optional: Unified IR translation
    pub unified_ir: Option<Arc<UnifiedIR>>,
}

impl VirtualDocument {
    /// Get or compute Unified IR
    pub async fn get_unified_ir(
        &self,
        extension: Option<&Arc<dyn VirtualLanguageExtension>>,
    ) -> Option<Arc<UnifiedIR>> {
        // Return cached if available
        if let Some(ir) = &self.unified_ir {
            return Some(ir.clone());
        }

        // Compute via extension
        if let Some(ext) = extension {
            return ext.to_unified_ir(self).await;
        }

        None
    }
}
```

## Example: MeTTa Integration

### MeTTa Language-Specific IR

```rust
// src/ir/metta_node.rs (already exists)

#[derive(Debug, Clone)]
pub enum MettaNode {
    Symbol { name: String, ... },
    Atom { elements: Vec<Arc<MettaNode>>, ... },
    Expression { operator: Arc<MettaNode>, args: Vec<Arc<MettaNode>>, ... },
    // ... more variants
}

impl MettaNode {
    /// Parse from Tree-Sitter
    pub fn from_tree_sitter(tree: &Tree, source: &str) -> Option<Arc<MettaNode>> {
        // Convert Tree-Sitter CST to MettaNode IR
    }
}
```

### MettaNode → UnifiedIR Translation

```rust
// In UnifiedIR implementation

impl UnifiedIR {
    /// Convert MettaNode to UnifiedIR
    pub fn from_metta(node: &Arc<MettaNode>) -> Arc<UnifiedIR> {
        let base = node.base().clone();

        match &**node {
            // Literals
            MettaNode::Symbol { name, metadata, .. } => Arc::new(UnifiedIR::Variable {
                base,
                name: name.clone(),
                metadata: metadata.clone(),
            }),

            // Function definition: (= (fib 0) 0)
            MettaNode::Expression { operator, args, metadata, .. }
                if is_equals_operator(operator) && args.len() == 2 =>
            {
                // Pattern: (= pattern body)
                Arc::new(UnifiedIR::Definition {
                    base,
                    name: extract_name(&args[0]),
                    parameters: extract_parameters(&args[0]),
                    body: UnifiedIR::from_metta(&args[1]),
                    metadata: metadata.clone(),
                })
            }

            // Function call: (fib 5)
            MettaNode::Expression { operator, args, metadata, .. } => {
                Arc::new(UnifiedIR::Invocation {
                    base,
                    target: UnifiedIR::from_metta(operator),
                    args: args.iter().map(UnifiedIR::from_metta).collect(),
                    metadata: metadata.clone(),
                })
            }

            // Atom/List: (a b c)
            MettaNode::Atom { elements, metadata, .. } => {
                Arc::new(UnifiedIR::Collection {
                    base,
                    kind: CollectionKind::List,
                    elements: elements.iter().map(UnifiedIR::from_metta).collect(),
                    metadata: metadata.clone(),
                })
            }

            // ... more translations
        }
    }
}
```

### MeTTa Extension with IR Translation

```rust
// src/lsp/extensions/metta_extension.rs

impl VirtualLanguageExtension for MettaExtension {
    fn ir_capabilities(&self) -> IRCapabilities {
        IRCapabilities {
            supports_unified_ir: true,
            has_language_ir: true,  // Uses MettaNode
            supports_from_unified_ir: false,  // One-way translation for now
        }
    }

    async fn to_unified_ir(&self, doc: &VirtualDocument) -> Option<Arc<UnifiedIR>> {
        // Phase 1: Get/create MettaNode IR
        let metta_ir = if let Some(cached) = &doc.language_ir {
            cached.downcast_ref::<MettaNode>()?.clone()
        } else {
            MettaNode::from_tree_sitter(&doc.tree, &doc.content)?
        };

        // Phase 2: Translate to UnifiedIR
        Some(UnifiedIR::from_metta(&metta_ir))
    }
}
```

## Example: SQL Integration (Direct Translation)

### SQL Extension without Intermediate IR

```rust
// src/lsp/extensions/sql_extension.rs

pub struct SqlExtension;

impl VirtualLanguageExtension for SqlExtension {
    fn language(&self) -> &str { "sql" }

    fn ir_capabilities(&self) -> IRCapabilities {
        IRCapabilities {
            supports_unified_ir: true,
            has_language_ir: false,  // Direct translation
            supports_from_unified_ir: false,
        }
    }

    async fn to_unified_ir(&self, doc: &VirtualDocument) -> Option<Arc<UnifiedIR>> {
        let tree = &doc.tree;
        let root = tree.root_node();

        Some(self.translate_node(root, &doc.content, &doc.uri))
    }

    fn translate_node(&self, node: TSNode, source: &str, uri: &Url) -> Arc<UnifiedIR> {
        let base = node_to_base(node, uri);

        match node.kind() {
            "select_statement" => {
                // SELECT x, y FROM table WHERE condition
                let mut cursor = node.walk();
                let children: Vec<_> = node.children(&mut cursor).collect();

                Arc::new(UnifiedIR::Query {
                    base,
                    // Extract SELECT columns, FROM tables, WHERE conditions
                    // ... translation logic ...
                })
            }

            "identifier" => Arc::new(UnifiedIR::Variable {
                base,
                name: source[node.byte_range()].to_string(),
                metadata: None,
            }),

            "number" => Arc::new(UnifiedIR::Literal {
                base,
                value: Literal::Integer(source[node.byte_range()].parse().unwrap()),
                metadata: None,
            }),

            // ... more node types

            _ => {
                // Default: wrap in Unknown node
                Arc::new(UnifiedIR::Unknown {
                    base,
                    kind: node.kind().to_string(),
                    metadata: None,
                })
            }
        }
    }
}
```

## Cross-Language Symbol Resolution

With Unified IR, symbols from virtual languages integrate with Rholang symbols:

```rust
// src/lsp/backend/symbol_operations.rs

impl RholangBackend {
    /// Get symbol at position (works across all languages)
    async fn get_symbol_at_position_unified(
        &self,
        uri: &Url,
        position: Position,
    ) -> Option<Arc<Symbol>> {
        // Check if position is in virtual document
        if let Some(virtual_doc) = self.virtual_docs.read().await
            .get_document_at_position(uri, position)
        {
            // Get extension for this language
            let extension = self.extension_registry.get(&virtual_doc.language);

            // Get Unified IR
            let unified_ir = virtual_doc.get_unified_ir(extension).await?;

            // Find symbol in Unified IR
            return self.find_symbol_in_unified_ir(&unified_ir, position);
        }

        // Fallback: Rholang symbol
        self.get_symbol_at_position_rholang(uri, position).await
    }

    fn find_symbol_in_unified_ir(
        &self,
        ir: &Arc<UnifiedIR>,
        position: Position,
    ) -> Option<Arc<Symbol>> {
        // Use UnifiedIR's SemanticNode trait
        let node = find_node_at_position(ir, &position_to_ir_position(position))?;

        match &**node {
            UnifiedIR::Variable { name, .. } => Some(Arc::new(Symbol {
                name: name.clone(),
                symbol_type: SymbolType::Variable,
                // ... extract from metadata
            })),

            UnifiedIR::Definition { name, .. } => Some(Arc::new(Symbol {
                name: name.clone(),
                symbol_type: SymbolType::Function,
                // ...
            })),

            // ... more symbol types

            _ => None,
        }
    }
}
```

## Cross-Language Goto Definition

```rust
impl RholangBackend {
    async fn goto_definition_cross_language(
        &self,
        uri: &Url,
        position: Position,
    ) -> LspResult<Option<GotoDefinitionResponse>> {
        // Step 1: Find symbol at cursor
        let symbol = self.get_symbol_at_position_unified(uri, position).await?;

        // Step 2: Search for definition across all documents (including virtual)

        // Search in Rholang documents
        if let Some(loc) = self.find_definition_in_rholang(&symbol).await {
            return Ok(Some(GotoDefinitionResponse::Scalar(loc)));
        }

        // Search in virtual documents (all languages)
        for virtual_doc in self.virtual_docs.read().await.all_documents() {
            // Get Unified IR for virtual document
            let extension = self.extension_registry.get(&virtual_doc.language);
            let unified_ir = virtual_doc.get_unified_ir(extension).await?;

            // Search for definition in Unified IR
            if let Some(def_node) = self.find_definition_in_unified_ir(&unified_ir, &symbol) {
                return Ok(Some(GotoDefinitionResponse::Scalar(Location {
                    uri: virtual_doc.uri.clone(),
                    range: ir_node_to_range(def_node),
                })));
            }
        }

        Ok(None)
    }
}
```

## Unified Type System

```rust
// Extend UnifiedIR with type information

#[derive(Debug, Clone, PartialEq)]
pub enum UnifiedType {
    Bool,
    Integer,
    Float,
    String,
    Function {
        params: Vec<UnifiedType>,
        return_type: Box<UnifiedType>,
    },
    Collection {
        kind: CollectionKind,
        element_type: Box<UnifiedType>,
    },
    Unknown,
}

impl UnifiedIR {
    /// Infer type from Unified IR node
    pub fn infer_type(&self) -> UnifiedType {
        match self {
            UnifiedIR::Literal { value, .. } => match value {
                Literal::Bool(_) => UnifiedType::Bool,
                Literal::Integer(_) => UnifiedType::Integer,
                Literal::String(_) => UnifiedType::String,
                // ...
            },

            UnifiedIR::Collection { kind, elements, .. } => {
                let element_type = if elements.is_empty() {
                    UnifiedType::Unknown
                } else {
                    elements[0].infer_type()
                };

                UnifiedType::Collection {
                    kind: *kind,
                    element_type: Box::new(element_type),
                }
            },

            // ... more type inference

            _ => UnifiedType::Unknown,
        }
    }
}
```

## Language Interoperability: MeTTa ↔ Rholang

### Example: Calling Rholang from MeTTa

```rho
new mettaCompiler in {
  // @metta
  @"
    ; MeTTa code can reference Rholang
    (defun call-rholang (data)
      (rho:send stdout data))  ; Call Rholang function
  "@(code) |

  mettaCompiler!(code)
}
```

Unified IR enables resolving `rho:send` reference:

```rust
// In MeTTa → UnifiedIR translation

MettaNode::Expression { operator, args, .. }
    if operator_text == "rho:send" =>
{
    // This is a cross-language call
    Arc::new(UnifiedIR::Invocation {
        target: Arc::new(UnifiedIR::ExternalReference {
            language: "rholang",
            symbol: "send",
            // ...
        }),
        args: translate_args(args),
        // ...
    })
}
```

## Document Processing Integration

```rust
// src/lsp/backend/document_processing.rs

impl RholangBackend {
    async fn process_virtual_document(
        &self,
        virtual_doc: &Arc<VirtualDocument>,
    ) -> Result<(), String> {
        // Step 1: Parse with Tree-Sitter (already done in VirtualDocument)

        // Step 2: Get language extension
        let extension = self.extension_registry.get(&virtual_doc.language);

        // Step 3: Translate to Unified IR (if supported)
        if let Some(ext) = extension {
            if ext.ir_capabilities().supports_unified_ir {
                let unified_ir = ext.to_unified_ir(virtual_doc).await?;

                // Step 4: Build symbol table from Unified IR
                let symbol_table = self.build_symbol_table_from_unified_ir(&unified_ir);

                // Step 5: Register symbols globally
                self.register_virtual_symbols(&virtual_doc.uri, symbol_table).await;
            }
        }

        Ok(())
    }
}
```

## Testing Strategy

### Unit Tests: Translation Correctness

```rust
#[test]
fn test_metta_to_unified_ir() {
    let metta_code = "(= (fib 0) 0)";
    let tree = parse_metta(metta_code);
    let metta_ir = MettaNode::from_tree_sitter(&tree, metta_code).unwrap();
    let unified_ir = UnifiedIR::from_metta(&metta_ir);

    // Verify translation
    assert!(matches!(*unified_ir, UnifiedIR::Definition { .. }));

    if let UnifiedIR::Definition { name, parameters, body, .. } = &*unified_ir {
        assert_eq!(name, "fib");
        assert_eq!(parameters.len(), 1);
        // ... more assertions
    }
}
```

### Integration Tests: Cross-Language Features

```rust
#[tokio::test]
async fn test_cross_language_goto_definition() {
    let backend = create_test_backend();

    // Document with MeTTa code
    let rho_code = r#"
        // @metta
        @"(= (foo x) (* x 2))"@(code) |

        // @metta
        @"(foo 5)"@(call)  // Click here
    "#;

    let uri = open_document(&backend, rho_code).await;

    // Goto definition from second metta region
    let position = Position { line: 5, character: 4 };  // On "foo"
    let result = backend.goto_definition(...).await.unwrap();

    // Should jump to first metta region
    assert_eq!(result.uri, uri);
    assert_eq!(result.range.start.line, 2);  // First @metta
}
```

## Summary

### Integration Points

1. **VirtualLanguageExtension::to_unified_ir()** - Translation method
2. **VirtualDocument::unified_ir** - Cached translation
3. **UnifiedIR::from_X()** - Language-specific translators
4. **Cross-language symbol resolution** - Via Unified IR
5. **Type inference** - Unified type system

### Translation Paths

**Simple Languages** (SQL, JSON, YAML):
```
Tree-Sitter CST → UnifiedIR
```

**Complex Languages** (MeTTa, JavaScript):
```
Tree-Sitter CST → LanguageIR → UnifiedIR
```

### Benefits

- ✅ Cross-language goto-definition
- ✅ Unified symbol tables
- ✅ Type checking across languages
- ✅ Language interoperability
- ✅ Shared refactoring operations
- ✅ Common semantic analysis

This architecture enables virtual languages to fully participate in the LSP ecosystem while maintaining language-specific optimizations through extensions.
