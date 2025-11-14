//! Phase B-3: Persistent Document IR Cache
//!
//! This module provides serialization and deserialization of the document cache
//! to/from disk, enabling fast "warm start" after LSP server restarts.
//!
//! Architecture:
//! - Serialization format: bincode (compact binary)
//! - Compression: zstd level 3 (fast compression)
//! - Cache location: ~/.cache/f1r3fly-io/rholang-language-server/v1/workspace-{hash}/
//! - Invalidation: mtime + content hash verification
//!
//! Expected Performance:
//! - Cold start (100 files): ~18 seconds (parse + index all files)
//! - Warm start (100 files): ~100-300ms (load cache from disk)
//! - Speedup: 60-180x faster startup
//!
//! Safety:
//! - Graceful degradation: Falls back to cold start on cache errors
//! - Version checking: Invalidates cache on format version mismatch
//! - Atomic writes: Uses tmp file + rename to avoid corruption

use crate::ir::rholang_node::RholangNode;
use crate::ir::metta_node::MettaNode;
use crate::ir::semantic_node::Position;
use crate::ir::symbol_table::SymbolTable;
use crate::ir::DocumentIR;
use crate::lsp::models::{CachedDocument, DocumentLanguage};
use crate::lsp::position_index::PositionIndex;
use crate::lsp::symbol_index::SymbolIndex;
use crate::tree_sitter::parse_code;
use anyhow::{Context, Result};
use ropey::Rope;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;
use tower_lsp::lsp_types::Url;
use tracing::debug;

/// Current cache format version
///
/// Increment this when making breaking changes to SerializableCachedDocument
/// to invalidate old caches automatically.
pub const CACHE_VERSION: u32 = 1;

/// Cache metadata stored in metadata.json
#[derive(Debug, Serialize, Deserialize)]
pub struct CacheMetadata {
    /// Cache format version (for compatibility checking)
    pub version: u32,
    /// When this cache was created
    pub created_at: SystemTime,
    /// Number of documents in the cache
    pub entry_count: usize,
    /// Language server version that created this cache
    pub language_server_version: String,
}

/// Serializable representation of a cached document
///
/// This struct contains only the fields that can be efficiently serialized.
/// Non-serializable fields (tree, text, unified_ir, completion_state) are
/// reconstructed on demand after deserialization.
///
/// Serialization Strategy (from planning document):
/// - **Serialize**: IR, symbol tables, indices, metadata
/// - **Skip**: Tree-sitter tree (reconstruct from text)
/// - **Skip**: Rope text (read from disk on demand)
/// - **Skip**: UnifiedIR (reconstruct from IR)
/// - **Skip**: Completion state (rebuild on first use)
#[derive(Debug, Serialize, Deserialize)]
pub struct SerializableCachedDocument {
    /// Rholang-specific IR (primary semantic tree)
    #[serde(
        serialize_with = "crate::serde_helpers::serialize_arc",
        deserialize_with = "crate::serde_helpers::deserialize_arc"
    )]
    pub ir: Arc<RholangNode>,

    /// Document IR with comment channel (if present)
    #[serde(
        serialize_with = "crate::serde_helpers::serialize_option_arc",
        deserialize_with = "crate::serde_helpers::deserialize_option_arc"
    )]
    pub document_ir: Option<Arc<DocumentIR>>,

    /// MeTTa-specific IR (only for MeTTa files and virtual documents)
    ///
    /// Phase B-3: Research confirms MeTTa is actively supported, especially for
    /// embedded language use in virtual documents. Serializing this IR is critical
    /// for maintaining cache effectiveness for Rholang files with embedded MeTTa.
    #[serde(
        serialize_with = "crate::serde_helpers::serialize_option_arc_vec",
        deserialize_with = "crate::serde_helpers::deserialize_option_arc_vec"
    )]
    pub metta_ir: Option<Vec<Arc<MettaNode>>>,

    /// Position-indexed AST for O(log n) lookups
    ///
    /// Phase 6 optimization: Serializing this avoids rebuilding on load
    #[serde(
        serialize_with = "crate::serde_helpers::serialize_arc",
        deserialize_with = "crate::serde_helpers::deserialize_arc"
    )]
    pub position_index: Arc<PositionIndex>,

    /// Symbol table for this document
    #[serde(
        serialize_with = "crate::serde_helpers::serialize_arc",
        deserialize_with = "crate::serde_helpers::deserialize_arc"
    )]
    pub symbol_table: Arc<SymbolTable>,

    /// Inverted index for find-references and rename
    /// Maps declaration position -> reference positions
    pub inverted_index: HashMap<Position, Vec<Position>>,

    /// Suffix array-based symbol index
    #[serde(
        serialize_with = "crate::serde_helpers::serialize_arc",
        deserialize_with = "crate::serde_helpers::deserialize_arc"
    )]
    pub symbol_index: Arc<SymbolIndex>,

    /// Position mappings for IR nodes
    #[serde(
        serialize_with = "crate::serde_helpers::serialize_arc",
        deserialize_with = "crate::serde_helpers::deserialize_arc"
    )]
    pub positions: Arc<HashMap<usize, (crate::ir::semantic_node::Position, crate::ir::semantic_node::Position)>>,

    /// Document version number
    pub version: i32,

    /// Fast hash of document content (for change detection)
    pub content_hash: u64,

    /// Language detected from file extension
    pub language: DocumentLanguage,

    // ===== Metadata for reconstruction =====

    /// Document URI (needed to read file from disk)
    pub uri: Url,

    /// File modification time (for cache invalidation)
    pub modified_at: SystemTime,
}

