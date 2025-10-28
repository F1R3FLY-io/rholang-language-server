# Debugging Stack Overflow Issues

This document explains how to identify and debug stack overflow issues in the Rholang Language Server.

## Stack Overflow Detection

The server has a custom panic hook installed (in `src/main.rs:796-846`) that captures detailed information about panics, including stack overflows.

### What Information is Captured

When a stack overflow occurs, you'll see output like:

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

### Thread Identification

The server names threads to make debugging easier:

1. **Main runtime threads**: Named `rholang-tokio-worker` (configured at `src/main.rs:855`)
2. **Test threads**: Named explicitly in tests (e.g., `test_robot_planning_lsp_operations`)
3. **Unnamed threads**: Show as `<unnamed>` - these are typically:
   - Threads spawned by external libraries
   - Blocking tasks that don't inherit tokio's configuration
   - Threads created without explicit names

### Stack Size Configuration

The server configures different stack sizes for different thread types:

1. **Tokio worker threads**: 16MB (src/main.rs:854)
   ```rust
   let runtime = tokio::runtime::Builder::new_multi_thread()
       .thread_stack_size(16 * 1024 * 1024)  // 16MB
       .thread_name("rholang-tokio-worker")
       .build()?;
   ```

2. **Test threads**: 16MB (various test files)
   ```rust
   std::thread::Builder::new()
       .stack_size(16 * 1024 * 1024)
       .spawn(|| { /* test code */ })
   ```

3. **Default system threads**: ~2MB (platform dependent)

## Enabling Backtraces

The VSCode extension automatically sets `RUST_BACKTRACE=1` when starting the server (see `rholang-vscode-client/src/extension.ts:255`).

For manual testing or other environments, set the environment variable:

```bash
RUST_BACKTRACE=1 rholang-language-server --stdio
```

Or for even more verbose output:

```bash
RUST_BACKTRACE=full rholang-language-server --stdio
```

## Log Files

The server logs all activity to files in:
- **Linux**: `~/.cache/f1r3fly-io/rholang-language-server/`
- **macOS**: `~/Library/Caches/f1r3fly-io/rholang-language-server/`
- **Windows**: `%LOCALAPPDATA%\f1r3fly-io\rholang-language-server\`

Log files are named: `session-YYYYMMDD-HHMMSS-PID.log`

**Important**: Log files always capture TRACE-level logs (everything), regardless of the stderr log level. This ensures you have complete debugging information even when the server is running in production mode.

## Common Stack Overflow Causes

### 1. Deep AST Recursion

**Symptom**: Stack overflow when parsing or visiting large/deeply nested files

**Example**: `robot_planning.rho` (546 lines, 20KB with embedded MeTTa)

**Solution**: Already implemented - 16MB stack size handles this

**Location**: Recursion happens in:
- `src/ir/visitor/visitor_trait.rs` - IR tree traversal
- `src/parsers/rholang/conversion/mod.rs` - Tree-sitter to IR conversion
- `src/ir/transforms/symbol_table_builder.rs` - Symbol table building

### 2. Unnamed Threads Without Stack Configuration

**Symptom**: Stack overflow in thread with ID but no name (`<unnamed>`)

**Cause**: Thread created without tokio runtime or without explicit stack size

**Locations to Check**:
- `spawn_blocking` calls that don't use tokio
- External library threads
- Rayon thread pool (if used)
- Custom thread spawns

**How to Fix**: Wrap in `tokio::task::spawn_blocking` or use `std::thread::Builder::new().stack_size()`

### 3. Infinite Recursion

**Symptom**: Stack overflow even with large stack size

**How to Detect**:
1. Look at the backtrace - are the same functions repeated many times?
2. Check log files for repeated patterns before the crash
3. Add depth tracking to recursive functions:

```rust
fn recursive_function(&self, node: &Node, depth: usize) -> Result<()> {
    const MAX_DEPTH: usize = 1000;
    if depth > MAX_DEPTH {
        error!("Maximum recursion depth {} exceeded", MAX_DEPTH);
        return Err(Error::MaxDepthExceeded);
    }
    // ... recursive logic ...
    self.recursive_function(child, depth + 1)
}
```

## Reproducing Stack Overflows

### Using Integration Tests

The test `tests/test_lsp_robot_planning_replay.rs` can reproduce LSP operations:

```bash
# Run the full LSP lifecycle test
cargo test test_robot_planning_full_lsp -- --nocapture

