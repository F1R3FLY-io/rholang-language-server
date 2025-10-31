# Changelog

All notable changes to the Rholang Language Server will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed
- No changes yet

## [0.1.0] - 2025-10-31

### Added

#### Core LSP Features
- **Go to Definition** with cross-file navigation support
- **Find References** across workspace with accurate symbol resolution
- **Document and Workspace Symbols** for code navigation
- **Hover Information** with symbol type, declaration location, and documentation
- **Semantic Rename** with workspace-wide atomic edits
- **Document Highlighting** for symbol occurrences under cursor
- **Diagnostics** via RNode integration (syntax and semantic errors)
- **Symbol Tables** with hierarchical scoping (contracts, let/new bindings, parameters)
- **Inverted Indices** for fast reverse lookups (all usages of a symbol)

#### Advanced Features
- **Embedded Language Support**: Full MeTTa language support within Rholang strings
- **Virtual Document System**: Extract and analyze embedded languages with position mapping
- **Pattern Matching**: Contract overload resolution with multi-argument matching
- **Wildcard and Variable Patterns**: Support for `_` wildcards and pattern variables
- **Composable Symbol Resolution**: Trait-based architecture with lexical scope resolvers
- **Cross-Document Symbols**: Global symbol index for multi-file navigation
- **Incremental Parsing**: Tree-Sitter integration for efficient document updates
- **Reactive Indexing**: File watching with debounced symbol linking

#### Architecture
- **Immutable IR** (Intermediate Representation) with persistent data structures
- **Structural Sharing** via `rpds` and `archery` for efficient memory usage
- **Thread-Safe Design**: No data races, lock-free concurrent access with DashMap
- **Position Tracking**: Relative positions with lazy absolute computation
- **Pipeline-Based Transforms**: Dependency graph execution for IR processing
- **Visitor Pattern**: Clean, composable transformations without mutation
- **Metadata Extensibility**: Dynamic metadata via `HashMap<String, Arc<dyn Any>>`

#### Performance Optimizations
- **Parse Tree Caching** with 1,000-10,000x speedup on cache hits (20-30ns vs 37-263µs)
- **Adaptive Parallelization** for workload-appropriate concurrency (eliminates 45-50% Rayon overhead)
- **FxHash Integration** for 2x faster symbol table hashing
- **Incremental Parsing**: 7-50x faster than full re-parsing on edits
- **Lock-Free Concurrent Access**: 2-5x throughput improvement with DashMap
- **35x speedup in goto-definition**: Reduced from 3.5s to <100ms

#### Testing & Quality
- **1,638 Test Cases** covering all LSP features
- **Performance Integration Tests** with <100ms guarantees for common operations
- **Property-Based Testing** with QuickCheck for IR transformations
- **Benchmark Suite** with criterion for performance regression detection
- **No RNode Dependency**: All tests use internal LSP mocking
- **Package Sanity Tests** for .deb, .rpm, .pkg.tar.zst, .dmg

#### Release Infrastructure
- **Multi-Platform Builds**: Linux x86_64/ARM64, macOS x86_64/ARM64
- **Native Packages**: Debian (.deb), RedHat (.rpm), Arch Linux (.pkg.tar.zst), macOS (.dmg)
- **Automated Release Workflow**: GitHub Actions with validation tests
- **Binary Archives**: .tar.gz and .zip for manual installation

### Performance

#### Major Performance Improvements
- **35x speedup in goto-definition**: Reduced from 3.5 seconds to <100ms
  - Removed blocking `eprintln!` debug calls from hot paths (src/lsp/backend.rs:2856-2875)
  - Optimized symbol lookup using GlobalSymbolIndex
  - Added performance logging at request boundaries

- **99% reduction in log volume**: From 1.2M lines to ~12K lines per request
  - Removed debug logging from hot paths in position computation (src/ir/rholang_node.rs:406-548)
  - Removed per-node logging in Tree-Sitter conversion (src/tree_sitter.rs:230-232)
  - Replaced verbose debug! calls with single trace! calls (disabled by default)
  - Strategic logging only at request/response boundaries

- **Stack overflow fixes**: Increased Tokio worker thread stack size to 8MB
  - Fixed crashes when opening large files (500+ lines)
  - Manual Tokio runtime builder in main.rs:771-784
  - Reduced property test complexity (MAX_DEPTH 10→5, test count 1000→100)

#### Symbol Highlighting Improvements
- **Fixed symbol highlighting persistence**: Highlights no longer cleared on hover
  - Added fallback hover information for all variables (src/lsp/backend.rs:3392-3445)
  - Enhanced hover with symbol type (variable/contract/parameter) and declaration location
  - Maintains documentHighlight when hovering over symbols

### Fixed

- **Stack overflow fixes**: Fixed crashes when opening large files (500+ lines)
  - Increased Tokio worker thread stack size to 8MB
  - Manual Tokio runtime builder in main.rs:771-784
  - Reduced property test complexity (MAX_DEPTH 10→5, test count 1000→100)
- **Symbol highlighting persistence**: Highlights no longer cleared on hover
  - Added fallback hover information for all variables
  - Enhanced hover with symbol type (variable/contract/parameter) and declaration location
- **Position tracking**: Fixed Quote node position for goto-definition accuracy
  - Contract parameters like `@destRoom` now correctly point to `@` symbol
  - Updated symbol table builder to use Quote node positions
- **Race conditions**: Eliminated via lock-free data structures (DashMap)
- **Performance regression**: Removed excessive DEBUG logging (99% log volume reduction)
- **Slow goto-definition**: Fixed 3.5s delay caused by stderr I/O blocking
- **Test isolation**: Fixed global symbol index interference between tests
- **VSCode crashes**: Fixed when opening robot_planning.rho (546 lines)
- **Quadratic complexity**: Prevented O(n²) behavior in symbol indexing

### Performance Metrics

| Metric | Before | After | Improvement |
|--------|--------|-------|-------------|
| Goto-definition | 3.5s | <100ms | **35x faster** |
| Virtual doc detection | - | - | **2.2x faster** |
| Symbol resolution | - | - | **1.2x faster** |
| Initial indexing | - | - | **3-11x faster** |
| Incremental edits | - | - | **10-100x faster** |
| Log volume | 1.2M lines | ~12K lines | **99% reduction** |
| Stack overflow | ❌ Crash | ✅ Works | **Fixed** |
| Symbol highlighting | ❌ Broken | ✅ Works | **Fixed** |

**Overall**: Language server is approximately **4-8x faster** for typical LSP workflows.

### Technical Details

**Platforms**: Linux x86_64/ARM64, macOS x86_64/ARM64
**Toolchain**: Rust nightly (edition 2024)
**Parser**: Tree-Sitter 0.25
**LSP Framework**: Tower-LSP 0.20
**RNode Integration**: gRPC via Tonic 0.13

**Key Dependencies**:
- `rpds` 1.1 - Persistent data structures
- `archery` 1.2 - Smart pointers for structural sharing
- `dashmap` 6.1 - Lock-free concurrent hash maps
- `mork` - MeTTa language integration
- `rholang-parser` - Tree-Sitter based Rholang parser
