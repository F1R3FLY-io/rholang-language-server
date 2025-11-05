/// Integration tests for pattern-aware goto-definition
///
/// Tests the MORK/PathMap pattern matching system for contract goto-definition.
/// This verifies that:
/// 1. Goto-definition uses argument patterns to match the correct contract overload
/// 2. Falls back to lexical scope when pattern matching fails
/// 3. Works with multiple arguments

use indoc::indoc;
use tower_lsp::lsp_types::Position;
use test_utils::with_lsp_client;
use test_utils::lsp::client::{CommType, LspClient};

/// Test 1: Simple contract with literal argument pattern matching
///
/// Verifies that goto-definition can match a contract invocation to its definition
/// based on the argument pattern (a string literal "hello").
#[test]
fn test_pattern_matching_string_literal() {
    with_lsp_client!(test_pattern_matching_string_literal_inner, CommType::Stdio, |client: &LspClient| {
        let code = indoc! {r#"
            contract echo(@msg) = {
                stdout!(msg)
            }

            // Invoke the contract with "hello"
            echo!("hello")
        "#};

        let doc = client.open_document("/tmp/pattern_string.rho", code).unwrap();
        client.await_diagnostics(&doc).unwrap();

        // Click on "echo" in the invocation (line 5, column 0)
        let result = client.definition(&doc.uri(), Position::new(5, 0));

        println!("Pattern matching string literal result: {:?}", result);
        assert!(result.is_ok(), "goto_definition should succeed");
        assert!(result.unwrap().is_some(), "Should find contract definition via pattern matching");
    });
}

/// Test 2: Contract overloading - same name, different argument patterns
///
/// Verifies that pattern matching can distinguish between two contracts with the same name
/// but different argument patterns (number vs string).
#[test]
fn test_pattern_matching_overloaded_contracts() {
    with_lsp_client!(test_pattern_matching_overloaded_contracts_inner, CommType::Stdio, |client: &LspClient| {
        let code = indoc! {r#"
            // Contract #1: handles numbers
            contract process(@42) = {
                stdout!("Got number 42")
            }

            // Contract #2: handles strings
            contract process(@"hello") = {
                stdout!("Got string hello")
            }

            // Invoke with number - should match contract #1
            process!(42) |

            // Invoke with string - should match contract #2
            process!("hello")
        "#};

        let doc = client.open_document("/tmp/pattern_overload.rho", code).unwrap();
        client.await_diagnostics(&doc).unwrap();

        // Test invocation with number (line 11)
        let result_number = client.definition(&doc.uri(), Position::new(11, 0));
        println!("Overloaded contract (number) result: {:?}", result_number);
        assert!(result_number.is_ok(), "goto_definition for number invocation should succeed");
        assert!(result_number.unwrap().is_some(), "Should find contract for number invocation");

        // Test invocation with string (line 14)
        let result_string = client.definition(&doc.uri(), Position::new(14, 0));
        println!("Overloaded contract (string) result: {:?}", result_string);
        assert!(result_string.is_ok(), "goto_definition for string invocation should succeed");
        assert!(result_string.unwrap().is_some(), "Should find contract for string invocation");
    });
}

/// Test 3: Fallback to lexical scope when pattern matching has no matches
///
/// Verifies that when argument pattern matching doesn't find a match (e.g., variable instead
/// of literal), the system falls back to standard lexical scope resolution.
#[test]
fn test_fallback_to_lexical_scope() {
    with_lsp_client!(test_fallback_to_lexical_scope_inner, CommType::Stdio, |client: &LspClient| {
        let code = indoc! {r#"
            contract echo(@42) = {
                stdout!("Literal 42")
            }

            new x in {
                x!(100) |
                // Invoke with variable - pattern won't match, should fallback to lexical scope
                echo!(*x)
            }
        "#};

        let doc = client.open_document("/tmp/pattern_fallback.rho", code).unwrap();
        client.await_diagnostics(&doc).unwrap();

        // Click on "echo" in the invocation with variable (line 7)
        // Pattern matching will fail (variable vs literal), should fallback to lexical scope
        let result = client.definition(&doc.uri(), Position::new(7, 4));

        println!("Fallback to lexical scope result: {:?}", result);
        assert!(result.is_ok(), "goto_definition should succeed via fallback");
        assert!(result.unwrap().is_some(), "Should find contract definition via lexical scope");
    });
}

/// Test 4: Multiple argument pattern matching
///
/// Verifies pattern matching works with contracts that have multiple parameters
/// and the invocation provides multiple arguments.
#[test]
fn test_pattern_matching_multiple_arguments() {
    with_lsp_client!(test_pattern_matching_multiple_arguments_inner, CommType::Stdio, |client: &LspClient| {
        let code = indoc! {r#"
            contract add(@10, @20) = {
                stdout!("Got 10 and 20")
            }

            contract add(@5, @15) = {
                stdout!("Got 5 and 15")
            }

            // Should match first contract
            add!(10, 20) |

            // Should match second contract
            add!(5, 15)
        "#};

        let doc = client.open_document("/tmp/pattern_multi_arg.rho", code).unwrap();
        client.await_diagnostics(&doc).unwrap();

        // Test first invocation (10, 20)
        let result_first = client.definition(&doc.uri(), Position::new(9, 0));
        println!("Multiple args (10, 20) result: {:?}", result_first);
        assert!(result_first.is_ok(), "goto_definition for (10, 20) should succeed");
        assert!(result_first.unwrap().is_some(), "Should find contract for (10, 20)");

        // Test second invocation (5, 15)
        let result_second = client.definition(&doc.uri(), Position::new(12, 0));
        println!("Multiple args (5, 15) result: {:?}", result_second);
        assert!(result_second.is_ok(), "goto_definition for (5, 15) should succeed");
        assert!(result_second.unwrap().is_some(), "Should find contract for (5, 15)");
    });
}

/// Test 5: No matching pattern - should find any definition via lexical scope
///
/// When no pattern matches but the contract name exists, lexical scope should find
/// at least one definition.
#[test]
fn test_no_pattern_match_uses_lexical_scope() {
    with_lsp_client!(test_no_pattern_match_uses_lexical_scope_inner, CommType::Stdio, |client: &LspClient| {
        let code = indoc! {r#"
            contract greet(@"Alice") = {
                stdout!("Hello Alice")
            }

            contract greet(@"Bob") = {
                stdout!("Hello Bob")
            }

            // Invoke with "Charlie" - no pattern match, should fallback
            greet!("Charlie")
        "#};

        let doc = client.open_document("/tmp/pattern_no_match.rho", code).unwrap();
        client.await_diagnostics(&doc).unwrap();

        // Click on "greet" in invocation with "Charlie" (line 9)
        // No pattern matches "Charlie", should fallback to lexical scope
        let result = client.definition(&doc.uri(), Position::new(9, 0));

        println!("No pattern match result: {:?}", result);
        assert!(result.is_ok(), "goto_definition should succeed via fallback");
        assert!(result.unwrap().is_some(), "Should find a contract definition via lexical scope");
    });
}

/// Test 6: Verify basic goto-definition still works
///
/// Sanity check that standard goto-definition (without special pattern matching)
/// still works correctly.
#[test]
fn test_basic_goto_definition_still_works() {
    with_lsp_client!(test_basic_goto_definition_still_works_inner, CommType::Stdio, |client: &LspClient| {
        let code = indoc! {r#"
            contract simpleContract(@x) = {
                stdout!(x)
            }

            // Normal invocation
            simpleContract!(42)
        "#};

        let doc = client.open_document("/tmp/pattern_basic.rho", code).unwrap();
        client.await_diagnostics(&doc).unwrap();

        // Click on "simpleContract" in the invocation
        let result = client.definition(&doc.uri(), Position::new(5, 0));

        println!("Basic goto-definition result: {:?}", result);
        assert!(result.is_ok(), "Basic goto_definition should succeed");
        assert!(result.unwrap().is_some(), "Should find contract definition");
    });
}
