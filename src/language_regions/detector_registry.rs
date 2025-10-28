//! Centralized registry for virtual document detectors
//!
//! Manages registration and execution of virtual document detectors,
//! enabling parallel detection and pluggable detector architecture.

use std::sync::Arc;
use tracing::{debug, trace};

use super::{VirtualDocumentDetector, LanguageRegion};
use ropey::Rope;
use tree_sitter::Tree;

/// Registry for managing virtual document detectors
///
/// Provides centralized management of detectors with support for:
/// - Dynamic registration of new detectors
/// - Priority-based detection ordering
/// - Parallel and sequential detection modes
/// - Detector lookup by name
///
/// # Thread Safety
///
/// The registry is designed to be shared across threads. All detectors
/// must be `Send + Sync` to support concurrent detection.
///
/// # Example
///
/// ```rust,ignore
/// let mut registry = DetectorRegistry::new();
/// registry.register(Arc::new(SemanticDetector));
/// registry.register(Arc::new(DirectiveParser));
///
/// let regions = registry.detect_all(source, &tree, &rope);
/// ```
pub struct DetectorRegistry {
    detectors: Vec<Arc<dyn VirtualDocumentDetector>>,
}

impl DetectorRegistry {
    /// Creates a new empty detector registry
    pub fn new() -> Self {
        Self {
            detectors: Vec::new(),
        }
    }

    /// Creates a registry with default detectors pre-registered
    ///
    /// Registers:
    /// - `DirectiveParser` - Comment directive detection (priority 100)
    /// - `SemanticDetector` - Semantic analysis detection (priority 50)
    /// - `ChannelFlowAnalyzer` - Channel flow detection (priority 25)
    pub fn with_defaults() -> Self {
        use super::{DirectiveParser, SemanticDetector, ChannelFlowAnalyzer};

        let mut registry = Self::new();

        // Register detectors in priority order (higher priority first)
        registry.register(Arc::new(DirectiveParser));
        registry.register(Arc::new(SemanticDetector));
        registry.register(Arc::new(ChannelFlowAnalyzer::new()));

        debug!(
            "Initialized detector registry with {} default detectors",
            registry.detectors.len()
        );

        registry
    }

    /// Registers a new detector
    ///
    /// Detectors are automatically sorted by priority after registration.
    /// Higher priority detectors run first.
    ///
    /// # Arguments
    ///
    /// * `detector` - The detector to register
    pub fn register(&mut self, detector: Arc<dyn VirtualDocumentDetector>) {
        let name = detector.name().to_string();
        trace!("Registering detector: {}", name);

        self.detectors.push(detector);

        // Sort by priority (descending - higher priority first)
        self.detectors.sort_by(|a, b| b.priority().cmp(&a.priority()));

        debug!(
            "Registered detector '{}' (total: {})",
            name,
            self.detectors.len()
        );
    }

    /// Unregisters a detector by name
    ///
    /// Returns `true` if a detector was removed, `false` if not found.
    pub fn unregister(&mut self, name: &str) -> bool {
        let initial_len = self.detectors.len();
        self.detectors.retain(|d| d.name() != name);
        let removed = self.detectors.len() < initial_len;

        if removed {
            debug!("Unregistered detector '{}'", name);
        } else {
            trace!("Detector '{}' not found for unregistration", name);
        }

        removed
    }

    /// Gets all registered detectors
    ///
    /// Returns detectors in priority order (highest priority first).
    pub fn get_all(&self) -> &[Arc<dyn VirtualDocumentDetector>] {
        &self.detectors
    }

    /// Gets a detector by name
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the detector to find
    ///
    /// # Returns
    ///
    /// The detector if found, or `None` if no detector with that name exists.
    pub fn get_by_name(&self, name: &str) -> Option<Arc<dyn VirtualDocumentDetector>> {
        self.detectors
            .iter()
            .find(|d| d.name() == name)
            .cloned()
    }

