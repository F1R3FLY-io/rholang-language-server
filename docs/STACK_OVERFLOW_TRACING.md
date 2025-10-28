# Stack Overflow Tracing - Quick Reference

## What's Been Added

### 1. Custom Panic Hook (src/main.rs:796-846)

A panic hook now captures detailed information about any panic, including stack overflows:

```
╔══════════════════════════════════════════════════════════════════════
║ STACK OVERFLOW DETECTED
╠══════════════════════════════════════════════════════════════════════
║ Thread Name: rholang-tokio-worker-0
║ Thread ID:   ThreadId(42)
║ Location:    src/ir/visitor/visitor_trait.rs:123:5
║ Message:     thread 'rholang-tokio-worker-0' has overflowed its stack
╚══════════════════════════════════════════════════════════════════════
```

### 2. Named Threads (src/main.rs:855)

Tokio worker threads are now named `rholang-tokio-worker` for easier identification.

### 3. Test Script (scripts/test-stack-overflow.sh)

A script to test the server with specific files:

```bash
./scripts/test-stack-overflow.sh tests/resources/robot_planning.rho
```

This script:
- Builds the server in debug mode
- Starts it with `RUST_BACKTRACE=full`
- Sends LSP messages to open the file
- Captures and displays any stack overflow
- Shows the latest log file

## Quick Debugging Steps

### When You See a Stack Overflow in VSCode

1. **Check the VSCode Output Panel**:
   - View → Output
   - Select "Rholang" from the dropdown
   - Look for the stack overflow box with thread info

2. **Check the Panic Log File**:
   ```bash
   # Linux/Mac - dedicated panic log (always written on crash)
   cat ~/.cache/f1r3fly-io/rholang-language-server/panic.log

   # View the latest session log
   tail -100 ~/.cache/f1r3fly-io/rholang-language-server/session-*.log
   ```

   **Important**: Panics are now written to TWO places:
   - `panic.log` - Dedicated file for ALL panics/stack overflows
   - VSCode Output panel - stderr capture (if VSCode is running)

3. **Identify the Thread**:
   - `rholang-tokio-worker-X` → Main server thread (has 16MB stack)
   - `<unnamed>` → External thread without proper stack size
   - Named test threads → From integration tests

4. **Get the File That Caused It**:
   - Look in the log for recent `textDocument/didOpen` messages
   - The URI will show which file was being opened

### Reproduce Locally

```bash
# Test with the problem file
./scripts/test-stack-overflow.sh /path/to/problem.rho

# Or run the integration tests
cargo test test_robot_planning_full_lsp -- --nocapture
```

### Analyze the Backtrace

The backtrace will show which functions are on the stack. Look for:

1. **Repeating patterns** - indicates recursion
2. **Function names** - which operation was happening
3. **Last successful operation** - what completed before the crash

Example backtrace pattern for deep AST recursion:
```
visit_node
  visit_par
    visit_node
      visit_par
        visit_node
          [repeats 1000+ times]
```

## Common Solutions

### Solution 1: Increase Stack Size (Already Done)

The server already uses 16MB for worker threads. If you still get overflows:

```rust
// In src/main.rs:854
.thread_stack_size(32 * 1024 * 1024)  // Increase to 32MB
```

### Solution 2: Fix Unnamed Threads

If the overflow is in `<unnamed>` thread, find where it's created and add:

```rust
std::thread::Builder::new()
    .name("my-operation".to_string())
    .stack_size(16 * 1024 * 1024)
    .spawn(|| { /* ... */ })
```

Or use tokio which inherits the configured stack:
```rust
tokio::task::spawn_blocking(|| { /* ... */ })
```

### Solution 3: Add Depth Limits

For infinite recursion bugs, add depth tracking:

```rust
fn visit_node(&self, node: &Node, depth: usize) -> Result<Node> {
    if depth > 1000 {
        return Err(Error::MaxDepthExceeded(depth));
    }
    // ... visit logic ...
    self.visit_child(child, depth + 1)
}
```

## VSCode Extension Configuration

The extension (rholang-vscode-client/src/extension.ts:255) already sets:

```typescript
options: {
    env: {
        RUST_BACKTRACE: '1'
    }
}
```

So backtraces are automatically captured in the Output panel.

## File Size Guidelines

Based on testing with robot_planning.rho (20KB, 546 lines, deep nesting):

| File Size | Max Nesting | Stack Needed | Status        |
|-----------|-------------|--------------|---------------|
| < 50KB    | < 500       | 16MB         | ✅ Works now   |
| 50-100KB  | 500-1000    | 16-32MB      | May need bump |
| > 100KB   | > 1000      | 32MB+        | Needs testing |

## Testing Strategy

### Test Files in Priority Order

1. **robot_planning.rho** - Already tested, works with 16MB
2. **Your actual files** - Use the test script on real workload
3. **Synthetic deep files** - Create test cases with known depth

### Automated Testing

The integration tests in `tests/test_lsp_robot_planning_replay.rs` provide:
- `test_robot_planning_lsp_operations` - Direct parsing
- `test_robot_planning_full_lsp` - Full LSP lifecycle
- `test_robot_planning_direct_parse` - Simple parse

Run them:
```bash
cargo test test_robot_planning --test test_lsp_robot_planning_replay
```

## Stack Size Configuration

**Current allocation** (src/main.rs:873-907):
- **Debug builds**: 16MB (8-16x safety margin for ~1-2 MB usage)
- **Release builds**: 8MB (160x safety margin for ~50 KB usage)

The configuration uses conditional compilation (`#[cfg(debug_assertions)]`) to optimize memory usage while providing sufficient margins for both build types. Debug builds need slightly more stack because their stack frames are 10-20x larger than release builds due to no inlining, bounds checking, and debug assertions.

**Critical**: Stack size is configured for BOTH thread pools:
1. **Tokio runtime** (lines 900-906): For async tasks and LSP message handling
2. **Rayon global pool** (lines 894-898): For parallel workspace indexing via `par_iter()`

Both pools need the same stack size because both perform deep AST parsing via `parse_to_ir()`. **The rayon configuration was the key fix** - without it, rayon threads would use the default 2MB stack and overflow when parsing complex files during workspace indexing, even though the tokio runtime had sufficient stack size. This was the root cause of the stack overflows that persisted even with 64MB and 128MB tokio stack allocations.

## Resources

- **Full debugging guide**: `docs/DEBUGGING_STACK_OVERFLOW.md`
- **Test script**: `scripts/test-stack-overflow.sh`
- **Panic hook code**: `src/main.rs:796-871`
- **Thread configuration**: `src/main.rs:873-889`
- **VSCode extension**: `rholang-vscode-client/src/extension.ts`

## Next Steps If Stack Overflow Persists

1. **Capture the exact thread info** from the panic box
2. **Get the backtrace** from VSCode Output or log file
3. **Identify if it's**:
   - Named tokio worker (check stack size config)
   - Unnamed thread (find where it's spawned)
   - Infinite recursion (check for repeating patterns)
4. **Share**:
   - Thread name/ID
   - Location (file:line)
   - Backtrace
   - The .rho file (if possible)
   - Log file excerpt

With this information, we can:
- Increase stack size further if needed
- Fix unnamed thread spawns
- Add depth limits to prevent infinite recursion
- Optimize the recursive algorithms
