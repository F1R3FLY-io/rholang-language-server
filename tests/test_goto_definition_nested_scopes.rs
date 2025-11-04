//! Integration tests for goto_definition in deeply nested scopes
//!
//! This test suite verifies that goto_definition works correctly for symbols
//! in deeply nested scope structures involving contract parameters, new bindings,
//! and multiple levels of for bindings.
//!
//! Test cases from robot_planning.rho:
//! 1. queryResult: new binding used in nested for scope (7 levels deep)
//! 2. fromRoom: contract parameter used deep in nested scopes (5 levels deep)
//!
//! Scope structure for these tests:
//! ```
//! contract robotAPI(@"all_connections", @fromRoom, ret) = {  // Level 1: ContractBind
//!   new initState in {                                       // Level 2: NewBind
//!     for (@state <- initState) {                            // Level 3: InputBind
//!       new queryCode, queryResult in {                      // Level 4: NewBind (queryResult defined)
//!         queryCode!("... " ++ fromRoom ++ ")") |            // fromRoom usage
//!         for (@code <- queryCode) {                         // Level 5: InputBind
//!           for (@compiledQuery <- mettaCompile!?(code)) {   // Level 6: InputBind
//!             for (@result <- queryResult) {                 // Level 7: InputBind (queryResult usage)
//! ```

use std::fs;
use test_utils::with_lsp_client;
use test_utils::lsp::client::{CommType, LspClient};
use tower_lsp::lsp_types::Position;

/// Test 1: goto_definition for queryResult (new binding in nested scope)
///
/// **Context:**
/// - Definition: line 207, col 24 (1-indexed) - `new queryCode, queryResult in {`
/// - Usage: line 212, col 36 (1-indexed) - `for (@result <- queryResult) {`
///
/// **Scope chain from usage to definition:**
/// 1. for (@result <- ...) scope
/// 2. for (@compiledQuery <- ...) scope
/// 3. for (@code <- ...) scope
/// 4. new queryCode, queryResult scope ← **Definition here**
/// 5. for (@state <- ...) scope
/// 6. new initState scope
/// 7. contract robotAPI scope
///
/// **Test objective:** Verify symbol resolution through 4 nested for scopes to find
/// a new binding 3 scopes up.
with_lsp_client!(test_goto_definition_queryresult_nested_new, CommType::Stdio, |client: &LspClient| {
    println!("\n=== Test 1: goto_definition for queryResult (new binding) ===");

    let file_path = "tests/resources/robot_planning.rho";
    let source = fs::read_to_string(file_path)
        .expect("Failed to read robot_planning.rho");

    println!("Opening document with {} bytes", source.len());

    let doc = client.open_document("/test/robot_planning.rho", &source)
        .expect("Failed to open document");

    println!("✓ Document opened successfully");

    let _diagnostics = client.await_diagnostics(&doc)
        .expect("Failed to receive diagnostics");

    println!("✓ Diagnostics received");

    // Usage: line 212 (1-indexed) = line 211 (0-indexed)
    // The column provided was 36 (1-indexed), but let's verify by examining the line:
    // "for (@result <- queryResult) {"
    // Counting characters: "for (@result <- " = 16 chars, so "queryResult" starts at char 16
    // In 1-indexed that's column 17, but the user said column 36...
    // Let me use column 30 (0-indexed) which is where queryResult starts based on actual file inspection
    let usage_position = Position {
        line: 211,      // Line 212 in 1-indexed
        character: 30,  // Start of "queryResult" (verified from file)
    };

    // Expected definition: line 207 (1-indexed) = line 206 (0-indexed)
    // "new queryCode, queryResult in {"
    // "queryResult" starts at character 23 (0-indexed)
    let expected_line = 206u32;
    let expected_char = 23u32;

    println!("\n=== Test Details ===");
    println!("Usage position: line {} (0-indexed), character {} (0-indexed)",
             usage_position.line, usage_position.character);
    println!("                (line 212, column 31 in 1-indexed)");
    println!("                Context: queryResult in 'for (@result <- queryResult)'");
    println!();
    println!("Expected definition: line {} (0-indexed), character {} (0-indexed)",
             expected_line, expected_char);
    println!("                     (line 207, column 24 in 1-indexed)");
    println!("                     Context: new queryCode, queryResult in {{");
    println!();

    println!("Requesting goto_definition...");
    match client.definition(&doc.uri(), usage_position) {
        Ok(Some(location)) => {
            let def_line = location.range.start.line;
            let def_char = location.range.start.character;

            println!("✓ Found definition at line {}, character {} (1-indexed: {}, {})",
                   def_line, def_char, def_line + 1, def_char + 1);

            if def_line == expected_line && def_char == expected_char {
                println!("\n=== TEST PASSED ===");
                println!("Found correct definition for queryResult");
            } else {
                println!("\n=== TEST FAILED ===");
                println!("Found incorrect definition location:");
                println!("  Expected: line {}, character {}", expected_line, expected_char);
                println!("  Got:      line {}, character {}", def_line, def_char);

                client.close_document(&doc).expect("Failed to close document");
                panic!("goto_definition returned wrong location for queryResult");
            }
        }
        Ok(None) => {
            println!("\n=== TEST FAILED ===");
            println!("No definition found for queryResult");

            client.close_document(&doc).expect("Failed to close document");
            panic!("goto_definition returned None for queryResult (expected line {}, char {})",
                   expected_line, expected_char);
        }
        Err(e) => {
            println!("\n=== TEST FAILED ===");
            println!("Error during goto_definition request: {}", e);

            client.close_document(&doc).expect("Failed to close document");
            panic!("goto_definition request failed: {}", e);
        }
    }

    client.close_document(&doc).expect("Failed to close document");
    println!("✓ Document closed");
});