    /// Runs all registered detectors sequentially
    ///
    /// Detectors run in priority order. Results from all detectors are
    /// combined into a single vector with deduplication.
    ///
    /// **Deduplication Strategy:**
    /// If multiple detectors find overlapping regions, only the region from
    /// the highest-priority detector is kept. This ensures that explicit
    /// language directives override automatic semantic detection.
    ///
    /// # Arguments
    ///
    /// * `source` - The source text to analyze
    /// * `tree` - The Tree-Sitter parse tree
    /// * `rope` - The rope representation of the source
    ///
    /// # Returns
    ///
    /// All detected language regions from all detectors, deduplicated by priority.
    pub fn detect_all(
        &self,
        source: &str,
        tree: &Tree,
        rope: &Rope,
    ) -> Vec<LanguageRegion> {
        let mut all_regions = Vec::new();

        for detector in &self.detectors {
            trace!("Running detector: {}", detector.name());
            let regions = detector.detect(source, tree, rope);

            debug!(
                "Detector '{}' found {} regions",
                detector.name(),
                regions.len()
            );

            all_regions.extend(regions);
        }

        let initial_count = all_regions.len();

        // Deduplicate overlapping regions, keeping higher-priority ones
        let deduplicated = Self::deduplicate_regions(all_regions);

        debug!(
            "Total regions detected by {} detectors: {} (deduplicated from {})",
            self.detectors.len(),
            deduplicated.len(),
            initial_count
        );

        deduplicated
    }

    /// Deduplicates overlapping regions, keeping the first occurrence
    ///
    /// Since detectors run in priority order and results are collected
    /// in order, the first region for any overlapping area will be from
    /// the highest-priority detector.
    ///
    /// Two regions are considered overlapping if they share any byte positions.
    fn deduplicate_regions(regions: Vec<LanguageRegion>) -> Vec<LanguageRegion> {
        let mut deduplicated = Vec::new();

        for region in regions {
            // Check if this region overlaps with any already-accepted region
            let overlaps = deduplicated.iter().any(|existing: &LanguageRegion| {
                Self::regions_overlap(&region, existing)
            });

            if !overlaps {
                deduplicated.push(region);
            } else {
                trace!(
                    "Skipping overlapping region at byte {}-{} (lower priority)",
                    region.start_byte,
                    region.end_byte
                );
            }
        }

        deduplicated
    }

    /// Checks if two regions overlap in byte positions
    ///
    /// Regions overlap if they share any byte positions.
    fn regions_overlap(a: &LanguageRegion, b: &LanguageRegion) -> bool {
        // Two ranges [a_start, a_end] and [b_start, b_end] overlap if:
        // a_start < b_end AND b_start < a_end
        a.start_byte < b.end_byte && b.start_byte < a.end_byte
    }

    /// Runs detectors that can execute in parallel
    ///
    /// This method separates detectors into parallel and sequential groups:
    /// - Parallel detectors: Run concurrently using provided executor
    /// - Sequential detectors: Run one at a time in priority order
    ///
    /// # Arguments
    ///
    /// * `source` - The source text to analyze
    /// * `tree` - The Tree-Sitter parse tree
    /// * `rope` - The rope representation of the source
    ///
    /// # Returns
    ///
    /// All detected language regions from all detectors.
    ///
    /// # Note
    ///
    /// This is a synchronous method. For true async parallel execution,
    /// use `detect_all_async` (to be implemented in async worker).
    pub fn detect_all_with_parallelism(
        &self,
        source: &str,
        tree: &Tree,
        rope: &Rope,
    ) -> Vec<LanguageRegion> {
        let mut all_regions = Vec::new();

        // Separate parallel and sequential detectors
        let (parallel_detectors, sequential_detectors): (Vec<_>, Vec<_>) = self
            .detectors
            .iter()
            .partition(|d| d.can_run_in_parallel());

        debug!(
            "Running {} parallel detectors and {} sequential detectors",
            parallel_detectors.len(),
            sequential_detectors.len()
        );

        // Run parallel detectors (currently sequential, will be async in worker)
        for detector in parallel_detectors {
            trace!("Running parallel detector: {}", detector.name());
            let regions = detector.detect(source, tree, rope);
            all_regions.extend(regions);
        }

        // Run sequential detectors in order
        for detector in sequential_detectors {
            trace!("Running sequential detector: {}", detector.name());
            let regions = detector.detect(source, tree, rope);
            all_regions.extend(regions);
        }

        all_regions
    }

