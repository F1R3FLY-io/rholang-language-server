//! End-to-end tests for semantic MeTTa detection
//!
//! Tests that MeTTa code sent to compiler channels is automatically detected
//! without requiring comment directives. This includes both direct sends to
//! compiler channels and sends via channel flow analysis.

use rholang_language_server::language_regions::{ChannelFlowAnalyzer, DirectiveParser, SemanticDetector, RegionSource};
use rholang_language_server::tree_sitter::parse_code;
use ropey::Rope;

#[test]
fn test_semantic_detection_without_directive() {
    let source = r#"
@"rho:metta:compile"!("(= factorial (lambda (n) 42))")
"#;

    let tree = parse_code(source);
    let rope = Rope::from_str(source);

    // Semantic detection should find the region
    let regions = SemanticDetector::detect_regions(source, &tree, &rope);

    assert_eq!(regions.len(), 1, "Should detect one MeTTa region");
    assert_eq!(regions[0].language, "metta");
    assert_eq!(regions[0].source, RegionSource::SemanticAnalysis);
    assert!(regions[0].content.contains("factorial"));
}

#[test]
fn test_both_directive_and_semantic_detection() {
    let source = r#"
// @metta
@"rho:metta:compile"!("(= foo 1)")
"#;

    let tree = parse_code(source);
    let rope = Rope::from_str(source);

    // Both methods should find the region
    let directive_regions = DirectiveParser::scan_directives(source, &tree, &rope);
    let semantic_regions = SemanticDetector::detect_regions(source, &tree, &rope);

    assert_eq!(directive_regions.len(), 1, "Directive parser should find it");
    assert_eq!(semantic_regions.len(), 1, "Semantic detector should find it");

    // They should detect the same region
    assert_eq!(
        directive_regions[0].content, semantic_regions[0].content,
        "Both should detect the same content"
    );
}

#[test]
fn test_semantic_detection_multiple_sends() {
    let source = r#"
@"rho:metta:compile"!("(= foo 1)") |
@"rho:metta:compile"!("(= bar 2)") |
@"rho:metta:eval"!("(+ 1 2)")
"#;

    let tree = parse_code(source);
    let rope = Rope::from_str(source);

    let regions = SemanticDetector::detect_regions(source, &tree, &rope);

    assert_eq!(regions.len(), 3, "Should detect three MeTTa regions");

    // Verify all are MeTTa
    for region in &regions {
        assert_eq!(region.language, "metta");
        assert_eq!(region.source, RegionSource::SemanticAnalysis);
    }
}

#[test]
fn test_semantic_detection_ignores_other_channels() {
    let source = r#"
@"rho:io:stdout"!("hello") |
@"rho:registry:lookup"!("key") |
@"rho:metta:compile"!("(= test 1)")
"#;

    let tree = parse_code(source);
    let rope = Rope::from_str(source);

    let regions = SemanticDetector::detect_regions(source, &tree, &rope);

    assert_eq!(
        regions.len(),
        1,
        "Should only detect the MeTTa compiler send"
    );
    assert!(regions[0].content.contains("test"));
}

#[test]
fn test_combined_detection_no_duplicates() {
    let source = r#"
// @metta
@"rho:metta:compile"!("(= with_directive 1)")

@"rho:metta:compile"!("(= without_directive 2)")
"#;

    let tree = parse_code(source);
    let rope = Rope::from_str(source);

    let directive_regions = DirectiveParser::scan_directives(source, &tree, &rope);
    let semantic_regions = SemanticDetector::detect_regions(source, &tree, &rope);

    // Directive parser finds the first one
    assert_eq!(directive_regions.len(), 1);
    assert!(directive_regions[0].content.contains("with_directive"));

    // Semantic detector finds both
    assert_eq!(semantic_regions.len(), 2);

    // When combined (like in the backend), we should get both without duplicates
    let mut all_regions = directive_regions.clone();

    for semantic_region in semantic_regions {
        let overlaps = all_regions.iter().any(|r| {
            (semantic_region.start_byte >= r.start_byte
                && semantic_region.start_byte < r.end_byte)
                || (semantic_region.end_byte > r.start_byte
                    && semantic_region.end_byte <= r.end_byte)
                || (semantic_region.start_byte <= r.start_byte
                    && semantic_region.end_byte >= r.end_byte)
        });

        if !overlaps {
            all_regions.push(semantic_region);
        }
    }

    assert_eq!(
        all_regions.len(),
        2,
        "Should have 2 unique regions after merging"
    );
}

#[test]
fn test_semantic_detection_nested_in_block() {
    let source = r#"
new x in {
  @"rho:metta:compile"!("(= nested 42)")
}
"#;

    let tree = parse_code(source);
    let rope = Rope::from_str(source);

    let regions = SemanticDetector::detect_regions(source, &tree, &rope);

    assert_eq!(
        regions.len(),
        1,
        "Should detect MeTTa even when nested in blocks"
    );
    assert!(regions[0].content.contains("nested"));
}

