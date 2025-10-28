# Position Debugging Strategy

This document describes the systematic approach used to debug 2-byte position offset errors in the Rholang language server.

## Problem Statement

When goto-definition fails at specific character positions (e.g., positions 34-35 in `robotAPI`), it indicates that the reconstructed positions in the IR don't match the actual file positions. The symptoms:
- Tree-Sitter reports correct positions during parsing
- Position reconstruction computes incorrect positions
- The error cascades through all subsequent nodes

## Core Concepts

### Position Storage System

The language server uses a delta-encoding system:

1. **During Conversion (Tree-Sitter → IR)**:
   - Each node receives `prev_end` (the end position of the previous node/sibling)
   - Tree-Sitter provides absolute `start_byte` and `end_byte`
   - We compute `delta_bytes = start_byte - prev_end.byte`
   - NodeBase stores:
     - `relative_start.delta_bytes`: offset from previous node
     - `content_length`: semantic extent (up to last child)
     - `syntactic_length`: full extent (includes closing delimiters like `}`, `)`)

2. **During Reconstruction (IR → Positions)**:
   - Start with initial prev_end (usually `{row:0, column:0, byte:0}`)
   - For each node: `start = prev_end + delta_bytes`
   - End computed as: `end = start + syntactic_length`
   - Pass computed end to next sibling as their prev_end

### The Cascading Effect

If ANY node computes an incorrect end position during reconstruction, all subsequent sibling nodes will start at wrong positions. The error propagates through the entire chain.

## Debugging Strategy

### Phase 1: Confirm Tree-Sitter Positions Are Correct

Add logging during conversion (`src/parsers/rholang/conversion/mod.rs`):

```rust
if absolute_start.byte >= TARGET_START && absolute_start.byte <= TARGET_END {
    eprintln!("CHAIN: {} at TS_byte={} TS_end={} (passed prev_end.byte={}, diff={})",
        ts_node.kind(), absolute_start.byte, absolute_end.byte, prev_end.byte,
        absolute_start.byte as i64 - prev_end.byte as i64);
}
```

This shows:
- What Tree-Sitter reports for each node
- What prev_end each node receives
- The delta between them

**Key insight**: If CHAIN output shows Tree-Sitter positions are correct, the bug is in reconstruction or storage, not parsing.

### Phase 2: Identify Reconstruction Errors

Add logging during reconstruction (`src/ir/rholang_node/position_tracking.rs`):

```rust
if start.byte >= TARGET_START && start.byte <= TARGET_END {
    eprintln!("RECON: {} TS_should_be={} ACTUAL_start={} delta={} syntactic_len={} ACTUAL_end={}",
             node_type, start.byte - base.delta_bytes(), start.byte, base.delta_bytes(),
             base.syntactic_length(), end.byte);
}
```

Where:
- `TS_should_be = start.byte - delta_bytes` is the prev_end that was passed to this node during reconstruction
- `ACTUAL_start` is the reconstructed start position
- `ACTUAL_end` is the reconstructed end position

**Key insight**: Compare `ACTUAL_start` with Tree-Sitter's `TS_byte` from CHAIN output. If they differ, this node received an incorrect prev_end.

### Phase 3: Trace Backwards to Find Root Cause

When you find a node at position X that reconstructs incorrectly:

1. **Identify what should have passed prev_end**:
   - From CHAIN: node at X received `prev_end.byte=Y` (Y should be correct)
   - From RECON: node at X received `TS_should_be=Z` (Z is what was actually passed)
   - If Z ≠ Y, then the node ending at Y is being reconstructed to end at Z

2. **Find the predecessor node**:
   - Look in CHAIN output for a node with `TS_end=Y`
   - This node should end at Y, but reconstructs to end at Z
   - The difference (Z - Y) is the offset being introduced

