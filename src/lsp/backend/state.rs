//! Backend state management
//!
//! This module defines the RholangBackend struct, which maintains all state
//! for the LSP server including document cache, workspace index, and validation.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicI32, AtomicU32};
use std::sync::mpsc::{Receiver, Sender};

use dashmap::DashMap;
use tokio::sync::RwLock;
use tower_lsp::Client;
use tower_lsp::lsp_types::Url;
use notify::RecommendedWatcher;

use crate::language_regions::{VirtualDocumentRegistry, DetectionWorkerHandle, DetectorRegistry};
use crate::lsp::models::{LspDocument, WorkspaceState};
use crate::lsp::semantic_validator::SemanticValidator;
use crate::lsp::diagnostic_provider::DiagnosticProvider;

/// Document change event for debouncing
#[derive(Debug, Clone)]
pub(super) struct DocumentChangeEvent {
    pub(super) uri: Url,
    pub(super) version: i32,
    pub(super) document: Arc<LspDocument>,
    pub(super) text: Arc<String>,
}

/// Diagnostic update event for debounced publishing
#[derive(Debug, Clone)]
pub(super) struct DiagnosticUpdate {
    pub(super) uri: Url,
    pub(super) diagnostics: Vec<tower_lsp::lsp_types::Diagnostic>,
    pub(super) version: Option<i32>,
}

/// Workspace indexing task for progressive indexing
#[derive(Debug, Clone, Eq, PartialEq)]
pub(super) struct IndexingTask {
    pub(super) uri: Url,
    pub(super) text: String,
    pub(super) priority: u8,  // 0 = high (current file), 1 = normal
}

/// Workspace change event for hot observable pattern
///
/// Broadcast to all subscribers when workspace state changes (file indexed, symbols linked, etc.)
#[derive(Debug, Clone)]
pub(super) struct WorkspaceChangeEvent {
    /// Number of indexed files
    pub(super) file_count: usize,
    /// Number of global symbols
    pub(super) symbol_count: usize,
    /// Most recent change type
    pub(super) change_type: WorkspaceChangeType,
}

/// Type of workspace change
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WorkspaceChangeType {
    /// File was indexed/re-indexed
    FileIndexed,
    /// Symbols were linked across files
    SymbolsLinked,
    /// Workspace initialized
    Initialized,
}

/// Event broadcast when diagnostics are published for a document
#[derive(Debug, Clone)]
pub struct DiagnosticPublished {
    /// URI of the document
    pub uri: Url,
    /// Version of the document
    pub version: Option<i32>,
    /// Number of diagnostics
    pub diagnostic_count: usize,
}

/// The Rholang language server backend, managing state and handling LSP requests.
#[derive(Clone)]
pub struct RholangBackend {
    pub(super) client: Client,
    /// Lock-free concurrent document cache by URI (Phase 3 optimization)
    pub(super) documents_by_uri: Arc<DashMap<Url, Arc<LspDocument>>>,
    /// Lock-free concurrent document cache by ID (Phase 3 optimization)
    pub(super) documents_by_id: Arc<DashMap<u32, Arc<LspDocument>>>,
    pub(super) serial_document_id: Arc<AtomicU32>,
    /// Pluggable diagnostic provider (Rust interpreter or gRPC to RNode)
    pub(super) diagnostic_provider: Arc<Box<dyn DiagnosticProvider>>,
    /// Direct access to SemanticValidator for validate_parsed optimization (if using Rust backend)
    pub(super) semantic_validator: Option<SemanticValidator>,
    pub(super) client_process_id: Arc<tokio::sync::Mutex<Option<u32>>>,
    pub(super) pid_channel: Option<tokio::sync::mpsc::Sender<u32>>,
    // Reactive channels
    pub(super) doc_change_tx: tokio::sync::mpsc::Sender<DocumentChangeEvent>,
    pub(super) validation_cancel: Arc<tokio::sync::Mutex<HashMap<Url, tokio::sync::oneshot::Sender<()>>>>,
    pub(super) indexing_tx: tokio::sync::mpsc::Sender<IndexingTask>,
    /// Workspace state with lock-free concurrent collections (Phase 1 optimization)
    /// No outer RwLock needed - internal DashMaps provide lock-free concurrent access
    pub(super) workspace: Arc<WorkspaceState>,
    pub(super) file_watcher: Arc<Mutex<Option<RecommendedWatcher>>>,
    pub(super) file_events: Arc<Mutex<Receiver<notify::Result<notify::Event>>>>,
    pub(super) file_sender: Arc<Mutex<Sender<notify::Result<notify::Event>>>>,
    pub(super) version_counter: Arc<AtomicI32>,
    pub(super) root_dir: Arc<RwLock<Option<PathBuf>>>,
    pub(super) shutdown_tx: Arc<tokio::sync::broadcast::Sender<()>>,
    /// Virtual document registry for embedded language regions
    pub(super) virtual_docs: Arc<RwLock<VirtualDocumentRegistry>>,
    /// Hot observable for workspace changes (ReactiveX Phase 2)
    /// Multiple subscribers can watch for workspace state updates
    pub(super) workspace_changes: Arc<tokio::sync::watch::Sender<WorkspaceChangeEvent>>,
    /// Broadcast channel for diagnostic completion events
    /// Tests can subscribe to this to know when diagnostics have been published for a document
    pub(super) diagnostics_published: Arc<tokio::sync::broadcast::Sender<DiagnosticPublished>>,
    /// Channel to request debounced symbol linking
    /// Sending to this channel queues a link_symbols request, which will be batched
    pub(super) link_symbols_tx: tokio::sync::mpsc::Sender<()>,
    /// Channel to request debounced diagnostic publishing
    /// Sending diagnostics to this channel batches them before publishing to the client
    pub(super) diagnostics_tx: tokio::sync::mpsc::Sender<DiagnosticUpdate>,
    /// Async virtual document detection worker (hybrid spawn_blocking + rayon)
    /// Provides 18-19x faster throughput than blocking detection
    pub(super) detection_worker: DetectionWorkerHandle,
    /// Detector registry for virtual document detection
    pub(super) detector_registry: Arc<DetectorRegistry>,
}

// Manual Debug implementation since DiagnosticProvider doesn't implement Debug
impl std::fmt::Debug for RholangBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RholangBackend")
            .field("backend", &self.diagnostic_provider.backend_name())
            .field("documents_count", &"<HashMap>")
            .finish()
    }
}
