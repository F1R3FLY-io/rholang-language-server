//! Diagnostic module to debug query_multi issues
//!
//! This tests query_multi with patterns matching MeTTaTron's exact format

use mork::space::Space;
use mork_expr::{Expr, ExprZipper};
use mork_frontend::bytestring_parser::{Parser, Context};
use pathmap::zipper::*;

/// Test query_multi with MeTTaTron's exact format: (= <lhs> <rhs>)
pub fn test_metta_format() -> Result<(), String> {
    eprintln!("\n=== Testing MeTTaTron Format ===");

    let mut space = Space::new();

    // Store a rule in MeTTaTron's format: (= (double 5) 10)
    let rule = "(= (double 5) 10)";
    eprintln!("Storing rule: {}", rule);
    space.load_all_sexpr_impl(rule.as_bytes(), true)
        .map_err(|e| format!("Failed to store rule: {}", e))?;

    // Verify it was stored
    let mut count = 0;
    let mut rz = space.btm.read_zipper();
    while rz.to_next_val() {
        count += 1;
        eprintln!("  Stored entry {}: path_len={}", count, rz.path().len());
    }
    eprintln!("Total entries stored: {}", count);

    // Query with MeTTaTron's pattern: (= (double 5) $rhs)
    let query_pattern = "(= (double 5) $rhs)";
    eprintln!("\nQuery pattern: {}", query_pattern);

    // Parse query pattern to MORK Expr
    let mut parse_buffer = vec![0u8; 4096];
    let mut pdp = mork::space::ParDataParser::new(&space.sm);
    let mut ez = ExprZipper::new(Expr {
        ptr: parse_buffer.as_mut_ptr(),
    });
    let mut context = Context::new(query_pattern.as_bytes());

    pdp.sexpr(&mut context, &mut ez)
        .map_err(|e| format!("Parse error: {:?}", e))?;

    let pattern_expr = Expr {
        ptr: parse_buffer.as_ptr().cast_mut(),
    };

    eprintln!("Pattern parsed successfully");
    eprintln!("  Pattern length: {}", ez.loc);
    eprintln!("  Pattern newvars: {}", pattern_expr.newvars());

    // Check args
    let mut pat_args = vec![];
    use mork_expr::ExprEnv;
    ExprEnv::new(0, pattern_expr).args(&mut pat_args);
    eprintln!("  Pattern args: {}", pat_args.len());

    // Check PathMap structure before query
    eprintln!("\nChecking PathMap structure...");
    let rz = space.btm.read_zipper();
    eprintln!("  Root child_mask: {:?}", rz.child_mask());

    // Debug: Check if trie has entries at all
    let mut test_rz = space.btm.read_zipper();
    let mut entry_count = 0;
    while test_rz.to_next_val() {
        entry_count += 1;
        let path = test_rz.path();
        eprintln!("  Entry {}: len={}, first 10 bytes: {:?}",
                  entry_count, path.len(), &path[..path.len().min(10)]);
    }
    eprintln!("  Total entries via iteration: {}", entry_count);

    // Try query_multi
    eprintln!("\nCalling query_multi...");
    let mut matches = 0;
    let result = Space::query_multi(&space.btm, pattern_expr, |result, matched_expr| {
        eprintln!("  Callback invoked!");
        if let Err(bindings) = result {
            eprintln!("    Match found! Bindings: {}", bindings.len());
            matches += 1;
        } else {
            eprintln!("    Ok result (no bindings)");
        }
        true
    });

    eprintln!("query_multi returned: {}", result);
    eprintln!("Matches found: {}", matches);

    if result > 0 {
        Ok(())
    } else {
        Err("query_multi returned 0 - no matches found".to_string())
    }
}

/// Test with our current format: (pattern-key <pattern> <value>)
pub fn test_our_format() -> Result<(), String> {
    eprintln!("\n=== Testing Our Format ===");

    let mut space = Space::new();

    // Store in our format: (pattern-key 42 "handler")
    let entry = "(pattern-key 42 \"handler\")";
    eprintln!("Storing entry: {}", entry);
    space.load_all_sexpr_impl(entry.as_bytes(), true)
        .map_err(|e| format!("Failed to store: {}", e))?;

    // Verify stored
    let mut count = 0;
    let mut rz = space.btm.read_zipper();
    while rz.to_next_val() {
        count += 1;
        eprintln!("  Stored entry {}: path_len={}", count, rz.path().len());
    }
    eprintln!("Total entries stored: {}", count);

    // Query: (pattern-key 42 $value)
    let query_pattern = "(pattern-key 42 $value)";
    eprintln!("\nQuery pattern: {}", query_pattern);

    // Parse to MORK Expr
    let mut parse_buffer = vec![0u8; 4096];
    let mut pdp = mork::space::ParDataParser::new(&space.sm);
    let mut ez = ExprZipper::new(Expr {
        ptr: parse_buffer.as_mut_ptr(),
    });
    let mut context = Context::new(query_pattern.as_bytes());

    pdp.sexpr(&mut context, &mut ez)
        .map_err(|e| format!("Parse error: {:?}", e))?;

    let pattern_expr = Expr {
        ptr: parse_buffer.as_ptr().cast_mut(),
    };

    eprintln!("Pattern parsed successfully");
    eprintln!("  Pattern length: {}", ez.loc);
    eprintln!("  Pattern newvars: {}", pattern_expr.newvars());

    // Check args
    let mut pat_args = vec![];
    use mork_expr::ExprEnv;
    ExprEnv::new(0, pattern_expr).args(&mut pat_args);
    eprintln!("  Pattern args: {}", pat_args.len());

    // Try query_multi
    eprintln!("\nCalling query_multi...");
    let mut matches = 0;
    let result = Space::query_multi(&space.btm, pattern_expr, |result, matched_expr| {
        eprintln!("  Callback invoked!");
        if let Err(bindings) = result {
            eprintln!("    Match found! Bindings: {}", bindings.len());
            matches += 1;
        } else {
            eprintln!("    Ok result (no bindings)");
        }
        true
    });

    eprintln!("query_multi returned: {}", result);
    eprintln!("Matches found: {}", matches);

    if result > 0 {
        Ok(())
    } else {
        Err("query_multi returned 0 - no matches found".to_string())
    }
}

