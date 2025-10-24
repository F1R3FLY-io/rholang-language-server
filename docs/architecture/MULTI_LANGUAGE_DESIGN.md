# Multi-Language Document Support Design

**Project**: Multi-Language LSP with Embedded Language Regions
**Status**: Design Document
**Created**: 2025-10-24
**Context**: Rholang documents with embedded MeTTa code sections

## Overview

This document describes the architecture for supporting Rholang documents that contain embedded code from other languages (initially MeTTa). The system must detect language regions, parse them appropriately, and provide full LSP features across language boundaries.

## Use Case

A Rholang file may contain MeTTa code sections that are:

1. **Explicitly marked** via comment directives above string literals:
   ```rholang
   new codeFile in {
     // @language metta
     // or: // @metta
     codeFile!("
       // Robot planning knowledge base
       (connected room_a room_b)
       (connected room_b room_c)
       (object_at ball1 room_c)

       (= (find_path $from $to)
          (match & self (connected $from $to) $to))
     ") |

     for (@code <- codeFile) {
       for (@state <- mettaCompile!?(code)) {
         // Use the MeTTa state
       }
     }
   }
   ```

   The comment directive `// @language metta` or `// @metta` above the string literal indicates that the string contains MeTTa code, enabling syntax highlighting and LSP features within the string.

2. **Semantically identified** by being sent to the MeTTaTron compiler service:
   ```rholang
   new mettaCode in {
     mettaCode!("(: add (-> Number Number Number))") |
     @"rho:metta:compile"!(mettaCode)  // Send to MeTTa compiler
   }
   ```

3. **Indirectly identified** via channel analysis:
   ```rholang
   contract compile(@code, return) = {
     @"rho:metta:compile"!(code, return)  // Forwarding to MeTTa
   } |
   new ret in {
     compile!("(= (fact 0) 1)", *ret)  // This string is MeTTa code
   }
   ```

## Current Architecture Support

### ✅ What We Already Have

1. **SemanticNode Trait** (`src/ir/semantic_node.rs`)
   - Language-agnostic interface implemented by both RholangNode and MettaNode
   - `semantic_category()` provides universal categorization
   - `children_count()` and `child_at()` enable uniform traversal
   - `as_any()` allows downcasting when language-specific access needed

2. **UnifiedIR** (`src/ir/unified_ir.rs`)
   - Language-agnostic IR with 12 universal construct types
   - **`RholangExt` variant**: Wraps language-specific Rholang nodes
   - **`MettaExt` variant**: Wraps language-specific MeTTa nodes
   - Conversion functions from both RholangNode and MettaNode

3. **Index-Based Traversal** (completed 2025-10-24)
   - GenericVisitor works across all SemanticNode implementations
   - Enables language-agnostic symbol table building
   - TransformVisitor for immutable tree transformations

4. **Metadata System** (`src/ir/node.rs`)
   - `HashMap<String, Arc<dyn Any>>` allows arbitrary metadata
   - Can store language region information on nodes
   - Extensible without breaking existing code

5. **Symbol Table Architecture** (`src/ir/symbol_table.rs`)
   - Hierarchical scoping already supports nested contexts
   - Inverted index maps symbols to locations
   - Global symbol table connects cross-file references

### ❌ What Needs to Be Added

## Phase 1: Language Region Detection

### 1.1 Comment Directive Parser

**File**: `src/language_regions/directive_parser.rs` (new)

```rust
/// Detects language directives in comments
pub struct DirectiveParser {
    /// Map of byte ranges to language names
    regions: Vec<LanguageRegion>,
}

#[derive(Debug, Clone)]
pub struct LanguageRegion {
    /// Start byte offset in document
    pub start: usize,
    /// End byte offset in document
    pub end: usize,
    /// Language identifier (e.g., "metta", "rholang")
    pub language: String,
    /// Detection method
    pub source: RegionSource,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RegionSource {
    /// Detected via comment directive (// language: X)
    CommentDirective,
    /// Detected via semantic analysis (sent to rho:metta:compile)
    SemanticAnalysis,
    /// Detected via channel flow analysis
    ChannelFlowAnalysis,
}

impl DirectiveParser {
    /// Scans document for language directives above string literals
    pub fn scan_directives(source: &str, tree: &Tree) -> Vec<LanguageRegion> {
        // 1. Parse Rholang AST to find all string literals
        // 2. For each string literal, check if there's a comment directive above it
        // 3. Look for patterns: // @language metta, // @metta, /* @metta */
        // 4. If found, create LanguageRegion for the string literal's contents
        // Returns regions with CommentDirective source
    }

    /// Checks if a comment contains a language directive
    fn parse_directive(comment: &str) -> Option<String> {
        // Matches:
        //   // @language metta  → Some("metta")
        //   // @metta           → Some("metta")
        //   /* @language metta */ → Some("metta")
        //   /* @metta */         → Some("metta")
    }

    /// Finds string literal node that follows a comment with directive
    fn find_string_literal_after_comment(
        comment_pos: Position,
        tree: &Tree,
    ) -> Option<Node> {
        // Walk AST to find the next string literal after comment position
        // Must be within a reasonable distance (e.g., next 5 lines)
    }
}
```

