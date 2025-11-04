//! Type extraction and constraint checking for Rholang patterns
//!
//! This module provides utilities for extracting type constraints from Rholang patterns
//! and checking if arguments satisfy those constraints.
//!
//! # Pattern Conjunction Syntax
//!
//! Rholang supports pattern conjunctions with type annotations:
//! ```ignore
//! contract foo(@{x /\ Int}, @{y /\ String}) = { ... }
//! ```
//!
//! # Architecture
//!
//! - **TypeConstraint**: Represents a type constraint extracted from a pattern
//! - **TypeExtractor**: Extracts type constraints from RholangNode patterns
//! - **TypeChecker**: Checks if values satisfy type constraints
//!
//! # Phase
//! Part of Phase 3: Type-Based Matching

use crate::ir::rholang_node::RholangNode;
use std::sync::Arc;
use std::collections::HashMap;

/// Type constraint extracted from a Rholang pattern
///
/// Represents the type information from pattern conjunctions like `@{x /\ Int}`.
///
/// # Examples
///
/// ```ignore
/// TypeConstraint::Simple("Int".to_string())     // @{x /\ Int}
/// TypeConstraint::Simple("String".to_string())  // @{y /\ String}
/// TypeConstraint::Simple("Bool".to_string())    // @{flag /\ Bool}
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeConstraint {
    /// Simple type constraint (e.g., Int, String, Bool)
    Simple(String),

    /// Any type - no constraint
    Any,

    /// Compound type constraint (for future extension)
    /// e.g., List<Int>, Map<String, Int>
    Compound {
        base: String,
        params: Vec<TypeConstraint>,
    },
}

impl TypeConstraint {
    /// Check if this constraint matches another constraint
    ///
    /// # Arguments
    /// - `other`: The constraint to check against
    ///
    /// # Returns
    /// - `true` if the constraints are compatible
    /// - `false` otherwise
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let int_constraint = TypeConstraint::Simple("Int".to_string());
    /// let any_constraint = TypeConstraint::Any;
    ///
    /// assert!(any_constraint.matches(&int_constraint));  // Any matches anything
    /// assert!(int_constraint.matches(&int_constraint));   // Same types match
    /// ```
    pub fn matches(&self, other: &TypeConstraint) -> bool {
        match (self, other) {
            // Any matches anything
            (TypeConstraint::Any, _) | (_, TypeConstraint::Any) => true,

            // Simple types must be exactly equal
            (TypeConstraint::Simple(a), TypeConstraint::Simple(b)) => a == b,

            // Compound types must have matching base and parameters
            (
                TypeConstraint::Compound {
                    base: base_a,
                    params: params_a,
                },
                TypeConstraint::Compound {
                    base: base_b,
                    params: params_b,
                },
            ) => {
                base_a == base_b
                    && params_a.len() == params_b.len()
                    && params_a
                        .iter()
                        .zip(params_b.iter())
                        .all(|(a, b)| a.matches(b))
            }

            // Different constraint types don't match
            _ => false,
        }
    }
}

/// Type extractor for Rholang patterns
///
/// Extracts type constraints from pattern conjunctions in contract definitions.
///
/// # Example
///
/// ```ignore
/// let extractor = TypeExtractor::new();
/// let pattern = // ... RholangNode representing @{x /\ Int}
/// let constraint = extractor.extract_from_pattern(&pattern);
/// ```
#[derive(Debug)]
pub struct TypeExtractor {
    /// Cache of extracted type constraints
    cache: HashMap<String, TypeConstraint>,
}

impl TypeExtractor {
    /// Create a new type extractor with empty cache
    pub fn new() -> Self {
        TypeExtractor {
            cache: HashMap::new(),
        }
    }

