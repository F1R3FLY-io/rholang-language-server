// Rholang IR Node Module
//
// This module provides the core intermediate representation (IR) for Rholang code.
// It has been split into focused submodules for better maintainability:
//
// - node_types: Core RholangNode enum and supporting type definitions
// - position_tracking: Position computation and node lookup utilities
// - node_operations: Pattern matching, contract matching, and collection functions
// - node_impl: Trait implementations (PartialEq, Ord, SemanticNode, etc.)

pub mod node_types;
pub mod position_tracking;
pub mod node_operations;
pub mod node_impl;

// Re-export all public items for backward compatibility
pub use node_types::*;
pub use position_tracking::{compute_absolute_positions, compute_end_position, find_node_at_position, find_node_at_position_with_path};
pub use node_operations::{match_pat, match_contract, collect_contracts, collect_calls};

// Note: node_impl provides trait implementations and doesn't need explicit re-exports
// as the traits are implemented on types from node_types