/// Test manual unification without query_multi
pub fn test_manual_unification() -> Result<(), String> {
    eprintln!("\n=== Testing Manual Unification ===");

    let mut space = Space::new();

    // Store a rule: (= (double 5) 10)
    let rule = "(= (double 5) 10)";
    eprintln!("Storing rule: {}", rule);
    space.load_all_sexpr_impl(rule.as_bytes(), true)
        .map_err(|e| format!("Failed to store rule: {}", e))?;

    // Parse the SAME pattern to verify we can get the stored expression
    let stored_pattern = "(= (double 5) 10)";
    let mut parse_buffer1 = vec![0u8; 4096];
    let mut pdp1 = mork::space::ParDataParser::new(&space.sm);
    let mut ez1 = ExprZipper::new(Expr { ptr: parse_buffer1.as_mut_ptr() });
    let mut context1 = Context::new(stored_pattern.as_bytes());
    pdp1.sexpr(&mut context1, &mut ez1)
        .map_err(|e| format!("Parse stored error: {:?}", e))?;
    let stored_expr_from_parse = Expr { ptr: parse_buffer1.as_ptr().cast_mut() };

    // Parse the query pattern: (= (double 5) $rhs)
    let query_pattern = "(= (double 5) $rhs)";
    let mut parse_buffer2 = vec![0u8; 4096];
    let mut pdp2 = mork::space::ParDataParser::new(&space.sm);
    let mut ez2 = ExprZipper::new(Expr { ptr: parse_buffer2.as_mut_ptr() });
    let mut context2 = Context::new(query_pattern.as_bytes());
    pdp2.sexpr(&mut context2, &mut ez2)
        .map_err(|e| format!("Parse query error: {:?}", e))?;
    let query_expr = Expr { ptr: parse_buffer2.as_ptr().cast_mut() };

    // Get the stored expression from trie via iteration
    let mut rz = space.btm.read_zipper();
    if !rz.to_next_val() {
        return Err("No entries in trie".to_string());
    }
    let stored_expr_from_trie = Expr { ptr: rz.path().as_ptr().cast_mut() };

    eprintln!("\nTesting unification:");
    eprintln!("  Query pattern: {}", query_pattern);
    eprintln!("  Stored pattern: {}", stored_pattern);

    // Try to unify the query pattern with the stored expression from trie
    use mork_expr::{ExprEnv, unify};
    let pairs = vec![
        (ExprEnv::new(0, query_expr), ExprEnv::new(1, stored_expr_from_trie))
    ];

    match unify(pairs) {
        Ok(bindings) => {
            eprintln!("  ✅ Unification succeeded!");
            eprintln!("  Bindings: {}", bindings.len());
            for ((space_id, var_id), bound_expr) in &bindings {
                eprintln!("    ({}, {}) -> bound value exists", space_id, var_id);
            }
            Ok(())
        }
        Err(e) => {
            eprintln!("  ❌ Unification failed: {:?}", e);
            Err(format!("Unification failed: {:?}", e))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manual_unification_works() {
        // Verify that MORK unification works as expected
        match test_manual_unification() {
            Ok(()) => println!("✅ Manual unification works!"),
            Err(e) => println!("❌ Manual unification failed: {}", e),
        }
    }

    #[test]
    fn test_metta_format_query_multi() {
        // Test if MeTTaTron's format works with query_multi
        match test_metta_format() {
            Ok(()) => println!("✅ MeTTaTron format works!"),
            Err(e) => println!("❌ MeTTaTron format failed: {}", e),
        }
    }

    #[test]
    fn test_our_format_query_multi() {
        // Test if our format works with query_multi
        match test_our_format() {
            Ok(()) => println!("✅ Our format works!"),
            Err(e) => println!("❌ Our format failed: {}", e),
        }
    }
}
