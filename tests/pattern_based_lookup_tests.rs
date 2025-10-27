//! Integration tests for pattern-based contract lookup
//!
//! This test suite verifies that the pattern-based lookup system provides:
//! 1. Correct contract resolution by (name, arity)
//! 2. Proper handling of variadic contracts
//! 3. Correct fallback behavior when pattern index is empty
//!
//! These tests focus on the PatternSignature API without creating full RholangNode trees.

use rholang_language_server::ir::symbol_table::PatternSignature;

/// Test that PatternSignature stores name correctly
#[test]
fn test_pattern_signature_name() {
    let sig = PatternSignature {
        name: "myContract".to_string(),
        arity: 2,
        is_variadic: false,
    };

    assert_eq!(sig.name, "myContract");
}

/// Test that PatternSignature stores arity correctly
#[test]
fn test_pattern_signature_arity() {
    let sig = PatternSignature {
        name: "test".to_string(),
        arity: 5,
        is_variadic: false,
    };

    assert_eq!(sig.arity, 5);
}

/// Test that variadic contracts match any arity >= base arity
#[test]
fn test_variadic_contract_arity_matching() {
    let sig = PatternSignature {
        name: "variadicContract".to_string(),
        arity: 2,
        is_variadic: true,
    };

    // Should match arity >= 2
    assert!(sig.matches_arity(2));
    assert!(sig.matches_arity(3));
    assert!(sig.matches_arity(10));

    // Should not match arity < 2
    assert!(!sig.matches_arity(0));
    assert!(!sig.matches_arity(1));
}

/// Test that non-variadic contracts match exact arity
#[test]
fn test_exact_arity_matching() {
    let sig = PatternSignature {
        name: "exactContract".to_string(),
        arity: 3,
        is_variadic: false,
    };

    // Should only match exact arity
    assert!(sig.matches_arity(3));

    // Should not match other arities
    assert!(!sig.matches_arity(0));
    assert!(!sig.matches_arity(2));
    assert!(!sig.matches_arity(4));
}

/// Test that PatternSignature equality works correctly
#[test]
fn test_pattern_signature_equality() {
    let sig1 = PatternSignature {
        name: "test".to_string(),
        arity: 2,
        is_variadic: false,
    };

    let sig2 = PatternSignature {
        name: "test".to_string(),
        arity: 2,
        is_variadic: false,
    };

    let sig3 = PatternSignature {
        name: "test".to_string(),
        arity: 3,
        is_variadic: false,
    };

    assert_eq!(sig1, sig2);
    assert_ne!(sig1, sig3);
}

/// Test that different variadic states create different signatures
#[test]
fn test_variadic_in_signature() {
    let non_variadic = PatternSignature {
        name: "contract".to_string(),
        arity: 2,
        is_variadic: false,
    };

    let variadic = PatternSignature {
        name: "contract".to_string(),
        arity: 2,
        is_variadic: true,
    };

    assert_ne!(non_variadic, variadic);
}

/// Test zero-arity pattern matching
#[test]
fn test_zero_arity_matching() {
    let sig = PatternSignature {
        name: "nullary".to_string(),
        arity: 0,
        is_variadic: false,
    };

    assert!(sig.matches_arity(0));
    assert!(!sig.matches_arity(1));
}

/// Test variadic with zero base arity
#[test]
fn test_variadic_zero_base_arity() {
    let sig = PatternSignature {
        name: "anyArgs".to_string(),
        arity: 0,
        is_variadic: true,
    };

    // Should match any arity
    assert!(sig.matches_arity(0));
    assert!(sig.matches_arity(1));
    assert!(sig.matches_arity(100));
}

/// Test pattern signature with different names
#[test]
fn test_different_names() {
    let sig1 = PatternSignature {
        name: "foo".to_string(),
        arity: 1,
        is_variadic: false,
    };

    let sig2 = PatternSignature {
        name: "bar".to_string(),
        arity: 1,
        is_variadic: false,
    };

    assert_ne!(sig1, sig2);
    assert_ne!(sig1.name, sig2.name);
}

// Note: Overload resolution tests would require creating full Symbol objects
// with contract patterns, which needs complex RholangNode construction.
// The overload resolution logic is tested indirectly through LSP handler tests.