// ==================== CHANNEL FLOW ANALYSIS TESTS ====================

#[test]
fn test_channel_flow_basic() {
    let source = r#"
new metta in {
  for (metta <- @"rho:metta:compile") {
    metta!("(= factorial 42)")
  }
}
"#;

    let tree = parse_code(source);
    let rope = Rope::from_str(source);

    let regions = ChannelFlowAnalyzer::analyze(source, &tree, &rope);

    assert_eq!(regions.len(), 1, "Should detect region via flow analysis");
    assert_eq!(regions[0].language, "metta");
    assert_eq!(regions[0].source, RegionSource::ChannelFlow);
    assert!(regions[0].content.contains("factorial"));
}

#[test]
fn test_channel_flow_multiple_sends() {
    let source = r#"
new metta in {
  for (metta <- @"rho:metta:compile") {
    metta!("(= foo 1)") |
    metta!("(= bar 2)")
  }
}
"#;

    let tree = parse_code(source);
    let rope = Rope::from_str(source);

    let regions = ChannelFlowAnalyzer::analyze(source, &tree, &rope);

    assert_eq!(regions.len(), 2, "Should detect both sends");
    assert!(regions[0].content.contains("foo"));
    assert!(regions[1].content.contains("bar"));
}

#[test]
fn test_channel_flow_scoping() {
    let source = r#"
new metta in {
  for (metta <- @"rho:metta:compile") {
    metta!("(= outer 1)")
  }
} |
new metta in {
  for (metta <- @"rho:io:stdout") {
    metta!("not metta code")
  }
}
"#;

    let tree = parse_code(source);
    let rope = Rope::from_str(source);

    let regions = ChannelFlowAnalyzer::analyze(source, &tree, &rope);

    // Only the first metta binding should be detected
    assert_eq!(regions.len(), 1, "Should only detect MeTTa channel binding");
    assert!(regions[0].content.contains("outer"));
}

#[test]
fn test_combined_detection_all_three_methods() {
    let source = r#"
// @metta
@"rho:metta:compile"!("(= with_directive 1)")

@"rho:metta:compile"!("(= semantic_only 2)")

new metta in {
  for (metta <- @"rho:metta:compile") {
    metta!("(= flow_only 3)")
  }
}
"#;

    let tree = parse_code(source);
    let rope = Rope::from_str(source);

    let directive_regions = DirectiveParser::scan_directives(source, &tree, &rope);
    let semantic_regions = SemanticDetector::detect_regions(source, &tree, &rope);
    let flow_regions = ChannelFlowAnalyzer::analyze(source, &tree, &rope);

    // Directive finds the first one
    assert_eq!(directive_regions.len(), 1);
    assert!(directive_regions[0].content.contains("with_directive"));

    // Semantic finds the first two
    assert_eq!(semantic_regions.len(), 2);

    // Flow finds the third one
    assert_eq!(flow_regions.len(), 1);
    assert!(flow_regions[0].content.contains("flow_only"));

    // Combined, we should have 3 unique regions
    let mut all_regions = directive_regions.clone();

    for semantic_region in semantic_regions {
        let overlaps = all_regions.iter().any(|r| {
            (semantic_region.start_byte >= r.start_byte && semantic_region.start_byte < r.end_byte)
                || (semantic_region.end_byte > r.start_byte && semantic_region.end_byte <= r.end_byte)
                || (semantic_region.start_byte <= r.start_byte && semantic_region.end_byte >= r.end_byte)
        });
        if !overlaps {
            all_regions.push(semantic_region);
        }
    }

    for flow_region in flow_regions {
        let overlaps = all_regions.iter().any(|r| {
            (flow_region.start_byte >= r.start_byte && flow_region.start_byte < r.end_byte)
                || (flow_region.end_byte > r.start_byte && flow_region.end_byte <= r.end_byte)
                || (flow_region.start_byte <= r.start_byte && flow_region.end_byte >= r.end_byte)
        });
        if !overlaps {
            all_regions.push(flow_region);
        }
    }

    assert_eq!(all_regions.len(), 3, "Should have 3 unique regions total");
}

#[test]
fn test_channel_flow_no_false_positives() {
    let source = r#"
new metta in {
  metta!("(= should_not_detect 42)")
}
"#;

    let tree = parse_code(source);
    let rope = Rope::from_str(source);

    let regions = ChannelFlowAnalyzer::analyze(source, &tree, &rope);

    assert_eq!(
        regions.len(),
        0,
        "Should not detect without channel binding"
    );
}
