//! Cached Tree-Sitter kind IDs for O(1) node type checking
//!
//! This module provides fast node type checking using integer comparison
//! instead of string comparison. Kind IDs are cached using `OnceLock` for
//! thread-safe lazy initialization.
//!
//! # Performance
//!
//! - String comparison (`.kind() == "string"`): O(n) where n = string length
//! - Integer comparison (helper functions): O(1)
//! - FFI overhead: Calling `.kind_id()` requires FFI, so cache the result
//!
//! # Usage Pattern
//!
//! **GOOD - Cache kind_id once:**
//! ```rust,ignore
//! let kind_id = node.kind_id();  // FFI call - cache it!
//! if is_var(kind_id) { /* ... */ }
//! if is_quote(kind_id) { /* ... */ }  // Reuse cached value
//! ```
//!
//! **BAD - Multiple FFI calls:**
//! ```rust,ignore
//! if node.kind() == "var" { /* ... */ }  // String alloc + comparison
//! if is_quote(node.kind_id()) { /* ... */ }  // Second FFI call
//! ```

use std::sync::OnceLock;
use tree_sitter::Language;

/// Get the Tree-Sitter language singleton
#[inline(always)]
fn language() -> Language {
    rholang_tree_sitter::LANGUAGE.into()
}

// ============================================================================
// Collection nodes
// ============================================================================

static PROC_REMAINDER_KIND: OnceLock<u16> = OnceLock::new();
static KEY_VALUE_PAIR_KIND: OnceLock<u16> = OnceLock::new();

/// Check if node is `_proc_remainder` (used in list/set/map/pathmap)
#[inline(always)]
pub(crate) fn is_proc_remainder(kind_id: u16) -> bool {
    let id = *PROC_REMAINDER_KIND.get_or_init(|| {
        language().id_for_node_kind("_proc_remainder", true)
    });
    kind_id == id
}

/// Check if node is `key_value_pair` (used in map literals)
#[inline(always)]
pub(crate) fn is_key_value_pair(kind_id: u16) -> bool {
    let id = *KEY_VALUE_PAIR_KIND.get_or_init(|| {
        language().id_for_node_kind("key_value_pair", true)
    });
    kind_id == id
}

// ============================================================================
// Pattern matching nodes
// ============================================================================

static CASE_KIND: OnceLock<u16> = OnceLock::new();
static BRANCH_KIND: OnceLock<u16> = OnceLock::new();

/// Check if node is `case` (used in match expressions)
#[inline(always)]
pub(crate) fn is_case(kind_id: u16) -> bool {
    let id = *CASE_KIND.get_or_init(|| language().id_for_node_kind("case", true));
    kind_id == id
}

/// Check if node is `branch` (used in choice expressions)
#[inline(always)]
pub(crate) fn is_branch(kind_id: u16) -> bool {
    let id = *BRANCH_KIND.get_or_init(|| language().id_for_node_kind("branch", true));
    kind_id == id
}

// ============================================================================
// Process nodes
// ============================================================================

static INPUT_OR_SOURCE_KIND: OnceLock<u16> = OnceLock::new();
static RECEIPTS_KIND: OnceLock<u16> = OnceLock::new();
static INPUTS_KIND: OnceLock<u16> = OnceLock::new();
static NAMES_KIND: OnceLock<u16> = OnceLock::new();
static LINEAR_BIND_KIND: OnceLock<u16> = OnceLock::new();

/// Check if node is `_input_or_source` (used in for/receive)
#[inline(always)]
pub(crate) fn is_input_or_source(kind_id: u16) -> bool {
    let id = *INPUT_OR_SOURCE_KIND.get_or_init(|| {
        language().id_for_node_kind("_input_or_source", true)
    });
    kind_id == id
}

/// Check if node is `receipts` (used in receive expressions)
#[inline(always)]
pub(crate) fn is_receipts(kind_id: u16) -> bool {
    let id = *RECEIPTS_KIND.get_or_init(|| {
        language().id_for_node_kind("receipts", true)
    });
    kind_id == id
}

/// Check if node is `inputs` (used in send expressions)
#[inline(always)]
pub(crate) fn is_inputs(kind_id: u16) -> bool {
    let id = *INPUTS_KIND.get_or_init(|| {
        language().id_for_node_kind("inputs", true)
    });
    kind_id == id
}

/// Check if node is `names` (used in new/contract declarations)
#[inline(always)]
pub(crate) fn is_names(kind_id: u16) -> bool {
    let id = *NAMES_KIND.get_or_init(|| {
        language().id_for_node_kind("names", true)
    });
    kind_id == id
}