    /// Extract type constraint from a Rholang pattern node
    ///
    /// Handles pattern conjunctions like `@{x /\ Int}` and extracts the type portion.
    ///
    /// # Arguments
    /// - `pattern`: The pattern node to extract from
    ///
    /// # Returns
    /// - `Some(TypeConstraint)` if a type constraint was found
    /// - `None` if the pattern has no type constraint
    ///
    /// # Pattern Types
    ///
    /// 1. **Simple patterns** (no type): `@x`, `_`, `@"literal"` → `None`
    /// 2. **Type conjunctions**: `@{x /\ Int}` → `Some(TypeConstraint::Simple("Int"))`
    /// 3. **Complex patterns**: Future extension for compound types
    ///
    /// # Phase
    /// Phase 3.3: extract_type_from_pattern() implementation
    pub fn extract_from_pattern(&mut self, pattern: &Arc<RholangNode>) -> Option<TypeConstraint> {
        match &**pattern {
            // Variable without type constraint
            RholangNode::Var { .. } => None,

            // Wildcard without type constraint
            RholangNode::Wildcard { .. } => None,

            // Quote: check for pattern conjunction inside
            RholangNode::Quote { quotable, .. } => self.extract_from_pattern(quotable),

            // TODO: ConnPat: Pattern conjunction `@{x /\ Type}`
            // When pattern conjunctions are added to RholangNode AST,
            // extract type constraints here.
            // For now, pattern conjunctions are not yet implemented in the parser.

            // Other patterns have no type constraints
            _ => None,
        }
    }

    /// Extract type information from a node representing a type expression
    ///
    /// # Arguments
    /// - `type_node`: Node representing the type (e.g., the "Int" in `@{x /\ Int}`)
    ///
    /// # Returns
    /// - `Some(TypeConstraint)` if the type could be extracted
    /// - `None` if the node doesn't represent a recognizable type
    fn extract_type_from_node(&mut self, type_node: &Arc<RholangNode>) -> Option<TypeConstraint> {
        match &**type_node {
            // Simple type name: Var representing a type like "Int", "String", "Bool"
            RholangNode::Var { name, .. } => {
                // Check cache first
                if let Some(cached) = self.cache.get(name) {
                    return Some(cached.clone());
                }

                // Create new constraint and cache it
                let constraint = TypeConstraint::Simple(name.clone());
                self.cache.insert(name.clone(), constraint.clone());
                Some(constraint)
            }

            // Quote: unwrap and extract from inner node
            RholangNode::Quote { quotable, .. } => self.extract_type_from_node(quotable),

            // Other node types: not recognized as types
            _ => None,
        }
    }

    /// Clear the cache (useful for testing or when processing many files)
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    /// Get the number of cached type constraints
    pub fn cache_size(&self) -> usize {
        self.cache.len()
    }
}

impl Default for TypeExtractor {
    fn default() -> Self {
        Self::new()
    }
}

/// Type checker for argument values
///
/// Checks if argument values satisfy type constraints.
///
/// # Example
///
/// ```ignore
/// let checker = TypeChecker::new();
/// let int_constraint = TypeConstraint::Simple("Int".to_string());
/// let arg_node = // ... RholangNode representing 42
///
/// assert!(checker.satisfies_constraint(&arg_node, &int_constraint));
/// ```
#[derive(Debug)]
pub struct TypeChecker;

impl TypeChecker {
    /// Create a new type checker
    pub fn new() -> Self {
        TypeChecker
    }

    /// Check if an argument value satisfies a type constraint
    ///
    /// # Arguments
    /// - `arg`: The argument node to check
    /// - `constraint`: The type constraint to check against
    ///
    /// # Returns
    /// - `true` if the argument satisfies the constraint
    /// - `false` otherwise
    ///
    /// # Type Checking Rules
    ///
    /// 1. **Any constraint**: Always satisfied
    /// 2. **Int constraint**: Satisfied by LongLiteral nodes
    /// 3. **String constraint**: Satisfied by StringLiteral nodes
    /// 4. **Bool constraint**: Satisfied by BoolLiteral nodes
    /// 5. **Custom types**: Conservative - returns false (future extension)
    ///
    /// # Phase
    /// Phase 3.5: Type constraint checking in matches_pattern()
    pub fn satisfies_constraint(
        &self,
        arg: &Arc<RholangNode>,
        constraint: &TypeConstraint,
    ) -> bool {
        match constraint {
            // Any constraint is always satisfied
            TypeConstraint::Any => true,

            // Simple type constraints
            TypeConstraint::Simple(type_name) => match type_name.as_str() {
                "Int" | "Long" => matches!(**arg, RholangNode::LongLiteral { .. }),
                "String" => matches!(**arg, RholangNode::StringLiteral { .. }),
                "Bool" | "Boolean" => matches!(**arg, RholangNode::BoolLiteral { .. }),

                // Unknown type: conservative - don't match
                // This allows for future extension without breaking existing code
                _ => false,
            },

            // Compound type constraints (future extension)
            TypeConstraint::Compound { .. } => {
                // TODO: Implement compound type checking
                false
            }
        }
    }
}