    /// Returns the number of registered detectors
    pub fn len(&self) -> usize {
        self.detectors.len()
    }

    /// Returns `true` if no detectors are registered
    pub fn is_empty(&self) -> bool {
        self.detectors.is_empty()
    }

    /// Returns the names of all registered detectors in priority order
    pub fn detector_names(&self) -> Vec<String> {
        self.detectors
            .iter()
            .map(|d| d.name().to_string())
            .collect()
    }
}

impl Default for DetectorRegistry {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language_regions::RegionSource;

    /// Mock detector for testing
    struct MockDetector {
        name: &'static str,
        priority: i32,
        regions: Vec<LanguageRegion>,
        can_parallel: bool,
    }

    impl VirtualDocumentDetector for MockDetector {
        fn name(&self) -> &str {
            self.name
        }

        fn detect(&self, _source: &str, _tree: &Tree, _rope: &Rope) -> Vec<LanguageRegion> {
            self.regions.clone()
        }

        fn priority(&self) -> i32 {
            self.priority
        }

        fn can_run_in_parallel(&self) -> bool {
            self.can_parallel
        }
    }

    fn create_mock_region(language: &str) -> LanguageRegion {
        LanguageRegion {
            language: language.to_string(),
            start_byte: 0,
            end_byte: 10,
            start_line: 0,
            start_column: 0,
            source: RegionSource::SemanticAnalysis,
            content: "test".to_string(),
            concatenation_chain: None,
        }
    }

    #[test]
    fn test_new_registry_is_empty() {
        let registry = DetectorRegistry::new();
        assert_eq!(registry.len(), 0);
        assert!(registry.is_empty());
    }

    #[test]
    fn test_with_defaults_registers_detectors() {
        let registry = DetectorRegistry::with_defaults();
        assert_eq!(registry.len(), 3);
        assert!(!registry.is_empty());

        let names = registry.detector_names();
        assert!(names.contains(&"directive-parser".to_string()));
        assert!(names.contains(&"semantic-detector".to_string()));
        assert!(names.contains(&"channel-flow-analyzer".to_string()));
    }

    #[test]
    fn test_register_detector() {
        let mut registry = DetectorRegistry::new();

        let detector = Arc::new(MockDetector {
            name: "test-detector",
            priority: 0,
            regions: vec![],
            can_parallel: true,
        });

        registry.register(detector);
        assert_eq!(registry.len(), 1);
        assert!(registry.get_by_name("test-detector").is_some());
    }

    #[test]
    fn test_unregister_detector() {
        let mut registry = DetectorRegistry::new();

        let detector = Arc::new(MockDetector {
            name: "test-detector",
            priority: 0,
            regions: vec![],
            can_parallel: true,
        });

        registry.register(detector);
        assert_eq!(registry.len(), 1);

        let removed = registry.unregister("test-detector");
        assert!(removed);
        assert_eq!(registry.len(), 0);
        assert!(registry.get_by_name("test-detector").is_none());
    }

    #[test]
    fn test_unregister_nonexistent_detector() {
        let mut registry = DetectorRegistry::new();
        let removed = registry.unregister("nonexistent");
        assert!(!removed);
    }

    #[test]
    fn test_priority_ordering() {
        let mut registry = DetectorRegistry::new();

        // Register in random order
        registry.register(Arc::new(MockDetector {
            name: "low-priority",
            priority: 10,
            regions: vec![],
            can_parallel: true,
        }));

        registry.register(Arc::new(MockDetector {
            name: "high-priority",
            priority: 100,
            regions: vec![],
            can_parallel: true,
        }));

        registry.register(Arc::new(MockDetector {
            name: "medium-priority",
            priority: 50,
            regions: vec![],
            can_parallel: true,
        }));

        // Check they're sorted by priority (descending)
        let detectors = registry.get_all();
        assert_eq!(detectors[0].name(), "high-priority");
        assert_eq!(detectors[1].name(), "medium-priority");
        assert_eq!(detectors[2].name(), "low-priority");
    }

