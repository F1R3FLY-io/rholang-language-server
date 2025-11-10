# Position Tracking in Rholang Language Server

## Table of Contents

1. [Overview](#overview)
2. [Architecture](#architecture)
3. [Phase 1: Parsing (Tree-Sitter → IR)](#phase-1-parsing-tree-sitter--ir)
4. [Phase 2: IR Storage](#phase-2-ir-storage)
5. [Phase 3: Symbol Table Construction](#phase-3-symbol-table-construction)
6. [Phase 4: LSP Operations](#phase-4-lsp-operations)
7. [Phase 5: Special Cases](#phase-5-special-cases)
8. [Complete Data Flow](#complete-data-flow)
9. [Migration History](#migration-history)
10. [Code Reference Map](#code-reference-map)
11. [Theory & Invariants](#theory--invariants)

---

## Overview

The Rholang Language Server implements a comprehensive position tracking system that enables precise source code navigation, symbol resolution, and LSP (Language Server Protocol) features. This document describes the complete end-to-end flow of position information from parsing through LSP operations.

### Key Design Principles

1. **Absolute Position Storage**: Positions are stored as absolute coordinates (row, column, byte) directly from Tree-Sitter, eliminating delta computation errors
2. **Dual-Length Tracking**: Nodes track both semantic length (content) and syntactic length (including delimiters) for accurate position reconstruction
3. **UTF-8/UTF-16 Conversion**: Proper handling of character encoding differences between Rope (UTF-8) and LSP (UTF-16)
4. **Performance Optimization**: Cached Tree-Sitter calls, singleton metadata, and efficient rope-based byte offset computation

### Position Coordinate Systems

The system works with three coordinate systems:

| System | Format | Row Base | Column Base | Column Unit | Usage |
|--------|--------|----------|-------------|-------------|-------|
| **Tree-Sitter** | `(row, column, byte)` | 0 | 0 | Byte | Parser output |
| **IR (Internal)** | `Position { row, column, byte }` | 0 | 0 | Byte | Internal representation |
| **LSP (Client)** | `Position { line, character }` | 0 | 0 | UTF-16 code unit | Editor communication |

---

## Architecture

### High-Level Data Flow

```
┌─────────────────┐
│  Source Code    │
│   (.rho file)   │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  Tree-Sitter    │  ← Provides absolute positions
│    Parser       │     (row, column, byte)
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│ IR Conversion   │  ← Creates NodeBase with positions
│  (AST → IR)     │     Stores absolute start + lengths
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│ Symbol Table    │  ← Extracts positions from nodes
│   Builder       │     Indexes symbols by position
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│ LSP Features    │  ← Converts positions for client
│ (goto-def, etc) │     Maps IR positions ↔ LSP positions
└─────────────────┘
```

### Core Data Structures

```rust
// src/ir/semantic_node.rs:32-36
pub struct Position {
    pub row: usize,      // Line number (0-based)
    pub column: usize,   // Column number (0-based)
    pub byte: usize,     // Byte offset from file start
}

// src/ir/semantic_node.rs:58-89
pub struct NodeBase {
    start: Position,              // Absolute start (from Tree-Sitter)
    content_length: usize,        // Semantic length (up to last child)
    syntactic_length: usize,      // Syntactic length (includes closing delimiters)
    span_lines: usize,            // Number of lines spanned
    span_columns: usize,          // Columns on last line
    metadata: Arc<HashMap<...>>,  // Optional metadata
}
```

---

## Phase 1: Parsing (Tree-Sitter → IR)

**Location**: `src/parsers/rholang/conversion/mod.rs`

### Tree-Sitter Position Extraction

Tree-Sitter provides four position methods for every node:

```rust
// Tree-Sitter API
ts_node.start_position()  // Returns { row: usize, column: usize }
ts_node.end_position()    // Returns { row: usize, column: usize }
ts_node.start_byte()      // Returns usize (byte offset)
ts_node.end_byte()        // Returns usize (byte offset)
```

### Conversion Process

The `convert_ts_node_to_ir()` function (lines 102-150) extracts positions:

```rust
// src/parsers/rholang/conversion/mod.rs:107-121
let start_pos = ts_node.start_position();  // Cached to avoid redundant calls
let end_pos = ts_node.end_position();
let start_byte = ts_node.start_byte();
let end_byte = ts_node.end_byte();

let absolute_start = Position {
    row: start_pos.row,
    column: start_pos.column,
    byte: start_byte,
};

let absolute_end = Position {
    row: end_pos.row,
    column: end_pos.column,
    byte: end_byte,
};
```

**Performance Optimization**: Positions are cached in local variables to reduce Tree-Sitter API calls from 6 to 4 per node.

### Position Flow Diagram

```
Tree-Sitter Node
    │
    ├─ start_position() → { row: 5, column: 10 }
    ├─ start_byte()     → 142
    ├─ end_position()   → { row: 5, column: 15 }
    └─ end_byte()       → 147
         │
         ▼
    Position { row: 5, column: 10, byte: 142 }
         │
         ▼
    NodeBase { start: Position, content_length: 5, ... }
         │
         ▼
    RholangNode (stored in IR)
```

### Helper Functions

**Location**: `src/parsers/position_utils.rs`

Two main helper functions create NodeBase instances:

#### 1. `create_node_base_from_absolute` (lines 118-161)

For nodes with closing delimiters (blocks, lists, maps, etc.):

```rust
pub fn create_node_base_from_absolute(
    absolute_start: Position,
    absolute_end: Position,
    content_end: Position,      // End of content (before closing delimiter)
    prev_end: &mut Position,    // Updated for next sibling
) -> NodeBase {
    let content_length = content_end.byte - absolute_start.byte;
    let syntactic_length = absolute_end.byte - absolute_start.byte;

    // Update prev_end for next sibling
    *prev_end = absolute_end;

    NodeBase {
        start: absolute_start,
        content_length,
        syntactic_length,
        span_lines: absolute_end.row - absolute_start.row,
        span_columns: if absolute_end.row == absolute_start.row {
            absolute_end.column - absolute_start.column
        } else {
            absolute_end.column
        },
        metadata: SINGLETON_EMPTY_METADATA.clone(),
    }
}
```

**Example**:
```rholang
{ x!(42) }
^        ^
│        └─ absolute_end (byte 8)
│       ^
│       └─ content_end (byte 7, before closing '}')
└─ absolute_start (byte 0)
```

Result:
- `content_length = 7` (semantic length)
- `syntactic_length = 8` (includes closing delimiter)

#### 2. `create_simple_node_base` (lines 195-201)

For simple nodes without closing delimiters:

```rust
pub fn create_simple_node_base(
    absolute_start: Position,
    absolute_end: Position,
    prev_end: &mut Position,
) -> NodeBase {
    create_node_base_from_absolute(
        absolute_start,
        absolute_end,
        absolute_end,  // content_end = absolute_end (no delimiter)
        prev_end,
    )
}
```

**Example**:
```rholang
x
^
└─ Single-character variable: content_length = syntactic_length = 1
```

### Dual-Length System

The IR uses two length measurements for precise position tracking:

| Length Type | Purpose | Measurement | Example: `{ x!(42) }` |
|-------------|---------|-------------|----------------------|
| **content_length** | Semantic operations | Start to last child's end | 7 bytes (excludes `}`) |
| **syntactic_length** | Position reconstruction | Start to syntactic end | 8 bytes (includes `}`) |

**Why Both?**

Container nodes (blocks, lists, maps) have content followed by closing delimiters:

```rholang
{ x!(42) }
^      ^ ^
│      │ └─ syntactic_length ends here (includes '}')
│      └─ content_length ends here (last child)
└─ start position
```

- **Semantic operations** (symbol resolution, traversal) care about content
- **Position reconstruction** (computing sibling positions) needs the full syntactic extent

---

## Phase 2: IR Storage

### NodeBase Structure Details

**Location**: `src/ir/semantic_node.rs:58-89`

```rust
pub struct NodeBase {
    start: Position,              // Absolute start position
    content_length: usize,        // Bytes from start to content end
    syntactic_length: usize,      // Bytes from start to syntactic end
    span_lines: usize,            // Lines spanned by node
    span_columns: usize,          // Columns on last line
    metadata: Arc<HashMap<String, Arc<dyn Any + Send + Sync>>>,
}
```

### Key Methods

#### Position Computation

```rust
impl NodeBase {
    // Get absolute start position
    pub fn start(&self) -> &Position {
        &self.start
    }

    // Compute absolute end position (semantic)
    pub fn end(&self) -> Position {
        Position {
            row: self.start.row + self.span_lines,
            column: if self.span_lines == 0 {
                self.start.column + self.span_columns
            } else {
                self.span_columns
            },
            byte: self.start.byte + self.content_length,
        }
    }

    // Compute syntactic end (includes delimiters)
    pub fn syntactic_end(&self) -> Position {
        Position {
            row: self.start.row + self.span_lines,
            column: if self.span_lines == 0 {
                self.start.column + self.span_columns
            } else {
                self.span_columns
            },
            byte: self.start.byte + self.syntactic_length,
        }
    }
}
```

### Position Computation Example

```rholang
{
  x!(42)
}
```

Given:
- `start = Position { row: 0, column: 0, byte: 0 }`
- `content_length = 9` (up to end of `x!(42)`)
- `syntactic_length = 11` (includes `\n}`)
- `span_lines = 2`
- `span_columns = 1` (column of `}` on last line)

Computed:
- `end() = Position { row: 2, column: 1, byte: 9 }` (semantic)
- `syntactic_end() = Position { row: 2, column: 1, byte: 11 }` (syntactic)

### Memory Optimization

**Singleton Empty Metadata** (lines 40-56):

```rust
// Pre-allocated empty metadata shared by all nodes
lazy_static! {
    pub static ref SINGLETON_EMPTY_METADATA: Arc<HashMap<String, Arc<dyn Any + Send + Sync>>> =
        Arc::new(HashMap::new());
}
```

**Benefit**: 80-90% reduction in metadata allocation overhead. Only nodes with actual metadata allocate new HashMaps.

### RholangNode Integration

**Location**: `src/ir/rholang_node.rs`

Every `RholangNode` variant contains a `NodeBase`:

```rust
pub enum RholangNode {
    Var { base: NodeBase, name: String },
    Send { base: NodeBase, channel: Arc<RholangNode>, ... },
    Par { base: NodeBase, left: Arc<RholangNode>, right: Arc<RholangNode> },
    // ... all variants have base: NodeBase
}

impl SemanticNode for RholangNode {
    fn base(&self) -> &NodeBase {
        match self {
            RholangNode::Var { base, .. } => base,
            RholangNode::Send { base, .. } => base,
            RholangNode::Par { base, .. } => base,
            // ... all variants return their base
        }
    }
}
```

This design ensures **every IR node has position information**.

---

## Phase 3: Symbol Table Construction

**Location**: `src/ir/transforms/symbol_table_builder.rs`

During IR traversal, the symbol table builder extracts position information and indexes symbols by their definitions.

### Position Extraction Process

```rust
impl<'a> Visitor<RholangNode> for SymbolTableBuilder<'a> {
    fn visit_contract(&mut self, node: &Arc<RholangNode>) -> Arc<RholangNode> {
        if let RholangNode::Contract { base, name, formals, body, .. } = node.as_ref() {
            let start_pos = base.start();
            let end_pos = base.end();

            // Store symbol with absolute positions
            self.symbol_table.insert(
                name.clone(),
                SymbolInfo {
                    name: name.clone(),
                    kind: SymbolKind::Contract,
                    definition_range: Range {
                        start: *start_pos,
                        end: end_pos,
                    },
                    scope_id: self.current_scope,
                    // ...
                }
            );
        }
        // Continue traversal...
    }
}
```

### Symbol Storage

Symbols are stored with **absolute positions** extracted directly from `NodeBase`:

```rust
pub struct SymbolInfo {
    pub name: String,
    pub kind: SymbolKind,
    pub definition_range: Range,    // Absolute positions
    pub scope_id: ScopeId,
    // ... metadata
}

pub struct Range {
    pub start: Position,    // Absolute start (from base.start())
    pub end: Position,      // Absolute end (from base.end())
}
```

**No transformation needed** - positions are already absolute and ready for use.

### Global Symbol Index

**Location**: `src/ir/global_index.rs`

The `GlobalSymbolIndex` maintains workspace-wide symbol tables:

```rust
pub struct GlobalSymbolIndex {
    // Maps symbol name → list of locations
    symbols: HashMap<String, Vec<SymbolLocation>>,
    // Pattern index for contract matching
    pattern_index: RholangPatternIndex,
    // ...
}

pub struct SymbolLocation {
    pub uri: Url,           // File URI
    pub range: Range,       // Position range (absolute)
    pub kind: SymbolKind,
}
```

This enables cross-file navigation with precise position tracking.

---

## Phase 4: LSP Operations

### LSP Position Conversion

**Location**: `src/lsp/features/node_finder.rs`

#### LSP → IR Position Conversion (lines 283-289)

```rust
pub fn lsp_to_ir_position(lsp_pos: LspPosition) -> Position {
    Position {
        row: lsp_pos.line as usize,
        column: lsp_pos.character as usize,
        byte: 0,  // Computed separately using Rope
    }
}
```

**Note**: LSP uses `line` and `character`, while IR uses `row` and `column`. The byte offset is computed separately because LSP uses UTF-16 code units while IR uses UTF-8 bytes.

#### Byte Offset Computation

**Location**: `src/lsp/backend.rs:655-671`

```rust
pub fn byte_offset_from_position(text: &Rope, line: usize, character: usize) -> Option<usize> {
    text.try_line_to_byte(line).ok().map(|line_start_byte| {
        let line_text = text.line(line);

        // Convert UTF-16 character offset to UTF-8 byte offset
        let char_offset = character.min(line_text.len_chars());
        let byte_in_line = line_text.char_to_byte(char_offset);

        line_start_byte + byte_in_line
    })
}
```

**Why Rope?** The `ropey` crate provides efficient:
- Line-to-byte offset conversion (O(log n))
- UTF-8 to UTF-16 character mapping
- Incremental text updates

**UTF-16 vs UTF-8 Example**:

```
Text: "Hello 世界"
      ^     ^
LSP character positions:  0-5   6-7  (UTF-16 code units)
UTF-8 byte positions:     0-5   6-11 (3 bytes per CJK character)
```

The `Rope` handles this conversion automatically via `char_to_byte()`.

#### IR → LSP Position Conversion (lines 298-303)

```rust
pub fn ir_to_lsp_position(ir_pos: &Position) -> LspPosition {
    LspPosition {
        line: ir_pos.row as u32,
        character: ir_pos.column as u32,
    }
}
```

Simple type conversion - IR positions are already in the correct coordinate system.

### Node Lookup by Position

**Location**: `src/lsp/features/node_finder.rs:11-151`

The `find_node_at_position()` function performs a tree traversal to find the deepest node containing a target position:

```rust
pub fn find_node_at_position<'a>(
    node: &'a Arc<RholangNode>,
    target: &Position,
) -> Option<&'a Arc<RholangNode>> {
    let base = node.base();
    let start = base.start();
    let end = base.end();

    // Check if target is within this node's range
    if target.byte < start.byte || target.byte > end.byte {
        return None;
    }

    // Check if target is before node starts
    if target.row < start.row || (target.row == start.row && target.column < start.column) {
        return None;
    }

    // Check if target is after node ends
    if target.row > end.row || (target.row == end.row && target.column > end.column) {
        return None;
    }

    // Target is within this node - check children
    match node.as_ref() {
        RholangNode::Send { channel, inputs, .. } => {
            // Check channel first
            if let Some(child_result) = find_node_at_position(channel, target) {
                return Some(child_result);
            }
            // Check inputs
            for input in inputs {
                if let Some(child_result) = find_node_at_position(input, target) {
                    return Some(child_result);
                }
            }
        }
        // ... handle other node types
    }

    // No child contains target - return this node
    Some(node)
}
```

**Algorithm**:
1. Check if target is within node's byte range (fast rejection)
2. Check if target is within node's (row, column) range (precise check)
3. Recursively check children
4. Return deepest matching node

**Complexity**: O(log n) for balanced trees, O(n) worst case

### LSP Feature Implementation Example: Goto-Definition

**Location**: `src/lsp/backend/unified_handlers.rs`

```rust
pub(super) async fn goto_definition(
    &self,
    params: GotoDefinitionParams,
) -> LspResult<Option<GotoDefinitionResponse>> {
    let uri = params.text_document_position_params.text_document.uri;
    let lsp_position = params.text_document_position_params.position;

    // Step 1: Convert LSP position → IR position
    let ir_position = lsp_to_ir_position(lsp_position);

    // Step 2: Get document and compute byte offset
    let doc = self.workspace.get_document(&uri).await?;
    let byte_offset = byte_offset_from_position(&doc.rope, ir_position.row, ir_position.column)?;
    let ir_position_with_byte = Position {
        row: ir_position.row,
        column: ir_position.column,
        byte: byte_offset,
    };

    // Step 3: Find node at position
    let node = find_node_at_position(&doc.ir_root, &ir_position_with_byte)?;

    // Step 4: Extract symbol and resolve
    let symbol_name = extract_symbol_name(node)?;
    let locations = self.resolver.resolve_symbol(
        &symbol_name,
        &ir_position_with_byte,
        &context,
    );

    // Step 5: Convert back to LSP locations
    let lsp_locations: Vec<Location> = locations.iter()
        .map(|loc| Location {
            uri: loc.uri.clone(),
            range: LspRange {
                start: ir_to_lsp_position(&loc.range.start),
                end: ir_to_lsp_position(&loc.range.end),
            },
        })
        .collect();

    Ok(Some(GotoDefinitionResponse::Array(lsp_locations)))
}
```

### Position Conversion Flow

```
┌─────────────────────────────────────────────────────────────┐
│ LSP Client sends goto-definition request                    │
│ Position { line: 5, character: 10 }                         │
└────────────────────┬────────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────────┐
│ lsp_to_ir_position()                                         │
│ Position { row: 5, column: 10, byte: 0 }                    │
└────────────────────┬────────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────────┐
│ byte_offset_from_position(rope, 5, 10)                      │
│ Position { row: 5, column: 10, byte: 142 }                  │
└────────────────────┬────────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────────┐
│ find_node_at_position(ir_root, position)                    │
│ Returns: &Arc<RholangNode>                                   │
└────────────────────┬────────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────────┐
│ Extract symbol, resolve via symbol table                    │
│ Returns: Vec<SymbolLocation> with absolute positions        │
└────────────────────┬────────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────────┐
│ ir_to_lsp_position() for each location                      │
│ Convert: Position { row, column, byte } → { line, character }│
└────────────────────┬────────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────────┐
│ LSP Client receives response                                │
│ Location { uri, range: { start, end } }                     │
└─────────────────────────────────────────────────────────────┘
```

---

## Phase 5: Special Cases

### Quote Nodes (@-prefix)

**Location**: `src/parsers/rholang/conversion/quote.rs`

Quote nodes have special position handling because the `@` symbol precedes the quoted expression:

```rholang
@process
^       ^
│       └─ quoted expression starts here (byte 1)
└─ quote node starts here (byte 0)
```

**Implementation**:

```rust
pub fn convert_quote(ts_node: TSNode, rope: &Rope, prev_end: Position) -> Result<Arc<RholangNode>> {
    let absolute_start = Position {
        row: ts_node.start_position().row,
        column: ts_node.start_position().column,
        byte: ts_node.start_byte(),
    };
    let absolute_end = Position {
        row: ts_node.end_position().row,
        column: ts_node.end_position().column,
        byte: ts_node.end_byte(),
    };

    // Find the '@' symbol child
    let quote_child = ts_node.child_by_field_name("quote")?;

    // Quoted expression starts AFTER the '@'
    let mut quoted_prev_end = Position {
        row: quote_child.end_position().row,
        column: quote_child.end_position().column,
        byte: quote_child.end_byte(),  // After '@'
    };

    // Convert quoted expression with adjusted prev_end
    let quoted_expr = convert_ts_node_to_ir(quoted_child, rope, quoted_prev_end)?;

    let base = create_simple_node_base(absolute_start, absolute_end, &mut prev_end);

    Ok(Arc::new(RholangNode::Quote {
        base,
        quoted: quoted_expr,
    }))
}
```

**Key Point**: The `prev_end` for the quoted expression is set to the position AFTER the `@` symbol, not at the quote node's start.

### Send Nodes (Channel Positioning)

**Location**: `src/parsers/rholang/conversion/send.rs`

Send nodes have multiple components with specific position ordering:

```rholang
channel!(arg1, arg2)
^      ^^          ^
│      │└─ inputs start here
│      └─ send type operator ('!' or '!!')
└─ channel starts here
```

**Component Order**:
1. Channel expression (starts at send node's start)
2. Send type operator (`!` or `!!`)
3. Input arguments (in parentheses)

**Implementation**:

```rust
pub fn convert_send(ts_node: TSNode, rope: &Rope, mut prev_end: Position) -> Result<Arc<RholangNode>> {
    let absolute_start = Position { /* from ts_node.start_position() */ };
    let absolute_end = Position { /* from ts_node.end_position() */ };

    // Channel comes first
    let channel_child = ts_node.child_by_field_name("channel")?;
    let channel = convert_ts_node_to_ir(channel_child, rope, prev_end)?;

    // Update prev_end to after channel
    prev_end = channel.base().syntactic_end();

    // Send type operator
    let send_type_child = ts_node.child_by_field_name("send_type")?;
    prev_end = Position {
        row: send_type_child.end_position().row,
        column: send_type_child.end_position().column,
        byte: send_type_child.end_byte(),
    };

    // Inputs
    let inputs_child = ts_node.child_by_field_name("inputs")?;
    let inputs = convert_inputs(inputs_child, rope, prev_end)?;

    let base = create_simple_node_base(absolute_start, absolute_end, &mut prev_end);

    Ok(Arc::new(RholangNode::Send {
        base,
        channel,
        send_type,
        inputs,
    }))
}
```

### Par Nodes (Binary vs N-ary)

**Location**: `src/ir/rholang_node.rs`

Par (parallel composition) nodes have two variants:

```rust
pub enum RholangNode {
    // Binary Par: exactly two processes
    Par {
        base: NodeBase,
        left: Arc<RholangNode>,
        right: Arc<RholangNode>,
    },

    // N-ary Par: list of processes
    ParVector {
        base: NodeBase,
        processes: Vec<Arc<RholangNode>>,
    },

    // ...
}
```

**Position Tracking**:
- Binary Par: Position spans from left's start to right's end
- N-ary Par: Position spans from first process's start to last process's end

**Implementation** (binary):

```rust
pub fn convert_par_binary(
    left_node: Arc<RholangNode>,
    right_node: Arc<RholangNode>,
) -> Arc<RholangNode> {
    let absolute_start = *left_node.base().start();
    let absolute_end = right_node.base().end();

    let mut prev_end = absolute_start;
    let base = create_simple_node_base(absolute_start, absolute_end, &mut prev_end);

    Arc::new(RholangNode::Par {
        base,
        left: left_node,
        right: right_node,
    })
}
```

### Virtual Documents (Embedded MeTTa)

**Location**: `src/language_regions/virtual_document.rs`

Virtual documents have **two coordinate systems**:

1. **Virtual Coordinates**: Position within extracted content (0-based)
2. **Parent Coordinates**: Position within parent .rho file

**Example**:

```rholang
// Parent file (example.rho)
new metta in {
  @"#!metta
  (= (fact 0) 1)
  (= (fact $n) (* $n (fact (- $n 1))))
  "!(metta)
}
```

Virtual document extraction:
```metta
(= (fact 0) 1)
(= (fact $n) (* $n (fact (- $n 1))))
```

**Position Mapping**:

```rust
impl VirtualDocument {
    // Virtual position → Parent position
    pub fn map_position_to_parent(&self, virtual_pos: LspPosition) -> LspPosition {
        LspPosition {
            line: self.parent_start.line + virtual_pos.line,
            character: if virtual_pos.line == 0 {
                // First line: add parent's starting column
                self.parent_start.character + virtual_pos.character
            } else {
                // Subsequent lines: use virtual column directly
                virtual_pos.character
            },
        }
    }

    // Check if parent position is within virtual document
    pub fn contains_parent_position(&self, parent_pos: &LspPosition) -> bool {
        parent_pos >= &self.parent_start && parent_pos <= &self.parent_end
    }

    // Parent position → Virtual position
    pub fn map_parent_to_virtual_position(&self, parent_pos: &LspPosition) -> Option<LspPosition> {
        if !self.contains_parent_position(parent_pos) {
            return None;
        }

        Some(LspPosition {
            line: parent_pos.line - self.parent_start.line,
            character: if parent_pos.line == self.parent_start.line {
                parent_pos.character - self.parent_start.character
            } else {
                parent_pos.character
            },
        })
    }
}
```

**Position Mapping Example**:

```
Parent file positions:
Line 0: new metta in {
Line 1:   @"#!metta
Line 2:   (= (fact 0) 1)          ← Virtual line 0
Line 3:   (= (fact $n) ...)       ← Virtual line 1
Line 4:   "!(metta)
Line 5: }

Virtual document:
Line 0: (= (fact 0) 1)
Line 1: (= (fact $n) ...)

Mapping:
  Virtual { line: 0, char: 3 } → Parent { line: 2, char: 6 }
                                           (line 2, after "  @\"#!metta\n  ")
  Virtual { line: 1, char: 0 } → Parent { line: 3, char: 3 }
                                           (line 3, after "  ")
```

---

## Complete Data Flow

### End-to-End Position Tracking

```
┌─────────────────────────────────────────────────────────────────────────┐
│ SOURCE CODE                                                              │
│ example.rho:                                                             │
│   Line 5, Column 10: contract process(@"init", ret) = { ... }          │
└───────────────────────────────┬─────────────────────────────────────────┘
                                │
                                ▼
┌─────────────────────────────────────────────────────────────────────────┐
│ PHASE 1: PARSING (Tree-Sitter)                                          │
│                                                                           │
│ Tree-Sitter Parser                                                        │
│   ├─ Reads source text                                                   │
│   ├─ Builds CST (Concrete Syntax Tree)                                  │
│   └─ Provides absolute positions for every node                          │
│                                                                           │
│ ts_node.start_position() → { row: 5, column: 10 }                       │
│ ts_node.start_byte()     → 142                                           │
│ ts_node.end_position()   → { row: 5, column: 50 }                       │
│ ts_node.end_byte()       → 182                                           │
└───────────────────────────────┬─────────────────────────────────────────┘
                                │
                                ▼
┌─────────────────────────────────────────────────────────────────────────┐
│ PHASE 2: IR CONVERSION                                                   │
│                                                                           │
│ convert_ts_node_to_ir()                                                  │
│   ├─ Extracts positions from Tree-Sitter node                           │
│   ├─ Creates Position struct                                             │
│   └─ Builds NodeBase with absolute positions                             │
│                                                                           │
│ Position {                                                               │
│   row: 5,         ← from ts_node.start_position().row                   │
│   column: 10,     ← from ts_node.start_position().column                │
│   byte: 142       ← from ts_node.start_byte()                           │
│ }                                                                         │
│                                                                           │
│ NodeBase {                                                               │
│   start: Position { row: 5, column: 10, byte: 142 },                   │
│   content_length: 38,                                                    │
│   syntactic_length: 40,                                                  │
│   span_lines: 0,                                                         │
│   span_columns: 40,                                                      │
│ }                                                                         │
│                                                                           │
│ RholangNode::Contract {                                                  │
│   base: NodeBase { ... },                                                │
│   name: "process",                                                       │
│   formals: [...],                                                        │
│   body: ...                                                              │
│ }                                                                         │
└───────────────────────────────┬─────────────────────────────────────────┘
                                │
                                ▼
┌─────────────────────────────────────────────────────────────────────────┐
│ PHASE 3: SYMBOL TABLE CONSTRUCTION                                       │
│                                                                           │
│ SymbolTableBuilder traverses IR                                          │
│   ├─ Visits Contract node                                                │
│   ├─ Extracts name: "process"                                            │
│   ├─ Extracts positions from base.start() and base.end()                │
│   └─ Stores in symbol table                                              │
│                                                                           │
│ SymbolInfo {                                                             │
│   name: "process",                                                       │
│   kind: SymbolKind::Contract,                                            │
│   definition_range: Range {                                              │
│     start: Position { row: 5, column: 10, byte: 142 },                 │
│     end: Position { row: 5, column: 50, byte: 182 }                    │
│   },                                                                      │
│   scope_id: 1,                                                           │
│   metadata: {...}                                                        │
│ }                                                                         │
│                                                                           │
│ GlobalSymbolIndex:                                                       │
│   symbols["process"] → [                                                 │
│     SymbolLocation {                                                     │
│       uri: file:///path/to/example.rho,                                  │
│       range: Range { start: {...}, end: {...} },                        │
│       kind: Contract                                                     │
│     }                                                                     │
│   ]                                                                       │
└───────────────────────────────┬─────────────────────────────────────────┘
                                │
                                ▼
┌─────────────────────────────────────────────────────────────────────────┐
│ PHASE 4: LSP OPERATION (e.g., goto-definition)                          │
│                                                                           │
│ Client Request:                                                          │
│   textDocument/definition                                                │
│   uri: file:///path/to/example.rho                                       │
│   position: { line: 10, character: 5 }  ← LSP coordinates               │
│                                                                           │
│ Step 1: LSP → IR Position Conversion                                    │
│   lsp_to_ir_position()                                                   │
│     Position { row: 10, column: 5, byte: 0 }                            │
│                                                                           │
│ Step 2: Compute Byte Offset                                             │
│   byte_offset_from_position(rope, 10, 5)                                │
│     → Handles UTF-16 (LSP) to UTF-8 (Rope) conversion                   │
│     → Returns byte offset: 245                                           │
│     Position { row: 10, column: 5, byte: 245 }                          │
│                                                                           │
│ Step 3: Find Node at Position                                           │
│   find_node_at_position(ir_root, position)                              │
│     ├─ Traverses IR tree                                                 │
│     ├─ Checks: target.byte ∈ [node.start.byte, node.end.byte]          │
│     ├─ Recursively checks children                                       │
│     └─ Returns deepest matching node                                     │
│     → RholangNode::Var { name: "process" }                              │
│                                                                           │
│ Step 4: Extract Symbol and Resolve                                      │
│   extract_symbol_name(node) → "process"                                 │
│   resolver.resolve_symbol("process", position, context)                 │
│     ├─ Pattern-aware resolver (checks contract patterns)                 │
│     ├─ Lexical scope resolver (fallback)                                 │
│     └─ Global symbol index                                               │
│     → Returns Vec<SymbolLocation> with absolute positions               │
│                                                                           │
│ Step 5: IR → LSP Position Conversion                                    │
│   for each SymbolLocation:                                               │
│     ir_to_lsp_position(location.range.start)                            │
│       Position { row: 5, column: 10, byte: 142 }                        │
│       → LspPosition { line: 5, character: 10 }                          │
│                                                                           │
│ Server Response:                                                         │
│   Location {                                                             │
│     uri: file:///path/to/example.rho,                                    │
│     range: {                                                             │
│       start: { line: 5, character: 10 },                                │
│       end: { line: 5, character: 50 }                                   │
│     }                                                                     │
│   }                                                                       │
└───────────────────────────────┬─────────────────────────────────────────┘
                                │
                                ▼
┌─────────────────────────────────────────────────────────────────────────┐
│ CLIENT RESULT                                                            │
│                                                                           │
│ Editor jumps cursor to:                                                  │
│   File: example.rho                                                      │
│   Line: 5 (0-based: line 6 in editor)                                   │
│   Column: 10                                                             │
│                                                                           │
│ Highlights: contract process(@"init", ret) = { ... }                    │
└─────────────────────────────────────────────────────────────────────────┘
```

### State Diagram: Position Transformations

```
                    ┌─────────────────────┐
                    │   Tree-Sitter       │
                    │   (row, col, byte)  │
                    │   Absolute          │
                    └──────────┬──────────┘
                               │
                               │ convert_ts_node_to_ir()
                               ▼
                    ┌─────────────────────┐
                    │   IR Position       │
                    │   (row, col, byte)  │
                    │   Absolute          │
                    └──────────┬──────────┘
                               │
                               │ stored in NodeBase
                               ▼
                    ┌─────────────────────┐
         ┌──────────│   NodeBase          │──────────┐
         │          │   start: Position   │          │
         │          │   lengths           │          │
         │          └─────────────────────┘          │
         │                                            │
         │ base.end()                                 │ stored in
         │ (semantic)                                 │ symbol table
         ▼                                            ▼
┌─────────────────────┐                   ┌─────────────────────┐
│   End Position      │                   │   SymbolLocation    │
│   (computed)        │                   │   (stored)          │
│   row = start.row + │                   │   range: Range      │
│       span_lines    │                   └──────────┬──────────┘
│   col = ...         │                              │
│   byte = start.byte │                              │
│        + content_len│                              │
└─────────────────────┘                              │
                                                      │
         ┌────────────────────────────────────────────┘
         │
         │ LSP operation (goto-def, references, etc.)
         ▼
┌─────────────────────┐
│   LSP Position      │
│   (line, character) │───┐
│   UTF-16 code units │   │ lsp_to_ir_position()
└─────────────────────┘   │ + byte_offset_from_position()
                          ▼
                   ┌─────────────────────┐
                   │   IR Position       │
                   │   (with byte)       │
                   └──────────┬──────────┘
                              │
                              │ find_node_at_position()
                              │ + symbol resolution
                              ▼
                   ┌─────────────────────┐
                   │   SymbolLocation    │
                   │   (IR positions)    │
                   └──────────┬──────────┘
                              │
                              │ ir_to_lsp_position()
                              ▼
                   ┌─────────────────────┐
                   │   LSP Location      │
                   │   (line, character) │
                   └─────────────────────┘
```

---

## Migration History

### From Relative to Absolute Position Tracking

The codebase recently underwent a significant migration from **relative (delta-based)** to **absolute position tracking** (completed by November 2025).

#### Old System: Relative Positions

**Structure** (deprecated):
```rust
pub struct RelativePosition {
    delta_lines: usize,    // Lines from prev_end
    delta_columns: usize,  // Columns from prev_end
    delta_bytes: usize,    // Bytes from prev_end
}
```

**Approach**:
- Stored position **deltas** relative to previous sibling's end
- Required **reconstruction** to compute absolute positions
- Formula: `absolute_start = prev_end + RelativePosition`

**Example**:
```rholang
x!(42) | y!(100)
^      ^ ^
│      │ └─ relative: { delta_lines: 0, delta_columns: 3, delta_bytes: 3 }
│      └─ prev_end after x!(42)
└─ absolute: { row: 0, column: 0, byte: 0 }
```

**Problems**:
1. **Cascading Errors**: Error in one node's position propagates to all siblings
2. **Complex Reconstruction**: Computing absolute positions required traversing from root
3. **Debugging Difficulty**: Hard to verify positions without full tree context
4. **Performance**: O(n) reconstruction for nth sibling

#### New System: Absolute Positions

**Structure** (current):
```rust
pub struct Position {
    row: usize,      // Absolute row
    column: usize,   // Absolute column
    byte: usize,     // Absolute byte offset
}

pub struct NodeBase {
    start: Position,              // Absolute start (directly from Tree-Sitter)
    content_length: usize,        // Length in bytes
    syntactic_length: usize,      // Full length including delimiters
    span_lines: usize,            // Lines spanned
    span_columns: usize,          // Columns on last line
}
```

**Approach**:
- Store **absolute positions** directly from Tree-Sitter
- Compute **end positions** on demand from start + lengths
- No reconstruction needed

**Benefits**:
1. **No Cascading Errors**: Each node's position is independent
2. **Simple Lookup**: O(1) access to any node's absolute position
3. **Easy Debugging**: Can verify any position without context
4. **Performance**: Constant-time position access

**Migration Evidence**:

1. **Function Names**: `create_node_base_from_absolute()` (src/parsers/position_utils.rs:118)
2. **Field Names**: `start: Position` (not `relative_start`)
3. **Comments**: "Positions are now stored as absolute" (src/ir/rholang_node/position_tracking.rs:1-10)
4. **Tree-Sitter Integration**: Direct use of `start_position()` and `start_byte()` without delta computation

#### Performance Improvements

**Before (Relative)**:
- Position reconstruction: O(n) for nth sibling
- Cascading updates: O(n²) for tree modifications
- Memory: Smaller (3 usizes per position)

**After (Absolute)**:
- Position access: O(1) for any node
- Tree modifications: O(1) position updates
- Memory: Same (3 usizes per position)
- Additional optimizations: Singleton metadata (80-90% allocation reduction)

**Net Result**: Simpler, faster, more reliable position tracking with minimal memory overhead.

---

## Code Reference Map

### Core Files and Functions

| File | Line Range | Purpose | Key Functions |
|------|-----------|---------|---------------|
| **Position Data Structures** ||||
| `src/ir/semantic_node.rs` | 32-36 | Position struct | `Position { row, column, byte }` |
| `src/ir/semantic_node.rs` | 58-89 | NodeBase struct | `start()`, `end()`, `syntactic_end()` |
| **Parsing & Conversion** ||||
| `src/parsers/rholang/conversion/mod.rs` | 102-150 | Tree-Sitter → IR | `convert_ts_node_to_ir()` |
| `src/parsers/position_utils.rs` | 118-161 | Create NodeBase (complex) | `create_node_base_from_absolute()` |
| `src/parsers/position_utils.rs` | 195-201 | Create NodeBase (simple) | `create_simple_node_base()` |
| `src/parsers/rholang/conversion/quote.rs` | 1-100 | Quote node conversion | `convert_quote()` |
| `src/parsers/rholang/conversion/send.rs` | 1-150 | Send node conversion | `convert_send()` |
| **Symbol Tables** ||||
| `src/ir/transforms/symbol_table_builder.rs` | 1-500 | Build symbol tables | `visit_contract()`, `visit_new()`, etc. |
| `src/ir/global_index.rs` | 1-300 | Global symbol index | `GlobalSymbolIndex`, `add_symbol()` |
| **LSP Integration** ||||
| `src/lsp/features/node_finder.rs` | 283-289 | LSP → IR conversion | `lsp_to_ir_position()` |
| `src/lsp/features/node_finder.rs` | 298-303 | IR → LSP conversion | `ir_to_lsp_position()` |
| `src/lsp/backend.rs` | 655-671 | Byte offset computation | `byte_offset_from_position()` |
| `src/lsp/features/node_finder.rs` | 11-151 | Node lookup | `find_node_at_position()` |
| `src/lsp/backend/unified_handlers.rs` | 1-1000 | LSP handlers | `goto_definition()`, `references()`, etc. |
| **Virtual Documents** ||||
| `src/language_regions/virtual_document.rs` | 1-400 | Virtual doc management | `map_position_to_parent()`, `contains_parent_position()` |
| `src/language_regions/directive_parser.rs` | 1-200 | Language detection | `parse_directive()` |
| `src/language_regions/region_extractor.rs` | 1-300 | Region extraction | `extract_regions()` |

### Quick Function Reference

#### Position Creation
```rust
// From Tree-Sitter node (absolute)
let pos = Position {
    row: ts_node.start_position().row,
    column: ts_node.start_position().column,
    byte: ts_node.start_byte(),
};
```

#### NodeBase Creation
```rust
// Complex node (with closing delimiter)
let base = create_node_base_from_absolute(
    absolute_start,
    absolute_end,
    content_end,       // End before closing delimiter
    &mut prev_end,
);

// Simple node (no closing delimiter)
let base = create_simple_node_base(
    absolute_start,
    absolute_end,
    &mut prev_end,
);
```

#### Position Access
```rust
// Get start position
let start = node.base().start();  // &Position

// Get end position (semantic)
let end = node.base().end();  // Position (computed)

// Get syntactic end (includes delimiters)
let syntactic_end = node.base().syntactic_end();  // Position (computed)
```

#### Position Conversion
```rust
// LSP → IR
let ir_pos = lsp_to_ir_position(lsp_position);

// Compute byte offset (UTF-16 → UTF-8)
let byte = byte_offset_from_position(&rope, ir_pos.row, ir_pos.column)?;
let full_pos = Position { row: ir_pos.row, column: ir_pos.column, byte };

// IR → LSP
let lsp_pos = ir_to_lsp_position(&ir_position);
```

#### Node Lookup
```rust
// Find node at position
let node = find_node_at_position(&ir_root, &position)?;

// Extract symbol name
let symbol_name = extract_symbol_name(node)?;

// Resolve symbol
let locations = resolver.resolve_symbol(&symbol_name, &position, &context);
```

---

## Theory & Invariants

### Position Ordering Guarantees

The position tracking system maintains several critical invariants:

#### Invariant 1: Position Ordering

For any node in the IR tree:

```
node.base().start() ≤ node.base().end() ≤ node.base().syntactic_end()
```

**Proof**:
- `start` is absolute position from Tree-Sitter
- `end()` = `start + content_length`
- `syntactic_end()` = `start + syntactic_length`
- By construction: `content_length ≤ syntactic_length`
- Therefore: `start ≤ end ≤ syntactic_end`

#### Invariant 2: Parent-Child Containment

For any parent-child relationship:

```
parent.start ≤ child.start < child.end ≤ parent.end
```

**Proof**:
- Tree-Sitter guarantees CST nodes form a proper tree
- Children are converted with `prev_end` tracking
- Parent's `end()` is computed from last child's end
- Therefore, all children are contained within parent's range

#### Invariant 3: Sibling Ordering

For adjacent siblings `A` and `B` (where B follows A):

```
A.start < A.syntactic_end ≤ B.start
```

**Proof**:
- `prev_end` is updated to `A.syntactic_end()` after converting A
- B is converted with `prev_end` from A
- B's `absolute_start` comes from Tree-Sitter (independent of A)
- Tree-Sitter guarantees sibling ordering
- Therefore: `A.syntactic_end ≤ B.start`

**Note**: Equality holds when there's no whitespace between siblings.

#### Invariant 4: Position Comparison

Position comparison uses lexicographic ordering:

```rust
impl PartialOrd for Position {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match self.row.cmp(&other.row) {
            Ordering::Equal => self.column.cmp(&other.column),
            other => Some(other),
        }
    }
}
```

**Properties**:
- `byte` is **not** used for comparison (treated as metadata)
- Ordering is based on `(row, column)` only
- This allows position comparison without byte offset computation

**Rationale**: Byte offsets may not always be computed (e.g., when creating positions from LSP requests). The `(row, column)` pair uniquely identifies a position in the text.

### Tree Traversal Invariants

#### Depth-First Position Ordering

For a depth-first traversal of the IR tree, positions are encountered in ascending order:

```
∀ nodes A, B where A is visited before B in DFS:
  A.start ≤ B.start
```

**Proof by induction**:

**Base case**: Root node is visited first, and all other nodes are descendants (contained within root's range).

**Inductive step**: Assume true for node N and all its descendants. Consider N's next sibling S:
- N is fully processed (all descendants visited)
- By Invariant 3: `N.syntactic_end ≤ S.start`
- S and its descendants are visited after N
- Therefore, all positions in S's subtree ≥ S.start ≥ N.syntactic_end ≥ N.start

#### Find Node at Position Correctness

The `find_node_at_position()` function returns the **deepest** (most specific) node containing the target position.

**Algorithm**:
1. Check if target is within current node's range
2. If yes, recursively check children
3. If any child contains target, recurse into that child
4. If no child contains target, return current node

**Correctness**:

**Lemma 1**: If a node N contains position P, then at most one child of N can contain P.

**Proof**: By Invariant 3 (sibling ordering), siblings have non-overlapping ranges. If child A contains P, then `A.start ≤ P < A.end ≤ next_sibling.start`, so no sibling can contain P. □

**Theorem**: `find_node_at_position(root, P)` returns the deepest node containing P.

**Proof by induction on tree depth**:

**Base case** (depth 0): If P is in root and root is a leaf, return root. Correct by definition.

**Inductive step**: Assume true for all nodes at depth ≤ k. Consider node N at depth k:
- If P is not in N's range, return None (correct)
- If P is in N's range, check children (depth k+1)
- By induction hypothesis, child search returns deepest node among children
- If child search succeeds, return that result (correct by IH)
- If child search fails, N is the deepest (correct)

Therefore, the algorithm returns the unique deepest node containing P. □

**Complexity**: O(log n) average case for balanced trees, O(n) worst case for skewed trees.

### Byte Offset Computation Correctness

The `byte_offset_from_position(rope, line, character)` function converts `(line, character)` to byte offset.

**Algorithm**:
1. Convert line number to byte offset of line start: `line_start_byte = rope.line_to_byte(line)`
2. Get line text: `line_text = rope.line(line)`
3. Convert character offset (UTF-16) to byte offset (UTF-8): `byte_in_line = line_text.char_to_byte(character)`
4. Return `line_start_byte + byte_in_line`

**Correctness**:

**Lemma 2**: `rope.line_to_byte(line)` returns the byte offset of the first character of `line`.

**Proof**: By `ropey` crate specification. Rope maintains line break indices and provides O(log n) line-to-byte conversion. □

**Lemma 3**: `line_text.char_to_byte(character)` converts UTF-16 code unit offset to UTF-8 byte offset within the line.

**Proof**: `ropey` uses `RopeSlice::char_to_byte()`, which handles:
- ASCII characters: 1 byte = 1 char
- Multi-byte UTF-8 sequences: N bytes per char (where N = UTF-8 encoding length)
- Surrogate pairs (UTF-16): 2 code units → corresponding UTF-8 bytes

The function correctly accumulates byte offsets accounting for UTF-8 encoding. □

**Theorem**: `byte_offset_from_position(rope, line, character)` returns the correct absolute byte offset.

**Proof**: By Lemmas 2 and 3:
- `line_start_byte` = offset to start of line (correct)
- `byte_in_line` = offset within line (correct)
- Sum = absolute byte offset (correct by addition)

□

**Edge Cases**:
- **Character beyond line end**: `character.min(line_text.len_chars())` clamps to line length
- **Empty lines**: `byte_in_line = 0` (correct)
- **Multi-byte characters**: UTF-8 encoding handled by `ropey`

### Position Mapping for Virtual Documents

Virtual documents maintain bidirectional position mappings between virtual and parent coordinate systems.

**Invariant 5**: Virtual Document Containment

For a virtual document V with parent P:

```
V.parent_start ≤ any position in V ≤ V.parent_end
```

**Mapping Functions**:

```rust
// Virtual → Parent
parent_pos = {
    line: parent_start.line + virtual_pos.line,
    character: if virtual_pos.line == 0 {
        parent_start.character + virtual_pos.character
    } else {
        virtual_pos.character
    }
}

// Parent → Virtual (if parent_pos in range)
virtual_pos = {
    line: parent_pos.line - parent_start.line,
    character: if parent_pos.line == parent_start.line {
        parent_pos.character - parent_start.character
    } else {
        parent_pos.character
    }
}
```

**Correctness**:

**Theorem**: Virtual → Parent → Virtual mapping is an identity function for positions within the virtual document.

**Proof**: Let V be virtual position.

Case 1: V.line = 0 (first line of virtual doc)
```
P = map_to_parent(V)
  = { line: parent_start.line + 0,
      character: parent_start.character + V.character }

V' = map_to_virtual(P)
   = { line: P.line - parent_start.line
             = parent_start.line - parent_start.line = 0,
       character: P.character - parent_start.character
                = (parent_start.character + V.character) - parent_start.character
                = V.character }
   = V  ✓
```

Case 2: V.line > 0 (subsequent lines)
```
P = map_to_parent(V)
  = { line: parent_start.line + V.line,
      character: V.character }

V' = map_to_virtual(P)
   = { line: P.line - parent_start.line
             = (parent_start.line + V.line) - parent_start.line
             = V.line,
       character: P.character = V.character }
   = V  ✓
```

Therefore, the mapping is consistent. □

### Performance Characteristics

| Operation | Complexity | Notes |
|-----------|-----------|-------|
| **Position Access** |||
| `node.base().start()` | O(1) | Direct field access |
| `node.base().end()` | O(1) | Computed from start + length |
| **Tree Traversal** |||
| Find node at position | O(log n) avg, O(n) worst | Depends on tree balance |
| Visit all nodes | O(n) | Depth-first traversal |
| **Symbol Resolution** |||
| Local scope lookup | O(k) | k = scope chain depth |
| Global symbol lookup | O(1) avg | HashMap lookup |
| Pattern-based lookup | O(log m) | m = trie depth (PathMap) |
| **Position Conversion** |||
| LSP → IR position | O(1) | Simple field mapping |
| Byte offset computation | O(log n) | Rope line-to-byte conversion |
| IR → LSP position | O(1) | Simple field mapping |
| **Virtual Document** |||
| Virtual ↔ Parent mapping | O(1) | Arithmetic computation |

---

## Summary

The Rholang Language Server implements a robust, performant position tracking system with the following key characteristics:

1. **Absolute Position Storage**: Positions are stored directly from Tree-Sitter, eliminating delta computation errors and simplifying the implementation.

2. **Dual-Length Tracking**: Nodes track both semantic content length and syntactic length (including delimiters) for accurate position reconstruction.

3. **UTF-8/UTF-16 Conversion**: Proper handling of character encoding differences between internal representation (UTF-8 via Rope) and LSP protocol (UTF-16).

4. **Comprehensive Position Mapping**: Support for virtual documents with bidirectional position mapping between virtual and parent coordinate systems.

5. **Performance Optimization**: Cached Tree-Sitter calls, singleton metadata, and efficient rope-based operations ensure minimal overhead.

6. **Correctness Guarantees**: Formal invariants ensure position ordering, parent-child containment, and correct node lookup.

This architecture enables precise source code navigation, symbol resolution, and LSP features across both Rholang and embedded languages like MeTTa.

---

## References

### Related Documentation

- [Pattern Matching Enhancement](../pattern_matching_enhancement.md) - Contract pattern matching for goto-definition
- [Virtual Documents](../virtual_documents.md) - Embedded language support (if exists)

### Source Code

- **IR Core**: `src/ir/semantic_node.rs`, `src/ir/rholang_node.rs`
- **Parsing**: `src/parsers/rholang/conversion/`
- **LSP Integration**: `src/lsp/features/node_finder.rs`, `src/lsp/backend/unified_handlers.rs`
- **Symbol Resolution**: `src/ir/symbol_resolution/`
- **Virtual Documents**: `src/language_regions/`

### External Dependencies

- **tree-sitter**: CST parsing with absolute position information
- **ropey**: Efficient text rope with UTF-8/UTF-16 conversion
- **tower-lsp**: LSP protocol implementation
- **rpds** + **archery**: Persistent data structures for immutable IR