**Examples**:
```rholang
new codeFile in {
  // @metta
  codeFile!("
    (connected room_a room_b)
    (= (get_neighbors $room)
       (match & self (connected $room $target) $target))
  ") |

  for (@code <- codeFile) {
    mettaCompile!?(code) // String is detected as MeTTa code
  }
}

// Alternative syntax
/* @language metta */
queryCode!("!(find_path room_a room_d)")

// Without directive - detected via semantic analysis only
regularCode!("Some Rholang string")  // Not marked, but if sent to
                                      // mettaCompile, still detected
```

### 1.2 Semantic Region Detector

**File**: `src/language_regions/semantic_detector.rs` (new)

```rust
/// Detects language regions via semantic analysis
pub struct SemanticDetector {
    /// Known compiler service URIs and their languages
    compiler_services: HashMap<String, String>,
}

impl SemanticDetector {
    pub fn new() -> Self {
        let mut services = HashMap::new();
        services.insert("rho:metta:compile".to_string(), "metta".to_string());
        // Can add more: "rho:python:eval", "rho:wasm:compile", etc.
        Self { compiler_services: services }
    }

    /// Analyzes IR to find code sent to compiler services
    pub fn detect_regions(&self, ir: &Arc<RholangNode>) -> Vec<LanguageRegion> {
        // 1. Find all @"rho:metta:compile"!(...) calls
        // 2. Trace back to find string literals or channel sources
        // 3. Mark those string literal ranges as MeTTa regions
        // Returns regions with SemanticAnalysis source
    }

    /// Traces channel flows to find indirect sends
    pub fn trace_channel_flows(&self, ir: &Arc<RholangNode>,
                                global_symbols: &GlobalSymbolTable)
        -> Vec<LanguageRegion> {
        // 1. Find contracts that forward to compiler services
        // 2. Find calls to those contracts
        // 3. Mark arguments as language regions
        // Returns regions with ChannelFlowAnalysis source
    }
}
```

**Example detection**:
```rholang
// CASE 1: Direct send - easy to detect
new code in {
  code!("(= (fact 0) 1)") |  // <- Detected: bytes 25-40 = MeTTa
  @"rho:metta:compile"!(code)
}

// CASE 2: Indirect via contract - needs flow analysis
contract evalMetta(@src, ret) = {
  @"rho:metta:compile"!(src, ret)
} |
evalMetta!("(+ 1 2)", *output)  // <- Detected: bytes 85-93 = MeTTa
```

### 1.3 Multi-Language Document Model

**File**: `src/ir/multi_language_document.rs` (new)

```rust
/// Represents a document with multiple language regions
pub struct MultiLanguageDocument {
    /// The document URI
    pub uri: Url,
    /// Complete source text
    pub source: Rope,
    /// Detected language regions (sorted by start offset)
    pub regions: Vec<LanguageRegion>,
    /// Primary language (usually "rholang")
    pub primary_language: String,
    /// Parsed IR for each region
    pub region_irs: HashMap<usize, RegionIR>,
}

#[derive(Debug, Clone)]
pub enum RegionIR {
    /// Rholang region with RholangNode IR
    Rholang(Arc<RholangNode>),
    /// MeTTa region with MettaNode IR
    Metta(Arc<MettaNode>),
    /// Unknown language - stored as string
    Unknown(String),
}

impl MultiLanguageDocument {
    /// Parses document with language region detection
    pub fn parse(uri: Url, source: &str) -> Result<Self, ParseError> {
        // 1. Parse entire document as primary language (Rholang)
        // 2. Scan for comment directives
        // 3. Run semantic detection on Rholang IR
        // 4. Merge detected regions (comment directives take precedence)
        // 5. Re-parse embedded regions with appropriate parsers
        // 6. Build composite IR structure
    }

    /// Gets the language at a specific byte offset
    pub fn language_at(&self, offset: usize) -> &str {
        for region in &self.regions {
            if offset >= region.start && offset < region.end {
                return &region.language;
            }
        }
        &self.primary_language
    }

    /// Gets the IR for a specific position
    pub fn ir_at(&self, position: &Position) -> Option<&RegionIR> {
        let offset = self.source.position_to_byte(position)?;
        // Find region containing offset
        // Return corresponding RegionIR
    }
}
```

