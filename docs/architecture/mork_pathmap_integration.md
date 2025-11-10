# MORK and PathMap Integration for Pattern Matching

## Overview

This document describes how to correctly integrate and use MORK (MeTTa Optimal Reduction Kernel) and PathMap for pattern matching in the Rholang Language Server. It covers the threading model, API usage patterns, and performance considerations for parallel queries and updates.

## Table of Contents

1. [Threading Model](#threading-model)
2. [Core Components](#core-components)
3. [Integration Pattern](#integration-pattern)
4. [Parallel Operations](#parallel-operations)
5. [Performance Considerations](#performance-considerations)
6. [Common Pitfalls](#common-pitfalls)
7. [Testing](#testing)

---

## Threading Model

### Critical Understanding

MORK's threading model is built around two key types:

1. **`SharedMappingHandle`**: Thread-safe (`Send + Sync`)
   - Can be cloned and shared across threads
   - Provides symbol interning (string → u64 mapping)
   - Immutable once created

2. **`Space`**: NOT thread-safe (contains `Cell<u64>`)
   - Must be created per-thread or per-operation
   - Contains `btm: PathMap<T>`, `sm: SharedMappingHandle`, `mmaps: HashMap<...>`
   - Cloning `PathMap` and `SharedMappingHandle` is cheap

### Correct Threading Pattern

```rust
use mork::space::Space;
use mork_interning::{SharedMapping, SharedMappingHandle};
use pathmap::PathMap;
use std::collections::HashMap;

pub struct PatternMatcher {
    /// THREAD-SAFE: Can be cloned across threads
    shared_mapping: SharedMappingHandle,

    /// THREAD-SAFE: PathMap is immutable after cloning
    btm: PathMap<Metadata>,
}

impl PatternMatcher {
    pub fn new() -> Self {
        PatternMatcher {
            shared_mapping: SharedMapping::new(),
            btm: PathMap::new(),
        }
    }

    /// Update operation: Create thread-local Space
    pub fn add_pattern(&mut self, pattern: &Pattern) -> Result<(), String> {
        // Create thread-local Space for this operation
        let mut space = Space {
            btm: self.btm.clone(),        // Cheap clone (immutable structure sharing)
            sm: self.shared_mapping.clone(), // Cheap clone (Arc internally)
            mmaps: HashMap::new(),         // Empty for this operation
        };

        // Perform MORK operations using thread-local space
        space.load_all_sexpr_impl(pattern.as_bytes(), true)?;

        // Update shared PathMap with modified version
        self.btm = space.btm;

        Ok(())
    }

    /// Query operation: Create thread-local Space (read-only)
    pub fn query_pattern(&self, query: &Query) -> Result<Vec<Match>, String> {
        // Create thread-local Space for this operation
        let space = Space {
            btm: self.btm.clone(),        // Cheap clone
            sm: self.shared_mapping.clone(), // Cheap clone
            mmaps: HashMap::new(),         // Empty for this operation
        };

        // Perform MORK queries using thread-local space
        // space.btm remains unchanged (no need to copy back)
        let zipper = space.btm.read_zipper();
        // ... perform query ...

        Ok(results)
    }
}
```

### Why This Pattern?

From MORK's source code (`/home/dylon/Workspace/f1r3fly.io/MORK/interning/src/handle.rs`):

```rust
/// SAFETY: SharedMappingHandle is explicitly marked Send + Sync
unsafe impl Send for SharedMappingHandle {}
unsafe impl Sync for SharedMappingHandle {}
```

But `Space` contains `ArenaCompactTree` which uses `Cell<u64>`:

```rust
// PathMap's ArenaCompactTree
pub struct ArenaCompactTree<A: Allocator> {
    next_id: Cell<u64>,  // ❌ NOT Sync!
    // ...
}
```

**Result**: Cannot wrap `Space` in `Arc<Space>` or store in a struct that implements `SymbolResolver: Send + Sync`.

---

## Core Components

### 1. MettaPatternMatcher

**Location**: `src/ir/metta_pattern_matching.rs`

**Purpose**: Pattern matching for MeTTa function definitions in virtual documents

**Structure**:
```rust
pub struct MettaPatternMatcher {
    /// Thread-safe: SharedMappingHandle for symbol interning
    shared_mapping: SharedMappingHandle,

    /// Legacy maps (may be deprecated)
    pattern_to_def: HashMap<String, Vec<MettaDefinition>>,
    name_index: HashMap<String, Vec<MettaDefinition>>,

    /// MORK pattern index: pattern bytes → LSP locations
    pattern_locations: HashMap<Vec<u8>, Vec<Location>>,
}
```

**Threading Pattern**:
- Stores only `SharedMappingHandle`
- Creates thread-local `Space` in `add_definition()` and `query()` methods
- No `Arc<Space>` anywhere

**Example Usage**:
```rust
let mut matcher = MettaPatternMatcher::new();

// Add definition
matcher.add_definition(&function_name, &parameters, location)?;
// Internally: Creates Space, converts to MORK, stores in pattern_locations

// Query
let matches = matcher.query(&call_name, &arguments)?;
// Internally: Creates Space, converts args to MORK, queries pattern_locations
```

### 2. RholangPatternIndex

**Location**: `src/ir/rholang_pattern_index.rs`

**Purpose**: Trie-based pattern matching for Rholang contracts using PathMap

**Structure**:
```rust
pub struct RholangPatternIndex {
    /// PathMap trie: contract patterns → metadata
    patterns: PathMap<PatternMetadata>,

    /// Thread-safe: SharedMappingHandle for MORK conversion
    shared_mapping: SharedMappingHandle,
}

pub struct PatternMetadata {
    location: SymbolLocation,      // Where contract is defined
    name: String,                  // Contract name
    arity: usize,                  // Parameter count
    param_patterns: Vec<Vec<u8>>,  // MORK bytes for each param
    param_names: Option<Vec<String>>,
}
```

**PathMap Structure**:
```
Path: ["contract", <name_bytes>, <param0_mork>, <param1_mork>, ...]

Example:
root
└─ "contract"
   ├─ "echo" → VarPattern("x") → Metadata{line: 5, arity: 1}
   │         → Literal(42) → Metadata{line: 10, arity: 1}
   └─ "process" → Literal("start") → Metadata{line: 15, arity: 1}
                → Literal("stop") → Metadata{line: 20, arity: 1}
```

**Threading Pattern**:
```rust
impl RholangPatternIndex {
    pub fn new() -> Self {
        use mork_interning::SharedMapping;
        RholangPatternIndex {
            patterns: PathMap::new(),
            shared_mapping: SharedMapping::new(),
        }
    }

    pub fn index_contract(&mut self, contract: &RholangNode, location: SymbolLocation)
        -> Result<(), String>
    {
        // Create thread-local Space
        let space = Space {
            btm: PathMap::new(),
            sm: self.shared_mapping.clone(),
            mmaps: HashMap::new(),
        };

        // Convert contract formals to MORK patterns
        let mork_patterns = convert_formals_to_mork(&contract.parameters, &space)?;

        // Build path: ["contract", name, param0_mork, param1_mork, ...]
        let mut path = vec![b"contract".to_vec()];
        path.push(contract.name.as_bytes().to_vec());
        for pattern_bytes in mork_patterns {
            path.push(pattern_bytes);
        }

        // Insert into PathMap trie
        let metadata = PatternMetadata { location, name, arity, param_patterns, param_names };
        self.patterns.insert(&path, metadata)?;

        Ok(())
    }

    pub fn query_call_site(&self, contract_name: &str, arguments: &[&RholangNode])
        -> Result<Vec<PatternMetadata>, String>
    {
        // Create thread-local Space
        let space = Space {
            btm: PathMap::new(),
            sm: self.shared_mapping.clone(),
            mmaps: HashMap::new(),
        };

        // Convert call-site arguments to MORK values
        let mork_values = convert_args_to_mork(arguments, &space)?;

        // Build path: ["contract", name, arg0_mork, arg1_mork, ...]
        let mut path = vec![b"contract".to_vec()];
        path.push(contract_name.as_bytes().to_vec());
        for value_bytes in mork_values {
            path.push(value_bytes);
        }

        // Query PathMap trie
        let results = self.patterns.query(&path)?;

        Ok(results)
    }
}
```

### 3. RholangPatternMatcher (Legacy)

**Location**: `src/ir/pattern_matching.rs`

**Purpose**: Legacy MORK-based pattern matching (being phased out, replaced by RholangPatternIndex)

**Structure**:
```rust
pub struct RholangPatternMatcher {
    shared_mapping: SharedMappingHandle,
    btm: pathmap::PathMap<()>,  // Separate PathMap instance
}
```

**Status**: Used in `GlobalSymbolIndex.contract_definitions` but being replaced by `pattern_index`

---

## Integration Pattern

### Dependency Setup

**Cargo.toml**:
```toml
[dependencies]
mork = { git = "https://github.com/trueagi-io/MORK.git", branch = "main", features = ["interning"] }
mork-expr = { git = "https://github.com/trueagi-io/MORK.git", branch = "main" }
mork-frontend = { git = "https://github.com/trueagi-io/MORK.git", branch = "main" }
mork-interning = { git = "https://github.com/trueagi-io/MORK.git", branch = "main" }
pathmap = { git = "https://github.com/Adam-Vandervorst/PathMap.git", branch = "master", features = ["jemalloc", "arena_compact"] }

[patch.'https://github.com/trueagi-io/MORK.git']
mork = { path = "../MORK/kernel" }
mork-expr = { path = "../MORK/expr" }
mork-frontend = { path = "../MORK/frontend" }
mork-interning = { path = "../MORK/interning" }

[patch.'https://github.com/Adam-Vandervorst/PathMap.git']
pathmap = { path = "../PathMap" }
```

### Import Pattern

```rust
use mork::space::Space;
use mork_interning::{SharedMapping, SharedMappingHandle};
use mork_expr::{Expr, ExprZipper};
use mork_frontend::bytestring_parser::{Parser, Context};
use pathmap::PathMap;
use pathmap::zipper::*;  // For read_zipper(), descend_to_existing(), etc.
use std::collections::HashMap;
```

### Complete Example: Pattern Matcher with Threading

```rust
use mork::space::Space;
use mork_interning::{SharedMapping, SharedMappingHandle};
use pathmap::PathMap;
use std::collections::HashMap;
use std::sync::Arc;

pub struct ThreadSafePatternMatcher {
    shared_mapping: SharedMappingHandle,
    btm: PathMap<String>,  // Stores pattern → handler mappings
}

impl ThreadSafePatternMatcher {
    pub fn new() -> Self {
        ThreadSafePatternMatcher {
            shared_mapping: SharedMapping::new(),
            btm: PathMap::new(),
        }
    }

    /// Add a pattern (requires &mut self for PathMap modification)
    pub fn add_pattern(&mut self, pattern_str: &str, handler: String) -> Result<(), String> {
        // Create thread-local Space
        let mut space = Space {
            btm: self.btm.clone(),
            sm: self.shared_mapping.clone(),
            mmaps: HashMap::new(),
        };

        // Parse pattern using MORK
        let pattern_bytes = pattern_str.as_bytes();
        let mut parse_buffer = vec![0u8; 4096];
        let mut pdp = mork::space::ParDataParser::new(&space.sm);
        let mut ez = mork_expr::ExprZipper::new(mork_expr::Expr {
            ptr: parse_buffer.as_mut_ptr(),
        });
        let mut context = mork_frontend::bytestring_parser::Context::new(pattern_bytes);

        pdp.sexpr(&mut context, &mut ez)
            .map_err(|e| format!("Parse error: {:?}", e))?;

        // Get MORK Expr
        let pattern_expr = mork_expr::Expr {
            ptr: parse_buffer.as_ptr().cast_mut(),
        };

        // Convert to MORK bytes
        let mork_bytes = unsafe {
            pattern_expr.span()
                .as_ref()
                .ok_or("Expression has no span")?
                .to_vec()
        };

        // Insert into PathMap
        space.btm.insert(&[mork_bytes.as_slice()], handler)?;

        // Update shared PathMap
        self.btm = space.btm;

        Ok(())
    }

    /// Query patterns (read-only, can be called from multiple threads via &self)
    pub fn query(&self, query_str: &str) -> Result<Vec<String>, String> {
        // Create thread-local Space (read-only)
        let space = Space {
            btm: self.btm.clone(),
            sm: self.shared_mapping.clone(),
            mmaps: HashMap::new(),
        };

        // Parse query
        let query_bytes = query_str.as_bytes();
        let mut parse_buffer = vec![0u8; 4096];
        let mut pdp = mork::space::ParDataParser::new(&space.sm);
        let mut ez = mork_expr::ExprZipper::new(mork_expr::Expr {
            ptr: parse_buffer.as_mut_ptr(),
        });
        let mut context = mork_frontend::bytestring_parser::Context::new(query_bytes);

        pdp.sexpr(&mut context, &mut ez)
            .map_err(|e| format!("Parse error: {:?}", e))?;

        let query_expr = mork_expr::Expr {
            ptr: parse_buffer.as_ptr().cast_mut(),
        };

        // Convert to MORK bytes
        let mork_bytes = unsafe {
            query_expr.span()
                .as_ref()
                .ok_or("Expression has no span")?
                .to_vec()
        };

        // Query PathMap using zipper
        let mut zipper = space.btm.read_zipper();
        let mut results = Vec::new();

        if zipper.descend_to_existing(&mork_bytes) == mork_bytes.len() {
            // Found prefix, collect values
            while zipper.to_next_val() {
                if let Some(handler) = zipper.value() {
                    results.push(handler.clone());
                }
            }
        }

        Ok(results)
    }
}

// Can be safely shared across threads with Arc<RwLock<...>>
// because SharedMappingHandle is Send + Sync,
// and PathMap cloning is cheap
unsafe impl Send for ThreadSafePatternMatcher {}
unsafe impl Sync for ThreadSafePatternMatcher {}
```

---

## Parallel Operations

### Read-Heavy Workload (Multiple Queries)

For LSP goto-definition, we have many concurrent read queries:

```rust
use std::sync::Arc;
use parking_lot::RwLock;
use rayon::prelude::*;

pub struct GlobalPatternIndex {
    matcher: Arc<RwLock<RholangPatternIndex>>,
}

impl GlobalPatternIndex {
    pub fn new() -> Self {
        GlobalPatternIndex {
            matcher: Arc::new(RwLock::new(RholangPatternIndex::new())),
        }
    }

    /// Parallel queries across multiple contracts
    pub fn query_all_contracts(&self, queries: Vec<(String, Vec<RholangNode>)>)
        -> Vec<Result<Vec<PatternMetadata>, String>>
    {
        // Read lock held by each thread individually
        queries.par_iter().map(|(name, args)| {
            let matcher = self.matcher.read();
            let arg_refs: Vec<&RholangNode> = args.iter().collect();
            matcher.query_call_site(name, &arg_refs)
        }).collect()
    }

    /// Single query (typical goto-definition use case)
    pub fn query_contract(&self, name: &str, arguments: &[&RholangNode])
        -> Result<Vec<PatternMetadata>, String>
    {
        let matcher = self.matcher.read();

        // Each thread creates its own Space internally
        // SharedMappingHandle is cloned (cheap)
        // PathMap is cloned (cheap due to structural sharing)
        matcher.query_call_site(name, arguments)
    }
}
```

**Performance**:
- Query time: ~9µs per query (from benchmarks)
- PathMap clone: O(1) (structural sharing via Arc internally)
- SharedMappingHandle clone: O(1) (Arc wrapper)
- Multiple threads can query simultaneously with read lock

### Write-Heavy Workload (Workspace Indexing)

For initial workspace indexing, we index many contracts:

```rust
use rayon::prelude::*;

impl GlobalPatternIndex {
    /// Parallel indexing during workspace initialization
    pub fn index_workspace(&self, contracts: Vec<(RholangNode, SymbolLocation)>)
        -> Result<(), String>
    {
        // Collect all contract data first
        let mork_data: Vec<_> = contracts.par_iter()
            .map(|(contract, location)| {
                // Each thread creates its own Space for MORK conversion
                let space = Space {
                    btm: PathMap::new(),
                    sm: self.matcher.read().shared_mapping.clone(),
                    mmaps: HashMap::new(),
                };

                // Convert contract to MORK patterns
                let name = extract_contract_name(contract)?;
                let params = extract_parameters(contract)?;
                let mork_patterns = convert_formals_to_mork(&params, &space)?;

                Ok((name, mork_patterns, location.clone()))
            })
            .collect::<Result<Vec<_>, String>>()?;

        // Sequential insertion (PathMap modification requires exclusive access)
        let mut matcher = self.matcher.write();
        for (name, patterns, location) in mork_data {
            let metadata = PatternMetadata {
                location,
                name: name.clone(),
                arity: patterns.len(),
                param_patterns: patterns.clone(),
                param_names: None,
            };

            let mut path = vec![b"contract".to_vec()];
            path.push(name.as_bytes().to_vec());
            for pattern in patterns {
                path.push(pattern);
            }

            matcher.patterns.insert(&path, metadata)?;
        }

        Ok(())
    }
}
```

**Performance**:
- MORK conversion: ~1-3µs per argument (parallelizable)
- PathMap insertion: ~29µs per contract (sequential)
- Total for 100 contracts: ~2.9ms insertion + ~300µs conversion (parallel)

### Mixed Workload (Incremental Updates)

For document changes (add/remove contracts):

```rust
impl GlobalPatternIndex {
    /// Update single contract (LSP didChange)
    pub fn update_contract(&self, contract: &RholangNode, location: SymbolLocation)
        -> Result<(), String>
    {
        // Write lock for PathMap modification
        let mut matcher = self.matcher.write();

        // Create thread-local Space for MORK conversion
        let space = Space {
            btm: PathMap::new(),
            sm: matcher.shared_mapping.clone(),
            mmaps: HashMap::new(),
        };

        // Convert and insert
        matcher.index_contract(contract, location)
    }

    /// Remove contracts from a file (LSP didClose)
    pub fn remove_contracts_by_uri(&self, uri: &Url) -> Result<(), String> {
        // This requires PathMap traversal and deletion
        // Current limitation: PathMap doesn't support efficient deletion
        // Workaround: Rebuild index without deleted contracts

        let matcher = self.matcher.read();
        let all_contracts = matcher.all_contracts_except_uri(uri)?;
        drop(matcher);

        // Rebuild with filtered contracts
        let mut new_matcher = RholangPatternIndex::new();
        for (contract, location) in all_contracts {
            new_matcher.index_contract(&contract, location)?;
        }

        // Replace
        *self.matcher.write() = new_matcher;

        Ok(())
    }
}
```

---

## Performance Considerations

### MORK Conversion (Pattern vs Value)

**Critical**: Use correct conversion function based on context:

1. **Contract Formals** (patterns):
   ```rust
   rholang_pattern_to_mork(node: &RholangNode) -> Result<MorkForm>
   // Returns: MapPattern, VarPattern, WildcardPattern, etc.
   ```

2. **Call-Site Arguments** (values):
   ```rust
   rholang_node_to_mork(node: &RholangNode) -> Result<MorkForm>
   // Returns: Map, List, Literal, etc.
   ```

**Performance**:
- Pattern conversion: ~1-3µs per parameter
- Value conversion: ~1-3µs per argument
- Serialization to bytes: ~1-3µs per form
- Total: ~3-9µs per argument

### PathMap Operations

**Insertion**:
- Single contract (3 parameters): ~29µs
- Bulk insert (100 contracts): ~2.9ms
- Complexity: O(k) where k = path depth (typically 3-5)

**Query**:
- Single query (exact match): ~9µs
- Prefix scan (10 matches): ~90µs
- Complexity: O(k + m) where k = path depth, m = matches

**Clone**:
- PathMap clone: O(1) (structural sharing)
- Space clone: O(1) (SharedMappingHandle is Arc)

### Memory Usage

**Per-Thread Overhead**:
- `Space` instance: ~200 bytes
- `mmaps` HashMap: ~48 bytes (empty)
- Total: ~250 bytes per operation

**Shared Data**:
- `SharedMappingHandle`: One Arc<RwLock<SymbolMap>> for entire workspace
- `PathMap`: One trie for entire workspace, shared via structural sharing
- Memory efficient: No per-thread duplication of pattern data

### Optimization Tips

1. **Reuse Queries**: Cache common queries (e.g., stdlib contracts)
2. **Batch Indexing**: Group contract indexing to minimize lock contention
3. **Profile First**: Use `cargo bench` to identify bottlenecks before optimizing
4. **Prefer Read Operations**: Design for read-heavy workloads (goto-definition is 95% of LSP traffic)

---

## Common Pitfalls

### ❌ Mistake 1: Storing Space in Arc

```rust
// ❌ WRONG: Space is not Sync
pub struct BadPatternMatcher {
    space: Arc<Space>,  // Compile error: Cell<u64> is not Sync
}
```

**Fix**:
```rust
// ✅ CORRECT: Store thread-safe components only
pub struct GoodPatternMatcher {
    shared_mapping: SharedMappingHandle,  // Send + Sync
    btm: PathMap<T>,                      // Send + Sync (after clone)
}
```

### ❌ Mistake 2: Wrong MORK Conversion

```rust
// ❌ WRONG: Using value conversion for patterns
let contract_params = contract.formals.iter()
    .map(|param| rholang_node_to_mork(param))  // Returns Map, not MapPattern!
    .collect();
```

**Fix**:
```rust
// ✅ CORRECT: Use pattern conversion for formals
let contract_params = contract.formals.iter()
    .map(|param| rholang_pattern_to_mork(param))  // Returns MapPattern
    .collect();
```

### ❌ Mistake 3: Forgetting to Update PathMap After Mutation

```rust
// ❌ WRONG: Local Space modifications not persisted
pub fn add_pattern(&mut self, pattern: &Pattern) {
    let mut space = Space {
        btm: self.btm.clone(),
        sm: self.shared_mapping.clone(),
        mmaps: HashMap::new(),
    };

    space.load_all_sexpr_impl(pattern.as_bytes(), true)?;
    // Missing: self.btm = space.btm;
}
```

**Fix**:
```rust
// ✅ CORRECT: Update shared PathMap
pub fn add_pattern(&mut self, pattern: &Pattern) {
    let mut space = Space { /* ... */ };
    space.load_all_sexpr_impl(pattern.as_bytes(), true)?;
    self.btm = space.btm;  // Persist modifications
}
```

### ❌ Mistake 4: Not Using Zipper Correctly

```rust
// ❌ WRONG: Iterating without prefix filtering
let mut zipper = space.btm.read_zipper();
while zipper.to_next_val() {  // Iterates ENTIRE trie
    // Check every single entry...
}
```

**Fix**:
```rust
// ✅ CORRECT: Navigate to prefix first
let mut zipper = space.btm.read_zipper();
if zipper.descend_to_existing(&prefix_bytes) == prefix_bytes.len() {
    // Now only iterate matching subtree
    while zipper.to_next_val() {
        // Only entries with matching prefix
    }
}
```

---

## Testing

### Unit Tests

**Test MORK Conversion**:
```rust
#[test]
fn test_pattern_vs_value_conversion() {
    let formal = RholangNode::Ground(GroundNode::Map { /* ... */ });
    let argument = RholangNode::Ground(GroundNode::Map { /* ... */ });

    let pattern = rholang_pattern_to_mork(&formal).unwrap();
    let value = rholang_node_to_mork(&argument).unwrap();

    match pattern {
        MorkForm::MapPattern(_) => {}, // ✓ Correct
        _ => panic!("Should be MapPattern"),
    }

    match value {
        MorkForm::Map(_) => {}, // ✓ Correct
        _ => panic!("Should be Map"),
    }
}
```

**Test Thread Safety**:
```rust
#[test]
fn test_concurrent_queries() {
    use std::sync::Arc;
    use std::thread;

    let matcher = Arc::new(RwLock::new(RholangPatternIndex::new()));

    // Index some contracts
    {
        let mut m = matcher.write();
        for i in 0..100 {
            m.index_contract(&contract, location)?;
        }
    }

    // Spawn 10 threads, each doing 100 queries
    let handles: Vec<_> = (0..10).map(|_| {
        let matcher = Arc::clone(&matcher);
        thread::spawn(move || {
            for _ in 0..100 {
                let m = matcher.read();
                let results = m.query_call_site("test", &args);
                assert!(results.is_ok());
            }
        })
    }).collect();

    for handle in handles {
        handle.join().unwrap();
    }
}
```

### Integration Tests

**Location**: `tests/test_pattern_matching_performance.rs`

**Benchmarks**:
- MORK serialization: 1-3µs per operation
- PathMap insertion: 29µs per contract
- PathMap lookup: 9µs per query
- Multi-arg patterns: 74µs insertion, 29µs lookup

**Run**:
```bash
cargo test --test test_pattern_matching_performance -- --nocapture
```

### Debugging

**Enable Logging**:
```bash
RUST_LOG=rholang_language_server::ir::rholang_pattern_index=trace cargo run
```

**Check MORK Bytes**:
```rust
let mork_bytes = mork_form.to_mork_bytes(&space)?;
tracing::debug!("MORK bytes: {:?}", mork_bytes);
```

**Verify PathMap Structure**:
```rust
let zipper = self.patterns.read_zipper();
while zipper.to_next_val() {
    let path = zipper.path();
    tracing::debug!("Path: {:?}", path);
}
```

---

## Summary

**Threading Model**:
- ✅ Store `SharedMappingHandle` + `PathMap` (both thread-safe after clone)
- ✅ Create thread-local `Space` per operation
- ❌ Never store `Arc<Space>` (Cell<u64> is not Sync)

**MORK Conversion**:
- ✅ Use `rholang_pattern_to_mork()` for contract formals
- ✅ Use `rholang_node_to_mork()` for call-site arguments
- ❌ Never mix pattern and value conversions

**PathMap Operations**:
- ✅ Navigate to prefix before iteration (`descend_to_existing()`)
- ✅ Update `self.btm` after mutations (`self.btm = space.btm`)
- ❌ Never iterate entire trie without filtering

**Parallelization**:
- ✅ Parallel queries: Each thread creates own Space
- ✅ Parallel MORK conversion: Separate Space per thread
- ❌ PathMap mutations must be sequential (write lock)

**Performance**:
- MORK conversion: ~1-3µs per argument
- PathMap insertion: ~29µs per contract
- PathMap query: ~9µs per lookup
- Target: <200ms total LSP response time ✓
