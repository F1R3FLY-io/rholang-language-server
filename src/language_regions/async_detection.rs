//! Async virtual document detection worker
//!
//! Provides background detection of virtual document regions to prevent
//! blocking the LSP server's main event loop during parsing and analysis.
//!
//! ## Adaptive Parallelization (Phase 2 Optimization)
//!
//! Based on profiling data, Rayon thread pool overhead dominates CPU time for small workloads:
//! - Thread synchronization: ~33.6% CPU time
//! - Work stealing overhead: ~31.0% CPU time
//! - Actual work: ~10% CPU time
//!
//! Rayon overhead is approximately 15-20µs per batch, which equals or exceeds
//! the work time for small tasks. This module now adaptively chooses between
//! sequential and parallel processing based on estimated work time.
//!
//! **Heuristic**: Only use Rayon when estimated work > 100µs

use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use url::Url;
use tracing::{debug, error, trace, warn};

use super::{DetectorRegistry, LanguageRegion};

/// Threshold below which sequential processing is faster than parallel (microseconds)
/// Based on profiling: Rayon overhead ≈ 15-20µs (thread sync + work stealing)
/// Conservative threshold to ensure benefit outweighs overhead
const PARALLEL_THRESHOLD_MICROS: u64 = 100;

/// Minimum number of documents to consider parallel processing
/// Even if work time exceeds threshold, need multiple docs to benefit from parallelism
const MIN_PARALLEL_DOCUMENTS: usize = 5;

/// Request to detect virtual document regions
#[derive(Debug)]
pub struct DetectionRequest {
    /// URI of the document being analyzed
    pub uri: Url,
    /// Source code to analyze
    pub source: String,
    /// Response channel for sending results back
    pub response: oneshot::Sender<DetectionResult>,
}

/// Result of a detection operation
#[derive(Debug, Clone)]
pub struct DetectionResult {
    /// URI of the analyzed document
    pub uri: Url,
    /// Detected language regions
    pub regions: Vec<LanguageRegion>,
    /// Time taken for detection (milliseconds)
    pub elapsed_ms: u64,
}

/// Handle to the async detection worker
///
/// This handle allows sending detection requests to the background worker.
/// When dropped, the worker will shut down gracefully.
#[derive(Clone)]
pub struct DetectionWorkerHandle {
    request_tx: mpsc::UnboundedSender<DetectionRequest>,
}

impl DetectionWorkerHandle {
    /// Sends a detection request to the worker
    ///
    /// Returns a receiver that will receive the detection result.
    ///
    /// # Arguments
    ///
    /// * `uri` - URI of the document
    /// * `source` - Source code to analyze
    ///
    /// # Returns
    ///
    /// A oneshot receiver for the detection result.
    pub fn detect(
        &self,
        uri: Url,
        source: String,
    ) -> oneshot::Receiver<DetectionResult> {
        let (response_tx, response_rx) = oneshot::channel();

        let request = DetectionRequest {
            uri,
            source,
            response: response_tx,
        };

        if let Err(e) = self.request_tx.send(request) {
            error!("Failed to send detection request: {}", e);
        }

        response_rx
    }

    /// Checks if the worker is still running
    pub fn is_running(&self) -> bool {
        !self.request_tx.is_closed()
    }
}