impl Default for TypeChecker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::rholang_node::{NodeBase, Position};

    #[test]
    fn test_type_constraint_matches_any() {
        let any = TypeConstraint::Any;
        let int = TypeConstraint::Simple("Int".to_string());

        assert!(any.matches(&int));
        assert!(int.matches(&any));
        assert!(any.matches(&any));
    }

    #[test]
    fn test_type_constraint_matches_simple() {
        let int1 = TypeConstraint::Simple("Int".to_string());
        let int2 = TypeConstraint::Simple("Int".to_string());
        let string = TypeConstraint::Simple("String".to_string());

        assert!(int1.matches(&int2));
        assert!(!int1.matches(&string));
        assert!(!string.matches(&int1));
    }

    #[test]
    fn test_type_extractor_var() {
        let mut extractor = TypeExtractor::new();

        let var_node = Arc::new(RholangNode::Var {
            name: "x".to_string(),
            base: NodeBase::new_simple(
                Position {
                    row: 0,
                    column: 0,
                    byte: 0,
                },
                1,
                0,
                1,
            ),
            metadata: None,
        });

        let result = extractor.extract_from_pattern(&var_node);
        assert!(result.is_none(), "Simple variable should have no type constraint");
    }

    #[test]
    fn test_type_checker_int() {
        let checker = TypeChecker::new();

        let int_node = Arc::new(RholangNode::LongLiteral {
            value: 42,
            base: NodeBase::new_simple(
                Position {
                    row: 0,
                    column: 0,
                    byte: 0,
                },
                2,
                0,
                2,
            ),
            metadata: None,
        });

        let int_constraint = TypeConstraint::Simple("Int".to_string());
        let string_constraint = TypeConstraint::Simple("String".to_string());

        assert!(checker.satisfies_constraint(&int_node, &int_constraint));
        assert!(!checker.satisfies_constraint(&int_node, &string_constraint));
    }

    #[test]
    fn test_type_checker_string() {
        let checker = TypeChecker::new();

        let string_node = Arc::new(RholangNode::StringLiteral {
            value: "hello".to_string(),
            base: NodeBase::new_simple(
                Position {
                    row: 0,
                    column: 0,
                    byte: 0,
                },
                7,
                0,
                7,
            ),
            metadata: None,
        });

        let string_constraint = TypeConstraint::Simple("String".to_string());
        let int_constraint = TypeConstraint::Simple("Int".to_string());

        assert!(checker.satisfies_constraint(&string_node, &string_constraint));
        assert!(!checker.satisfies_constraint(&string_node, &int_constraint));
    }

    #[test]
    fn test_type_checker_any() {
        let checker = TypeChecker::new();

        let int_node = Arc::new(RholangNode::LongLiteral {
            value: 42,
            base: NodeBase::new_simple(
                Position {
                    row: 0,
                    column: 0,
                    byte: 0,
                },
                2,
                0,
                2,
            ),
            metadata: None,
        });

        let any_constraint = TypeConstraint::Any;

        assert!(checker.satisfies_constraint(&int_node, &any_constraint));
    }

    #[test]
    fn test_type_extractor_cache() {
        let mut extractor = TypeExtractor::new();

        // Manually add to cache
        extractor
            .cache
            .insert("Int".to_string(), TypeConstraint::Simple("Int".to_string()));

        assert_eq!(extractor.cache_size(), 1);

        extractor.clear_cache();
        assert_eq!(extractor.cache_size(), 0);
    }
}