    #[test]
    fn test_get_by_name() {
        let mut registry = DetectorRegistry::new();

        registry.register(Arc::new(MockDetector {
            name: "test-detector",
            priority: 0,
            regions: vec![],
            can_parallel: true,
        }));

        let found = registry.get_by_name("test-detector");
        assert!(found.is_some());
        assert_eq!(found.unwrap().name(), "test-detector");

        let not_found = registry.get_by_name("nonexistent");
        assert!(not_found.is_none());
    }

    #[test]
    fn test_detect_all() {
        use crate::tree_sitter::parse_code;

        let mut registry = DetectorRegistry::new();

        // Create non-overlapping regions so they all get kept after deduplication
        let region1 = LanguageRegion {
            language: "lang1".to_string(),
            start_byte: 0,
            end_byte: 10,
            start_line: 0,
            start_column: 0,
            source: RegionSource::SemanticAnalysis,
            content: "test1".to_string(),
            concatenation_chain: None,
        };

        let region2 = LanguageRegion {
            language: "lang2".to_string(),
            start_byte: 20,
            end_byte: 30,
            start_line: 1,
            start_column: 0,
            source: RegionSource::SemanticAnalysis,
            content: "test2".to_string(),
            concatenation_chain: None,
        };

        let region3 = LanguageRegion {
            language: "lang3".to_string(),
            start_byte: 40,
            end_byte: 50,
            start_line: 2,
            start_column: 0,
            source: RegionSource::SemanticAnalysis,
            content: "test3".to_string(),
            concatenation_chain: None,
        };

        registry.register(Arc::new(MockDetector {
            name: "detector-1",
            priority: 0,
            regions: vec![region1],
            can_parallel: true,
        }));

        registry.register(Arc::new(MockDetector {
            name: "detector-2",
            priority: 0,
            regions: vec![region2, region3],
            can_parallel: true,
        }));

        let source = "";
        let tree = parse_code(source);
        let rope = Rope::from_str(source);

        let regions = registry.detect_all(source, &tree, &rope);

        assert_eq!(regions.len(), 3);
        assert_eq!(regions[0].language, "lang1");
        assert_eq!(regions[1].language, "lang2");
        assert_eq!(regions[2].language, "lang3");
    }

    #[test]
    fn test_detect_all_with_parallelism_separation() {
        use crate::tree_sitter::parse_code;

        let mut registry = DetectorRegistry::new();

        registry.register(Arc::new(MockDetector {
            name: "parallel-detector",
            priority: 0,
            regions: vec![create_mock_region("parallel")],
            can_parallel: true,
        }));

        registry.register(Arc::new(MockDetector {
            name: "sequential-detector",
            priority: 0,
            regions: vec![create_mock_region("sequential")],
            can_parallel: false,
        }));

        let source = "";
        let tree = parse_code(source);
        let rope = Rope::from_str(source);

        let regions = registry.detect_all_with_parallelism(source, &tree, &rope);

        assert_eq!(regions.len(), 2);
    }

