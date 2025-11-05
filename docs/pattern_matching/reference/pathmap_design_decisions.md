# Step 2: RholangPatternIndex with PathMap - Design Document

## Goal

Create a pattern matching index for Rholang contract signatures using PathMap, enabling:
1. Efficient contract signature matching for goto-definition
2. Overload resolution (multiple contracts with same name but different signatures)
3. Map key path navigation (e.g., `contract foo(@{"user": {"email": x}})`)

## Architecture Overview

```
RholangNode (Contract) → Extract Signature → MorkForm → MORK bytes
                                                            ↓
                                          PathMap (trie storage)
                                                            ↓
                                          Query with call site pattern
                                                            ↓
                                          Return matching definitions
```

## Data Structures

### 1. RholangPatternIndex

```rust
/// Pattern matching index for Rholang contracts using PathMap
pub struct RholangPatternIndex {
    /// PathMap storing contract patterns
    /// Path structure: ["contract_name", "param0_pattern", "param1_pattern", ...]
    /// Value: PatternMetadata (location, arity, etc.)
    patterns: PathMap<PatternMetadata>,

    /// MORK Space for symbol interning
    space: Arc<Space>,
}
```

### 2. PatternMetadata

```rust
/// Metadata about a contract pattern
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternMetadata {
    /// Location of the contract definition
    pub location: SymbolLocation,

    /// Contract name
    pub name: String,

    /// Number of parameters
    pub arity: usize,

    /// Parameter patterns (serialized MORK bytes)
    pub param_patterns: Vec<Vec<u8>>,

    /// Optional: Parameter names if available
    pub param_names: Option<Vec<String>>,
}
```

### 3. Path Structure for Patterns

PathMap paths will use this structure:

```
["contract", <name>, <param0_mork_bytes>, <param1_mork_bytes>, ...]
```

Example:
```rholang
contract echo(@x) = { x }
```

Path: `["contract", "echo", <mork_bytes_for_var_pattern_x>]`

Example with map pattern:
```rholang
contract processUser(@{"name": n, "email": e}) = { ... }
```

Path: `["contract", "processUser", <mork_bytes_for_map_pattern>]`

## Implementation Plan

### Phase 2A: Core Structure (30 min)

1. Create `src/ir/rholang_pattern_index.rs`
2. Define `RholangPatternIndex` struct
3. Define `PatternMetadata` struct
4. Implement basic construction

### Phase 2B: Pattern Extraction (45 min)

1. Extract contract signatures from RholangNode IR
2. Convert parameters to MorkForm
3. Serialize to MORK bytes
4. Build PathMap paths

### Phase 2C: Index Building with WriteZipper (45 min)

1. Use PathMap's WriteZipper to insert patterns
2. Store PatternMetadata at leaf nodes
3. Handle multiple contracts with same name (overloads)

### Phase 2D: Query with ReadZipper (45 min)

1. Extract call-site pattern from Send node
2. Convert to MORK bytes
3. Use PathMap's ReadZipper to query
4. Filter matches by arity and pattern compatibility

### Phase 2E: Integration with GlobalSymbolIndex (30 min)

1. Add `contract_patterns: RholangPatternIndex` to GlobalSymbolIndex
2. Populate during workspace indexing
3. Query during goto-definition

## API Design

### Building the Index

```rust
impl RholangPatternIndex {
    pub fn new() -> Self {
        Self {
            patterns: PathMap::new(),
            space: Arc::new(Space::new()),
        }
    }

    /// Extract and index a contract from IR
    pub fn index_contract(
        &mut self,
        contract_node: &RholangNode,
        location: SymbolLocation,
    ) -> Result<(), String> {
        // Extract contract name and parameters
        let (name, params) = extract_contract_signature(contract_node)?;

        // Convert parameters to MORK bytes
        let param_patterns: Vec<Vec<u8>> = params
            .iter()
            .map(|p| pattern_to_mork_bytes(p, &self.space))
            .collect::<Result<_, _>>()?;

        // Build path: ["contract", name, param0_bytes, param1_bytes, ...]
        let mut path = vec!["contract".as_bytes(), name.as_bytes()];
        path.extend(param_patterns.iter().map(|b| b.as_slice()));

        // Create metadata
        let metadata = PatternMetadata {
            location,
            name: name.clone(),
            arity: params.len(),
            param_patterns: param_patterns.clone(),
            param_names: extract_param_names(&params),
        };

        // Use WriteZipper to insert
        let mut wz = WriteZipper::new(&mut self.patterns);
        wz.descend_path(&path)?;
        wz.set_val(metadata);

        Ok(())
    }
}
```

### Querying the Index

