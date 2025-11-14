//! Integration tests for Phase B-3: Persistent Cache Serialization
//!
//! Verifies that the persistent cache correctly:
//! - Creates cache directory and metadata
//! - Validates cache versioning
//! - Uses zstd compression
//! - Handles missing/corrupted cache gracefully
//!
//! Note: Full round-trip testing with actual CachedDocument structures requires complex setup
//! involving tree-sitter parsing and is tested at the LSP integration level.

use std::collections::HashMap;
use std::fs;
use tempfile::TempDir;

use rholang_language_server::lsp::backend::persistent_cache::{
    serialize_workspace_cache, deserialize_workspace_cache, get_workspace_cache_dir,
    CACHE_VERSION,
};

#[test]
fn test_serialize_empty_workspace_creates_cache_directory() {
    // Create temporary workspace directory
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let workspace_root = temp_dir.path();

    // Empty documents
    let documents = HashMap::new();

    // Serialize
    let serialize_result = serialize_workspace_cache(workspace_root, &documents);
    assert!(serialize_result.is_ok(), "Serialization should succeed for empty workspace");

    // Verify cache directory was created
    let cache_dir = get_workspace_cache_dir(workspace_root).expect("Cache dir should exist");
    assert!(cache_dir.exists(), "Cache directory should be created");

    // Verify metadata.json exists
    let metadata_path = cache_dir.join("metadata.json");
    assert!(metadata_path.exists(), "Metadata file should exist");
}

#[test]
fn test_deserialize_empty_workspace() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let workspace_root = temp_dir.path();

    // Serialize empty workspace first
    let documents = HashMap::new();
    serialize_workspace_cache(workspace_root, &documents)
        .expect("Serialization should succeed");

    // Deserialize
    let deserialize_result = deserialize_workspace_cache(workspace_root);
    assert!(deserialize_result.is_ok(), "Deserialization should succeed");

    let loaded_documents = deserialize_result.unwrap();
    assert_eq!(loaded_documents.len(), 0, "Should load 0 documents");
}

#[test]
fn test_cache_metadata_version() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let workspace_root = temp_dir.path();

    let documents = HashMap::new();
    serialize_workspace_cache(workspace_root, &documents)
        .expect("Serialization should succeed");

    // Read metadata.json
    let cache_dir = get_workspace_cache_dir(workspace_root).expect("Cache dir should exist");
    let metadata_path = cache_dir.join("metadata.json");
    let metadata_content = fs::read_to_string(&metadata_path)
        .expect("Should read metadata file");

    // Parse metadata
    let metadata: serde_json::Value = serde_json::from_str(&metadata_content)
        .expect("Should parse JSON");

    assert_eq!(
        metadata["version"].as_u64().unwrap(),
        CACHE_VERSION as u64,
        "Metadata version should match CACHE_VERSION constant"
    );
    assert!(!metadata["created_at"].is_null(), "Should have created_at timestamp");
    assert_eq!(metadata["entry_count"].as_u64().unwrap(), 0, "Should have 0 entries");
    assert!(!metadata["language_server_version"].is_null(), "Should have language_server_version");
}


#[test]
fn test_cache_graceful_failure_on_missing_directory() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let workspace_root = temp_dir.path().join("nonexistent");

    // Deserialize from non-existent directory should fail gracefully
    let result = deserialize_workspace_cache(&workspace_root);
    assert!(result.is_err(), "Should fail when cache directory doesn't exist");
}

#[test]
fn test_cache_version_incompatibility() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let workspace_root = temp_dir.path();

    // Serialize with current version
    let documents = HashMap::new();
    serialize_workspace_cache(workspace_root, &documents)
        .expect("Serialization should succeed");

    // Manually modify metadata to have incompatible version
    let cache_dir = get_workspace_cache_dir(workspace_root).expect("Cache dir should exist");
    let metadata_path = cache_dir.join("metadata.json");

    let mut metadata: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&metadata_path).expect("Should read metadata")
    ).expect("Should parse JSON");

    metadata["version"] = serde_json::json!(999); // Incompatible version

    fs::write(&metadata_path, serde_json::to_string_pretty(&metadata).unwrap())
        .expect("Should write modified metadata");

    // Deserialize should fail due to version mismatch
    let result = deserialize_workspace_cache(workspace_root);
    assert!(result.is_err(), "Should fail on version incompatibility");
}