## Virtual Document Architecture

### Key Design Decision: Regions as Virtual Sub-Documents

Instead of treating embedded language regions as parts of a single composite document, **each region is modeled as a separate virtual document**. This approach:

✅ **Fits existing LSP patterns** (used in HTML/CSS/JS, Markdown, Jupyter notebooks)
✅ **Enables clean separation** - each virtual document has its own parser, validator, symbol table
✅ **Simplifies LSP features** - goto-definition, diagnostics, etc. work without modification
✅ **Improves caching** - virtual docs can be cached/invalidated independently

### Virtual Document URI Scheme

```
rholang://file:///path/to/file.rho                    (primary document)
rholang://file:///path/to/file.rho#metta:0            (1st MeTTa region)
rholang://file:///path/to/file.rho#metta:1            (2nd MeTTa region)
rholang://file:///path/to/file.rho#metta:L15-L20      (region at lines 15-20)
```

### Virtual Document Structure

**File**: `src/lsp/virtual_document.rs` (new)

```rust
/// Represents a virtual sub-document within a parent document
#[derive(Debug, Clone)]
pub struct VirtualDocument {
    /// Virtual URI for this sub-document
    pub uri: Url,

    /// Reference to parent document URI
    pub parent_uri: Url,

    /// Language identifier (e.g., "metta")
    pub language_id: String,

    /// Extracted source text for this region
    pub text: Rope,

    /// Byte range in parent document
    pub parent_range: Range,

    /// Position offset in parent document
    pub parent_offset: Position,

    /// How this region was detected
    pub detection_source: RegionSource,

    /// Parsed IR for this virtual document
    pub ir: Option<VirtualDocumentIR>,

    /// Symbol table for this virtual document
    pub symbols: Option<Arc<SymbolTable>>,

    /// Version number (for incremental updates)
    pub version: i32,
}

#[derive(Debug, Clone)]
pub enum VirtualDocumentIR {
    Rholang(Arc<RholangNode>),
    Metta(Arc<MettaNode>),
}

impl VirtualDocument {
    /// Creates virtual URI from parent URI and region info
    pub fn create_virtual_uri(parent: &Url, language: &str, index: usize) -> Url {
        let fragment = format!("{}:{}", language, index);
        let mut virtual_uri = parent.clone();
        virtual_uri.set_fragment(Some(&fragment));
        virtual_uri
    }

    /// Maps position in virtual doc to parent doc position
    pub fn virtual_to_parent_position(&self, pos: Position) -> Position {
        Position {
            line: pos.line + self.parent_offset.line,
            character: if pos.line == 0 {
                pos.character + self.parent_offset.character
            } else {
                pos.character
            },
        }
    }

    /// Maps position in parent doc to virtual doc position
    pub fn parent_to_virtual_position(&self, pos: Position) -> Option<Position> {
        if pos.line < self.parent_offset.line {
            return None;  // Before this region
        }

        let virtual_line = pos.line - self.parent_offset.line;
        let end_line = self.parent_range.end.line - self.parent_offset.line;

        if virtual_line > end_line {
            return None;  // After this region
        }

        Some(Position {
            line: virtual_line,
            character: if virtual_line == 0 {
                pos.character.saturating_sub(self.parent_offset.character)
            } else {
                pos.character
            },
        })
    }
}
```

### Virtual Document Registry

**File**: `src/lsp/virtual_document_registry.rs` (new)