/// Test 2: goto_definition for fromRoom (contract parameter used deep in nested scopes)
///
/// **Context:**
/// - Definition: line 203, col 42 (1-indexed) - `@fromRoom` in contract parameters
/// - Usage: line 208, col 44 (1-indexed) - `fromRoom` in string concatenation
///
/// **Scope chain from usage to definition:**
/// 1. new queryCode, queryResult scope (where usage occurs)
/// 2. for (@state <- ...) scope
/// 3. new initState scope
/// 4. contract robotAPI scope ← **Definition here**
///
/// **Test objective:** Verify symbol resolution from deep nested scope (level 4) up to
/// contract parameter scope (level 1), crossing multiple scope boundaries.
with_lsp_client!(test_goto_definition_fromroom_contract_param, CommType::Stdio, |client: &LspClient| {
    println!("\n=== Test 2: goto_definition for fromRoom (contract parameter) ===");

    let file_path = "tests/resources/robot_planning.rho";
    let source = fs::read_to_string(file_path)
        .expect("Failed to read robot_planning.rho");

    println!("Opening document with {} bytes", source.len());

    let doc = client.open_document("/test/robot_planning.rho", &source)
        .expect("Failed to open document");

    println!("✓ Document opened successfully");

    let _diagnostics = client.await_diagnostics(&doc)
        .expect("Failed to receive diagnostics");

    println!("✓ Diagnostics received");

    // Usage: line 208 (1-indexed) = line 207 (0-indexed)
    // queryCode!("!(get_neighbors " ++ fromRoom ++ ")") |
    // "fromRoom" starts at character 43 (0-indexed) based on actual file inspection
    let usage_position = Position {
        line: 207,      // Line 208 in 1-indexed
        character: 43,  // Start of "fromRoom"
    };

    // Expected definition: line 203 (1-indexed) = line 202 (0-indexed)
    // contract robotAPI(@"all_connections", @fromRoom, ret) = {
    // The @ symbol is at character 40, the 'f' in fromRoom is at character 41
    // Definition should point to the @ symbol (start of the Quote node)
    let expected_line = 202u32;
    let expected_char = 40u32;  // The @ symbol before fromRoom

    println!("\n=== Test Details ===");
    println!("Usage position: line {} (0-indexed), character {} (0-indexed)",
             usage_position.line, usage_position.character);
    println!("                (line 208, column 44 in 1-indexed)");
    println!("                Context: fromRoom in '\"!(get_neighbors \" ++ fromRoom ++ \")\"'");
    println!();
    println!("Expected definition: line {} (0-indexed), character {} (0-indexed)",
             expected_line, expected_char);
    println!("                     (line 203, column 41 in 1-indexed - @ symbol)");
    println!("                     Context: contract parameter @fromRoom");
    println!();

    println!("Requesting goto_definition...");
    match client.definition(&doc.uri(), usage_position) {
        Ok(Some(location)) => {
            let def_line = location.range.start.line;
            let def_char = location.range.start.character;

            println!("✓ Found definition at line {}, character {} (1-indexed: {}, {})",
                   def_line, def_char, def_line + 1, def_char + 1);

            if def_line == expected_line && def_char == expected_char {
                println!("\n=== TEST PASSED ===");
                println!("Found correct definition for fromRoom");
            } else {
                println!("\n=== TEST FAILED ===");
                println!("Found incorrect definition location:");
                println!("  Expected: line {}, character {}", expected_line, expected_char);
                println!("  Got:      line {}, character {}", def_line, def_char);

                client.close_document(&doc).expect("Failed to close document");
                panic!("goto_definition returned wrong location for fromRoom");
            }
        }
        Ok(None) => {
            println!("\n=== TEST FAILED ===");
            println!("No definition found for fromRoom");

            client.close_document(&doc).expect("Failed to close document");
            panic!("goto_definition returned None for fromRoom (expected line {}, char {})",
                   expected_line, expected_char);
        }
        Err(e) => {
            println!("\n=== TEST FAILED ===");
            println!("Error during goto_definition request: {}", e);

            client.close_document(&doc).expect("Failed to close document");
            panic!("goto_definition request failed: {}", e);
        }
    }

    client.close_document(&doc).expect("Failed to close document");
    println!("✓ Document closed");
});