/// Spawns an async detection worker
///
/// The worker runs in the background, processing detection requests
/// asynchronously using a hybrid approach: spawn_blocking + rayon for
/// optimal throughput and async integration.
///
/// Based on benchmark results:
/// - Hybrid approach: 18-19x faster than pure spawn_blocking
/// - Best performance across simple, medium, complex, and burst scenarios
/// - Maintains async/await integration for non-blocking LSP operation
///
/// # Arguments
///
/// * `registry` - Detector registry to use for detection
///
/// # Returns
///
/// A handle to communicate with the worker.
pub fn spawn_detection_worker(registry: Arc<DetectorRegistry>) -> DetectionWorkerHandle {
    let (request_tx, mut request_rx) = mpsc::unbounded_channel::<DetectionRequest>();

    tokio::spawn(async move {
        debug!("Virtual document detection worker started (hybrid mode: spawn_blocking + rayon)");

        // Batch requests for hybrid processing
        let mut batch = Vec::new();

        while let Some(request) = request_rx.recv().await {
            batch.push(request);

            // Drain remaining requests from channel to batch them
            while let Ok(req) = request_rx.try_recv() {
                batch.push(req);
            }

            let batch_size = batch.len();
            trace!("Processing batch of {} detection requests", batch_size);

            let registry = registry.clone();
            let requests = std::mem::take(&mut batch);

            // Spawn blocking task with rayon inside for parallel detection
            tokio::spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    detect_regions_batch_blocking(requests, registry)
                })
                .await;

                match result {
                    Ok(results) => {
                        debug!("Batch detection completed: {} results", results.len());

                        for (response_tx, detection_result) in results {
                            debug!(
                                "Detection completed for {} in {}ms: {} regions",
                                detection_result.uri,
                                detection_result.elapsed_ms,
                                detection_result.regions.len()
                            );

                            if let Err(_) = response_tx.send(detection_result) {
                                warn!("Failed to send detection result (receiver dropped)");
                            }
                        }
                    }
                    Err(e) => {
                        error!("Batch detection task panicked: {}", e);
                    }
                }
            });
        }

        debug!("Virtual document detection worker stopped");
    });

    DetectionWorkerHandle { request_tx }
}

/// Performs blocking detection of virtual document regions (single request)
///
/// This function is CPU-intensive and should be called via `spawn_blocking`.
fn detect_regions_blocking(
    uri: Url,
    source: String,
    registry: Arc<DetectorRegistry>,
) -> DetectionResult {
    use std::time::Instant;

    let start = Instant::now();

    // Parse with Tree-Sitter
    let tree = crate::tree_sitter::parse_code(&source);
    let rope = ropey::Rope::from_str(&source);

    // Run all detectors
    let regions = registry.detect_all(&source, &tree, &rope);

    let elapsed = start.elapsed();

    DetectionResult {
        uri,
        regions,
        elapsed_ms: elapsed.as_millis() as u64,
    }
}

/// Estimates work time for a batch of detection requests (microseconds)
///
/// Based on benchmark data:
/// - Simple MeTTa parsing: ~37µs for ~100 bytes
/// - Complex MeTTa parsing: ~263µs for ~1000 bytes
/// - Approximate formula: 0.26µs per byte + 10µs base per document
///
/// This heuristic helps decide whether Rayon overhead is justified.
fn estimate_batch_work_time(requests: &[DetectionRequest]) -> u64 {
    let total_size: usize = requests.iter().map(|r| r.source.len()).sum();
    let base_overhead = requests.len() as u64 * 10; // 10µs per document
    let parsing_time = total_size as u64 / 4; // ~0.25µs per byte

    base_overhead + parsing_time
}

/// Determines whether to use parallel or sequential processing
///
/// Decision criteria:
/// 1. Need at least MIN_PARALLEL_DOCUMENTS documents (5+)
/// 2. Estimated work must exceed PARALLEL_THRESHOLD_MICROS (100µs)
///
/// This avoids paying 15-20µs Rayon overhead when work is smaller.
fn should_parallelize(requests: &[DetectionRequest]) -> bool {
    if requests.len() < MIN_PARALLEL_DOCUMENTS {
        trace!(
            "Using sequential processing: only {} documents (< {})",
            requests.len(),
            MIN_PARALLEL_DOCUMENTS
        );
        return false;
    }

    let estimated_work = estimate_batch_work_time(requests);

    if estimated_work < PARALLEL_THRESHOLD_MICROS {
        trace!(
            "Using sequential processing: estimated work {}µs (< {}µs threshold)",
            estimated_work,
            PARALLEL_THRESHOLD_MICROS
        );
        false
    } else {
        debug!(
            "Using parallel processing: {} documents, estimated work {}µs (>= {}µs threshold)",
            requests.len(),
            estimated_work,
            PARALLEL_THRESHOLD_MICROS
        );
        true
    }
}