```rust
/// Manages virtual documents for all open files
pub struct VirtualDocumentRegistry {
    /// Maps virtual URIs to virtual documents
    virtual_docs: HashMap<Url, VirtualDocument>,

    /// Maps parent URIs to their virtual document URIs
    parent_to_virtual: HashMap<Url, Vec<Url>>,

    /// Reverse mapping: virtual URI → parent URI
    virtual_to_parent: HashMap<Url, Url>,
}

impl VirtualDocumentRegistry {
    /// Registers a new parent document and extracts virtual documents
    pub fn register_document(&mut self, uri: Url, text: &str, language: &str)
        -> Result<Vec<Url>> {
        // 1. Detect language regions (directive + semantic)
        let regions = self.detect_regions(text, language)?;

        // 2. Create virtual document for each region
        let mut virtual_uris = Vec::new();

        for (index, region) in regions.iter().enumerate() {
            let virtual_uri = VirtualDocument::create_virtual_uri(
                &uri,
                &region.language,
                index,
            );

            let virtual_doc = VirtualDocument {
                uri: virtual_uri.clone(),
                parent_uri: uri.clone(),
                language_id: region.language.clone(),
                text: self.extract_region_text(text, &region),
                parent_range: region.range,
                parent_offset: region.range.start,
                detection_source: region.source.clone(),
                ir: None,  // Lazily parsed
                symbols: None,
                version: 1,
            };

            self.virtual_docs.insert(virtual_uri.clone(), virtual_doc);
            virtual_uris.push(virtual_uri.clone());

            // Update mappings
            self.virtual_to_parent.insert(virtual_uri.clone(), uri.clone());
        }

        self.parent_to_virtual.insert(uri.clone(), virtual_uris.clone());
        Ok(virtual_uris)
    }

    /// Gets virtual document by URI
    pub fn get_virtual(&self, uri: &Url) -> Option<&VirtualDocument> {
        self.virtual_docs.get(uri)
    }

    /// Gets all virtual documents for a parent
    pub fn get_virtuals_for_parent(&self, parent_uri: &Url) -> Vec<&VirtualDocument> {
        if let Some(virtual_uris) = self.parent_to_virtual.get(parent_uri) {
            virtual_uris.iter()
                .filter_map(|uri| self.virtual_docs.get(uri))
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Finds virtual document containing a position in parent
    pub fn virtual_at_position(&self, parent_uri: &Url, position: Position)
        -> Option<&VirtualDocument> {
        let virtuals = self.get_virtuals_for_parent(parent_uri);
        virtuals.into_iter()
            .find(|v| v.parent_range.contains(position))
    }

    /// Updates virtual document when parent changes
    pub fn update_parent(&mut self, parent_uri: &Url, new_text: &str)
        -> Result<UpdateResult> {
        // 1. Re-detect regions in updated text
        // 2. Diff old vs new virtual documents
        // 3. Create/update/delete virtual docs as needed
        // 4. Return which virtual docs changed
    }

    /// Removes all virtual documents for a parent
    pub fn unregister_parent(&mut self, parent_uri: &Url) {
        if let Some(virtual_uris) = self.parent_to_virtual.remove(parent_uri) {
            for virtual_uri in virtual_uris {
                self.virtual_docs.remove(&virtual_uri);
                self.virtual_to_parent.remove(&virtual_uri);
            }
        }
    }
}

#[derive(Debug)]
pub struct UpdateResult {
    /// Virtual documents that were created
    pub created: Vec<Url>,
    /// Virtual documents that were modified
    pub modified: Vec<Url>,
    /// Virtual documents that were deleted
    pub deleted: Vec<Url>,
}
```

### Integration with RholangBackend

**File**: `src/lsp/backend.rs` (enhance existing)

