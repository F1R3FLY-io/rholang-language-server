//! Performance metrics collection for LSP operations
//!
//! This module provides lightweight metrics collection to monitor language server
//! performance in production. Metrics are stored in-memory and can be queried via
//! custom LSP requests or logged periodically.
//!
//! ## Metrics Tracked
//!
//! - Parse cache hit rate
//! - LSP request latencies (goto-definition, hover, etc.)
//! - Workspace indexing time
//! - Virtual document detection time
//! - Symbol resolution time
//!
//! ## Design
//!
//! - Lock-free atomic counters for high-frequency operations
//! - DashMap for low-contention histogram storage
//! - Minimal overhead (~10-20ns per metric update)

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use dashmap::DashMap;

/// Global metrics registry (singleton)
static METRICS: once_cell::sync::Lazy<Arc<Metrics>> = once_cell::sync::Lazy::new(|| {
    Arc::new(Metrics::new())
});

/// Get the global metrics instance
pub fn metrics() -> &'static Arc<Metrics> {
    &METRICS
}

/// Performance metrics registry
#[derive(Debug)]
pub struct Metrics {
    // Parse cache metrics
    parse_cache_hits: AtomicU64,
    parse_cache_misses: AtomicU64,

    // LSP request counters
    goto_definition_count: AtomicU64,
    hover_count: AtomicU64,
    references_count: AtomicU64,
    rename_count: AtomicU64,
    document_symbol_count: AtomicU64,

    // Timing histograms (operation name -> list of durations in microseconds)
    operation_timings: DashMap<String, Vec<u64>>,

    // Workspace stats
    workspace_index_count: AtomicUsize,
    total_files_indexed: AtomicUsize,

    // Error counters
    parse_errors: AtomicU64,
    validation_errors: AtomicU64,
}

impl Metrics {
    /// Creates a new metrics registry
    pub fn new() -> Self {
        Self {
            parse_cache_hits: AtomicU64::new(0),
            parse_cache_misses: AtomicU64::new(0),
            goto_definition_count: AtomicU64::new(0),
            hover_count: AtomicU64::new(0),
            references_count: AtomicU64::new(0),
            rename_count: AtomicU64::new(0),
            document_symbol_count: AtomicU64::new(0),
            operation_timings: DashMap::new(),
            workspace_index_count: AtomicUsize::new(0),
            total_files_indexed: AtomicUsize::new(0),
            parse_errors: AtomicU64::new(0),
            validation_errors: AtomicU64::new(0),
        }
    }

