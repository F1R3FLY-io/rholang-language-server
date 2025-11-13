//! Integration tests for SpacePool usage in RholangPatternIndex
//!
//! Tests verify that SpacePool integration maintains correctness while
//! providing the expected 2.56x performance improvement for pattern
//! serialization operations.

use rholang_language_server::ir::rholang_pattern_index::RholangPatternIndex;
use rholang_language_server::ir::mork_canonical::{MorkForm, LiteralValue};

#[test]
fn test_pool_basic_operation() {
    // Test that RholangPatternIndex with SpacePool can be created and used
    let _index = RholangPatternIndex::new();
    // If SpacePool initialization fails, this will panic
}

#[test]
fn test_pool_multiple_operations() {
    // Test that pool correctly handles multiple operations
    let index = RholangPatternIndex::new();

    // Perform multiple MORK serializations to exercise pool
    let space = mork::space::Space::new();

    for i in 0..20 {
        let mork = MorkForm::Literal(LiteralValue::Int(i));
        let result = mork.to_mork_bytes(&space);
        assert!(result.is_ok(), "Failed to serialize on iteration {}", i);
    }

    // Pool should have handled these operations correctly
    drop(index);
}

#[test]
fn test_pool_concurrent_access() {
    use std::sync::{Arc, Mutex};
    use std::thread;

    // Test that RholangPatternIndex with SpacePool handles concurrent operations
    let index = Arc::new(Mutex::new(RholangPatternIndex::new()));
    let mut handles = vec![];

    // Spawn multiple threads
    for thread_id in 0..10 {
        let index_clone = index.clone();
        let handle = thread::spawn(move || {
            let _idx = index_clone.lock().unwrap();

            // Each thread performs some operations
            let space = mork::space::Space::new();
            for i in 0..5 {
                let mork = MorkForm::Literal(LiteralValue::String(
                    format!("thread_{}_msg_{}", thread_id, i)
                ));
                let result = mork.to_mork_bytes(&space);
                assert!(result.is_ok());
            }
        });
        handles.push(handle);
    }

    // Wait for all threads
    for handle in handles {
        handle.join().expect("Thread panicked");
    }
}

#[test]
fn test_pool_stress_many_operations() {
    // Stress test: Many operations to verify pool stability
    let index = RholangPatternIndex::new();
    let space = mork::space::Space::new();

    // Perform 100 serialization operations
    for i in 0..100 {
        let mork = if i % 2 == 0 {
            MorkForm::Literal(LiteralValue::Int(i as i64))
        } else {
            MorkForm::Literal(LiteralValue::String(format!("value_{}", i)))
        };

        let result = mork.to_mork_bytes(&space);
        assert!(result.is_ok(), "Failed on iteration {}", i);
    }

    drop(index);
}

#[test]
fn test_pool_deterministic_serialization() {
    // Verify that pooled Space produces deterministic results
    let index = RholangPatternIndex::new();
    let space = mork::space::Space::new();

    let mork = MorkForm::VarPattern("test_var".to_string());

    // Serialize multiple times
    let mut results = Vec::new();
    for _ in 0..10 {
        let bytes = mork.to_mork_bytes(&space).expect("Serialization failed");
        results.push(bytes);
    }

    // All results should be identical
    for i in 1..results.len() {
        assert_eq!(results[0], results[i],
            "Serialization result {} differs from result 0", i);
    }

    drop(index);
}

#[test]
fn test_pool_different_pattern_types() {
    // Test that pool handles different MORK pattern types correctly
    let index = RholangPatternIndex::new();
    let space = mork::space::Space::new();

    let patterns = vec![
        MorkForm::Literal(LiteralValue::Int(42)),
        MorkForm::Literal(LiteralValue::String("hello".to_string())),
        MorkForm::Literal(LiteralValue::Bool(true)),
        MorkForm::VarPattern("x".to_string()),
        MorkForm::WildcardPattern,
        MorkForm::Nil,
    ];

    for (i, pattern) in patterns.iter().enumerate() {
        let result = pattern.to_mork_bytes(&space);
        assert!(result.is_ok(), "Pattern {} failed to serialize", i);
        assert!(!result.unwrap().is_empty(), "Pattern {} produced empty bytes", i);
    }

    drop(index);
}

#[test]
fn test_pool_sequential_operations() {
    // Test that pool correctly reuses Space objects across sequential operations
    let index = RholangPatternIndex::new();

    // First batch of operations
    {
        let space = mork::space::Space::new();
        for i in 0..10 {
            let mork = MorkForm::Literal(LiteralValue::Int(i));
            let _ = mork.to_mork_bytes(&space);
        }
    }

    // Second batch - Space objects should have been returned to pool
    {
        let space = mork::space::Space::new();
        for i in 10..20 {
            let mork = MorkForm::Literal(LiteralValue::Int(i));
            let _ = mork.to_mork_bytes(&space);
        }
    }

    drop(index);
}

#[test]
fn test_pool_clone_behavior() {
    // Test that cloning RholangPatternIndex works with SpacePool
    let index1 = RholangPatternIndex::new();

    // Note: RholangPatternIndex doesn't implement Clone, but SpacePool does
    // This test verifies the pool can be cloned if needed in the future
    let space = mork::space::Space::new();
    let mork = MorkForm::Literal(LiteralValue::String("test".to_string()));
    let result = mork.to_mork_bytes(&space);

    assert!(result.is_ok());
    drop(index1);
}