/// Check if node is `linear_bind` (used in branch expressions)
#[inline(always)]
pub(crate) fn is_linear_bind(kind_id: u16) -> bool {
    let id = *LINEAR_BIND_KIND.get_or_init(|| {
        language().id_for_node_kind("linear_bind", true)
    });
    kind_id == id
}

// ============================================================================
// Expression nodes
// ============================================================================

static VAR_KIND: OnceLock<u16> = OnceLock::new();
static QUOTE_KIND: OnceLock<u16> = OnceLock::new();
static STRING_LITERAL_KIND: OnceLock<u16> = OnceLock::new();
static CONCAT_KIND: OnceLock<u16> = OnceLock::new();
static ARROW_KIND: OnceLock<u16> = OnceLock::new();

/// Check if node is `var` (variable reference)
#[inline(always)]
pub(crate) fn is_var(kind_id: u16) -> bool {
    let id = *VAR_KIND.get_or_init(|| {
        language().id_for_node_kind("var", true)
    });
    kind_id == id
}

/// Check if node is `quote` (name/channel)
#[inline(always)]
pub(crate) fn is_quote(kind_id: u16) -> bool {
    let id = *QUOTE_KIND.get_or_init(|| {
        language().id_for_node_kind("quote", true)
    });
    kind_id == id
}

/// Check if node is `string_literal`
#[inline(always)]
pub(crate) fn is_string_literal(kind_id: u16) -> bool {
    let id = *STRING_LITERAL_KIND.get_or_init(|| {
        language().id_for_node_kind("string_literal", true)
    });
    kind_id == id
}

/// Check if node is `concat` (string concatenation)
#[inline(always)]
pub(crate) fn is_concat(kind_id: u16) -> bool {
    let id = *CONCAT_KIND.get_or_init(|| {
        language().id_for_node_kind("concat", true)
    });
    kind_id == id
}

/// Check if node is `=>` token (arrow in branches/cases)
#[inline(always)]
pub(crate) fn is_arrow(kind_id: u16) -> bool {
    let id = *ARROW_KIND.get_or_init(|| {
        language().id_for_node_kind("=>", false)  // Not a named node
    });
    kind_id == id
}

// ============================================================================
// Declaration nodes
// ============================================================================

static NAME_DECLS_KIND: OnceLock<u16> = OnceLock::new();
static NAME_DECL_KIND: OnceLock<u16> = OnceLock::new();

/// Check if node is `name_decls` (used in new declarations)
#[inline(always)]
pub(crate) fn is_name_decls(kind_id: u16) -> bool {
    let id = *NAME_DECLS_KIND.get_or_init(|| {
        language().id_for_node_kind("name_decls", true)
    });
    kind_id == id
}

/// Check if node is `name_decl` (single name declaration)
#[inline(always)]
pub(crate) fn is_name_decl(kind_id: u16) -> bool {
    let id = *NAME_DECL_KIND.get_or_init(|| {
        language().id_for_node_kind("name_decl", true)
    });
    kind_id == id
}

// ============================================================================
// Test-only nodes
// ============================================================================

#[cfg(test)]
static PAR_KIND: OnceLock<u16> = OnceLock::new();
#[cfg(test)]
static CONTRACT_KIND: OnceLock<u16> = OnceLock::new();

/// Check if node is `par` (parallel composition) - test only
#[cfg(test)]
#[inline(always)]
pub(crate) fn is_par(kind_id: u16) -> bool {
    let id = *PAR_KIND.get_or_init(|| {
        language().id_for_node_kind("par", true)
    });
    kind_id == id
}

/// Check if node is `contract` - test only
#[cfg(test)]
#[inline(always)]
pub(crate) fn is_contract(kind_id: u16) -> bool {
    let id = *CONTRACT_KIND.get_or_init(|| {
        language().id_for_node_kind("contract", true)
    });
    kind_id == id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kind_ids_are_cached() {
        // Get kind_id for testing
        let lang = language();
        let remainder_id = lang.id_for_node_kind("_proc_remainder", true);

        // First call initializes cache
        assert!(is_proc_remainder(remainder_id));

        // Second call uses cached value (verify by calling multiple times)
        assert!(is_proc_remainder(remainder_id));
        assert!(is_proc_remainder(remainder_id));
    }

    #[test]
    fn test_wrong_kind_returns_false() {
        let lang = language();
        let var_id = lang.id_for_node_kind("var", true);

        // var_id should not match proc_remainder
        assert!(!is_proc_remainder(var_id));
    }
}