```rust
pub struct RholangBackend {
    // Existing fields...

    /// Registry of virtual documents
    virtual_registry: Arc<RwLock<VirtualDocumentRegistry>>,
}

impl RholangBackend {
    /// Enhanced didOpen to create virtual documents
    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let text = params.text_document.text;

        // 1. Open primary document (existing logic)
        self.open_documents.write().await.insert(...);

        // 2. Detect and register virtual documents
        let virtual_uris = self.virtual_registry
            .write().await
            .register_document(uri.clone(), &text, "rholang")
            .unwrap_or_default();

        // 3. Parse each virtual document
        for virtual_uri in virtual_uris {
            self.parse_virtual_document(&virtual_uri).await;
        }

        // 4. Run diagnostics on primary + all virtuals
        self.validate_document_cascade(&uri).await;
    }

    /// Parses a virtual document with appropriate parser
    async fn parse_virtual_document(&self, virtual_uri: &Url) {
        let mut registry = self.virtual_registry.write().await;
        let virtual_doc = registry.get_mut(virtual_uri).unwrap();

        match virtual_doc.language_id.as_str() {
            "metta" => {
                // Parse with MeTTa parser
                let ir = parse_metta(&virtual_doc.text);
                virtual_doc.ir = Some(VirtualDocumentIR::Metta(ir));

                // Build symbol table for this virtual doc
                let symbols = build_metta_symbols(&ir);
                virtual_doc.symbols = Some(symbols);
            }
            "rholang" => {
                // Parse with Rholang parser
                let ir = parse_rholang(&virtual_doc.text);
                virtual_doc.ir = Some(VirtualDocumentIR::Rholang(ir));
            }
            _ => {
                // Unknown language - skip parsing
            }
        }
    }

    /// Validates document and all virtual sub-documents
    async fn validate_document_cascade(&self, parent_uri: &Url) {
        let mut all_diagnostics = HashMap::new();

        // 1. Validate primary document
        let primary_diags = self.validate_document(parent_uri).await;
        all_diagnostics.insert(parent_uri.clone(), primary_diags);

        // 2. Validate each virtual document
        let registry = self.virtual_registry.read().await;
        let virtuals = registry.get_virtuals_for_parent(parent_uri);

        for virtual in virtuals {
            let diags = self.validate_virtual_document(virtual).await;

            // Map diagnostics back to parent document positions
            let parent_diags = diags.into_iter()
                .map(|mut d| {
                    d.range = Range {
                        start: virtual.virtual_to_parent_position(d.range.start),
                        end: virtual.virtual_to_parent_position(d.range.end),
                    };
                    d
                })
                .collect();

            all_diagnostics.entry(parent_uri.clone())
                .or_insert_with(Vec::new)
                .extend(parent_diags);
        }

        // 3. Publish combined diagnostics for parent document
        for (uri, diags) in all_diagnostics {
            self.client.publish_diagnostics(uri, diags, None).await;
        }
    }

    /// Validates a single virtual document
    async fn validate_virtual_document(&self, virtual: &VirtualDocument)
        -> Vec<Diagnostic> {
        match &virtual.ir {
            Some(VirtualDocumentIR::Metta(ir)) => {
                // Use MeTTaTron validator
                self.metta_validator.validate(ir).await
                    .unwrap_or_default()
            }
            Some(VirtualDocumentIR::Rholang(ir)) => {
                // Use RNode validator
                self.rholang_validator.validate(ir).await
                    .unwrap_or_default()
            }
            None => Vec::new(),
        }
    }

    /// Enhanced goto-definition with virtual document awareness
    async fn goto_definition(&self, params: GotoDefinitionParams)
        -> Result<Option<GotoDefinitionResponse>> {
        let parent_uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        // 1. Check if position is in a virtual document
        let registry = self.virtual_registry.read().await;
        if let Some(virtual) = registry.virtual_at_position(&parent_uri, position) {
            // 2. Convert position to virtual document coordinates
            let virtual_pos = virtual.parent_to_virtual_position(position).unwrap();

            // 3. Find definition within virtual document
            let def_pos = self.find_definition_in_virtual(virtual, virtual_pos).await?;

            // 4. Convert back to parent document coordinates
            let parent_def_pos = virtual.virtual_to_parent_position(def_pos);

            return Ok(Some(GotoDefinitionResponse::Scalar(Location {
                uri: parent_uri,
                range: Range::new(parent_def_pos, parent_def_pos),
            })));
        }

        // 5. Position is in primary document - use standard logic
        self.goto_definition_standard(params).await
    }
}
```

## Phase 2: Multi-Language IR Integration

### 2.1 Enhanced UnifiedIR Construction

**File**: `src/ir/unified_ir.rs` (enhance existing)

```rust
impl UnifiedIR {
    /// Converts a multi-language document to UnifiedIR
    pub fn from_multi_language_document(doc: &MultiLanguageDocument)
        -> Arc<UnifiedIR> {
        // 1. Convert primary Rholang IR to UnifiedIR
        // 2. For each embedded region:
        //    - Convert region IR to UnifiedIR
        //    - Wrap in appropriate Ext variant (MettaExt, etc.)
        //    - Attach language metadata
        // 3. Return composite UnifiedIR tree
    }
}

// Example metadata for language regions:
pub const LANGUAGE_REGION_KEY: &str = "language_region";

#[derive(Debug, Clone)]
pub struct LanguageRegionMetadata {
    pub language: String,
    pub source: RegionSource,
    pub original_range: Range,
}
```

