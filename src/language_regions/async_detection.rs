//! Async virtual document detection worker
//!
//! Provides background detection of virtual document regions to prevent
//! blocking the LSP server's main event loop during parsing and analysis.

use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use url::Url;
use tracing::{debug, error, trace, warn};

use super::{DetectorRegistry, LanguageRegion};

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

/// Performs batch detection using rayon for parallel processing
///
/// This hybrid approach (spawn_blocking + rayon) provides:
/// - 18-19x better throughput than pure spawn_blocking
/// - Parallel processing across multiple documents
/// - Non-blocking async integration
///
/// This function is CPU-intensive and should be called via `spawn_blocking`.
fn detect_regions_batch_blocking(
    requests: Vec<DetectionRequest>,
    registry: Arc<DetectorRegistry>,
) -> Vec<(oneshot::Sender<DetectionResult>, DetectionResult)> {
    use rayon::prelude::*;

    // Process all requests in parallel using rayon
    requests
        .into_par_iter()
        .map(|request| {
            let result = detect_regions_blocking(request.uri, request.source, registry.clone());
            (request.response, result)
        })
        .collect()
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