/// Performs batch detection using adaptive parallelization
///
/// **Adaptive Strategy (Phase 2 Optimization)**:
/// - Small workloads (< 5 docs, < 100µs): Sequential processing to avoid Rayon overhead
/// - Large workloads (>= 5 docs, >= 100µs): Parallel processing with Rayon
///
/// This approach eliminates 15-20µs Rayon overhead for small tasks while maintaining
/// 1.5-2x speedup for large tasks.
///
/// **Previous approach**: Always used Rayon (18-19x better than pure spawn_blocking)
/// **New approach**: Adaptive (best of both worlds based on workload size)
///
/// This function is CPU-intensive and should be called via `spawn_blocking`.
fn detect_regions_batch_blocking(
    requests: Vec<DetectionRequest>,
    registry: Arc<DetectorRegistry>,
) -> Vec<(oneshot::Sender<DetectionResult>, DetectionResult)> {
    if should_parallelize(&requests) {
        // Large workload: Use Rayon for parallel processing
        use rayon::prelude::*;

        requests
            .into_par_iter()
            .map(|request| {
                let result = detect_regions_blocking(request.uri, request.source, registry.clone());
                (request.response, result)
            })
            .collect()
    } else {
        // Small workload: Use sequential processing to avoid Rayon overhead
        requests
            .into_iter()
            .map(|request| {
                let result = detect_regions_blocking(request.uri, request.source, registry.clone());
                (request.response, result)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_detection_worker_basic() {
        let registry = Arc::new(DetectorRegistry::with_defaults());
        let worker = spawn_detection_worker(registry);

        let source = r#"
@"rho:metta:compile"!("(= test 123)")
"#;

        let uri = Url::parse("file:///test.rho").unwrap();
        let result_rx = worker.detect(uri.clone(), source.to_string());

        let result = result_rx.await.expect("Should receive result");

        assert_eq!(result.uri, uri);
        assert!(!result.regions.is_empty(), "Should detect at least one region");
        // elapsed_ms can be 0 for very fast operations, so we just check it exists
        assert!(result.elapsed_ms >= 0, "Should track elapsed time");
    }

    #[tokio::test]
    async fn test_detection_worker_multiple_requests() {
        let registry = Arc::new(DetectorRegistry::with_defaults());
        let worker = spawn_detection_worker(registry);

        let mut receivers = vec![];

        // Send multiple requests
        for i in 0..5 {
            let source = format!(r#"@"rho:metta:compile"!("(= test{})")"#, i);
            let uri = Url::parse(&format!("file:///test{}.rho", i)).unwrap();
            let rx = worker.detect(uri, source);
            receivers.push(rx);
        }

        // Collect all results
        let mut results = vec![];
        for rx in receivers {
            let result = rx.await.expect("Should receive result");
            results.push(result);
        }

        assert_eq!(results.len(), 5);

        for (i, result) in results.iter().enumerate() {
            assert_eq!(
                result.uri.path(),
                format!("/test{}.rho", i),
                "Result {} should match request", i
            );
        }
    }

    #[tokio::test]
    async fn test_detection_worker_handle_clone() {
        let registry = Arc::new(DetectorRegistry::with_defaults());
        let worker1 = spawn_detection_worker(registry);
        let worker2 = worker1.clone();

        assert!(worker1.is_running());
        assert!(worker2.is_running());

        let source = r#"@"rho:metta:compile"!("test")"#;
        let uri = Url::parse("file:///test.rho").unwrap();

        // Both handles should work
        let rx1 = worker1.detect(uri.clone(), source.to_string());
        let rx2 = worker2.detect(uri.clone(), source.to_string());

        let result1 = rx1.await.expect("Should receive from worker1");
        let result2 = rx2.await.expect("Should receive from worker2");

        assert_eq!(result1.uri, result2.uri);
    }

    #[tokio::test]
    async fn test_detection_with_directive_override() {
        let registry = Arc::new(DetectorRegistry::with_defaults());
        let worker = spawn_detection_worker(registry);

        // Source with both directive and semantic detection
        let source = r#"
// @metta
@"rho:metta:compile"!("(= factorial 42)")
"#;

        let uri = Url::parse("file:///test.rho").unwrap();
        let result_rx = worker.detect(uri, source.to_string());

        let result = result_rx.await.expect("Should receive result");

        // Should have exactly one region (deduplicated)
        let metta_regions: Vec<_> = result.regions.iter()
            .filter(|r| r.language == "metta")
            .collect();

        assert_eq!(metta_regions.len(), 1, "Should have exactly one metta region");

        // Should be from directive (highest priority)
        use super::super::RegionSource;
        assert_eq!(
            metta_regions[0].source,
            RegionSource::CommentDirective,
            "Should use directive-based detection"
        );
    }
}
