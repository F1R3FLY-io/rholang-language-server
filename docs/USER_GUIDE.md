# Rholang Language Server - User Guide

This guide covers the features available in the Rholang Language Server for VSCode and other LSP-compatible editors.

## Table of Contents

- [Getting Started](#getting-started)
- [Features](#features)
  - [Go to Definition](#go-to-definition)
  - [Symbol Highlighting](#symbol-highlighting)
  - [Hover Information](#hover-information)
  - [Document Symbols](#document-symbols)
  - [Rename Symbol](#rename-symbol)
  - [Find References](#find-references)
- [Performance](#performance)
- [Troubleshooting](#troubleshooting)

## Getting Started

### Prerequisites

- VSCode or LSP-compatible editor
- Rholang Language Server binary installed
- RNode (optional, for semantic validation)

### Installation

1. Build the language server:
   ```bash
   cargo build --release
   ```

2. The binary will be located at:
   ```
   target/release/rholang-language-server
   ```

3. Configure your editor to use the language server for `.rho` files

### VSCode Configuration

Create or update `.vscode/settings.json` in your project:

```json
{
  "rholang.languageServer.path": "/path/to/rholang-language-server"
}
```

## Features

### Go to Definition

**What it does**: Navigate from a symbol usage to its definition

**How to use**:
- **Keyboard**: Place cursor on a symbol and press `F12`
- **Mouse**: `Ctrl+Click` (Windows/Linux) or `Cmd+Click` (Mac) on a symbol
- **Right-click menu**: Select "Go to Definition"

**Supported symbols**:
- Contract names
- Variable bindings from `new`
- Parameters in contracts and loops
- Pattern bindings

**Example**:
```rholang
contract myContract(@x) = { Nil }

// Place cursor on 'myContract' below and press F12
// Jumps to the contract definition above
new result in { myContract!(42) }
```

**Cross-file support**: Works across multiple `.rho` files in your workspace

**Performance**: <100ms response time for typical files

### Symbol Highlighting

**What it does**: Highlights all occurrences of a symbol in the current scope

**How to use**: Place your cursor on any variable or contract name

**Behavior**:
- Automatically highlights all uses of the symbol within its scope
- Different highlight styles for:
  - **Write access**: Variable definition/binding
  - **Read access**: Variable usage

**Scope awareness**: Only highlights symbols within the correct scope context

**Example**:
```rholang
new myVar in {
    myVar!(1) |        // highlighted
    myVar!(2) |        // highlighted
    for (@val <- myVar) {  // highlighted
        stdout!(val)
    }
}
```

**Note**: Highlights persist when hovering over symbols (unlike some LSP implementations)

### Hover Information

**What it does**: Shows information about a symbol when you hover over it

**How to use**: Hover your mouse over any symbol

**Information displayed**:
- **Symbol name** (bold)
- **Symbol type** (variable, contract, or parameter)
- **Declaration location** (file, line, and column)

**Example output**:
```
**myContract**

*contract*

Declared at line 5, column 10
```

**Supported symbols**:
- All variables, contracts, and parameters
- Falls back to basic hover if full symbol information is unavailable

### Document Symbols

**What it does**: Shows an outline of all symbols in the current file

**How to use**:
- **Keyboard**: Press `Ctrl+Shift+O` (Windows/Linux) or `Cmd+Shift+O` (Mac)
- **VSCode**: View → Open Symbol...

**Symbol hierarchy**:
- Top-level contracts
- Nested `new` bindings
- Contract parameters
- Loop bindings

**Features**:
- Quick navigation to any symbol
- Symbol search/filtering
- Hierarchical view of code structure

### Rename Symbol

**What it does**: Renames a symbol and all its references across files

**How to use**:
- **Keyboard**: Place cursor on symbol and press `F2`
- **Right-click menu**: Select "Rename Symbol"

**Scope**:
- Renames only within the symbol's scope
- Works across multiple files
- Preserves code structure and formatting

**Safety**:
- Preview changes before applying
- Undo supported (`Ctrl+Z`)

**Example**:
```rholang
// Before rename
contract oldName(@x) = { Nil }
new result in { oldName!(42) }

// After renaming 'oldName' to 'newName'
contract newName(@x) = { Nil }
new result in { newName!(42) }
```

### Find References

**What it does**: Finds all references to a symbol across your workspace

**How to use**:
- **Keyboard**: Place cursor on symbol and press `Shift+F12`
- **Right-click menu**: Select "Find All References"

**Results**:
- Shows all files containing references
- Displays context for each reference
- Click to navigate to reference location

**Performance**: Fast lookups using inverted index

## Performance

The language server is optimized for responsiveness:

| Operation | Target | Typical |
|-----------|--------|---------|
| Go to Definition | <100ms | ~2ms |
| Symbol Highlighting | <100ms | ~3ms |
| Hover Information | <50ms | <1ms |
| Document Symbols | <100ms | <10ms |

**Large file support**: Handles files with 500+ lines efficiently

**Memory efficient**: Uses immutable data structures with structural sharing

## Troubleshooting

### Slow Performance

**Symptoms**: Operations take >1 second

**Solutions**:
1. Check log files in `~/.cache/f1r3fly-io/rholang-language-server/`
2. Disable DEBUG logging if enabled
3. Ensure RNode is not running in validation mode if performance is critical

### Symbol Highlighting Not Working

**Symptoms**: Highlights disappear when hovering

**Solution**: Update to latest version - this issue was fixed in recent releases

### Go to Definition Not Working

**Symptoms**: "No definition found" or no response

**Possible causes**:
1. Symbol is not in scope
2. File not yet indexed
3. Syntax errors in file

**Solutions**:
1. Wait for file indexing to complete (check status bar)
2. Fix any syntax errors
3. Reload window (`Ctrl+Shift+P` → "Reload Window")

### Large Files Crashing

**Symptoms**: Editor crashes or becomes unresponsive with large files

**Solution**: Update to latest version with 8MB stack size fix

### Cross-File Navigation Not Working

**Symptoms**: Go to Definition only works within same file

**Solutions**:
1. Ensure workspace folder is opened (not just individual files)
2. Wait for workspace indexing to complete
3. Check that both files are `.rho` files

## Logging

### Enable Debug Logging

Set environment variable before starting VSCode:

```bash
export RUST_LOG=rholang_language_server=debug
code .
```

### Log Levels

- `error`: Errors only
- `warn`: Warnings and errors
- `info`: Request/response logging
- `debug`: Detailed operation logging
- `trace`: Very detailed logging (impacts performance)

### Log Location

Logs are written to:
```
~/.cache/f1r3fly-io/rholang-language-server/session-*.log
```

## Advanced Configuration

### Custom RNode Connection

If using RNode for semantic validation:

```json
{
  "rholang.rnode.host": "localhost",
  "rholang.rnode.port": 40401
}
```

### Disable RNode Validation

For faster local-only development:

```bash
rholang-language-server --no-rnode
```

## Tips and Tricks

1. **Quick Navigation**: Use `Ctrl+P` to quickly open files by name
2. **Breadcrumbs**: Enable breadcrumbs to see symbol hierarchy
3. **Peek Definition**: Use `Alt+F12` to peek at definition without navigating
4. **Multiple Cursors**: Select symbol and press `Ctrl+D` repeatedly to select all occurrences

## Getting Help

- Report issues: https://github.com/F1R3FLY-io/rholang-language-server/issues
- Documentation: Check CLAUDE.md and README.md
- Performance issues: Run with `RUST_LOG=info` and share logs

## See Also

- [CHANGELOG.md](../CHANGELOG.md) - Recent changes and improvements
- [CLAUDE.md](../CLAUDE.md) - Technical architecture documentation
- [README.md](../README.md) - Installation and development guide
