# Rholang Language Server

LSP-based Language Server for Rholang (Language Server Protocol).

## Dependencies

Clone [f1r3fly](https://github.com/F1R3FLY-io/f1r3fly) and compile `rnode`:

```shell
git clone https://github.com/F1R3FLY-io/f1r3fly.git
cd f1r3fly
export SBT_OPTS="-Xmx4g -Xss2m -Dsbt.supershell=false"
sbt clean bnfc:generate compile stage
# Optional: Add `rnode` to your $PATH:
export PATH="$PWD/node/target/universal/stage/bin:$PATH"
```

## Installing

Clone [rholang-language-server](https://github.com/F1R3FLY-io/rholang-language-server) and compile it:

```shell
git clone https://github.com/F1R3FLY-io/rholang-language-server.git
cd rholang-language-server
cargo build
# Optional: Add `rholang-language-server` to your $PATH:
export PATH="$PWD/target/debug:$PATH"
```

## Testing

1. From one terminal, launch RNode in standalone mode: `rnode run -s`.
2. From another terminal, `cd` into `rholang-language-server` root and run: `cargo test`.
   - This spawns `rholang-language-server` and runs tests against it, communicating with the standalone RNode.

## Intermediate Representation (IR) Design

The Rholang Language Server employs an Intermediate Representation (IR) to represent parsed Rholang code, designed with **immutability** and **persistence** as core properties:

- **Immutability**:
  - Once created, the IR tree cannot be modified. This ensures **thread safety** by eliminating data races in concurrent operations and maintains **consistency** across transformations, as original nodes remain unchanged.
  - **Why it matters**: Simplifies reasoning about code transformations (e.g., optimizations), making the system more predictable and debuggable.

- **Persistence**:
  - Utilizes structural sharing to allow new IR versions to reuse unchanged subtrees, reducing memory usage.
  - Enables **versioning** for features like undo/redo or transformation history with minimal overhead, and enhances **efficiency** by avoiding duplication of large tree segments.
  - **Why it matters**: Supports efficient handling of large codebases and facilitates backtracking or analysis without performance penalties.

## Symbol Table and Inverted Index

The language server now includes a hierarchical symbol table and inverted index, built as part of the IR pipeline:

- **Symbol Table**: Manages scoping for `new`, `let`, `contract`, `input`, `case`, and `branch` nodes. Symbols are stored with their type, declaration, and definition locations, accessible via node metadata.
- **Inverted Index**: Tracks all usage locations of symbols, enabling features like semantic renaming.
- **Usage**: Use `SymbolTableBuilder` in the pipeline to build these structures. Query them with `find_node_at_position` to access symbol information at any source position.

Example usage in the pipeline:

```rust
let mut pipeline = Pipeline::new();
pipeline.add_transform(Transform {
    id: "symbol_table".to_string(),
    dependencies: vec![],
    visitor: Arc::new(SymbolTableBuilder::new(ir.clone())),
});
let (transformed, inverted_index) = builder.build();
```

See `src/ir/transforms/symbol_table_builder.rs` for details.

## Workspace Indexing and Symbol Management

The Rholang Language Server now supports:

- **Dynamic Metadata**: The `metadata` field in IR nodes now uses a `HashMap` for flexible storage of version, symbol tables, and more.
- **Workspace Indexing**: On initialization, all `.rho` files in the workspace are indexed, with parsed IR, symbol tables, and inverted indices cached.
- **File Watching**: Changes to `.rho` files trigger reindexing, keeping caches current across platforms.
- **Cross-File Linking**: Symbols are linked across files, updating inverted indices for cross-references.
- **Explicit Document Handling**: Opened documents override on-disk versions, with Tree-Sitter enabling incremental updates.

### Usage

- **Initialization**: Provide a `rootUri` in `InitializeParams` to trigger workspace indexing.
- **Cache Access**: Access cached data via `RholangBackend::workspace`.
- **Debugging**: Enable `RUST_LOG=debug` for indexing and linking logs.

See `src/backend.rs` for implementation details.

### Benefits

- **Thread Safety**: Safe concurrent parsing and transformation.
- **Consistency**: Predictable transformation outcomes.
- **Versioning**: Track changes or revert transformations easily.
- **Efficiency**: Memory and performance optimization via structural sharing.
- **Facilitates Operations**: Ideal for optimization, analysis, and formatting tasks, as transformations produce new trees without altering originals.

For example, transforming `not not true` to `true` creates a new IR tree, preserving the original for reference or rollback, with shared subtrees minimizing resource use.

## Additional Considerations

- **Performance**: The `rholang-parser` leverages Tree-Sitter, maintaining consistent performance. Local parsing is lightweight compared to RNode communication.
- **IR Integration**: The `parse_to_ir` function in `src/tree_sitter.rs` uses Tree-Sitter directly:
  ```rust
  pub fn parse_to_ir<'a>(tree: &'a Tree, source_code: &'a str) -> Arc<Node<'a>> {
      debug!("Parsing Tree-Sitter tree into IR for source: {}", source_code);
      convert_ts_node_to_ir(tree.root_node(), source_code)
  }
  ```
  Modification is optional unless additional parser features (e.g., custom error handling) are needed.
- **Logging**: Debug-level logging is optional and controlled via `RUST_LOG=debug`, aiding troubleshooting without overwhelming output.

## Conclusion

This integration enhances the Rholang Language Server with local syntax validation via `rholang-parser`, improving responsiveness and error reporting. The immutable, persistent IR design ensures robust, efficient transformations, maintaining readability and maintainability through modular design and concise logging.