### 2.2 Cross-Language Symbol Table

**File**: `src/ir/symbol_table.rs` (enhance existing)

```rust
/// Extends SymbolInfo to track language
#[derive(Debug, Clone, PartialEq)]
pub struct LanguageAwareSymbolInfo {
    pub base: SymbolInfo,
    /// Language this symbol belongs to
    pub language: String,
    /// For cross-language references (e.g., Rholang var with MeTTa value)
    pub cross_language_ref: Option<CrossLanguageRef>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CrossLanguageRef {
    /// Source language
    pub from_language: String,
    /// Target language
    pub to_language: String,
    /// How the reference crosses languages
    pub bridge: CrossLanguageBridge,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CrossLanguageBridge {
    /// String literal containing embedded code
    StringLiteral,
    /// Channel send to compiler service
    CompilerService(String),  // e.g., "rho:metta:compile"
    /// Foreign function interface
    FFI,
}
```

**Example**:
```rholang
// Symbol: `mettaCode` - Rholang variable
// Value: "(+ 1 2)" - MeTTa expression
// Bridge: CompilerService("rho:metta:compile")
new mettaCode in {
  mettaCode!("(+ 1 2)") |
  @"rho:metta:compile"!(mettaCode, *result)
}
```

## Phase 3: LSP Feature Integration

### 3.1 Multi-Language Diagnostics

**File**: `src/lsp/multi_language_diagnostics.rs` (new)

```rust
/// Routes diagnostics to appropriate validators
pub struct MultiLanguageDiagnosticProvider {
    rholang_validator: Arc<dyn SemanticValidator>,
    metta_validator: Arc<dyn SemanticValidator>,
}

impl MultiLanguageDiagnosticProvider {
    pub async fn validate_document(&self, doc: &MultiLanguageDocument)
        -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();

        // 1. Validate primary Rholang structure
        let rholang_diags = self.rholang_validator
            .validate(&doc.region_irs[&0]).await;
        diagnostics.extend(rholang_diags);

        // 2. Validate each embedded region
        for (offset, region_ir) in &doc.region_irs {
            if *offset == 0 { continue; }  // Skip primary

            match region_ir {
                RegionIR::Metta(ir) => {
                    let metta_diags = self.metta_validator
                        .validate(ir).await;
                    diagnostics.extend(metta_diags);
                }
                RegionIR::Unknown(_) => {
                    // Warning: Unknown language region
                }
                _ => {}
            }
        }

        diagnostics
    }
}
```

### 3.2 Cross-Language Go-To-Definition

**File**: `src/lsp/cross_language_navigation.rs` (new)

```rust
impl RholangBackend {
    /// Enhanced goto_definition with cross-language support
    pub async fn goto_definition_multi_language(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let doc = self.get_multi_language_document(&params.text_document)?;
        let position = params.position;

        // 1. Determine language at cursor position
        let language = doc.language_at_position(&position);

        // 2. Get appropriate IR
        let ir = doc.ir_at(&position)?;

        // 3. Find symbol at position
        let symbol = self.find_symbol_at_position(ir, &position)?;

        // 4. Check if cross-language reference
        if let Some(cross_ref) = &symbol.cross_language_ref {
            match cross_ref.bridge {
                CrossLanguageBridge::CompilerService(ref service) => {
                    // Navigate to the compiler service definition
                    return self.find_compiler_service_definition(service);
                }
                CrossLanguageBridge::StringLiteral => {
                    // Navigate into the string literal (embedded code)
                    return self.navigate_to_embedded_region(symbol);
                }
                _ => {}
            }
        }

        // 5. Standard same-language navigation
        self.goto_definition_standard(symbol)
    }
}
```

### 3.3 Language-Aware Hover

```rust
impl RholangBackend {
    pub async fn hover_multi_language(
        &self,
        params: HoverParams,
    ) -> Result<Option<Hover>> {
        let doc = self.get_multi_language_document(&params.text_document)?;
        let position = params.position;
        let language = doc.language_at_position(&position);

        // Show language in hover information
        let mut contents = Vec::new();
        contents.push(format!("**Language**: {}", language));

        // Add language-specific type/signature info
        match language {
            "rholang" => {
                // Rholang-specific hover
            }
            "metta" => {
                // MeTTa type information
                // E.g., "(: factorial (-> Number Number))"
            }
            _ => {}
        }

        Ok(Some(Hover { contents, range: None }))
    }
}
```

