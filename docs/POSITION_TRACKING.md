# Position Tracking System

This document describes the position tracking system used in the Rholang language server's Intermediate Representation (IR). The system uses delta encoding to efficiently store position information and reconstruct absolute positions on demand.

## Table of Contents

1. [Overview](#overview)
2. [Core Data Structures](#core-data-structures)
3. [Absolute to Relative Transformation (IR Conversion)](#absolute-to-relative-transformation-ir-conversion)
4. [Relative to Absolute Reconstruction (LSP Operations)](#relative-to-absolute-reconstruction-lsp-operations)
5. [Position Tracking Flow](#position-tracking-flow)
6. [Dual-Length System](#dual-length-system)
7. [Special Cases](#special-cases)
8. [Symbol Lookup by Position](#symbol-lookup-by-position)
9. [Debugging Position Issues](#debugging-position-issues)

## Overview

The position tracking system serves two main purposes:

1. **Memory efficiency**: Store positions as relative offsets instead of absolute coordinates, reducing memory usage through structural sharing
2. **On-demand computation**: Reconstruct absolute positions only when needed for LSP operations (goto-definition, hover, etc.)

### High-Level Flow

```
┌─────────────────┐         ┌──────────────────┐         ┌─────────────────┐
│  Tree-Sitter    │         │   IR Conversion  │         │   IR Storage    │
│  (Absolute      │─────────▶   (Transform)    │─────────▶  (Relative      │
│   Positions)    │         │                  │         │   Positions)    │
└─────────────────┘         └──────────────────┘         └─────────────────┘
                                                                    │
                                                                    │
                                                                    ▼
┌─────────────────┐         ┌──────────────────┐         ┌─────────────────┐
│   LSP Client    │◀────────│  Reconstruction  │◀────────│   LSP Request   │
│  (Receives      │         │  (On-demand)     │         │  (Triggers)     │
│   Absolute)     │         │                  │         │                 │
└─────────────────┘         └──────────────────┘         └─────────────────┘
```

## Core Data Structures

### Position (Absolute)

Represents an absolute position in source code. All coordinates are zero-based.

```rust
pub struct Position {
    pub row: usize,    // Line number (0-based)
    pub column: usize, // Column number (0-based)
    pub byte: usize,   // Byte offset from start of file
}
```

**Example**:
```rholang
// Byte:   0       5         13
// Row:    0       0          1
// Column: 0       5          0
new x in {
  x!(42)
}
```

- `new` starts at: `Position { row: 0, column: 0, byte: 0 }`
- `x` (first) starts at: `Position { row: 0, column: 4, byte: 4 }`
- `x` (second) starts at: `Position { row: 1, column: 2, byte: 13 }`

### RelativePosition (Delta)

Represents position relative to the previous node's end position.

```rust
pub struct RelativePosition {
    pub delta_lines: i32,    // Difference in line numbers
    pub delta_columns: i32,  // Difference in columns (or start column if new line)
    pub delta_bytes: usize,  // Difference in byte offsets
}
```

**Key insight**: If `delta_lines != 0`, then `delta_columns` is the absolute column on the new line, not a delta.

### NodeBase

Contains all position and span information for an IR node.

```rust
pub struct NodeBase {
    relative_start: RelativePosition,  // Position relative to previous node's end
    content_length: usize,             // "Soft" length: up to last child (semantics)
    syntactic_length: usize,           // "Hard" length: includes closing delimiters (reconstruction)
    span_lines: usize,                 // Number of lines spanned by the node
    span_columns: usize,               // Columns on the last line
}
```

## Absolute to Relative Transformation (IR Conversion)

During IR conversion, Tree-Sitter provides absolute positions for every node. We transform these to relative positions for storage.

### Conversion Algorithm

**Location**: `src/parsers/rholang/conversion/mod.rs` (lines 66-123)

```rust
pub(crate) fn convert_ts_node_to_ir(
    ts_node: TSNode,
    rope: &Rope,
    prev_end: Position
) -> (Arc<RholangNode>, Position)
```

**Steps**:

1. **Extract Tree-Sitter absolute positions**:
   ```rust
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
   ```

2. **Compute relative deltas from `prev_end`**:
   ```rust
   let delta_lines = absolute_start.row as i32 - prev_end.row as i32;
   let delta_columns = if delta_lines == 0 {
       absolute_start.column as i32 - prev_end.column as i32
   } else {
       absolute_start.column as i32  // Absolute column on new line
   };
   let delta_bytes = absolute_start.byte - prev_end.byte;
   ```

3. **Create NodeBase with relative position**:
   ```rust
   let relative_start = RelativePosition {
       delta_lines,
       delta_columns,
       delta_bytes,
   };
   let length = absolute_end.byte - absolute_start.byte;
   let span_lines = absolute_end.row - absolute_start.row;
   let span_columns = if span_lines == 0 {
       absolute_end.column - absolute_start.column
   } else {
       absolute_end.column
   };
   let base = NodeBase::new_simple(relative_start, length, span_lines, span_columns);
   ```

4. **Return node and absolute_end** (critical for next sibling):
   ```rust
   (arc_node, absolute_end)  // Next sibling uses absolute_end as its prev_end
   ```

### Visual Example: Converting Siblings

Consider this Rholang code:
```rholang
new x, y in { x!(1) | y!(2) }
```

Tree-Sitter reports:
```
NameDecl "x": start_byte=4,  end_byte=5   (len=1)
NameDecl "y": start_byte=7,  end_byte=8   (len=1)
Block:        start_byte=12, end_byte=27  (len=15)
```

**Conversion flow**:

```
Step 1: Convert first NameDecl
  Tree-Sitter: absolute_start=4, absolute_end=5
  prev_end: {byte: 3}  (end of "new")
  ┌─────────────────────────────────────────┐
  │ delta_bytes = 4 - 3 = 1                 │
  │ Creates: RelativePosition { ..., delta_bytes: 1 } │
  │ Returns: (NameDecl_node, Position { byte: 5 })    │
  └─────────────────────────────────────────┘

Step 2: Convert second NameDecl (uses first's end as prev_end)
  Tree-Sitter: absolute_start=7, absolute_end=8
  prev_end: {byte: 5}  (end of first NameDecl)
  ┌─────────────────────────────────────────┐
  │ delta_bytes = 7 - 5 = 2                 │
  │ Creates: RelativePosition { ..., delta_bytes: 2 } │
  │ Returns: (NameDecl_node, Position { byte: 8 })    │
  └─────────────────────────────────────────┘

Step 3: Convert Block (uses second NameDecl's end as prev_end)
  Tree-Sitter: absolute_start=12, absolute_end=27
  prev_end: {byte: 8}  (end of second NameDecl)
  ┌─────────────────────────────────────────┐
  │ delta_bytes = 12 - 8 = 4                │
  │ Creates: RelativePosition { ..., delta_bytes: 4 } │
  │ Returns: (Block_node, Position { byte: 27 })      │
  └─────────────────────────────────────────┘
```

**Critical invariant**: Each node must return Tree-Sitter's `absolute_end`, NOT the last child's end. This ensures the next sibling receives the correct `prev_end`.

### Helper Function: create_correct_node_base

**Location**: `src/parsers/rholang/conversion/mod.rs` (lines 26-63)

For nodes with closing delimiters (Block, List, etc.), use this helper to create a NodeBase with dual lengths:

```rust
fn create_correct_node_base(
    absolute_start: Position,
    content_end: Position,      // After last child
    syntactic_end: Position,    // Includes closing delimiter
    prev_end: Position
) -> NodeBase
```

**Example**: Block with closing `}`
```rholang
{ x!(1) }
^       ^
│       └─ syntactic_end (byte: 9)
└─ absolute_start (byte: 0)
      ^
      └─ content_end (byte: 7, after x!(1))
```

Creates NodeBase:
- `content_length = 7 - 0 = 7` (for semantic operations)
- `syntactic_length = 9 - 0 = 9` (for reconstruction)

## Relative to Absolute Reconstruction (LSP Operations)

When an LSP operation needs to find a node at a specific position, we reconstruct absolute positions from relative deltas on demand.

### Reconstruction Algorithm

**Location**: `src/ir/rholang_node/position_tracking.rs` (lines 31-403)

```rust
fn compute_positions_helper(
    node: &Arc<RholangNode>,
    prev_end: Position,
    positions: &mut HashMap<usize, (Position, Position)>,
) -> Position
```

**Steps**:

1. **Reconstruct absolute start from relative delta**:
   ```rust
   let base = node.base();
   let relative_start = base.relative_start();

   let start = Position {
       row: (prev_end.row as i32 + relative_start.delta_lines) as usize,
       column: if relative_start.delta_lines == 0 {
           (prev_end.column as i32 + relative_start.delta_columns) as usize
       } else {
           relative_start.delta_columns as usize  // Absolute on new line
       },
       byte: prev_end.byte + relative_start.delta_bytes,
   };
   ```

2. **Compute absolute end using syntactic_length**:
   ```rust
   let end = compute_end_position(
       start,
       base.span_lines(),
       base.span_columns(),
       base.syntactic_length()  // Must use syntactic_length!
   );
   ```

3. **Process children with cascading prev_end**:
   ```rust
   let mut current_prev = start;

   // For Par with left/right:
   current_prev = compute_positions_helper(left, current_prev, positions);
   current_prev = compute_positions_helper(right, current_prev, positions);

   // For Send with channel and inputs:
   let channel_end = compute_positions_helper(channel, start, positions);
   // ... process inputs starting from channel_end
   ```

4. **Cache and return end position**:
   ```rust
   positions.insert(key, (start, end));
   end  // Next sibling uses this as prev_end
   ```

### Visual Example: Reconstructing Siblings

Using the same example from conversion:
```rholang
new x, y in { x!(1) | y!(2) }
```

Stored in IR:
```
NameDecl "x": RelativePosition { delta_bytes: 1 }, syntactic_length: 1
NameDecl "y": RelativePosition { delta_bytes: 2 }, syntactic_length: 1
Block:        RelativePosition { delta_bytes: 4 }, syntactic_length: 15
```

**Reconstruction flow** (starting with `prev_end = {byte: 3}` after "new"):

```
Step 1: Reconstruct first NameDecl
  Stored: delta_bytes=1, syntactic_length=1
  Input prev_end: {byte: 3}
  ┌─────────────────────────────────────────┐
  │ start.byte = 3 + 1 = 4                  │
  │ end.byte = 4 + 1 = 5                    │
  │ Returns: Position { byte: 5 }           │
  └─────────────────────────────────────────┘

Step 2: Reconstruct second NameDecl (uses first's end)
  Stored: delta_bytes=2, syntactic_length=1
  Input prev_end: {byte: 5}  (from step 1)
  ┌─────────────────────────────────────────┐
  │ start.byte = 5 + 2 = 7                  │
  │ end.byte = 7 + 1 = 8                    │
  │ Returns: Position { byte: 8 }           │
  └─────────────────────────────────────────┘

Step 3: Reconstruct Block (uses second NameDecl's end)
  Stored: delta_bytes=4, syntactic_length=15
  Input prev_end: {byte: 8}  (from step 2)
  ┌─────────────────────────────────────────┐
  │ start.byte = 8 + 4 = 12                 │
  │ end.byte = 12 + 15 = 27                 │
  │ Returns: Position { byte: 27 }          │
  └─────────────────────────────────────────┘
```

**Result**: All absolute positions reconstructed correctly!

### The Cascading Effect

**Critical concept**: Position reconstruction is a chain operation. If ANY node computes an incorrect end position, ALL subsequent siblings will have wrong start positions.

```
Node A: prev_end=0  →  start=5,  end=10  ✓ Correct
                    ↓
Node B: prev_end=10 →  start=15, end=20  ✓ Correct
                    ↓
Node C: prev_end=20 →  start=25, end=30  ✓ Correct

If Node B's end is wrong:
Node A: prev_end=0  →  start=5,  end=10  ✓ Correct
                    ↓
Node B: prev_end=10 →  start=15, end=22  ✗ Wrong! (should be 20)
                    ↓
Node C: prev_end=22 →  start=27, end=32  ✗ Wrong! (should be 25-30)
```

## Position Tracking Flow

### Complete Flow Diagram

```
┌───────────────────────────────────────────────────────────────────────────┐
│                         PARSING PHASE (One-time)                          │
└───────────────────────────────────────────────────────────────────────────┘

    Source Code:  "new x in { x!(42) }"
         ↓
    ┌──────────────┐
    │ Tree-Sitter  │ Reports absolute positions for every node
    │   Parser     │ x: start_byte=4, end_byte=5
    └──────────────┘ Block: start_byte=9, end_byte=17
         ↓
    ┌──────────────────────────────────────────────┐
    │  convert_ts_node_to_ir()                     │
    │  - Receives: TSNode, prev_end                │
    │  - Computes: deltas from prev_end            │
    │  - Creates: NodeBase with RelativePosition   │
    │  - Returns: (IR_node, absolute_end)          │
    └──────────────────────────────────────────────┘
         ↓
    ┌──────────────┐
    │  IR Storage  │ Stores only relative positions:
    │              │ - RelativePosition { delta_bytes, ... }
    │              │ - syntactic_length
    └──────────────┘ - span_lines, span_columns
         ↓
    (IR is cached in memory, ready for LSP requests)


┌───────────────────────────────────────────────────────────────────────────┐
│                    LSP REQUEST PHASE (On-demand)                          │
└───────────────────────────────────────────────────────────────────────────┘

    LSP Request: "goto-definition at byte 15"
         ↓
    ┌──────────────────────────────────────────────┐
    │  compute_absolute_positions(root)            │
    │  - Walks entire IR tree                      │
    │  - Starts with prev_end = {0, 0, 0}          │
    │  - For each node:                            │
    │    1. start = prev_end + delta               │
    │    2. end = start + syntactic_length         │
    │    3. Recurse to children with start         │
    │    4. Pass end to next sibling as prev_end   │
    └──────────────────────────────────────────────┘
         ↓
    ┌──────────────┐
    │  Position    │ HashMap: node_ptr → (start, end)
    │   Cache      │ Cached for duration of request
    └──────────────┘
         ↓
    ┌──────────────────────────────────────────────┐
    │  find_node_at_position(byte: 15)             │
    │  - Traverses IR using cached positions       │
    │  - Returns deepest node containing byte 15   │
    └──────────────────────────────────────────────┘
         ↓
    LSP Response: Definition at line X, column Y
```

## Dual-Length System

The IR uses two different length measurements for different purposes:

### content_length (Soft Length)

- **Purpose**: Semantic operations (understanding node structure)
- **Definition**: Byte extent from node start to the end of the last child
- **Excludes**: Closing delimiters like `}`, `)`, `]`

### syntactic_length (Hard Length)

- **Purpose**: Position reconstruction (computing next sibling's start)
- **Definition**: Full syntactic extent including closing delimiters
- **Includes**: Everything Tree-Sitter reports in `start_byte..end_byte`

### Why Both?

**Problem**: Container nodes (Block, List, etc.) have content followed by closing delimiters:

```rholang
{ x!(42) }
  └─┬──┘ │
    │    └── closing }
    └──── content (child processes)
```

- For semantic analysis, we care about content: "what processes are in this block?"
- For position tracking, we need syntactic extent: "where does this block end so the next node knows where it starts?"

### Visual Example

```rholang
// Byte:  0   4     9       16 17
new x in { x!(42) }
         ^         ^^
         │         │└── syntactic_end (byte 17)
         │         └─── content_end (byte 16, after child processes)
         └─────────────── absolute_start (byte 9)
```

**NodeBase for Block**:
```rust
NodeBase {
    relative_start: RelativePosition { delta_bytes: 4, ... },
    content_length: 16 - 9 = 7,      // Up to end of x!(42)
    syntactic_length: 17 - 9 = 8,    // Includes closing }
    span_lines: 0,
    span_columns: 8,
}
```

**During reconstruction**:
```rust
// Compute Block's end for passing to next sibling
let end = start + syntactic_length;  // Must use syntactic_length!
// end.byte = 9 + 8 = 17 (correct!)

// If we used content_length:
// end.byte = 9 + 7 = 16 (WRONG! Missing the closing })
// Next sibling would start 1 byte too early
```

### Implementation

**Location**: `src/ir/semantic_node.rs` (lines 38-127)

```rust
impl NodeBase {
    /// Full constructor with dual lengths
    pub fn new(
        relative_start: RelativePosition,
        content_length: usize,      // Soft
        syntactic_length: usize,    // Hard
        span_lines: usize,
        span_columns: usize,
    ) -> Self { ... }

    /// Convenience for nodes without closing delimiters
    pub fn new_simple(
        relative_start: RelativePosition,
        length: usize,  // Sets both lengths to same value
        span_lines: usize,
        span_columns: usize,
    ) -> Self {
        NodeBase::new(
            relative_start,
            length,        // content_length
            length,        // syntactic_length (same!)
            span_lines,
            span_columns,
        )
    }
}
```

**When to use each**:

| Constructor | Use Case | Examples |
|------------|----------|----------|
| `new_simple()` | Nodes without closing delimiters | Var, Literal, Send, New |
| `new()` with dual lengths | Nodes with closing delimiters | Block, Parenthesized, List, Set, Map |

## Special Cases

### 1. Quote Nodes (@-prefix)

Quotes have a special handling because the `@` symbol precedes the quoted expression:

```rholang
@P
^└── quoted expression starts here
└─── @ symbol (1 byte)
```

**During conversion**:
- Quote node starts at `@` (byte 0)
- Quoted expression's `prev_end` is AFTER the `@` (byte 1)

**During reconstruction** (`position_tracking.rs`, lines 254-262):
```rust
RholangNode::Quote { quotable, .. } => {
    // Adjust prev_end to after @ symbol
    let after_at = Position {
        row: start.row,
        column: start.column + 1,
        byte: start.byte + 1,
    };
    current_prev = compute_positions_helper(quotable, after_at, positions);
}
```

### 2. Send Nodes (Channel First)

Send nodes have a special structure: channel comes first, then send type operator, then inputs:

```rholang
ch!(arg1, arg2)
^^  ^^ ────────
││  │└── inputs
││  └─── send type (!)
│└────── channel expression
└─────── Send node start
```

**During reconstruction** (`position_tracking.rs`, lines 118-153):
```rust
RholangNode::Send { channel, inputs, send_type_delta, .. } => {
    // Channel starts at Send node's start (NOT current_prev)
    let channel_end = compute_positions_helper(channel, start, positions);

    // Send type position is relative to channel's end
    let send_type_end = channel_end + send_type_delta;

    // Inputs start after send type
    let mut temp_prev = send_type_end;
    for input in inputs {
        temp_prev = compute_positions_helper(input, temp_prev, positions);
    }
    current_prev = temp_prev;
}
```

### 3. Par Nodes (Parallel Composition)

Par nodes can be binary (left/right) or n-ary (processes vector):

```rust
// Binary Par
RholangNode::Par { left: Some(left), right: Some(right), .. } => {
    current_prev = compute_positions_helper(left, current_prev, positions);
    current_prev = compute_positions_helper(right, current_prev, positions);
}

// N-ary Par (more efficient for many processes)
RholangNode::Par { processes: Some(procs), .. } => {
    for proc in procs.iter() {
        current_prev = compute_positions_helper(proc, current_prev, positions);
    }
}
```

### 4. Comments (Filtered Out)

Comments are named nodes in Tree-Sitter but are NOT included in the IR:

**Location**: `src/parsers/rholang/helpers.rs` (lines 242-258)

```rust
pub(crate) fn is_comment(kind_id: u16) -> bool {
    // O(1) check using cached kind IDs
    kind_id == LINE_COMMENT_KIND || kind_id == BLOCK_COMMENT_KIND
}
```

**During conversion**:
```rust
for child in node.named_children(&mut cursor) {
    if is_comment(child.kind_id()) {
        continue;  // Skip comments entirely
    }
    // ... convert child
}
```

This means comments don't affect position tracking - they're simply ignored.

## Symbol Lookup by Position

Once positions have been reconstructed, the language server uses them to look up symbols at specific locations for LSP operations like goto-definition, hover, and references.

### Overall Lookup Flow

```
┌─────────────────────────────────────────────────────────────────────────┐
│                      LSP REQUEST: goto-definition                       │
│                    User clicks at line 5, column 10                     │
└─────────────────────────────────────────────────────────────────────────┘
                                    ↓
┌─────────────────────────────────────────────────────────────────────────┐
│  STEP 1: Convert LSP Position → IR Position                            │
│  - LSP Position: { line: 5, character: 10 }                            │
│  - Compute byte offset from line/column using Rope                     │
│  - IR Position: { row: 5, column: 10, byte: 142 }                      │
└─────────────────────────────────────────────────────────────────────────┘
                                    ↓
┌─────────────────────────────────────────────────────────────────────────┐
│  STEP 2: Reconstruct Absolute Positions for IR Tree                    │
│  - Call compute_absolute_positions(root)                               │
│  - Returns HashMap: node_ptr → (start_pos, end_pos)                    │
│  - Cached for duration of request                                      │
└─────────────────────────────────────────────────────────────────────────┘
                                    ↓
┌─────────────────────────────────────────────────────────────────────────┐
│  STEP 3: Find Node at Position                                         │
│  - Call find_node_at_position_with_path(root, positions, byte 142)     │
│  - Traverses IR tree, checking: start.byte ≤ 142 ≤ end.byte            │
│  - Returns: (deepest_matching_node, ancestor_path)                     │
│  - Example: (Var("x"), [Source, New, Block, Send, Var])                │
└─────────────────────────────────────────────────────────────────────────┘
                                    ↓
┌─────────────────────────────────────────────────────────────────────────┐
│  STEP 4: Identify Symbol Type from Node                                │
│  - Match on node type: Var, Contract, Send, Quote, etc.                │
│  - Use ancestor path for context (is Var a contract name?)             │
│  - Extract symbol name                                                  │
└─────────────────────────────────────────────────────────────────────────┘
                                    ↓
┌─────────────────────────────────────────────────────────────────────────┐
│  STEP 5: Lookup Symbol in Symbol Tables                                │
│  - For local variables: symbol_table.lookup(name)                      │
│  - For contracts: workspace.global_symbols.get(name)                   │
│  - For contract calls: GlobalSymbolIndex.find_contract_definition()    │
│  - Returns: Symbol { name, type, declaration_location, ... }           │
└─────────────────────────────────────────────────────────────────────────┘
                                    ↓
┌─────────────────────────────────────────────────────────────────────────┐
│  STEP 6: Convert IR Position → LSP Location                            │
│  - symbol.declaration_location → LSP Range                             │
│  - Return: Location { uri, range }                                     │
└─────────────────────────────────────────────────────────────────────────┘
```

### Step-by-Step Walkthrough

#### Step 1: LSP Position to Byte Offset

**Location**: `src/lsp/backend/handlers.rs` (lines 443-449)

The LSP client sends line/column coordinates (0-based). We need to convert to byte offset for IR lookup:

```rust
// Convert LSP position to byte offset
let byte_offset = Self::byte_offset_from_position(
    text,                           // Rope (efficient line/column → byte)
    lsp_pos.line as usize,          // Line number (0-based)
    lsp_pos.character as usize      // Column number (0-based, UTF-16 code units)
);

// Create IR Position with all three coordinates
let ir_pos = IrPosition {
    row: lsp_pos.line as usize,
    column: lsp_pos.character as usize,
    byte: byte_offset,
};
```

**Important**: LSP uses UTF-16 code units for columns, but Ropes use UTF-8 bytes. The `byte_offset_from_position` function handles this conversion.

#### Step 2: Position Reconstruction

**Location**: `src/ir/rholang_node/position_tracking.rs` (lines 9-403)

Reconstruct absolute positions for all nodes from stored relative positions:

```rust
// Reconstruct positions on-demand
let positions: HashMap<usize, (Position, Position)> =
    compute_absolute_positions(&doc.ir);

// positions maps: node_ptr → (start_position, end_position)
// Used for finding nodes by position
```

This is the same reconstruction process described earlier in this document (see [Relative to Absolute Reconstruction](#relative-to-absolute-reconstruction-lsp-operations)).

#### Step 3: Find Node at Position

**Location**: `src/ir/rholang_node/position_tracking.rs` (lines 434-664)

Find the deepest IR node containing the requested position:

```rust
pub fn find_node_at_position_with_path(
    root: &Arc<RholangNode>,
    positions: &HashMap<usize, (Position, Position)>,
    position: Position,
) -> Option<(Arc<RholangNode>, Vec<Arc<RholangNode>>)>
```

**Algorithm**:
1. Start at root node
2. For each node, check: `start.byte ≤ position.byte ≤ end.byte`
3. If match, recursively check children
4. Track depth - prefer deeper matches (more specific)
5. Return deepest matching node + path from root to node

**Example**:
```rholang
// Cursor at byte 15 (the 'x' in x!(42))
new x in { x!(42) }
           ^
           └─ position.byte = 15

Matches:
- Source (byte 0-19, depth 0)
- New (byte 0-19, depth 1)
- Block (byte 9-19, depth 2)
- Send (byte 11-17, depth 3)
- Var("x") (byte 11-12, depth 4)  ← deepest match!

Returns: (Var("x"), [Source, New, Block, Send, Var])
```

#### Step 4: Get Symbol at Position

**Location**: `src/lsp/backend/symbols.rs` (lines 134-294)

Determine what symbol the node represents based on its type and context:

```rust
pub(crate) async fn get_symbol_at_position(
    &self,
    uri: &Url,
    position: LspPosition,
) -> Option<Arc<Symbol>>
```

**Algorithm**:
1. Find node at position (from Step 3)
2. Extract symbol table from node metadata or document
3. Match on node type:
   - `Var`: Check if it's a contract name (via parent context) or local variable
   - `Contract`: Return contract symbol
   - `Send`/`SendSync`: Extract channel name (might be quoted contract)
   - `Quote`: Recursively handle quoted expression
   - `Block`/`Parenthesized`: Unwrap and handle inner node
4. Lookup symbol in appropriate scope

**Node Type Handlers**:

| Node Type | Handler | Description |
|-----------|---------|-------------|
| `Var` | `handle_var_symbol` | Checks if Var is contract name via parent, else looks up in symbol table |
| `Contract` | `handle_contract_symbol` | Returns global contract symbol |
| `Send`/`SendSync` | `handle_send_symbol` | Extracts channel name, handles quoted contracts |
| `Quote` | `handle_quote_symbol` | Recursively processes quoted expression |
| `Block`/`Parenthesized` | (inline) | Unwraps and delegates to inner expression |

#### Step 5: Symbol Table Lookup

**Location**: `src/lsp/backend/symbols.rs` (lines 296-333)

##### Local Variables (handle_var_symbol)

```rust
async fn handle_var_symbol(
    &self,
    uri: &Url,
    position: LspPosition,
    name: &str,
    path: &[Arc<RholangNode>],
    symbol_table: &Arc<SymbolTable>,
) -> Option<Arc<Symbol>>
```

**Algorithm**:
1. Check if Var is a contract name:
   - Look at parent in path: `if path[path.len()-2] is Contract`
   - Check if `Contract.name == current Var`
   - If yes, lookup in global symbols: `workspace.global_symbols.get(name)`

2. Otherwise, lookup as local variable:
   - `symbol_table.lookup(name)` - searches current scope + parent scopes
   - Returns first matching symbol from innermost to outermost scope

**Visual Example**:
```rholang
new x in {          // x declared in outer scope
  new y in {        // y declared in inner scope
    x!(y)           // Both x and y visible here
  }
}
```

Symbol table hierarchy:
```
InnerScope (y declared here)
    ↓ parent
OuterScope (x declared here)
    ↓ parent
GlobalScope (contracts)
```

Lookup for `y`:
1. Check InnerScope → Found! Return symbol for `y`

Lookup for `x`:
1. Check InnerScope → Not found
2. Check OuterScope (parent) → Found! Return symbol for `x`

##### Contracts (handle_contract_symbol)

Contracts are always global symbols:

```rust
// Look up in global symbol table
let (def_uri, def_pos) = workspace.global_symbols.get(contract_name)?;

return Some(Arc::new(Symbol {
    name: contract_name.to_string(),
    symbol_type: SymbolType::Contract,
    declaration_uri: def_uri,
    declaration_location: def_pos,
    ...
}));
```

##### Contract Calls (Fast Path via GlobalSymbolIndex)

**Location**: `src/lsp/backend/handlers.rs` (lines 524-534)

For Send nodes calling contracts, use the optimized global index:

```rust
// Fast path: O(1) lookup by name
if let Ok(global_index_guard) = global_index.read() {
    if let Ok(Some(symbol_loc)) = global_index_guard.find_contract_definition(&contract_name) {
        return Ok(Some(GotoDefinitionResponse::Scalar(symbol_loc.to_lsp_location())));
    }
}
```

The `GlobalSymbolIndex` provides O(1) lookups for contract definitions across the entire workspace.

#### Step 6: IR Position to LSP Location

**Location**: `src/lsp/backend/handlers.rs` (various)

Convert IR Position back to LSP Range:

```rust
fn position_to_range(start: Position, length_chars: usize) -> Range {
    Range {
        start: LspPosition {
            line: start.row as u32,
            character: start.column as u32,  // UTF-16 code units
        },
        end: LspPosition {
            line: start.row as u32,
            character: (start.column + length_chars) as u32,
        },
    }
}

// Create LSP Location
let loc = Location {
    uri: symbol.declaration_uri.clone(),
    range: Self::position_to_range(symbol.declaration_location, symbol.name.len()),
};
```

### Symbol Table Architecture

The symbol table system uses hierarchical scoping with parent pointers:

```
Document Symbol Table (root)
    │
    ├─ Global Symbols (contracts)
    │   │
    │   ├─ contractA → Symbol { type: Contract, location: ... }
    │   └─ contractB → Symbol { type: Contract, location: ... }
    │
    └─ Local Scopes (nested)
        │
        ├─ OuterScope (e.g., from 'new x, y in')
        │   │
        │   ├─ x → Symbol { type: Variable, location: ... }
        │   ├─ y → Symbol { type: Variable, location: ... }
        │   │
        │   └─ InnerScope (e.g., from 'for (z <- ch)')
        │       │
        │       └─ z → Symbol { type: Parameter, location: ... }
        │
        └─ Another Branch (parallel scope)
            └─ ...
```

**Lookup algorithm** (`SymbolTable::lookup`):
```rust
pub fn lookup(&self, name: &str) -> Option<Arc<Symbol>> {
    // Check current scope
    if let Some(symbol) = self.symbols.read().unwrap().get(name) {
        return Some(symbol.clone());
    }

    // Check parent scopes recursively
    if let Some(parent) = &self.parent {
        return parent.lookup(name);
    }

    None
}
```

### Performance Optimizations

| Optimization | Description | Benefit |
|--------------|-------------|---------|
| **Position caching** | HashMap created once per request, reused for all lookups | Avoid O(n) tree traversal per symbol |
| **GlobalSymbolIndex** | O(1) contract lookup by name | Fast goto-definition for contract calls |
| **Pattern index** | O(1) contract lookup by (name, arity) | Fast overload resolution |
| **Depth-first best** | Find deepest matching node first | Avoid unnecessary traversals |
| **Metadata attachment** | Symbol tables stored in node metadata | Local scope lookup without global search |

### Common Lookup Patterns

#### 1. Local Variable Reference

```rholang
new x in { x!(42) }
           ^
           └─ Click here
```

**Lookup flow**:
1. Find node: Var("x") at byte 11
2. Check parent: Send node (not Contract), so it's a variable reference
3. Get symbol table from Send node's metadata
4. Lookup "x" in symbol table → Found in outer scope (from `new x in`)
5. Return Symbol { name: "x", type: Variable, declaration_location: byte 4 }

#### 2. Contract Name in Declaration

```rholang
contract foo(x) = { x!(42) }
         ^──────
         └─ Click here
```

**Lookup flow**:
1. Find node: Var("foo") at byte 9
2. Check parent: Contract node, and Contract.name == Var("foo")
3. Recognize this is a contract declaration
4. Lookup "foo" in workspace.global_symbols
5. Return Symbol { name: "foo", type: Contract, declaration_location: byte 9 }

#### 3. Contract Call (Quote Channel)

```rholang
@"foo"!(42)
   ^──────
   └─ Click here
```

**Lookup flow**:
1. Find node: StringLiteral("foo") at byte 2
2. Parent is Quote, grandparent is Send
3. Send handler detects quoted channel
4. Extract contract name "foo" from StringLiteral
5. Fast path: GlobalSymbolIndex.find_contract_definition("foo")
6. Return contract definition location

### Edge Cases

#### Word Boundary Adjustment

When cursor is at right word boundary, LSP operation checks one position to the left:

```rholang
new x in { x!(42) }
            ^
            └─ Cursor at byte 12 (space after 'x')
```

**Handler** (`handlers.rs` lines 684-700):
```rust
if let Some(symbol) = self.get_symbol_at_position(&uri, lsp_pos).await {
    // Found at exact position
} else if lsp_pos.character > 0 {
    // Try one column left for right word boundary
    let left_pos = LspPosition {
        line: lsp_pos.line,
        character: lsp_pos.character - 1,
    };
    if let Some(symbol) = self.get_symbol_at_position(&uri, left_pos).await {
        // Found at left position!
    }
}
```

This matches IDE conventions where clicking at the end of a word still triggers operations on that word.

#### Block/Parenthesized Unwrapping

These nodes are just wrappers - we unwrap to find the actual symbol:

```rholang
new x in { (x!(42)) }
           ^^────^^
           └─ Block and Parenthesized wrappers
```

**Handler**: Recursively processes inner node until reaching a Var, Send, or other concrete node.

## Debugging Position Issues

If you encounter position offset errors (e.g., goto-definition fails at certain positions), use the systematic debugging approach documented in:

**See**: [`docs/POSITION_DEBUGGING_STRATEGY.md`](./POSITION_DEBUGGING_STRATEGY.md)

### Quick Summary

The debugging strategy involves:

1. **Add CHAIN logging** during conversion to capture Tree-Sitter positions
2. **Add RECON logging** during reconstruction to capture computed positions
3. **Compare CHAIN vs RECON** to find discrepancies
4. **Trace backwards** from first error to root cause
5. **Fix the conversion** by ensuring nodes return Tree-Sitter's `absolute_end`

### Common Bugs

| Bug Pattern | Symptom | Fix |
|------------|---------|-----|
| Returning child's end instead of Tree-Sitter's end | Cascading offset errors | Return `absolute_end` from conversion |
| Using `new_simple()` for nodes with delimiters | Wrong syntactic_length | Use `create_correct_node_base()` with dual lengths |
| Using content_length in reconstruction | Next sibling starts too early | Always use `syntactic_length` for reconstruction |

### Example Debug Session

See the real-world case study in `POSITION_DEBUGGING_STRATEGY.md` (lines 236-396) for a complete walkthrough of debugging a 2-byte offset error in the NameDecl node.

## Key Takeaways

1. **Invariant**: `node.start = prev_end + delta_bytes` and `node.end = node.start + syntactic_length`

2. **Conversion**: Always return Tree-Sitter's `absolute_end`, never a child's end

3. **Reconstruction**: Always use `syntactic_length`, never `content_length`

4. **Dual Lengths**: Use `content_length` for semantics, `syntactic_length` for reconstruction

5. **Cascading**: Position errors propagate forward through all subsequent siblings

6. **On-Demand**: Positions are reconstructed only when needed, not stored

7. **Memory Efficient**: Relative positions + structural sharing = minimal memory overhead

## Implementation Files

| File | Purpose | Key Functions |
|------|---------|--------------|
| `src/ir/semantic_node.rs` | Position data structures | `Position`, `RelativePosition`, `NodeBase` |
| `src/parsers/rholang/conversion/mod.rs` | Absolute → Relative | `convert_ts_node_to_ir()`, `create_correct_node_base()` |
| `src/ir/rholang_node/position_tracking.rs` | Relative → Absolute | `compute_absolute_positions()`, `compute_positions_helper()` |
| `src/parsers/rholang/helpers.rs` | Conversion utilities | `collect_named_descendants()`, `is_comment()` |

## References

- **Debugging Guide**: [`POSITION_DEBUGGING_STRATEGY.md`](./POSITION_DEBUGGING_STRATEGY.md) - Systematic approach to fixing position bugs
- **Project Overview**: [`../CLAUDE.md`](../.claude/CLAUDE.md) - Architecture and development commands
- **Tree-Sitter Docs**: [tree-sitter.github.io](https://tree-sitter.github.io/) - Parser framework documentation
