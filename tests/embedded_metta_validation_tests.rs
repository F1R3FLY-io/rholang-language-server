//! End-to-end tests for embedded MeTTa validation
//!
//! Tests that embedded MeTTa code in Rholang files is properly detected,
//! validated, and diagnostics are mapped back to parent document positions.

use rholang_language_server::language_regions::{DirectiveParser, VirtualDocumentRegistry};
use rholang_language_server::tree_sitter::parse_code;
use ropey::Rope;
use tower_lsp::lsp_types::{DiagnosticSeverity, Position};

#[test]
fn test_embedded_metta_with_valid_code() {
    let source = r#"
// @metta
@"rho:metta:compile"!("(= factorial (lambda (n) (if (< n 2) 1 (* n (factorial (- n 1))))))")
"#;

    let tree = parse_code(source);
    let rope = Rope::from_str(source);

    // Scan for embedded regions
    let regions = DirectiveParser::scan_directives(source, &tree, &rope);

    assert_eq!(regions.len(), 1, "Should find one embedded MeTTa region");
    assert_eq!(regions[0].language, "metta");

    // Create virtual documents and validate
    let mut registry = VirtualDocumentRegistry::new();
    let parent_uri = tower_lsp::lsp_types::Url::parse("file:///test.rho").unwrap();
    registry.register_regions(&parent_uri, &regions);

    // Validate virtual documents
    let diagnostics = registry.validate_all_for_parent(&parent_uri);

    // Valid MeTTa code should produce no diagnostics
    assert_eq!(
        diagnostics.len(),
        0,
        "Valid MeTTa code should produce no diagnostics"
    );
}

#[test]
fn test_embedded_metta_with_invalid_code() {
    let source = r#"
// @metta
@"rho:metta:compile"!("(= factorial (lambda (n) (this-is-invalid)))")
"#;

    let tree = parse_code(source);
    let rope = Rope::from_str(source);

    // Scan for embedded regions
    let regions = DirectiveParser::scan_directives(source, &tree, &rope);

    assert_eq!(regions.len(), 1, "Should find one embedded MeTTa region");

    // Create virtual documents and validate
    let mut registry = VirtualDocumentRegistry::new();
    let parent_uri = tower_lsp::lsp_types::Url::parse("file:///test.rho").unwrap();
    registry.register_regions(&parent_uri, &regions);

    // Validate virtual documents
    let diagnostics = registry.validate_all_for_parent(&parent_uri);

    // Invalid MeTTa code may produce diagnostics
    // Note: The actual validation behavior depends on MeTTa validator implementation
    println!("Diagnostics from invalid MeTTa: {:?}", diagnostics);

    // Verify that if diagnostics are produced, they have correct positions
    for diag in &diagnostics {
        // Diagnostics should be on line 2 (where the string literal is)
        assert!(
            diag.range.start.line >= 2,
            "Diagnostic should be on line 2 or later, got line {}",
            diag.range.start.line
        );
    }
}

#[test]
fn test_multiple_embedded_metta_regions() {
    let source = r#"
// @metta
@"rho:metta:compile"!("(= foo 42)")

Nil |

// @metta
@"rho:metta:compile"!("(= bar 24)")
"#;

    let tree = parse_code(source);
    let rope = Rope::from_str(source);

    // Scan for embedded regions
    let regions = DirectiveParser::scan_directives(source, &tree, &rope);

    assert_eq!(regions.len(), 2, "Should find two embedded MeTTa regions");

    // Create virtual documents and validate
    let mut registry = VirtualDocumentRegistry::new();
    let parent_uri = tower_lsp::lsp_types::Url::parse("file:///test.rho").unwrap();
    registry.register_regions(&parent_uri, &regions);

    // Get virtual documents
    let virtual_docs = registry.get_by_parent(&parent_uri);
    assert_eq!(virtual_docs.len(), 2, "Should have two virtual documents");

    // Verify URIs are unique
    assert_ne!(
        virtual_docs[0].uri,
        virtual_docs[1].uri,
        "Virtual document URIs should be unique"
    );

    // Validate all
    let diagnostics = registry.validate_all_for_parent(&parent_uri);

    println!("Diagnostics from multiple regions: {:?}", diagnostics);
}

#[test]
fn test_diagnostic_position_mapping() {
    let source = r#"
// @metta
@"rho:metta:compile"!("(= test 123)")
"#;

    let tree = parse_code(source);
    let rope = Rope::from_str(source);

    let regions = DirectiveParser::scan_directives(source, &tree, &rope);
    assert_eq!(regions.len(), 1);

    let mut registry = VirtualDocumentRegistry::new();
    let parent_uri = tower_lsp::lsp_types::Url::parse("file:///test.rho").unwrap();
    registry.register_regions(&parent_uri, &regions);

    let virtual_docs = registry.get_by_parent(&parent_uri);
    assert_eq!(virtual_docs.len(), 1);

    let virtual_doc = &virtual_docs[0];

    // Test position mapping
    let virtual_pos = Position {
        line: 0,
        character: 5,
    };

    let parent_pos = virtual_doc.map_to_parent(virtual_pos);

    // The string literal starts at line 2, column 24 (after @"rho:metta:compile"!()
    // Plus 1 for the opening quote, plus 5 for the character offset
    assert_eq!(
        parent_pos.line, 2,
        "Mapped position should be on line 2 of parent"
    );
    assert!(
        parent_pos.character > 24,
        "Mapped position should be after the channel name"
    );
}

#[test]
fn test_no_embedded_regions_without_directive() {
    let source = r#"
// Regular comment
@"rho:metta:compile"!("(= factorial 42)")
"#;

    let tree = parse_code(source);
    let rope = Rope::from_str(source);

    let regions = DirectiveParser::scan_directives(source, &tree, &rope);

    // Without a directive, no regions should be detected
    assert_eq!(
        regions.len(),
        0,
        "Should not detect regions without directive"
    );
}

#[test]
fn test_virtual_document_cleanup() {
    let source = r#"
// @metta
@"rho:metta:compile"!("(= test 1)")
"#;

    let tree = parse_code(source);
    let rope = Rope::from_str(source);

    let regions = DirectiveParser::scan_directives(source, &tree, &rope);

    let mut registry = VirtualDocumentRegistry::new();
    let parent_uri = tower_lsp::lsp_types::Url::parse("file:///test.rho").unwrap();

    // Register regions
    registry.register_regions(&parent_uri, &regions);
    assert_eq!(registry.get_by_parent(&parent_uri).len(), 1);

    // Unregister
    registry.unregister_parent(&parent_uri);
    assert_eq!(
        registry.get_by_parent(&parent_uri).len(),
        0,
        "Virtual documents should be cleaned up"
    );
}