## Phase 4: Validation Pipeline

### 4.1 MeTTaTron Validator Integration

**File**: `src/lsp/metta_validator.rs` (new)

```rust
/// Validator that communicates with MeTTaTron compiler
pub struct MettaTronValidator {
    /// gRPC client to MeTTaTron service
    client: MettaTronClient<Channel>,
    address: String,
}

#[async_trait]
impl SemanticValidator for MettaTronValidator {
    async fn validate(&self, ir: &Arc<MettaNode>)
        -> Result<Vec<Diagnostic>, ValidationError> {
        // 1. Convert MettaNode IR to MeTTa source code
        let source = ir.to_metta_source();

        // 2. Send to MeTTaTron compiler service
        let response = self.client
            .validate(ValidateRequest { source })
            .await?;

        // 3. Convert MeTTaTron errors to LSP Diagnostics
        let diagnostics = response.errors.into_iter()
            .map(|err| Diagnostic {
                range: err.range,
                severity: Some(DiagnosticSeverity::ERROR),
                message: err.message,
                source: Some("mettatron".to_string()),
                ..Default::default()
            })
            .collect();

        Ok(diagnostics)
    }
}
```

### 4.2 Pluggable Validator Backend (Enhanced)

**File**: `src/lsp/validator_backend.rs` (enhance existing)

```rust
/// Multi-language validator backend
pub enum ValidatorBackend {
    /// RNode validator for Rholang
    Rholang(Arc<dyn SemanticValidator>),
    /// MeTTaTron validator for MeTTa
    Metta(Arc<dyn SemanticValidator>),
    /// Rust-based validators (fallback)
    Rust(Arc<RustSemanticValidator>),
    /// Composite validator for multi-language docs
    MultiLanguage(MultiLanguageDiagnosticProvider),
}

impl ValidatorBackend {
    /// Creates appropriate validator for language
    pub fn for_language(language: &str) -> Self {
        match language {
            "rholang" => ValidatorBackend::Rholang(Arc::new(GrpcValidator::new())),
            "metta" => ValidatorBackend::Metta(Arc::new(MettaTronValidator::new())),
            _ => ValidatorBackend::Rust(Arc::new(RustSemanticValidator)),
        }
    }

    /// Validates with appropriate backend
    pub async fn validate_region(&self, ir: &RegionIR)
        -> Result<Vec<Diagnostic>> {
        match (self, ir) {
            (ValidatorBackend::Rholang(v), RegionIR::Rholang(ir)) => {
                v.validate(ir).await
            }
            (ValidatorBackend::Metta(v), RegionIR::Metta(ir)) => {
                v.validate(ir).await
            }
            _ => Ok(Vec::new()),  // Mismatched language/validator
        }
    }
}
```

## Phase 5: Workspace Integration

### 5.1 Multi-Language Workspace State

**File**: `src/lsp/workspace.rs` (enhance existing)

```rust
pub struct WorkspaceState {
    // Existing fields...

    /// Multi-language documents in workspace
    pub multi_language_docs: HashMap<Url, MultiLanguageDocument>,

    /// Cross-file language region index
    /// Maps compiler service URIs to all regions that use them
    pub service_usage_index: HashMap<String, Vec<(Url, LanguageRegion)>>,
}

impl WorkspaceState {
    /// Indexes all language regions across workspace
    pub fn index_language_regions(&mut self) {
        for (uri, doc) in &self.multi_language_docs {
            for region in &doc.regions {
                if let RegionSource::SemanticAnalysis = region.source {
                    // Track which files use which compiler services
                    self.service_usage_index
                        .entry(region.language.clone())
                        .or_insert_with(Vec::new)
                        .push((uri.clone(), region.clone()));
                }
            }
        }
    }

    /// Finds all MeTTa regions in workspace
    pub fn find_metta_regions(&self) -> Vec<(Url, LanguageRegion)> {
        self.service_usage_index
            .get("metta")
            .cloned()
            .unwrap_or_default()
    }
}
```

## Implementation Roadmap