3. **Check the predecessor's length**:
   - From CHAIN: `expected_length = TS_end - TS_byte`
   - From RECON: `actual_syntactic_len` (what's stored)
   - If `actual_syntactic_len ≠ expected_length`, you've found the bug!

4. **Trace further back if needed**:
   - If the predecessor's length is correct, its START must be wrong
   - Apply the same process recursively
   - Keep tracing backwards until you find the first node with incorrect length or start

### Phase 4: Examine the Buggy Node Conversion

Once you identify the problematic node (e.g., "Var at byte 14701 has syntactic_length=5 but should be 3"):

1. **Verify with file content**:
   ```python
   content = open("file.rho").read()
   print(repr(content[14701:14704]))  # Should show the actual text
   ```

2. **Find the conversion code**:
   - Search `src/parsers/rholang/conversion/mod.rs` for the node type
   - Check how it computes `content_end` and `syntactic_end`
   - Look for bugs in:
     - `create_correct_node_base()` calls
     - Return value (`(node, absolute_end)` should be Tree-Sitter's end)
     - Whether it's using default `base` or creating custom NodeBase

3. **Common bug patterns**:
   - Using `new_simple()` when node has closing delimiter → syntactic_length wrong
   - Returning wrong end position (e.g., returning `child_end` when should return `absolute_end`)
   - Not accounting for delimiters in container nodes (Block, Par, Send, etc.)

## Example: Debugging 2-Byte Offset in robotAPI

### Step 1: CHAIN Output Analysis
```
CHAIN: input at TS_byte=14831 TS_end=16698 (passed prev_end.byte=14798, diff=33)
```
Tree-Sitter says Input starts at 14831.

### Step 2: RECON Output Analysis
```
RECON: Input TS_should_be=14800 ACTUAL_start=14833 delta=33
```
Input reconstructs to 14833 (should be 14831). It received prev_end=14800 (should be 14798).

**Conclusion**: Something ending at 14798 is reconstructing to end at 14800 (2 bytes too large).

### Step 3: Find Predecessor
```
CHAIN: send at TS_byte=14738 TS_end=14798
```
Send node ends at 14798. It must be reconstructing to end at 14800.

### Step 4: Check Send's Length
```
RECON: Send TS_should_be=14740 ACTUAL_start=14740 syntactic_len=60 ACTUAL_end=14800
```
Send has syntactic_len=60, starts at 14740, ends at 14800.
Expected: starts at 14738, ends at 14798 (length=60).

Send's length is CORRECT (60), but its START is wrong (14740 instead of 14738).

**Conclusion**: Send's predecessor is passing 14740 instead of 14738.

### Step 5: Continue Tracing
Keep applying the same process backwards through the chain until you find the node with incorrect `syntactic_length`.

## Tools and Techniques

### Adjusting Debug Range

Start wide and narrow down:
```rust
// Start: bytes 14700-14950
if absolute_start.byte >= 14700 && absolute_start.byte <= 14950 { ... }

// Narrow to: bytes 14600-14750 if needed
if absolute_start.byte >= 14600 && absolute_start.byte <= 14750 { ... }
```

### Filtering Output

Use grep to find specific nodes:
```bash
cargo test test_name -- --nocapture 2>&1 | grep "CHAIN:" | grep "TS_end=14798"
cargo test test_name -- --nocapture 2>&1 | grep "RECON:" | grep "Var(ack)"
```

### Verifying File Content

Always verify assumptions about file content:
```python
content = open("file.rho").read()
print("Bytes X-Y:", repr(content[X:Y]))
```

## Key Lessons

1. **Tree-Sitter is usually correct**: If CHAIN shows correct positions, don't question Tree-Sitter.

2. **Errors cascade forward**: A small error early in the file affects everything after it.

3. **Trace backwards systematically**: Don't guess - follow the chain from incorrect node back to root cause.

4. **Length vs. Start errors**:
   - If a node's length is wrong, fix its NodeBase creation
   - If a node's length is right but start is wrong, look at its predecessor

5. **Dual lengths matter**:
   - `content_length`: for semantic operations
   - `syntactic_length`: for reconstruction (must match Tree-Sitter's end - start)

## Common Fixes

### Fix 1: Node Has Closing Delimiter But Uses new_simple()

**Problem**: Block/Send/etc. uses default `base` created with `new_simple()`, which sets `content_length = syntactic_length`.

**Fix**: Use `create_correct_node_base()` with separate content_end and syntactic_end:
```rust
let content_end = last_child_end;  // After last child
let syntactic_end = absolute_end;  // Includes closing '}'
let corrected_base = create_correct_node_base(
    absolute_start, content_end, syntactic_end, prev_end
);
```

### Fix 2: Node Returns Wrong End Position

**Problem**: Node returns `child_end` but Tree-Sitter includes more (like closing delimiter).

**Fix**: Return `absolute_end` (Tree-Sitter's end):
```rust
(node, absolute_end)  // Not child_end!
```

### Fix 3: Container Node Doesn't Account for Delimiters

**Problem**: Par/Block processes children but forgets it has `{...}` wrapper.

**Fix**: Use Tree-Sitter's absolute_end which includes delimiters, not last child's end.

## Real-World Case Study: NameDecl Bug (January 2025)

This section documents a complete debugging session that fixed position offset errors in `robot_planning.rho`.

### Initial Symptoms

- Test: `test_goto_definition_robotapi_all_positions`
- Status: 7/9 positions passing
- Failing positions: characters at bytes 34-35 in `robotAPI` variable
- Manifestation: Goto-definition failed because reconstructed positions were +2 bytes off

### Phase 1: Find First Discrepancy

Added logging to capture ALL nodes from document start:

```rust
// In src/parsers/rholang/conversion/mod.rs (line ~79)
if absolute_start.byte <= 14950 {
    eprintln!("CHAIN: {} at TS_byte={} TS_end={} (passed prev_end.byte={}, diff={})",
        ts_node.kind(), absolute_start.byte, absolute_end.byte, prev_end.byte,
        absolute_start.byte as i64 - prev_end.byte as i64);
}

// In src/ir/rholang_node/position_tracking.rs (line ~73)
eprintln!("RECON: {} TS_should_be={} ACTUAL_start={} delta={} syntactic_len={} ACTUAL_end={}",
         node_type, start.byte - base.delta_bytes(), start.byte, base.delta_bytes(),
         base.syntactic_length(), end.byte);
```

Built Python script to systematically find ALL nodes with +2 offset:

```python
# Find first node with position discrepancy
for chain_node in sorted(chain_nodes, key=lambda n: n['start']):
    for recon_node in recon_by_kind[map_kind(chain_node['kind'])]:
        if (recon_node['start'] == chain_node['start'] + 2 and
            recon_node['syntactic_len'] == chain_node['end'] - chain_node['start']):
            # Found first mismatch!
```

**Result**: First node with +2 offset was `NameDecl at byte 627`:
- CHAIN: `name_decl: 627-635` (length=8)
- RECON: `NameDecl: 629-637` (length=8)

### Phase 2: Trace Backwards

The NameDecl has correct length but starts +2 bytes late. This means it received `prev_end=629` instead of `prev_end=627`.

Checked what should end at 627:
```
CHAIN nodes ending between 600-630:
  name_decl: 553-582 (len=29)  ← First NameDecl
  var: 588-600 (len=12)
  uri_literal: 601-620 (len=19)
```

No node ends exactly at 627! This revealed the real issue: something ending at a different position was passing wrong prev_end.

### Phase 3: Examine Earlier Nodes

Found a pattern - the second NameDecl was also offset:
- CHAIN: `name_decl at TS_byte=588 TS_end=621`
- RECON: `NameDecl TS_should_be=582 ACTUAL_start=589 delta=7`

**Key observation**:
- Tree-Sitter says start=588
- prev_end should be 582 (where first NameDecl ends)
- But delta=7, which means: `589 = 582 + 7`
- Expected delta: `588 - 582 = 6` ❌ Actual: 7

The second NameDecl is computing `delta_bytes=7` instead of `delta_bytes=6`.

### Phase 4: Find Why Delta is Wrong

Checked the CHAIN log for the second NameDecl:
```
CHAIN: name_decl at TS_byte=588 TS_end=621 (passed prev_end.byte=581, diff=7)
```

**Smoking gun!** The second NameDecl is receiving `prev_end.byte=581` instead of `582`.

The first NameDecl ends at 582 (Tree-Sitter), but something is returning 581.

### Phase 5: Examine First NameDecl Conversion

Checked children of first NameDecl:
```
CHAIN: var at TS_byte=553 TS_end=562
CHAIN: uri_literal at TS_byte=563 TS_end=581  ← Ends at 581!
```

The uri_literal ends at 581, and the NameDecl conversion was returning this:

```rust
// src/parsers/rholang/conversion/mod.rs line 904-914 (BEFORE FIX)
"name_decl" => {
    let var_ts = ts_node.named_child(0).expect("NameDecl node must have a variable");
    let (var, var_end) = convert_ts_node_to_ir(var_ts, rope, absolute_start);
    let uri = ts_node.child_by_field_name("uri")
        .map(|uri_ts| {
            let (uri_node, uri_end) = convert_ts_node_to_ir(uri_ts, rope, var_end);
            (uri_node, uri_end)
        });
    let node = Arc::new(RholangNode::NameDecl { base, var, uri: ... });
    (node, uri.map_or(var_end, |(_, end)| end))  // ← BUG! Returns 581
}
```

**The bug**: NameDecl returns `uri_end` (581) instead of Tree-Sitter's `absolute_end` (582).

The missing byte was likely a `)` or `,` separator after the URI.

Verified in file:
```python
content = open("tests/resources/robot_planning.rho").read()
print(repr(content[553:583]))  # 'insertArbitrary(`rho:registry:insertArbitrary`),'
                                #                                              ^ byte 582 is the comma
```

### The Fix

Changed line 913 to return Tree-Sitter's absolute end:

```rust
"name_decl" => {
    let var_ts = ts_node.named_child(0).expect("NameDecl node must have a variable");
    let (var, var_end) = convert_ts_node_to_ir(var_ts, rope, absolute_start);
    let uri = ts_node.child_by_field_name("uri")
        .map(|uri_ts| {
            let (uri_node, uri_end) = convert_ts_node_to_ir(uri_ts, rope, var_end);
            (uri_node, uri_end)
        });
    let node = Arc::new(RholangNode::NameDecl { base, var, uri: ... });
    (node, absolute_end)  // ✓ FIXED: Return Tree-Sitter's end
}
```

### Verification

```bash
$ cargo test test_goto_definition_robotapi_all_positions
=== All goto_definition tests PASSED ===
test test_goto_definition_robotapi_all_positions ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured
```

All 9/9 positions now pass! ✓

### Key Lessons from This Case

1. **Start from the beginning**: Don't assume the error is near the symptom. The +2 byte offset at byte 14932 was caused by a bug at byte 582.

2. **Remove range limits early**: Initially used `if absolute_start.byte >= 14700 && absolute_start.byte <= 14950` but this hid the root cause. Changed to `<= 14950` to capture from document start.

3. **Build automated analysis**: Manual grepping was slow and error-prone. Python script to find ALL discrepancies and sort by position was crucial.

4. **CHAIN vs RECON comparison is powerful**: The mismatch between what Tree-Sitter reports (CHAIN) and what gets reconstructed (RECON) pinpoints exactly where the bug is.

5. **Return values matter**: The node conversion must ALWAYS return `(node, absolute_end)` from Tree-Sitter, not the end of the last child processed. Child ends are for internal tracking during conversion.

6. **Off-by-one cascades**: A single byte error at byte 582 cascaded through thousands of nodes, ultimately manifesting as a 2-byte offset 14,000 bytes later in the file.

## Summary

1. Add CHAIN logging during conversion to verify Tree-Sitter positions
2. Add RECON logging during reconstruction to see actual computed positions
3. Compare CHAIN vs RECON to find first mismatch
4. Trace backwards through CHAIN to find node with incorrect length
5. Examine that node's conversion code and fix the length calculation
6. Verify fix with test
7. Remove debug logging when done

This systematic approach ensures you find the root cause rather than treating symptoms.
