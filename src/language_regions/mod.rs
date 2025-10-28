pub mod directive_parser;
pub mod semantic_detector;
pub mod channel_flow_analyzer;
pub mod virtual_document;
pub mod concatenation;

pub use directive_parser::{DirectiveParser, LanguageRegion, RegionSource};
pub use semantic_detector::SemanticDetector;
pub use channel_flow_analyzer::ChannelFlowAnalyzer;
pub use virtual_document::{VirtualDocument, VirtualDocumentRegistry};
pub use concatenation::{ConcatPart, ConcatenationChain, HoledPositionMap, extract_concatenation_chain};