### Sprint 1: Foundation (1-2 weeks)
- [ ] Implement `DirectiveParser` for comment-based detection
- [ ] Create `LanguageRegion` and `MultiLanguageDocument` types
- [ ] Update `didOpen`/`didChange` to detect language regions
- [ ] Add tests for comment directive parsing

### Sprint 2: Semantic Detection (1-2 weeks)
- [ ] Implement `SemanticDetector` for service-based detection
- [ ] Add channel flow analysis for indirect detection
- [ ] Integrate with existing symbol table builder
- [ ] Test detection accuracy with sample files

### Sprint 3: Multi-Language IR (1-2 weeks)
- [ ] Enhance UnifiedIR for composite documents
- [ ] Implement region IR parsing and caching
- [ ] Add cross-language symbol table support
- [ ] Create metadata system for language regions

### Sprint 4: LSP Features (2-3 weeks)
- [ ] Implement multi-language diagnostics routing
- [ ] Add cross-language navigation (goto-definition, references)
- [ ] Enhance hover with language context
- [ ] Update document symbols to show language regions

### Sprint 5: Validation Integration (1-2 weeks)
- [ ] Create MeTTaTron validator interface
- [ ] Implement gRPC client for MeTTaTron service
- [ ] Add validation routing based on language
- [ ] Test with real MeTTa code samples

### Sprint 6: Workspace Features (1 week)
- [ ] Implement workspace-wide language region indexing
- [ ] Add "find all MeTTa regions" functionality
- [ ] Create language region visualization for editors
- [ ] Performance optimization for large workspaces

## Testing Strategy

### Unit Tests
- Directive parser with various comment formats
- Semantic detector with known patterns
- Region IR construction and conversion
- Cross-language symbol resolution

### Integration Tests
```rust
#[test]
fn test_metta_region_detection() {
    let source = r#"
        // language: metta
        (: add (-> Number Number Number))
        (= (add $x $y) (+ $x $y))
        // language: rholang
        new result in {
          @"rho:metta:compile"!("(add 1 2)", *result)
        }
    "#;

    let doc = MultiLanguageDocument::parse(uri, source).unwrap();
    assert_eq!(doc.regions.len(), 2);
    assert_eq!(doc.regions[0].language, "metta");
    assert_eq!(doc.regions[0].source, RegionSource::CommentDirective);
    assert_eq!(doc.regions[1].language, "metta");
    assert_eq!(doc.regions[1].source, RegionSource::SemanticAnalysis);
}
```

### End-to-End Tests
- Open file with embedded MeTTa → verify diagnostics
- Goto-definition from Rholang to MeTTa region
- Hover over MeTTa code → shows MeTTa type
- Rename symbol used in both languages

## Performance Considerations

1. **Lazy Parsing**: Only parse embedded regions when needed (on-demand)
2. **Caching**: Cache region detection results, invalidate on change
3. **Incremental Updates**: Re-detect only changed regions on `didChange`
4. **Parallel Validation**: Validate different language regions concurrently
5. **Smart Indexing**: Index by language to avoid scanning all files

## Future Extensions

1. **Additional Languages**: Python, JavaScript, Scala (via similar patterns)
2. **Language Nesting**: MeTTa inside Rholang inside larger framework
3. **Bidirectional References**: MeTTa code referencing Rholang symbols
4. **Mixed Completions**: Suggest Rholang and MeTTa symbols based on context
5. **Visual Indicators**: Editor decorations showing language boundaries

## Security Considerations

1. **Sandboxing**: Embedded code validation happens in isolated processes
2. **Service Verification**: Validate compiler service URIs against whitelist
3. **Resource Limits**: Timeout and memory limits for embedded validators
4. **Code Injection**: Sanitize strings sent to compiler services

## Conclusion

The current SemanticNode/UnifiedIR architecture provides a **solid foundation** for multi-language support:

✅ **Strengths**:
- Language-agnostic traversal already works
- UnifiedIR has extension points (RholangExt, MettaExt)
- Metadata system supports arbitrary annotations
- Symbol tables are extensible

⚠️ **Gaps Identified**:
- No language region detection yet
- No multi-language document model
- No cross-language navigation
- No validator routing by language

📋 **Estimated Effort**: 8-12 weeks for full implementation

The design is **intentionally modular** to allow incremental rollout. Phase 1 (comment directives) can ship independently before semantic detection (Phase 2) is complete.