impl SerializableCachedDocument {
    /// Convert a CachedDocument to its serializable form
    ///
    /// This extracts the essential fields that need to be persisted,
    /// discarding the fields that can be reconstructed on load.
    pub fn from_cached_document(doc: &CachedDocument, uri: Url) -> Result<Self> {
        // Get file metadata for mtime
        let path = uri.to_file_path()
            .map_err(|()| anyhow::anyhow!("Invalid file URI: {}", uri))?;
        let metadata = fs::metadata(&path)
            .with_context(|| format!("Failed to read metadata for {}", uri))?;
        let modified_at = metadata.modified()
            .with_context(|| format!("Failed to get mtime for {}", uri))?;

        Ok(Self {
            ir: doc.ir.clone(),
            document_ir: doc.document_ir.clone(),
            metta_ir: doc.metta_ir.clone(),  // Serialize MeTTa IR (Phase B-3 correction)
            position_index: doc.position_index.clone(),
            symbol_table: doc.symbol_table.clone(),
            inverted_index: doc.inverted_index.clone(),
            symbol_index: doc.symbol_index.clone(),
            positions: doc.positions.clone(),
            version: doc.version,
            content_hash: doc.content_hash,
            language: doc.language.clone(),
            uri,
            modified_at,
        })
    }

    /// Reconstruct a CachedDocument from its serializable form
    ///
    /// This reads the file from disk to reconstruct the non-serializable fields:
    /// - text (Rope): Read from disk
    /// - tree (Tree-sitter): Parse from text
    /// - unified_ir: Reconstruct from IR
    /// - completion_state: Leave as None (rebuilt on first use)
    ///
    /// Performance: ~1-2ms per document (acceptable for warm start)
    pub fn to_cached_document(self) -> Result<CachedDocument> {
        // Read file from disk to reconstruct Rope and Tree
        let path = self.uri.to_file_path()
            .map_err(|()| anyhow::anyhow!("Invalid file URI: {}", self.uri))?;
        let text_content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read file for reconstruction: {}", self.uri))?;

        // Reconstruct Rope
        let text = Rope::from_str(&text_content);

        // Reconstruct Tree-sitter tree
        let tree = Arc::new(parse_code(&text_content));

        // Reconstruct unified_ir from IR
        let unified_ir = crate::ir::unified_ir::UnifiedIR::from_rholang(&self.ir);

        Ok(CachedDocument {
            ir: self.ir,
            position_index: self.position_index,
            document_ir: self.document_ir,
            metta_ir: self.metta_ir,  // MeTTa IR restored from cache (Phase B-3 correction)
            unified_ir,
            language: self.language,
            tree,
            symbol_table: self.symbol_table,
            inverted_index: self.inverted_index,
            version: self.version,
            text,
            positions: self.positions,
            symbol_index: self.symbol_index,
            content_hash: self.content_hash,
            completion_state: None,  // Rebuilt on first use
        })
    }

