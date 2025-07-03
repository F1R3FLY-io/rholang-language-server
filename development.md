# Developing the Rholang Language Server

Instructions for developing the `rholang-language-server`, an LSP-based server for Rholang, including setup, contribution guidelines, and IR pipeline details.

## Prerequisites

- **Rust**: Install via [rustup](https://rustup.rs/).
- **Dependencies**: Ensure `Cargo.toml` lists all required crates.
- **RNode**: Clone and compile from [f1r3fly](https://github.com/F1R3FLY-io/f1r3fly) per `README.md`.

## Setup

1. **Clone the Repository**:
   ```bash
   git clone https://github.com/F1R3FLY-io/rholang-language-server.git
   cd rholang-language-server
   ```

2. **Build the Project**:
   ```bash
   cargo build
   ```

3. **Run Tests**:
   - Start RNode: `rnode run -s`.
   - Run tests: `cargo test`.

## Contributing

- **Code Style**: Adhere to Rust conventions (`cargo fmt`, `cargo clippy`).
- **Commits**: Use clear messages (e.g., "feat(ir): enhance persistence").
- **Tests**: Update or add tests in `tests/` for new features or fixes.

## IR Pipeline Design

The IR pipeline (`src/ir/pipeline.rs`) transforms and analyzes Rholang code with an immutable, persistent design.

### Key Properties

- **Immutability**:
  - Nodes are unmodifiable post-creation, ensuring **thread safety** (no data races) and **consistency** (unchanged originals during transformations).
  - Transformations produce new trees, preserving the original structure.
  - **Purpose**: Simplifies debugging and reasoning about transformations, crucial for reliable optimization and analysis.

- **Persistence**:
  - Structural sharing reuses unchanged subtrees across versions, reducing memory footprint.
  - Enables **versioning** (e.g., transformation history, undo/redo) and **efficiency** (e.g., simplifying `not not true` to `true` shares unrelated nodes).
  - **Purpose**: Supports efficient large-scale code handling and feature-rich development tools.

### How It Works

1. **Parsing**: `src/tree_sitter.rs` converts code to an IR tree via Tree-Sitter.
2. **Transformation**: Visitors (e.g., `SimplifyDoubleUnary`) produce new immutable versions.
3. **Pipeline**: `Pipeline::apply` chains transformations, preserving intermediate states.

### Example

```rust
let code = "not not true";
let tree = parse_code(code);
let ir = parse_to_ir(&tree, code);
let simplified = SimplifyDoubleUnary.visit_node(&ir); // Produces "true"
```

The original `ir` persists, sharing structure with `simplified`, optimizing memory and enabling versioning.

### Scenarios

- **Thread Safety**: Concurrent operations without synchronization issues.
- **Debugging**: Inspect intermediate states via persistence (e.g., log `format(&ir, true)`).
- **Optimization**: Simplifies expressions (e.g., `not not P` → `P`) efficiently.
- **Analysis**: Versioning aids in tracking changes or reverting steps.

## Debugging the Service

- **Logging**: Use `tracing` crate with `RUST_LOG=debug`:
  ```bash
  RUST_LOG=debug cargo run
  ```
- **Breakpoints**: Set in IDEs (e.g., VSCode) in `src/main.rs`.
- **Client-Server**: Simulate requests with `curl`:
  ```bash
  curl -X POST -H "Content-Type: application/json" -d '{"jsonrpc": "2.0", "method": "initialize", "id": 1}' http://localhost:8080
  ```

## Debugging the IR Pipeline

- **Versioning**: Check `metadata.version`:
  ```rust
  if let Some(metadata) = node.metadata() {
      println!("Version: {}", metadata.version);
  }
  ```
- **Pretty Printing**: Use `PrettyPrinter` for readable output:
  ```rust
  let formatted = format(&ir, true).unwrap();
  println!("IR: {}", formatted);
  ```
- **Intermediate States**: Capture states with logging:
  ```rust
  let intermediate = visitor.visit_node(&ir);
  debug!("Intermediate IR: {}", format(&intermediate, true).unwrap());
  ```

## IR Pipeline: Analysis and Transformations

Transformations are implemented as visitors:

- **Optimizations**: E.g., `SimplifyDoubleUnary` removes double negations.
- **Metadata Updates**: E.g., `IncrementVersion` tracks transformation steps.
- **Rewrites**: Non-optimizing changes like variable renaming.

`Pipeline::apply` ensures correct execution order based on dependencies.

## Debugging Tips

- **Logging**: Use `debug!` for IR states.
- **Pretty Printing**: Format IR with `format(&ir, true)`.
- **Tests**: Use `cargo test` with `quickcheck` for property-based testing.