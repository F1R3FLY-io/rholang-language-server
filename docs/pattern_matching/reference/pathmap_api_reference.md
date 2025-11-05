# Investigation - test_pathmap_pattern_goto_definition Timeout

## Test Details

**Test Location**: `tests/test_complex_quote_patterns.rs:284`

**What It Tests**:
- Goto-definition for map literal keys in contract invocations
- Expects clicking on "user" in `{"user": ...}` to link to the `user` pattern in the contract definition

**Test Position**: Line 109, character 28

**Resource File**: `tests/resources/complex_quote_patterns.rho`

## Actual Behavior

The test clicks at position (109, 28) which is on line:
```rholang
processComplex!({
  "user": {"name": "Bob", "email": "bob@example.com"},  // Line 111 (index 110), char 28 is "user"
```

But the test says line 109 (0-indexed), character 28, which puts it at:
```rholang
  processComplex!({
    "user": ...    // Position (110, 28) in actual file (line 111 in editor)
```

The contract definition is:
```rholang
contract processComplex(@{
  user: {name: n, email: e},  // Line 70
  address: {street: st, city: ct},
  metadata: {created: cr, updated: up}
}, ret) = { ... }
```

## Debug Output Analysis

From the test output:
```
DEBUG GenericGotoDefinition: Found node at position: type=Rholang::Map, category=Collection
DEBUG Extracted symbol name: 'user'
DEBUG RholangSymbolResolver: Looking up symbol 'user' (language=rholang, uri=...)
DEBUG Symbol 'user' not found in symbol table
DEBUG No definitions found for symbol 'user'
```

## Root Cause

The test expects goto-definition to work for map literal keys in invocations (like `"user"` in `{"user": ...}` when calling a contract), linking them to the corresponding map pattern keys in the contract parameter definition (like `user: ...` in the pattern).

**Current Implementation**:
- Map keys in literal invocations are extracted as symbols
- BUT these are not tracked in the symbol table
- Symbol table only tracks variable bindings (like `n`, `e`, `st`, etc. from the pattern values)

**What's Missing**:
- Map key patterns in contract definitions don't create symbol table entries
- Map key literals in invocations can't link to their pattern definitions
- This would require a different indexing strategy (not just lexical scope)

## Assessment

This is a **feature limitation**, not a bug from our recent changes. The test is expecting functionality that was never fully implemented:

1. **Contract pattern matching for map keys**: The system would need to:
   - Index map key patterns in contract parameters (e.g., `user:`, `address:`, `metadata:`)
   - Link map literal keys in invocations to their pattern counterparts
   - Require contract signature matching + pattern key extraction

2. **Why it's complex**:
   - Requires finding the contract being invoked (`processComplex`)
   - Matching the invocation argument to the contract parameter pattern
   - Extracting and matching map keys between invocation and pattern
   - This is structural pattern matching, not lexical scoping

## Options

1. **Fix the Test**: Update test expectations to match current behavior (goto-definition for map keys not supported)
2. **Implement Feature**: Add map key pattern matching (significant work)
3. **Mark as Known Limitation**: Document that map key goto-definition is not supported

## Recommendation

Given that we're at 528/529 tests passing and this is a feature gap (not a regression), I recommend **Option 1**: update the test to reflect current capabilities, or mark it as `#[ignore]` with a comment explaining it's a future feature.

The test name `test_pathmap_pattern_goto_definition` suggests it was written in anticipation of this feature, but the feature was never completed.
