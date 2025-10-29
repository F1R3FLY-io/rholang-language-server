# Embedded Languages in Rholang - User Guide

This guide explains how the Rholang Language Server supports embedded languages like MeTTa within Rholang code, providing full IDE features for multi-language development.

## Table of Contents

- [Overview](#overview)
- [Quick Start](#quick-start)
- [How It Works](#how-it-works)
- [Supported Features](#supported-features)
- [Adding Your Own Language](#adding-your-own-language)
- [Examples](#examples)
- [Troubleshooting](#troubleshooting)

## Overview

The Rholang Language Server provides first-class support for embedding other languages within Rholang code. When you write MeTTa (or other supported languages) inside Rholang strings, you get full IDE features like:

- Go-to-definition
- Find references
- Rename symbols
- Hover information
- Document highlights
- Syntax validation

### Architecture Diagram

```
┌─────────────────────────────────────────────────────────────┐
│                    Rholang Source File                       │
│                                                               │
│  new metta in {                                              │
│    @"#!metta                                                 │
│      (= (factorial $n)                                       │
│          (if (== $n 0) 1 (* $n (factorial (- $n 1)))))      │
│      (factorial 5)                                           │
│    "!(metta)                                                 │
│  }                                                            │
└─────────────────────────────────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────────┐
│              Language Detection & Extraction                  │
│                                                               │
│  1. Directive Parser identifies: #!metta                     │
│  2. Region Extractor extracts content                        │
│  3. VirtualDocument created with URI:                        │
│     file:///path/to/file.rho#vdoc:0                         │
└─────────────────────────────────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────────┐
│                  Virtual Document Processing                  │
│                                                               │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐  │
│  │   MeTTa      │    │   Symbol     │    │  LSP Feature │  │
│  │   Parser     │───▶│   Tables     │───▶│   Handlers   │  │
│  └──────────────┘    └──────────────┘    └──────────────┘  │
│                                                               │
└─────────────────────────────────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────────┐
│               LSP Features (Editor Integration)               │
│                                                               │
│  • Ctrl+Click jumps to definition                            │
│  • F12 shows all references                                  │
│  • F2 renames across all occurrences                         │
│  • Hover shows type and docs                                 │
└─────────────────────────────────────────────────────────────┘
```

## Quick Start

### Basic Example

Add embedded MeTTa code to your Rholang file:

```rholang
new metta in {
  // Use the #!metta directive to mark embedded MeTTa code
  @"#!metta
  (= (get_neighbors $location)
     (match &locations $location $neighbors))

  (= (navigate $from $to)
     (let $neighbors (get_neighbors $from)
       (if (contains $neighbors $to)
           (move $to)
           (find_path $from $to))))
  "!(metta)
}
```

### What Happens

1. The language server detects the `#!metta` directive
2. It extracts the MeTTa code as a "virtual document"
3. The MeTTa code is parsed and analyzed independently
4. All LSP features work within the embedded region
5. Position mapping ensures correct file locations

### Supported LSP Operations

| Feature | Description | Keyboard Shortcut |
|---------|-------------|-------------------|
| **Go to Definition** | Jump to where a symbol is defined | `F12` or `Ctrl+Click` |
| **Find References** | Find all uses of a symbol | `Shift+F12` |
| **Rename Symbol** | Rename across all occurrences | `F2` |
| **Hover Information** | Show symbol details | Mouse hover |
| **Document Highlights** | Highlight related symbols | Cursor on symbol |
| **Syntax Validation** | Real-time error checking | Automatic |

## How It Works

### 1. Language Detection

The system looks for language directives in Rholang strings:

```rholang
@"#!language
  code here
"!(channel)
```

Supported directive formats:
- `#!metta` - Full word form
- `#!<language>` - Generic form for any language

### 2. Virtual Document Creation

When a directive is detected, the system:

1. **Extracts** the embedded code
2. **Creates** a virtual document with unique URI
3. **Parses** using language-specific parser
4. **Builds** symbol tables for the code
5. **Registers** the document for LSP queries

#### Virtual URI Format

```
file:///path/to/file.rho#vdoc:N
                        │     │
                        │     └─ Sequential number (0, 1, 2...)
                        └─ Virtual document marker
```

### 3. Position Mapping

The system maintains bidirectional position mapping:

```
Parent Document Position    ←→    Virtual Document Position
(Line 5, Column 10)               (Line 2, Column 5)
```

**Why This Matters:**
- Editor sends position in parent file
- System translates to virtual document
- Features work on translated position
- Results translate back to parent position
- Editor shows correct locations

#### Position Mapping Diagram

```
┌─────────────────────────────────────┐
│ Parent File: example.rho            │
│                                     │
│ 1: new metta in {                   │
│ 2:   @"#!metta                      │  ─┐
│ 3:     (= (factorial $n)            │   │ Extracted to
│ 4:        (if (== $n 0) 1           │   │ Virtual Doc
│ 5:           (* $n (factorial ..    │   │
│ 6:   "!(metta)                      │  ─┘
│ 7: }                                │
└─────────────────────────────────────┘
                  │
                  │ Position Mapping
                  ▼
┌─────────────────────────────────────┐
│ Virtual Doc: example.rho#vdoc:0     │
│                                     │
│ 1: (= (factorial $n)                │
│ 2:    (if (== $n 0) 1               │
│ 3:       (* $n (factorial ..        │
└─────────────────────────────────────┘
```

### 4. Symbol Resolution

The system uses a composable resolver architecture:

```
┌──────────────────────────────────────────────────────────┐
│           ComposableSymbolResolver                       │
│                                                          │
│  ┌────────────────────────────────────────────────────┐ │
│  │  Base Resolver: LexicalScopeResolver               │ │
│  │  • Searches scope chain (local → parent → global) │ │
│  │  • Returns candidate symbols                       │ │
│  └────────────────────────────────────────────────────┘ │
│                         │                                │
│                         ▼                                │
│  ┌────────────────────────────────────────────────────┐ │
│  │  Filters (Optional)                                │ │
│  │  • MettaPatternFilter: Refines by arity           │ │
│  │  • CustomFilter: Your language-specific logic     │ │
│  └────────────────────────────────────────────────────┘ │
│                         │                                │
│                         ▼                                │
│  ┌────────────────────────────────────────────────────┐ │
│  │  Fallback (Optional)                               │ │
│  │  • GlobalVirtualSymbolResolver                     │ │
│  │  • Searches across all virtual documents          │ │
│  └────────────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────────────┘
```

### 5. Cross-Document Linking

Symbols are linked across all virtual documents in the workspace:

```
┌─────────────────────────────────────────────────────────┐
│              Workspace Symbol Index                      │
│                                                          │
│  Language: "metta"                                       │
│  ├─ "factorial"                                         │
│  │  ├─ file:///robot.rho#vdoc:0  (Line 3)             │
│  │  └─ file:///utils.rho#vdoc:1  (Line 8)             │
│  │                                                      │
│  ├─ "navigate"                                          │
│  │  └─ file:///robot.rho#vdoc:0  (Line 12)            │
│  │                                                      │
│  └─ "get_neighbors"                                     │
│     ├─ file:///robot.rho#vdoc:0  (Line 5)             │
│     └─ file:///map.rho#vdoc:0    (Line 2)             │
└─────────────────────────────────────────────────────────┘
```

## Supported Features

### Go to Definition

Click on a symbol to jump to its definition, even if it's in a different file:

```rholang
new metta in {
  @"#!metta
  (= (factorial $n)    ← Definition is here
     (if (== $n 0) 1 (* $n (factorial (- $n 1)))))
                                 ↑
                          Ctrl+Click here jumps to definition
  "!(metta)
}
```

### Find References

Find all places where a symbol is used:

```rholang
new metta in {
  @"#!metta
  (= (helper $x) (* $x 2))         ← Definition

  (= (use_helper $a)
     (+ (helper $a) (helper 10)))  ← Two references
  "!(metta)
}
```

Press `Shift+F12` on `helper` to see:
- Definition at line 1
- Reference at line 4
- Reference at line 4 (second call)

### Rename Symbol

Rename a symbol across all occurrences:

1. Place cursor on symbol name
2. Press `F2`
3. Type new name
4. All occurrences update automatically

**Example:**
```rholang
Before:                          After (renamed calc → compute):
(= (calc $x) (* $x 2))          (= (compute $x) (* $x 2))
(calc 5)                        (compute 5)
(+ (calc 3) (calc 7))           (+ (compute 3) (compute 7))
```

### Hover Information

Hover over a symbol to see:
- Symbol type (function, variable, parameter)
- Definition location
- Scope information

```
┌─────────────────────────────────────┐
│ Hover on "factorial":               │
│                                     │
│ **factorial**                       │
│                                     │
│ *Function*                          │
│                                     │
│ Defined at line 3, column 6         │
│ Scope: global                       │
└─────────────────────────────────────┘
```

### Document Highlights

When you place the cursor on a symbol, all related occurrences are highlighted:

```rholang
new metta in {
  @"#!metta
  (= (factorial $n)                    ← Highlighted (definition)
     (if (== $n 0)
         1
         (* $n (factorial (- $n 1))))) ← Highlighted (reference)

  (factorial 5)                        ← Highlighted (reference)
  "!(metta)
}
```

## Adding Your Own Language

The system is designed to be extensible. Here's how to add support for a new embedded language:

### Step 1: Define Your Language IR

Implement the `SemanticNode` trait:

```rust
pub struct MyLanguageNode {
    pub kind: MyLanguageNodeKind,
    // ... other fields
}

impl SemanticNode for MyLanguageNode {
    fn children(&self) -> Vec<Arc<dyn SemanticNode + Send + Sync>> {
        // Return child nodes
    }

    fn position(&self) -> Option<Position> {
        // Return node position
    }
}
```

### Step 2: Create a Parser

```rust
pub fn parse_my_language(source: &str) -> Result<Arc<MyLanguageNode>, ParseError> {
    // Parse source code to IR
}
```

### Step 3: Build Symbol Tables

```rust
pub struct MyLanguageSymbolTableBuilder {
    // Symbol tracking
}

impl MyLanguageSymbolTableBuilder {
    pub fn build(&self, root: &MyLanguageNode) -> SymbolTable {
        // Build symbol table by traversing IR
    }
}
```

### Step 4: Integrate with Virtual Documents

```rust
// In VirtualDocument::parse_and_analyze()
match self.language.as_str() {
    "metta" => { /* existing MeTTa code */ },
    "mylang" => {
        let ir = parse_my_language(&self.content)?;
        let symbol_table = MyLanguageSymbolTableBuilder::new().build(&ir);
        // Store in VirtualDocument
    },
    _ => {}
}
```

### Step 5: Implement LSP Features

```rust
// In backend/mylang.rs
impl RholangBackend {
    pub(super) async fn goto_definition_mylang(
        &self,
        virtual_doc: &Arc<VirtualDocument>,
        virtual_position: LspPosition,
    ) -> LspResult<Option<GotoDefinitionResponse>> {
        // Use symbol table to find definition
        // Map position back to parent document
        // Return Location
    }
}
```

### Step 6: Add Language Detection

Update directive parser to recognize your language:

```rust
pub fn detect_language_directive(content: &str) -> Option<&str> {
    if content.starts_with("#!mylang") {
        Some("mylang")
    } else if content.starts_with("#!metta") {
        Some("metta")
    } else {
        None
    }
}
```

### Extension Flow Diagram

```
┌─────────────────────────────────────────────────────────────┐
│  Add New Language Support                                    │
│                                                               │
│  1. Directive          "#!mylang"                           │
│     └─ Registered in directive_parser.rs                     │
│                                                               │
│  2. Parser             parse_mylang(source)                 │
│     └─ Converts source → IR (implements SemanticNode)       │
│                                                               │
│  3. Symbol Table       MyLanguageSymbolTableBuilder          │
│     └─ Traverses IR, builds scopes and symbols              │
│                                                               │
│  4. LSP Handlers       goto_definition_mylang()             │
│     └─ Uses symbol table + position mapping                  │
│                                                               │
│  5. Integration        Update VirtualDocument.parse()        │
│     └─ Dispatch to your parser for "mylang"                 │
└─────────────────────────────────────────────────────────────┘
```

## Examples

### Example 1: Robot Navigation in MeTTa

```rholang
new metta, robotApi in {
  // Define navigation logic in MeTTa
  @"#!metta
  ;; Define locations and connections
  (= (connected room1 hallway) True)
  (= (connected hallway room2) True)
  (= (connected hallway kitchen) True)

  ;; Find neighbors for navigation
  (= (get_neighbors $location)
     (if (connected $location $next)
         $next
         (if (connected $next $location) $next Empty)))

  ;; Navigate from one location to another
  (= (navigate $from $to)
     (let $neighbors (get_neighbors $from)
       (if (contains $neighbors $to)
           (move $to)
           (find_path $from $to))))

  ;; Main navigation command
  (navigate room1 kitchen)
  "!(metta) |

  // Process results in Rholang
  for (@result <- metta) {
    @"Navigated to: "!(result)
  }
}
```

**Features in Action:**
- Hover over `get_neighbors` shows its definition
- `Ctrl+Click` on `navigate` jumps to its definition
- `F2` on `connected` renames all occurrences
- `Shift+F12` on `$location` shows all references

### Example 2: Multiple Embedded Regions

You can have multiple embedded language regions in the same file:

```rholang
new metta1, metta2, combiner in {
  // First MeTTa region: data definitions
  @"#!metta
  (= (user john) (age 30))
  (= (user alice) (age 25))
  "!(metta1) |

  // Second MeTTa region: data processing
  @"#!metta
  (= (process_users)
     (let $john_age (user john)
       (let $alice_age (user alice)
         (+ $john_age $alice_age))))
  "!(metta2) |

  // Combine results
  for (@users <- metta1; @result <- metta2) {
    @"Total age: "!(result)
  }
}
```

Each region gets its own virtual document:
- `file:///example.rho#vdoc:0` (metta1)
- `file:///example.rho#vdoc:1` (metta2)

### Example 3: Cross-File References

Symbols are linked across files:

**file1.rho:**
```rholang
new metta in {
  @"#!metta
  (= (shared_function $x) (* $x 2))
  "!(metta)
}
```

**file2.rho:**
```rholang
new metta in {
  @"#!metta
  ;; This reference can jump to file1.rho
  (shared_function 10)
  "!(metta)
}
```

The workspace symbol index enables cross-file navigation.

## Troubleshooting

### Language Directive Not Recognized

**Problem:** IDE features don't work in embedded code.

**Solution:** Ensure directive is on its own line at the start:

```rholang
❌ Wrong:  @"code here #!metta"!(metta)
✅ Right:  @"#!metta
           code here
           "!(metta)
```

### Position Mapping Issues

**Problem:** Go-to-definition jumps to wrong location.

**Solution:** Check that:
1. Embedded code starts immediately after directive
2. No extra whitespace before closing quote
3. File hasn't been edited without saving

### Cross-File Symbols Not Found

**Problem:** Can't find symbol defined in another file.

**Solution:**
1. Ensure workspace is fully indexed (wait a few seconds after opening)
2. Check that both files are in workspace scope
3. Verify symbol names match exactly

### Performance with Large Embedded Code

**Problem:** Slow response times with large MeTTa regions.

**Solution:**
1. Split large regions into multiple smaller ones
2. Use concatenation for related code segments
3. Consider extracting to separate `.metta` files if available

## Summary

The embedded language system provides:

✅ **Full IDE features** for embedded code
✅ **Language-agnostic architecture** supporting any language
✅ **Cross-document symbol linking** across files
✅ **Position-accurate mapping** between parent and virtual docs
✅ **Composable symbol resolution** with customizable filters
✅ **Extensible design** for adding new languages

For technical details, see:
- `docs/VIRTUAL_LANGUAGE_EXTENSION_SYSTEM.md` - Technical architecture
- `.claude/CLAUDE.md` - Developer guide
- `src/ir/symbol_resolution/` - Symbol resolution code

For questions or issues, please file an issue on GitHub.