```rust
impl RholangPatternIndex {
    /// Find contracts matching a call-site pattern
    pub fn query_call_site(
        &self,
        contract_name: &str,
        arguments: &[&RholangNode],
    ) -> Result<Vec<PatternMetadata>, String> {
        // Convert arguments to MORK bytes
        let arg_patterns: Vec<Vec<u8>> = arguments
            .iter()
            .map(|a| node_to_mork_bytes(a, &self.space))
            .collect::<Result<_, _>>()?;

        // Build query path
        let mut path = vec!["contract".as_bytes(), contract_name.as_bytes()];
        path.extend(arg_patterns.iter().map(|b| b.as_slice()));

        // Use ReadZipper to navigate
        let rz = ReadZipper::new(&self.patterns);

        // Try exact match first
        if let Ok(mut rz) = rz.descend_path(&path) {
            if let Some(metadata) = rz.val() {
                return Ok(vec![metadata.clone()]);
            }
        }

        // Fall back to pattern unification (using MORK's unify)
        // This handles variable patterns, wildcards, etc.
        self.unify_patterns(contract_name, &arg_patterns)
    }

    /// Use MORK's unify() for pattern matching
    fn unify_patterns(
        &self,
        contract_name: &str,
        arg_patterns: &[Vec<u8>],
    ) -> Result<Vec<PatternMetadata>, String> {
        let mut matches = Vec::new();

        // Get all contracts with this name
        let name_prefix = vec!["contract".as_bytes(), contract_name.as_bytes()];
        let rz = ReadZipper::new(&self.patterns);

        // Iterate over all patterns under this name
        for (stored_params, metadata) in rz.iter_prefix(&name_prefix) {
            // Check arity
            if stored_params.len() != arg_patterns.len() {
                continue;
            }

            // Try to unify each parameter
            let mut all_unify = true;
            for (stored, call_site) in stored_params.iter().zip(arg_patterns) {
                // Convert to MORK Expr
                let stored_expr = Expr { ptr: stored.as_ptr().cast_mut() };
                let call_expr = Expr { ptr: call_site.as_ptr().cast_mut() };

                // Use MORK's unify
                if !mork::unify(&self.space, stored_expr, call_expr) {
                    all_unify = false;
                    break;
                }
            }

            if all_unify {
                matches.push(metadata.clone());
            }
        }

        Ok(matches)
    }
}
```

## Integration with GlobalSymbolIndex

### Add to GlobalSymbolIndex

```rust
// In src/ir/global_index.rs
pub struct GlobalSymbolIndex {
    // ... existing fields ...

    /// Pattern-based contract index using PathMap
    pub contract_patterns: RholangPatternIndex,
}
```

### Populate During Indexing

```rust
// In symbol_index_builder.rs
impl SymbolIndexBuilder {
    fn visit_contract(&mut self, node: &RholangNode) {
        // Extract location
        let location = SymbolLocation {
            uri: self.current_uri.clone(),
            range: node_to_range(node),
        };

        // Index pattern
        self.global_index
            .contract_patterns
            .index_contract(node, location)
            .ok(); // Ignore errors for now
    }
}
```

### Query During Goto-Definition

```rust
// In unified_handlers.rs goto_definition
if let Some(send_node) = find_send_node_at_position(ir, position) {
    // Extract contract name and arguments
    let (name, args) = extract_call_site(&send_node)?;

    // Query pattern index
    let matches = global_index
        .contract_patterns
        .query_call_site(&name, &args)?;

    // Return first match (or all for overload resolution)
    if let Some(metadata) = matches.first() {
        return Ok(Some(location_to_lsp_location(&metadata.location)));
    }
}
```

## Example Usage

### Indexing

```rholang
contract echo(@x) = { x!(x) }
contract processUser(@{"name": n, "email": e}) = {
    stdout!(n)
}
```

Indexed as:
```
PathMap:
  ["contract", "echo", <var_pattern_x>] → PatternMetadata { location: ..., arity: 1 }
  ["contract", "processUser", <map_pattern>] → PatternMetadata { location: ..., arity: 1 }
```

### Querying

```rholang
echo!("hello")  // Call site
```

Query:
1. Extract: `name = "echo"`, `args = [Literal("hello")]`
2. Convert to MORK: `arg0 = <literal_string_hello>`
3. Query PathMap: `["contract", "echo", <literal_string_hello>]`
4. Unify with stored pattern `<var_pattern_x>`
5. Match! Return location

## Key Benefits of PathMap

1. **Efficient Prefix Matching** - Find all contracts with given name
2. **Trie Structure** - Shared prefixes (contract names) are stored once
3. **Zipper Navigation** - Fast traversal without copying
4. **MORK Integration** - Patterns stored as MORK bytes for unification

## Next Steps

1. Implement `RholangPatternIndex` structure
2. Add pattern extraction helpers
3. Integrate WriteZipper for indexing
4. Integrate ReadZipper for querying
5. Add to GlobalSymbolIndex
6. Test with complex patterns

## Time Estimate

- **Phase 2A**: 30 min (structures)
- **Phase 2B**: 45 min (extraction)
- **Phase 2C**: 45 min (indexing)
- **Phase 2D**: 45 min (querying)
- **Phase 2E**: 30 min (integration)

**Total**: ~3 hours for complete implementation
