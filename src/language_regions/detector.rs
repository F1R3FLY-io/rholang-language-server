//! Generic trait for virtual document detection
//!
//! Provides a standardized interface for detecting embedded language regions
//! within Rholang source code. Detectors can be registered dynamically and
//! run in parallel for improved performance.

use ropey::Rope;
use tree_sitter::Tree;

use super::LanguageRegion;

/// Trait for detecting embedded language regions in source code
///
/// Implementors analyze Rholang source code to find regions containing
/// embedded languages like MeTTa, SQL, GraphQL, etc.
///
/// # Thread Safety
///
/// Implementations must be `Send + Sync` to support parallel detection
/// in background tasks.
///
/// # Example
///
/// ```rust,ignore
/// pub struct MyDetector;
///
/// impl VirtualDocumentDetector for MyDetector {
///     fn name(&self) -> &str {
///         "my-language-detector"
///     }
///
///     fn detect(&self, source: &str, tree: &Tree, rope: &Rope) -> Vec<LanguageRegion> {
///         // Detection logic here
///         vec![]
///     }
/// }
/// ```
pub trait VirtualDocumentDetector: Send + Sync {
    /// Returns the unique name of this detector
    ///
    /// Used for logging, debugging, and registry management.
    fn name(&self) -> &str;

    /// Detects embedded language regions in the source code
    ///
    /// # Arguments
    ///
    /// * `source` - The source text to analyze
    /// * `tree` - The Tree-Sitter parse tree of the source
    /// * `rope` - The rope representation of the source
    ///
    /// # Returns
    ///
    /// A vector of detected language regions, potentially empty if none found.
    ///
    /// # Performance
    ///
    /// This method may be called from background tasks using `spawn_blocking`.
    /// Implementations should be CPU-bound (not I/O-bound) and avoid holding
    /// locks for extended periods.
    fn detect(&self, source: &str, tree: &Tree, rope: &Rope) -> Vec<LanguageRegion>;

    /// Indicates whether this detector supports incremental updates
    ///
    /// If true, the detector can efficiently re-detect regions when only
    /// a small portion of the source has changed.
    ///
    /// Default: false
    fn supports_incremental(&self) -> bool {
        false
    }

    /// Priority for detection ordering (higher = earlier)
    ///
    /// Detectors with higher priority run first. This is useful when
    /// one detector's results depend on another, or for performance
    /// optimization (run fast detectors first).
    ///
    /// Default: 0 (normal priority)
    fn priority(&self) -> i32 {
        0
    }

    /// Indicates whether this detector should run in parallel with others
    ///
    /// If false, this detector will run alone (useful for detectors that
    /// modify shared state or have ordering dependencies).
    ///
    /// Default: true (can run in parallel)
    fn can_run_in_parallel(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language_regions::RegionSource;

    /// Mock detector for testing
    struct MockDetector {
        name: &'static str,
        regions: Vec<LanguageRegion>,
    }

    impl VirtualDocumentDetector for MockDetector {
        fn name(&self) -> &str {
            self.name
        }

        fn detect(&self, _source: &str, _tree: &Tree, _rope: &Rope) -> Vec<LanguageRegion> {
            self.regions.clone()
        }
    }

    #[test]
    fn test_trait_defaults() {
        let detector = MockDetector {
            name: "test-detector",
            regions: vec![],
        };

        assert_eq!(detector.name(), "test-detector");
        assert!(!detector.supports_incremental());
        assert_eq!(detector.priority(), 0);
        assert!(detector.can_run_in_parallel());
    }

    #[test]
    fn test_detector_returns_regions() {
        let region = LanguageRegion {
            language: "test".to_string(),
            start_byte: 0,
            end_byte: 10,
            start_line: 0,
            start_column: 0,
            source: RegionSource::SemanticAnalysis,
            content: "test content".to_string(),
            concatenation_chain: None,
        };

        let detector = MockDetector {
            name: "test-detector",
            regions: vec![region.clone()],
        };

        let source = "";
        let tree = crate::tree_sitter::parse_code(source);
        let rope = Rope::from_str(source);

        let results = detector.detect(source, &tree, &rope);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].language, "test");
    }
}