    /// Records a parse cache hit
    pub fn record_parse_cache_hit(&self) {
        self.parse_cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a parse cache miss
    pub fn record_parse_cache_miss(&self) {
        self.parse_cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    /// Gets the parse cache hit rate (0.0 to 1.0)
    pub fn parse_cache_hit_rate(&self) -> f64 {
        let hits = self.parse_cache_hits.load(Ordering::Relaxed);
        let misses = self.parse_cache_misses.load(Ordering::Relaxed);
        let total = hits + misses;

        if total == 0 {
            0.0
        } else {
            hits as f64 / total as f64
        }
    }

    /// Records a goto-definition request
    pub fn record_goto_definition(&self) {
        self.goto_definition_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a hover request
    pub fn record_hover(&self) {
        self.hover_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a references request
    pub fn record_references(&self) {
        self.references_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a rename request
    pub fn record_rename(&self) {
        self.rename_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a document symbol request
    pub fn record_document_symbol(&self) {
        self.document_symbol_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Records the timing of an operation
    ///
    /// # Arguments
    /// * `operation` - Name of the operation (e.g., "goto_definition", "workspace_index")
    /// * `duration` - Duration of the operation
    pub fn record_timing(&self, operation: &str, duration: Duration) {
        let micros = duration.as_micros() as u64;

        self.operation_timings
            .entry(operation.to_string())
            .or_insert_with(Vec::new)
            .push(micros);
    }

    /// Records workspace indexing completion
    pub fn record_workspace_index(&self, file_count: usize) {
        self.workspace_index_count.fetch_add(1, Ordering::Relaxed);
        self.total_files_indexed.fetch_add(file_count, Ordering::Relaxed);
    }

    /// Records a parse error
    pub fn record_parse_error(&self) {
        self.parse_errors.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a validation error
    pub fn record_validation_error(&self) {
        self.validation_errors.fetch_add(1, Ordering::Relaxed);
    }

    /// Gets summary statistics for an operation
    pub fn operation_stats(&self, operation: &str) -> Option<OperationStats> {
        self.operation_timings.get(operation).map(|timings| {
            let mut sorted = timings.value().clone();
            sorted.sort_unstable();

            let count = sorted.len();
            if count == 0 {
                return OperationStats {
                    count: 0,
                    min_micros: 0,
                    max_micros: 0,
                    mean_micros: 0,
                    p50_micros: 0,
                    p95_micros: 0,
                    p99_micros: 0,
                };
            }

            let sum: u64 = sorted.iter().sum();
            let mean = sum / count as u64;

            let p50_idx = count / 2;
            let p95_idx = (count as f64 * 0.95) as usize;
            let p99_idx = (count as f64 * 0.99) as usize;

            OperationStats {
                count,
                min_micros: sorted[0],
                max_micros: sorted[count - 1],
                mean_micros: mean,
                p50_micros: sorted[p50_idx],
                p95_micros: sorted[p95_idx.min(count - 1)],
                p99_micros: sorted[p99_idx.min(count - 1)],
            }
        })
    }

    /// Gets a summary report of all metrics
    pub fn summary(&self) -> MetricsSummary {
        MetricsSummary {
            parse_cache_hits: self.parse_cache_hits.load(Ordering::Relaxed),
            parse_cache_misses: self.parse_cache_misses.load(Ordering::Relaxed),
            parse_cache_hit_rate: self.parse_cache_hit_rate(),
            goto_definition_count: self.goto_definition_count.load(Ordering::Relaxed),
            hover_count: self.hover_count.load(Ordering::Relaxed),
            references_count: self.references_count.load(Ordering::Relaxed),
            rename_count: self.rename_count.load(Ordering::Relaxed),
            document_symbol_count: self.document_symbol_count.load(Ordering::Relaxed),
            workspace_index_count: self.workspace_index_count.load(Ordering::Relaxed),
            total_files_indexed: self.total_files_indexed.load(Ordering::Relaxed),
            parse_errors: self.parse_errors.load(Ordering::Relaxed),
            validation_errors: self.validation_errors.load(Ordering::Relaxed),
        }
    }

    /// Resets all metrics (useful for testing)
    pub fn reset(&self) {
        self.parse_cache_hits.store(0, Ordering::Relaxed);
        self.parse_cache_misses.store(0, Ordering::Relaxed);
        self.goto_definition_count.store(0, Ordering::Relaxed);
        self.hover_count.store(0, Ordering::Relaxed);
        self.references_count.store(0, Ordering::Relaxed);
        self.rename_count.store(0, Ordering::Relaxed);
        self.document_symbol_count.store(0, Ordering::Relaxed);
        self.operation_timings.clear();
        self.workspace_index_count.store(0, Ordering::Relaxed);
        self.total_files_indexed.store(0, Ordering::Relaxed);
        self.parse_errors.store(0, Ordering::Relaxed);
        self.validation_errors.store(0, Ordering::Relaxed);
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics for a single operation
#[derive(Debug, Clone)]
pub struct OperationStats {
    pub count: usize,
    pub min_micros: u64,
    pub max_micros: u64,
    pub mean_micros: u64,
    pub p50_micros: u64,  // Median
    pub p95_micros: u64,
    pub p99_micros: u64,
}

/// Summary of all metrics
#[derive(Debug, Clone)]
pub struct MetricsSummary {
    pub parse_cache_hits: u64,
    pub parse_cache_misses: u64,
    pub parse_cache_hit_rate: f64,
    pub goto_definition_count: u64,
    pub hover_count: u64,
    pub references_count: u64,
    pub rename_count: u64,
    pub document_symbol_count: u64,
    pub workspace_index_count: usize,
    pub total_files_indexed: usize,
    pub parse_errors: u64,
    pub validation_errors: u64,
}

/// RAII guard for automatic timing measurement
///
/// Records the duration of a scope when dropped.
///
/// # Example
///
/// ```
/// use rholang_language_server::metrics::{metrics, TimingGuard};
///
/// fn my_operation() {
///     let _guard = TimingGuard::new("my_operation");
///     // ... do work ...
///     // Duration automatically recorded when _guard is dropped
/// }
/// ```
pub struct TimingGuard {
    operation: String,
    start: Instant,
}

impl TimingGuard {
    /// Creates a new timing guard for the given operation
    pub fn new(operation: impl Into<String>) -> Self {
        Self {
            operation: operation.into(),
            start: Instant::now(),
        }
    }
}

impl Drop for TimingGuard {
    fn drop(&mut self) {
        let duration = self.start.elapsed();
        metrics().record_timing(&self.operation, duration);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_parse_cache_metrics() {
        let m = Metrics::new();

        assert_eq!(m.parse_cache_hit_rate(), 0.0);

        m.record_parse_cache_hit();
        m.record_parse_cache_hit();
        m.record_parse_cache_miss();

        assert_eq!(m.parse_cache_hit_rate(), 2.0 / 3.0);
    }

    #[test]
    fn test_request_counters() {
        let m = Metrics::new();

        m.record_goto_definition();
        m.record_hover();
        m.record_hover();

        let summary = m.summary();
        assert_eq!(summary.goto_definition_count, 1);
        assert_eq!(summary.hover_count, 2);
    }

    #[test]
    fn test_operation_timing() {
        let m = Metrics::new();

        m.record_timing("test_op", Duration::from_micros(100));
        m.record_timing("test_op", Duration::from_micros(200));
        m.record_timing("test_op", Duration::from_micros(150));

        let stats = m.operation_stats("test_op").unwrap();
        assert_eq!(stats.count, 3);
        assert_eq!(stats.min_micros, 100);
        assert_eq!(stats.max_micros, 200);
        assert_eq!(stats.mean_micros, 150);
        assert_eq!(stats.p50_micros, 150);
    }

    #[test]
    fn test_timing_guard() {
        let m = Metrics::new();

        {
            let _guard = TimingGuard::new("test_guard");
            thread::sleep(Duration::from_millis(10));
        }

        let stats = metrics().operation_stats("test_guard").unwrap();
        assert_eq!(stats.count, 1);
        assert!(stats.min_micros >= 10_000); // At least 10ms
    }

    #[test]
    fn test_reset() {
        let m = Metrics::new();

        m.record_parse_cache_hit();
        m.record_goto_definition();
        m.record_timing("test", Duration::from_micros(100));

        m.reset();

        let summary = m.summary();
        assert_eq!(summary.parse_cache_hits, 0);
        assert_eq!(summary.goto_definition_count, 0);
        assert!(m.operation_stats("test").is_none());
    }
}