    #[test]
    fn test_detector_names() {
        let mut registry = DetectorRegistry::new();

        registry.register(Arc::new(MockDetector {
            name: "detector-1",
            priority: 0,
            regions: vec![],
            can_parallel: true,
        }));

        registry.register(Arc::new(MockDetector {
            name: "detector-2",
            priority: 0,
            regions: vec![],
            can_parallel: true,
        }));

        let names = registry.detector_names();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"detector-1".to_string()));
        assert!(names.contains(&"detector-2".to_string()));
    }

    #[test]
    fn test_default_trait() {
        let registry = DetectorRegistry::default();
        assert_eq!(registry.len(), 3); // Should have default detectors
    }

    #[test]
    fn test_deduplication_respects_priority() {
        use crate::tree_sitter::parse_code;

        let mut registry = DetectorRegistry::new();

        // High priority detector
        registry.register(Arc::new(MockDetector {
            name: "high-priority",
            priority: 100,
            regions: vec![create_mock_region("high-lang")],
            can_parallel: true,
        }));

        // Low priority detector with overlapping region
        registry.register(Arc::new(MockDetector {
            name: "low-priority",
            priority: 10,
            regions: vec![create_mock_region("low-lang")],
            can_parallel: true,
        }));

        let source = "";
        let tree = parse_code(source);
        let rope = Rope::from_str(source);

        let regions = registry.detect_all(source, &tree, &rope);

        // Should only get the high-priority region
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language, "high-lang");
    }

    #[test]
    fn test_non_overlapping_regions_kept() {
        use crate::tree_sitter::parse_code;

        let mut registry = DetectorRegistry::new();

        let region1 = LanguageRegion {
            language: "lang1".to_string(),
            start_byte: 0,
            end_byte: 10,
            start_line: 0,
            start_column: 0,
            source: RegionSource::SemanticAnalysis,
            content: "test1".to_string(),
            concatenation_chain: None,
        };

        let region2 = LanguageRegion {
            language: "lang2".to_string(),
            start_byte: 20, // No overlap with region1
            end_byte: 30,
            start_line: 1,
            start_column: 0,
            source: RegionSource::SemanticAnalysis,
            content: "test2".to_string(),
            concatenation_chain: None,
        };

        registry.register(Arc::new(MockDetector {
            name: "detector-1",
            priority: 100,
            regions: vec![region1],
            can_parallel: true,
        }));

        registry.register(Arc::new(MockDetector {
            name: "detector-2",
            priority: 50,
            regions: vec![region2],
            can_parallel: true,
        }));

        let source = "";
        let tree = parse_code(source);
        let rope = Rope::from_str(source);

        let regions = registry.detect_all(source, &tree, &rope);

        // Both non-overlapping regions should be kept
        assert_eq!(regions.len(), 2);
        assert_eq!(regions[0].language, "lang1");
        assert_eq!(regions[1].language, "lang2");
    }

    #[test]
    fn test_regions_overlap_detection() {
        let region1 = LanguageRegion {
            language: "test".to_string(),
            start_byte: 0,
            end_byte: 10,
            start_line: 0,
            start_column: 0,
            source: RegionSource::SemanticAnalysis,
            content: "test".to_string(),
            concatenation_chain: None,
        };

        let region2_overlap = LanguageRegion {
            language: "test".to_string(),
            start_byte: 5,
            end_byte: 15,
            start_line: 0,
            start_column: 5,
            source: RegionSource::SemanticAnalysis,
            content: "test".to_string(),
            concatenation_chain: None,
        };

        let region3_no_overlap = LanguageRegion {
            language: "test".to_string(),
            start_byte: 20,
            end_byte: 30,
            start_line: 1,
            start_column: 0,
            source: RegionSource::SemanticAnalysis,
            content: "test".to_string(),
            concatenation_chain: None,
        };

        assert!(DetectorRegistry::regions_overlap(&region1, &region2_overlap));
        assert!(!DetectorRegistry::regions_overlap(&region1, &region3_no_overlap));
    }

    #[test]
    fn test_directive_overrides_semantic_detection() {
        use crate::tree_sitter::parse_code;

        // Simulate a scenario where both directive and semantic detection
        // find the same MeTTa string, but directive should win

        let source = r#"
// @metta
@"rho:metta:compile"!("(= test 123)")
"#;

        let tree = parse_code(source);
        let rope = Rope::from_str(source);

        let registry = DetectorRegistry::with_defaults();
        let regions = registry.detect_all(source, &tree, &rope);

        // Should have detected the region
        assert!(!regions.is_empty(), "Should detect at least one region");

        // If both directive and semantic detection found it,
        // deduplication should have kept only one
        let metta_regions: Vec<_> = regions.iter()
            .filter(|r| r.language == "metta")
            .collect();

        // Should have exactly one metta region (deduplicated)
        assert_eq!(metta_regions.len(), 1);

        // The kept region should be from the highest priority detector (DirectiveParser)
        assert_eq!(metta_regions[0].source, RegionSource::CommentDirective);
    }
}