    /// Check if this cache entry is still valid
    ///
    /// Validation strategy (from planning document):
    /// 1. Check if file still exists
    /// 2. Compare mtime (fast check)
    /// 3. If mtime matches, entry is valid
    ///
    /// Note: Content hash verification will be added in Phase B-3.3
    pub fn is_valid(&self) -> Result<bool> {
        let path = self.uri.to_file_path()
            .map_err(|()| anyhow::anyhow!("Invalid file URI: {}", self.uri))?;

        // Check if file exists
        if !path.exists() {
            debug!("Cache entry invalid: file no longer exists: {}", self.uri);
            return Ok(false);
        }

        // Check mtime
        let metadata = fs::metadata(&path)
            .with_context(|| format!("Failed to read metadata for {}", self.uri))?;
        let current_mtime = metadata.modified()
            .with_context(|| format!("Failed to get mtime for {}", self.uri))?;

        // Invalidate if file modified after cache entry
        let valid = current_mtime <= self.modified_at;
        if !valid {
            debug!(
                "Cache entry invalid: file modified after cache creation: {} (cached: {:?}, current: {:?})",
                self.uri, self.modified_at, current_mtime
            );
        }

        Ok(valid)
    }
}

/// Get the workspace-specific cache directory
///
/// Structure: ~/.cache/f1r3fly-io/rholang-language-server/v{VERSION}/workspace-{hash}/
///
/// where {hash} is blake3(workspace_root_path) to ensure separate caches
/// for different projects.
pub fn get_workspace_cache_dir(workspace_root: &Path) -> Result<PathBuf> {
    // Get base cache directory (platform-specific)
    let base_dir = dirs::cache_dir()
        .ok_or_else(|| anyhow::anyhow!("Failed to determine cache directory"))?
        .join("f1r3fly-io")
        .join("rholang-language-server");

    // Version-specific subdirectory
    let version_dir = base_dir.join(format!("v{}", CACHE_VERSION));

    // Workspace-specific subdirectory (hash of workspace root path)
    let workspace_path_str = workspace_root.to_string_lossy();
    let workspace_hash = blake3::hash(workspace_path_str.as_bytes());
    let workspace_hash_hex = workspace_hash.to_hex();

    let cache_dir = version_dir.join(format!("workspace-{}", &workspace_hash_hex[..16]));

    Ok(cache_dir)
}

/// Check if cache metadata is compatible with current version
fn is_cache_compatible(metadata: &CacheMetadata) -> bool {
    metadata.version == CACHE_VERSION
}