# Run just the parsing operations
cargo test test_robot_planning_lsp_operations -- --nocapture
```

### Using the Test Script

Create a test script to check for stack overflow with specific files:

```bash
#!/bin/bash
RUST_BACKTRACE=1 timeout 10 \
    target/release/rholang-language-server --stdio \
    < test_input.json 2>&1 | tee stack_overflow.log
```

### Simulating VSCode Operations

Use the test utilities in `test_utils/` to simulate VSCode:

```rust
use test_utils::with_lsp_client;
use test_utils::lsp::client::CommType;

with_lsp_client!(test_name, CommType::Stdio, |client| {
    let doc = client.open_document("/test/file.rho", &source)?;
    let diagnostics = client.await_diagnostics(&doc)?;
    // ... more operations ...
});
```

## Thread Stack Size Recommendations

| File Size | Nesting Depth | Recommended Stack |
|-----------|---------------|-------------------|
| < 10KB    | < 100 levels  | 2MB (default)     |
| 10-50KB   | 100-500       | 8MB               |
| 50-100KB  | 500-1000      | 16MB              |
| > 100KB   | > 1000        | 32MB or more      |

**Note**: `robot_planning.rho` is 20KB but has deep nesting due to embedded MeTTa code, requiring 16MB.

## Monitoring Thread Creation

To track thread creation patterns, enable debug logging:

```bash
RUST_LOG=debug rholang-language-server --stdio
```

Look for these patterns in logs:
- "Spawning blocking task" - indicates tokio blocking task
- Thread IDs in panic messages
- Thread names in panic messages

## Platform Differences

### Linux
- Default thread stack: ~8MB
- Configurable via `ulimit -s`
- Process stack shown: `cat /proc/[pid]/status | grep Stack`

### macOS
- Default thread stack: ~512KB (much smaller!)
- Requires explicit configuration for large files
- Check with: `ulimit -s`

### Windows
- Default thread stack: ~1MB
- Configured via compiler flags or runtime
- Check process properties in Task Manager

## Best Practices

1. **Always name threads** for debugging:
   ```rust
   std::thread::Builder::new()
       .name("my-operation".to_string())
       .stack_size(16 * 1024 * 1024)
       .spawn(|| { /* ... */ })
   ```

2. **Use tokio for async work** - inherits runtime stack size:
   ```rust
   tokio::spawn(async { /* ... */ })  // Uses 16MB stack
   ```

3. **Track recursion depth** in visitor patterns:
   ```rust
   struct DepthTracker { depth: usize }
   impl DepthTracker {
       fn visit(&mut self, node: &Node) {
           self.depth += 1;
           if self.depth > MAX_DEPTH {
               panic!("Max depth exceeded");
           }
           // ... visit children ...
           self.depth -= 1;
       }
   }
   ```

4. **Test with real files** that have deep nesting (like `robot_planning.rho`)

5. **Check log files** after crashes - they contain TRACE-level details

## Getting Help

If you encounter a stack overflow:

1. Check the log file in `~/.cache/f1r3fly-io/rholang-language-server/`
2. Look for the thread name and location in the panic message
3. Set `RUST_BACKTRACE=full` and reproduce
4. Share:
   - Thread name and ID
   - Location (file:line:column)
   - Backtrace
   - The file being parsed (if applicable)
   - Log file contents

## See Also

- `src/main.rs:796-860` - Panic hook and runtime configuration
- `src/logging.rs` - Dual-level logging setup
- `tests/test_lsp_robot_planning_replay.rs` - Integration test examples
- `.claude/CLAUDE.md` - Project architecture overview
