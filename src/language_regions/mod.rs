pub mod directive_parser;
pub mod semantic_detector;
pub mod channel_flow_analyzer;
pub mod virtual_document;
pub mod concatenation;
pub mod detector;
pub mod detector_registry;
pub mod async_detection;

pub use directive_parser::{DirectiveParser, LanguageRegion, RegionSource};
pub use semantic_detector::SemanticDetector;
pub use channel_flow_analyzer::ChannelFlowAnalyzer;
pub use virtual_document::{VirtualDocument, VirtualDocumentRegistry};
pub use concatenation::{ConcatPart, ConcatenationChain, HoledPositionMap, extract_concatenation_chain};
pub use detector::VirtualDocumentDetector;
pub use detector_registry::DetectorRegistry;
pub use async_detection::{DetectionWorkerHandle, DetectionRequest, DetectionResult, spawn_detection_worker};