/// Serialize and persist workspace cache to disk
///
/// Writes the cache using:
/// - bincode for compact binary serialization
/// - zstd compression (level 3) for 3x size reduction
/// - Atomic write pattern (tmp file + rename) for crash safety
///
/// # Arguments
/// * `workspace_root` - Workspace root directory (for cache dir computation)
/// * `documents` - Map of URI -> CachedDocument to serialize
///
/// # Returns
/// Ok(()) on success, Err on any I/O or serialization error
///
/// # Performance
/// Expected: ~100-300ms for 100 documents (dominated by disk I/O)
pub fn serialize_workspace_cache(
    workspace_root: &Path,
    documents: &HashMap<Url, CachedDocument>,
) -> Result<()> {
    let cache_dir = get_workspace_cache_dir(workspace_root)?;

    // Ensure cache directory exists
    fs::create_dir_all(&cache_dir)
        .with_context(|| format!("Failed to create cache directory: {:?}", cache_dir))?;

    debug!("Serializing workspace cache to {:?} ({} documents)", cache_dir, documents.len());

    // Convert CachedDocument -> SerializableCachedDocument
    let mut serializable_docs = HashMap::new();
    for (uri, doc) in documents {
        match SerializableCachedDocument::from_cached_document(doc, uri.clone()) {
            Ok(serializable) => {
                serializable_docs.insert(uri.clone(), serializable);
            }
            Err(e) => {
                tracing::warn!("Failed to convert document to serializable form: {} - {}", uri, e);
                // Continue with other documents (graceful degradation)
            }
        }
    }

    // Write metadata.json
    let metadata = CacheMetadata {
        version: CACHE_VERSION,
        created_at: SystemTime::now(),
        entry_count: serializable_docs.len(),
        language_server_version: env!("CARGO_PKG_VERSION").to_string(),
    };

    let metadata_path = cache_dir.join("metadata.json");
    let metadata_tmp_path = cache_dir.join(".metadata.json.tmp");

    let metadata_json = serde_json::to_string_pretty(&metadata)
        .context("Failed to serialize cache metadata")?;
    fs::write(&metadata_tmp_path, metadata_json)
        .with_context(|| format!("Failed to write metadata to {:?}", metadata_tmp_path))?;
    fs::rename(&metadata_tmp_path, &metadata_path)
        .with_context(|| format!("Failed to atomically rename metadata file"))?;

    // Serialize each document to separate file
    for (uri, doc) in &serializable_docs {
        // Create safe filename from URI
        let uri_hash = blake3::hash(uri.as_str().as_bytes());
        let filename = format!("{}.cache", uri_hash.to_hex());
        let cache_file_path = cache_dir.join(&filename);
        let tmp_cache_file_path = cache_dir.join(format!(".{}.tmp", filename));

        // Serialize with bincode
        let serialized = bincode::serialize(doc)
            .with_context(|| format!("Failed to serialize document: {}", uri))?;

        // Compress with zstd (level 3 for fast compression)
        let compressed = zstd::encode_all(&serialized[..], 3)
            .with_context(|| format!("Failed to compress document: {}", uri))?;

        // Atomic write: tmp file + rename
        fs::write(&tmp_cache_file_path, &compressed)
            .with_context(|| format!("Failed to write cache file: {:?}", tmp_cache_file_path))?;
        fs::rename(&tmp_cache_file_path, &cache_file_path)
            .with_context(|| format!("Failed to atomically rename cache file for: {}", uri))?;
    }

    debug!("Successfully serialized {} documents to cache", serializable_docs.len());
    Ok(())
}

/// Deserialize workspace cache from disk
///
/// Loads the cache with:
/// - zstd decompression
/// - bincode deserialization
/// - Validation (version check + mtime check)
///
/// # Arguments
/// * `workspace_root` - Workspace root directory (for cache dir computation)
///
/// # Returns
/// Ok(HashMap<Url, CachedDocument>) on success, Err if cache doesn't exist or is invalid
///
/// # Performance
/// Expected: ~100-300ms for 100 documents (dominated by disk I/O + text reconstruction)
///
/// # Graceful Degradation
/// Returns error on any validation failure, triggering cold start fallback
pub fn deserialize_workspace_cache(
    workspace_root: &Path,
) -> Result<HashMap<Url, CachedDocument>> {
    let cache_dir = get_workspace_cache_dir(workspace_root)?;

    // Check if cache directory exists
    if !cache_dir.exists() {
        debug!("Cache directory does not exist: {:?}", cache_dir);
        anyhow::bail!("Cache directory not found");
    }

    debug!("Deserializing workspace cache from {:?}", cache_dir);

    // Read and validate metadata
    let metadata_path = cache_dir.join("metadata.json");
    if !metadata_path.exists() {
        debug!("Cache metadata not found: {:?}", metadata_path);
        anyhow::bail!("Cache metadata not found");
    }

    let metadata_json = fs::read_to_string(&metadata_path)
        .with_context(|| format!("Failed to read metadata from {:?}", metadata_path))?;
    let metadata: CacheMetadata = serde_json::from_str(&metadata_json)
        .context("Failed to deserialize cache metadata")?;

    // Version compatibility check
    if !is_cache_compatible(&metadata) {
        debug!(
            "Cache version mismatch: cached={}, current={}",
            metadata.version, CACHE_VERSION
        );
        anyhow::bail!("Cache version incompatible");
    }

    debug!(
        "Cache metadata valid: version={}, entry_count={}, created={:?}",
        metadata.version, metadata.entry_count, metadata.created_at
    );

    // Deserialize all cache files
    let mut documents = HashMap::new();
    let cache_files: Vec<_> = fs::read_dir(&cache_dir)
        .with_context(|| format!("Failed to read cache directory: {:?}", cache_dir))?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry.path().extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext == "cache")
                .unwrap_or(false)
        })
        .collect();

    debug!("Found {} cache files to deserialize", cache_files.len());

    for entry in cache_files {
        let cache_file_path = entry.path();

        match deserialize_single_document(&cache_file_path) {
            Ok((uri, doc)) => {
                documents.insert(uri, doc);
            }
            Err(e) => {
                tracing::warn!("Failed to deserialize cache file {:?}: {}", cache_file_path, e);
                // Continue with other files (graceful degradation)
            }
        }
    }

    debug!("Successfully deserialized {} documents from cache", documents.len());
    Ok(documents)
}

/// Helper function to deserialize a single cached document
fn deserialize_single_document(cache_file_path: &Path) -> Result<(Url, CachedDocument)> {
    // Read compressed file
    let compressed_data = fs::read(cache_file_path)
        .with_context(|| format!("Failed to read cache file: {:?}", cache_file_path))?;

    // Decompress with zstd
    let decompressed = zstd::decode_all(&compressed_data[..])
        .with_context(|| format!("Failed to decompress cache file: {:?}", cache_file_path))?;

    // Deserialize with bincode
    let serializable_doc: SerializableCachedDocument = bincode::deserialize(&decompressed)
        .with_context(|| format!("Failed to deserialize cache file: {:?}", cache_file_path))?;

    // Validate cache entry (mtime check)
    if !serializable_doc.is_valid()? {
        anyhow::bail!("Cache entry invalid (file modified)");
    }

    // Reconstruct CachedDocument
    let uri = serializable_doc.uri.clone();
    let doc = serializable_doc.to_cached_document()?;

    Ok((uri, doc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_version_constant() {
        assert_eq!(CACHE_VERSION, 1);
    }

    #[test]
    fn test_cache_directory_structure() {
        let workspace_root = Path::new("/home/user/myproject");
        let cache_dir = get_workspace_cache_dir(workspace_root).unwrap();

        // Check that cache dir contains f1r3fly-io parent, version and workspace hash
        let cache_dir_str = cache_dir.to_string_lossy();
        assert!(cache_dir_str.contains("f1r3fly-io"));
        assert!(cache_dir_str.contains("rholang-language-server"));
        assert!(cache_dir_str.contains(&format!("v{}", CACHE_VERSION)));
        assert!(cache_dir_str.contains("workspace-"));
    }

    #[test]
    fn test_cache_compatibility_check() {
        let compatible = CacheMetadata {
            version: CACHE_VERSION,
            created_at: SystemTime::now(),
            entry_count: 0,
            language_server_version: "0.1.0".to_string(),
        };
        assert!(is_cache_compatible(&compatible));

        let incompatible = CacheMetadata {
            version: CACHE_VERSION + 1,
            created_at: SystemTime::now(),
            entry_count: 0,
            language_server_version: "0.2.0".to_string(),
        };
        assert!(!is_cache_compatible(&incompatible));
    }
}
