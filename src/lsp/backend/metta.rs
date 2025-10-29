//! MeTTa language support operations
//!
//! This module provides LSP features specifically for MeTTa code embedded in Rholang files,
//! including hover, document highlights, go-to-definition, and rename operations.
//!
//! # Symbol Resolution Architecture
//!
//! Symbol resolution for MeTTa uses a composable, trait-based architecture defined in
//! `crate::ir::symbol_resolution`. The system supports:
//!
//! - **Default lexical scoping** via `LexicalScopeResolver`
//! - **Language-specific filtering** via `SymbolFilter` trait (e.g., `MettaPatternFilter`)
//! - **Fallback strategies** for cross-document resolution
//! - **Complete override** via `CustomScopeResolver` for non-standard scoping
//!
//! Example composable resolver setup:
//! ```ignore
//! use crate::ir::symbol_resolution::{
//!     ComposableSymbolResolver, LexicalScopeResolver, MettaPatternFilter,
//!     AsyncGlobalVirtualSymbolResolver, ResolutionContext,
//! };
//!
//! // Create a composable resolver with:
//! // 1. Lexical scope as base (traverses scope chain)
//! // 2. Pattern matching filter (refines by arity)
//! // 3. Global cross-document lookup as fallback
//! let resolver = ComposableSymbolResolver::new(
//!     Box::new(LexicalScopeResolver::new(symbol_table, "metta".to_string())),
//!     vec![Box::new(MettaPatternFilter::new(pattern_matcher))],
//!     Some(Box::new(GlobalVirtualSymbolResolver::new(workspace))),
//! );
//!
//! // Resolve a symbol
//! let context = ResolutionContext {
//!     uri: virtual_doc.uri.clone(),
//!     scope_id: Some(symbol.scope_id),
//!     ir_node: Some(call_node),  // For pattern matching
//!     language: "metta".to_string(),
//!     parent_uri: Some(virtual_doc.parent_uri.clone()),
//! };
//!
//! let locations = resolver.resolve_symbol(&symbol.name, &position, &context);
//! ```
//!
//! The current `goto_definition_metta` implementation uses specialized logic for MeTTa's
//! pattern matching. Future refactoring could integrate the composable resolver more directly.

use std::sync::Arc;
use tower_lsp::lsp_types::{
    DocumentHighlight, DocumentHighlightKind, GotoDefinitionResponse, Hover, HoverContents,
    Location, MarkupContent, MarkupKind, Position as LspPosition, Range, TextEdit,
    WorkspaceEdit,
};
use tracing::{debug, error};

use crate::ir::metta_node::MettaNode;
use crate::ir::semantic_node::{Position as IrPosition, SemanticNode};
use crate::ir::symbol_resolution::global::AsyncGlobalVirtualSymbolResolver;
use crate::language_regions::VirtualDocument;
use crate::lsp::models::CachedDocument;

use super::state::RholangBackend;
use super::utils::SemanticTokensBuilder;

type LspResult<T> = Result<T, tower_lsp::jsonrpc::Error>;

impl RholangBackend {
    /// Provides hover information for MeTTa files
    pub(super) async fn hover_metta(
        &self,
        doc: &Arc<CachedDocument>,
        position: LspPosition,
    ) -> LspResult<Option<Hover>> {
        debug!("MeTTa hover at position {:?}", position);

        // Get MeTTa IR
        let metta_ir = match &doc.metta_ir {
            Some(ir) => ir,
            None => {
                debug!("No MeTTa IR available");
                return Ok(None);
            }
        };

        // Find the node at the cursor position
        // For now, we do a simple linear search
        // TODO: Build position index for O(log n) lookup
        for (index, node) in metta_ir.iter().enumerate() {
            let base = node.base();
            let rel_start = base.relative_start();
            let node_line = rel_start.delta_lines.max(0) as u32;
            let node_col = rel_start.delta_columns.max(0) as u32;
            let node_end_col = node_col + base.length() as u32;

            // Check if position is within this node
            if position.line == node_line
                && position.character >= node_col
                && position.character <= node_end_col
            {
                return self.create_metta_hover_content(node, index, position);
            }
        }

        debug!("No MeTTa node found at position {:?}", position);
        Ok(None)
    }

    /// Creates hover content for a MeTTa node
    fn create_metta_hover_content(
        &self,
        node: &Arc<MettaNode>,
        _index: usize,
        position: LspPosition,
    ) -> LspResult<Option<Hover>> {
        let hover_text = match &**node {
            MettaNode::Definition { pattern, .. } => {
                let name = self.extract_metta_name(pattern).unwrap_or("definition".to_string());
                format!("```metta\n(= {} ...)\n```\n\n**MeTTa Definition**", name)
            }
            MettaNode::TypeAnnotation { expr, type_expr, .. } => {
                let expr_name = self.extract_metta_name(expr).unwrap_or("expr".to_string());
                let type_name = self.extract_metta_name(type_expr).unwrap_or("type".to_string());
                format!("```metta\n(: {} {})\n```\n\n**Type Annotation**", expr_name, type_name)
            }
            MettaNode::Atom { name, .. } => {
                format!("```metta\n{}\n```\n\n**Atom**", name)
            }
            MettaNode::Variable { name, var_type, .. } => {
                format!("```metta\n{}{}\n```\n\n**Variable** ({})",
                    var_type, name,
                    match var_type {
                        crate::ir::metta_node::MettaVariableType::Regular => "regular",
                        crate::ir::metta_node::MettaVariableType::Grounded => "grounded",
                        crate::ir::metta_node::MettaVariableType::Quoted => "quoted",
                    }
                )
            }
            MettaNode::Lambda { params, .. } => {
                let param_names = params.iter()
                    .filter_map(|p| self.extract_metta_name(p))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("```metta\n(λ ({}) ...)\n```\n\n**Lambda Function**", param_names)
            }
            MettaNode::Integer { value, .. } => {
                format!("```metta\n{}\n```\n\n**Integer**", value)
            }
            MettaNode::Float { value, .. } => {
                format!("```metta\n{}\n```\n\n**Float**", value)
            }
            MettaNode::String { value, .. } => {
                format!("```metta\n\"{}\"\n```\n\n**String**", value)
            }
            MettaNode::Bool { value, .. } => {
                format!("```metta\n{}\n```\n\n**Boolean**", value)
            }
            MettaNode::Match { .. } => {
                "```metta\n(match ...)\n```\n\n**Pattern Match**".to_string()
            }
            MettaNode::Let { .. } => {
                "```metta\n(let ...)\n```\n\n**Let Binding**".to_string()
            }
            MettaNode::If { .. } => {
                "```metta\n(if ...)\n```\n\n**Conditional**".to_string()
            }
            MettaNode::Eval { .. } => {
                "```metta\n!(expr)\n```\n\n**Evaluation**".to_string()
            }
            MettaNode::SExpr { elements, .. } => {
                let len = elements.len();
                format!("```metta\n(...)\n```\n\n**S-Expression** ({} elements)", len)
            }
            MettaNode::Nil { .. } => {
                "```metta\nNil\n```\n\n**Nil**".to_string()
            }
            MettaNode::Error { message, .. } => {
                format!("**Error**: {}", message)
            }
            MettaNode::Comment { text, .. } => {
                format!("**Comment**\n\n{}", text)
            }
        };

        Ok(Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: hover_text,
            }),
            range: Some(Range {
                start: position,
                end: LspPosition {
                    line: position.line,
                    character: position.character + 1,
                },
            }),
        }))
    }

    /// Extract name from a MeTTa node (helper for hover)
    fn extract_metta_name(&self, node: &Arc<MettaNode>) -> Option<String> {
        match &**node {
            MettaNode::Atom { name, .. } => Some(name.clone()),
            MettaNode::Variable { name, .. } => Some(format!("${}", name)),
            _ => None,
        }
    }

    /// Add semantic tokens for a MeTTa code region
    pub(super) async fn add_metta_semantic_tokens(
        &self,
        builder: &mut SemanticTokensBuilder,
        virtual_doc: &Arc<VirtualDocument>,
    ) {
        // Use cached Tree-Sitter tree from VirtualDocument
        let tree = match virtual_doc.get_or_parse_tree() {
            Some(tree) => tree,
            None => {
                error!("Failed to get or parse MeTTa tree for virtual document");
                return;
            }
        };

        // Token type indices (must match the order in initialize())
        const TOKEN_COMMENT: u32 = 0;
        const TOKEN_STRING: u32 = 1;
        const TOKEN_NUMBER: u32 = 2;
        const TOKEN_KEYWORD: u32 = 3;
        const TOKEN_OPERATOR: u32 = 4;
        const TOKEN_VARIABLE: u32 = 5;
        const TOKEN_FUNCTION: u32 = 6;
        const TOKEN_TYPE: u32 = 7;

        // Walk the tree and generate tokens
        let mut cursor = tree.walk();
        self.visit_metta_node(&mut cursor, builder, virtual_doc, TOKEN_COMMENT, TOKEN_STRING, TOKEN_NUMBER, TOKEN_KEYWORD, TOKEN_OPERATOR, TOKEN_VARIABLE, TOKEN_FUNCTION, TOKEN_TYPE);
    }

    /// Recursively visit MeTTa Tree-Sitter nodes and generate semantic tokens
    fn visit_metta_node(
        &self,
        cursor: &mut tree_sitter::TreeCursor,
        builder: &mut SemanticTokensBuilder,
        virtual_doc: &Arc<VirtualDocument>,
        token_comment: u32,
        token_string: u32,
        token_number: u32,
        token_keyword: u32,
        token_operator: u32,
        token_variable: u32,
        token_function: u32,
        token_type: u32,
    ) {
        let node = cursor.node();
        let kind = node.kind();

        // Get text content for keyword detection
        let node_text = node.utf8_text(virtual_doc.content.as_bytes()).ok();

        // Map Tree-Sitter node kinds to semantic token types with context-aware highlighting
        let semantic_token_type = match kind {
            // Comments
            "line_comment" | "block_comment" => Some(token_comment),

            // Literals
            "string_literal" => Some(token_string),
            "integer_literal" | "float_literal" => Some(token_number),
            "boolean_literal" => Some(token_keyword),  // True/False as keywords

            // Variables and wildcards
            "variable" => Some(token_variable),
            "wildcard" => Some(token_keyword),  // _ pattern

            // Identifiers - distinguish between keywords, functions, and atoms
            "identifier" => {
                if let Some(text) = node_text {
                    // Check if it's a known MeTTa keyword/special form
                    match text {
                        // Special forms and keywords
                        "match" | "case" | "let" | "if" | "lambda" | "λ" |
                        "import" | "pragma" | "include" | "quote" | "unquote" |
                        "eval" | "chain" | "function" | "return" |
                        // Common built-in functions that should stand out
                        "superpose" | "collapse" | "empty" | "get-metatype" |
                        "get-type" | "cons-atom" | "decons-atom" |
                        // Type-related keywords
                        "Type" | "Atom" | "Symbol" | "Expression" | "Variable" |
                        "Number" | "String" | "Bool" => Some(token_keyword),

                        // Check if it's the first child of a list (function position)
                        _ => {
                            let parent = node.parent();
                            if let Some(parent_node) = parent {
                                // Navigate up through atom_expression to list
                                let check_node = if parent_node.kind() == "atom_expression" {
                                    parent_node.parent().unwrap_or(parent_node)
                                } else {
                                    parent_node
                                };

                                if check_node.kind() == "list" {
                                    // Find the first expression child
                                    let mut first_expr_child = None;
                                    for i in 0..check_node.child_count() {
                                        if let Some(child) = check_node.child(i) {
                                            if child.kind() == "expression" || child.kind() == "atom_expression" {
                                                first_expr_child = Some(child);
                                                break;
                                            }
                                        }
                                    }

                                    // Check if this node is inside the first expression
                                    if let Some(first_expr) = first_expr_child {
                                        if first_expr.start_byte() <= node.start_byte()
                                            && node.end_byte() <= first_expr.end_byte() {
                                            Some(token_function)  // First element in list = function call
                                        } else {
                                            Some(token_type)  // Other positions = regular atom
                                        }
                                    } else {
                                        Some(token_type)
                                    }
                                } else {
                                    Some(token_type)  // Default for atoms
                                }
                            } else {
                                Some(token_type)  // Default for atoms
                            }
                        }
                    }
                } else {
                    Some(token_type)  // Default if we can't get text
                }
            },

            // Operators
            "arrow_operator" | "comparison_operator" | "assignment_operator" |
            "type_annotation_operator" | "rule_definition_operator" | "arithmetic_operator" |
            "logic_operator" | "punctuation_operator" | "operator" => Some(token_operator),

            // Prefixes (!, ?, ')
            "exclaim_prefix" | "question_prefix" | "quote_prefix" => Some(token_keyword),

            _ => None,
        };

        // Add token if this is a leaf node with a token type
        if let Some(token_type_value) = semantic_token_type {
            if node.child_count() == 0 || matches!(kind, "line_comment" | "block_comment" | "string_literal") {
                let start_point = node.start_position();
                let end_point = node.end_position();

                // Calculate absolute line and column in the original document
                let line = virtual_doc.parent_start.line + start_point.row as u32;
                let column = if start_point.row == 0 {
                    virtual_doc.parent_start.character + start_point.column as u32
                } else {
                    start_point.column as u32
                };

                let length = if start_point.row == end_point.row {
                    (end_point.column - start_point.column) as u32
                } else {
                    // Multi-line token - use the rest of the line
                    (node.end_byte() - node.start_byte()) as u32
                };

                builder.push(line, column, length, token_type_value);
            }
        }

        // Recurse into children
        if cursor.goto_first_child() {
            loop {
                self.visit_metta_node(cursor, builder, virtual_doc, token_comment, token_string, token_number, token_keyword, token_operator, token_variable, token_function, token_type);
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
            cursor.goto_parent();
        }
    }

    /// Document highlights for MeTTa symbols
    pub(super) async fn document_highlight_metta(
        &self,
        virtual_doc: &Arc<VirtualDocument>,
        virtual_position: LspPosition,
        _parent_position: LspPosition,
    ) -> LspResult<Option<Vec<DocumentHighlight>>> {
        use crate::ir::transforms::metta_symbol_table_builder::*;

        // Get symbol table
        let symbol_table = match virtual_doc.get_or_build_symbol_table() {
            Some(table) => table,
            None => {
                debug!("Failed to build symbol table for MeTTa virtual document");
                return Ok(None);
            }
        };

        // Find symbol at position
        debug!("Looking up MeTTa symbol at virtual position L{}:C{}",
            virtual_position.line, virtual_position.character);

        let symbol = match symbol_table.find_symbol_at_position(&virtual_position) {
            Some(sym) => {
                debug!("Found MeTTa symbol '{}' at virtual L{}:C{}-{} (scope {})",
                    sym.name,
                    sym.range.start.line, sym.range.start.character,
                    sym.range.end.character,
                    sym.scope_id);
                sym
            }
            None => {
                debug!("No MeTTa symbol at position L{}:C{}",
                    virtual_position.line, virtual_position.character);

                // Debug: show nearby symbols
                let nearby: Vec<_> = symbol_table.all_occurrences.iter()
                    .filter(|occ| {
                        let line_diff = (occ.range.start.line as i32 - virtual_position.line as i32).abs();
                        line_diff <= 1  // Within 1 line
                    })
                    .take(10)
                    .collect();

                if !nearby.is_empty() {
                    debug!("Nearby symbols (within 1 line of {}:{}):", virtual_position.line, virtual_position.character);
                    for occ in &nearby {
                        debug!("  '{}' at line {} char {}-{} (is_def={})",
                            occ.name,
                            occ.range.start.line,
                            occ.range.start.character,
                            occ.range.end.character,
                            occ.is_definition);
                    }
                } else {
                    debug!("No nearby symbols found on lines {}-{}",
                        virtual_position.line.saturating_sub(1),
                        virtual_position.line + 1);

                    // Show a sample of all symbols to understand the coordinate system
                    let sample: Vec<_> = symbol_table.all_occurrences.iter()
                        .take(10)
                        .collect();
                    if !sample.is_empty() {
                        debug!("Sample of symbols in table (total {}):", symbol_table.all_occurrences.len());
                        for occ in sample {
                            debug!("  '{}' at L{}:C{}-{} (scope {})",
                                occ.name,
                                occ.range.start.line, occ.range.start.character,
                                occ.range.end.character,
                                occ.scope_id);
                        }
                    }

                    // Show symbols on lines around where we're looking
                    debug!("Symbols on lines {} to {}:",
                        virtual_position.line.saturating_sub(5),
                        virtual_position.line + 5);
                    let range_symbols: Vec<_> = symbol_table.all_occurrences.iter()
                        .filter(|occ| {
                            occ.range.start.line >= virtual_position.line.saturating_sub(5) &&
                            occ.range.start.line <= virtual_position.line + 5
                        })
                        .take(20)
                        .collect();
                    for occ in range_symbols {
                        debug!("  '{}' at L{}:C{}-{} (scope {})",
                            occ.name,
                            occ.range.start.line, occ.range.start.character,
                            occ.range.end.character,
                            occ.scope_id);
                    }
                }

                return Ok(None);
            }
        };

        // Check if this is a function name (appears in pattern matcher index)
        let function_defs = symbol_table.pattern_matcher.get_definitions_by_name(&symbol.name);
        let is_function = !function_defs.is_empty();

        let references: Vec<&SymbolOccurrence> = if is_function {
            // For functions, find all occurrences with the same name across all scopes
            debug!("Symbol '{}' is a function with {} definitions, finding all usages",
                symbol.name, function_defs.len());
            symbol_table.all_occurrences.iter()
                .filter(|occ| occ.name == symbol.name)
                .collect()
        } else {
            // For variables, find references only in the same scope
            debug!("Symbol '{}' is a variable in scope {}, finding scope references",
                symbol.name, symbol.scope_id);
            symbol_table.find_symbol_references(symbol)
        };

        // Map virtual ranges to parent ranges
        let highlights: Vec<DocumentHighlight> = references
            .iter()
            .map(|occ| {
                let parent_range = virtual_doc.map_range_to_parent(occ.range);
                debug!(
                    "  Mapping MeTTa highlight '{}': virtual L{}:C{}-{} -> parent L{}:C{}-{}",
                    occ.name,
                    occ.range.start.line, occ.range.start.character,
                    occ.range.end.character,
                    parent_range.start.line, parent_range.start.character,
                    parent_range.end.character
                );
                DocumentHighlight {
                    range: parent_range,
                    kind: if occ.is_definition {
                        Some(DocumentHighlightKind::WRITE)
                    } else {
                        Some(DocumentHighlightKind::READ)
                    },
                }
            })
            .collect();

        debug!("Found {} MeTTa symbol highlights for '{}' (scope {}, is_function={})",
            highlights.len(), symbol.name, symbol.scope_id, is_function);
        Ok(Some(highlights))
    }

    /// Go-to-definition for MeTTa symbols
    pub(super) async fn goto_definition_metta(
        &self,
        virtual_doc: &Arc<VirtualDocument>,
        virtual_position: LspPosition,
    ) -> LspResult<Option<GotoDefinitionResponse>> {
        use crate::ir::transforms::metta_symbol_table_builder::*;

        // Get symbol table
        let symbol_table = match virtual_doc.get_or_build_symbol_table() {
            Some(table) => table,
            None => {
                debug!("Failed to build symbol table for MeTTa virtual document");
                return Ok(None);
            }
        };

        // Find symbol at position
        let symbol = match symbol_table.find_symbol_at_position(&virtual_position) {
            Some(sym) => sym,
            None => {
                debug!("No MeTTa symbol at position {:?}", virtual_position);
                return Ok(None);
            }
        };

        // First, check if this symbol is in a function call position
        // If so, use pattern matching instead of scope-based lookup
        // The virtual_position is already in the coordinate system of the extracted MeTTa content
        // (lines start from 0 of the extracted content, NOT offset by parent_start).
        // The symbol table positions use LspPosition which starts from line 0 of extracted content.
        // The IR node positions also start from line 0 of extracted content.
        // So we can use virtual_position directly!
        let ir_pos = IrPosition {
            row: virtual_position.line as usize,
            column: virtual_position.character as usize,
            byte: 0,
        };

        debug!("Attempting to find function call at position for symbol '{}' (symbol table has {} IR nodes, position L{}:C{})",
            symbol.name, symbol_table.ir_nodes.len(),
            ir_pos.row, ir_pos.column);

        // Try to find the containing SExpr (function call)
        if let Some(call_node) = self.find_metta_call_at_position(&symbol_table.ir_nodes, &ir_pos) {
            debug!("Found call node for symbol '{}'", symbol.name);

            // Check if the clicked symbol is the function name (first element of SExpr)
            let is_function_name = match &call_node {
                MettaNode::SExpr { elements, .. } if elements.len() > 0 => {
                    // Check if the position is within the first element (function name)
                    if let Some(first_elem_name) = elements[0].name() {
                        debug!("Comparing first element name '{}' with symbol name '{}'", first_elem_name, symbol.name);
                        first_elem_name == symbol.name
                    } else {
                        debug!("First element has no name");
                        false
                    }
                }
                _ => {
                    debug!("Call node is not an SExpr or has no elements");
                    false
                }
            };

            debug!("is_function_name = {} for symbol '{}'", is_function_name, symbol.name);

            if is_function_name {
                debug!("Symbol '{}' is in function call position, using pattern matching", symbol.name);

                // Find all matching definitions using pattern matcher
                let matching_locations = symbol_table.find_function_definitions(&call_node);

                if matching_locations.is_empty() {
                    debug!("No pattern-matched definitions found for '{}'", symbol.name);

                    // Fallback: Find all occurrences of this symbol
                    // This handles cases like (connected room1 room2) in knowledge bases
                    // where the symbol isn't a function definition but appears in S-expressions
                    let all_usages: Vec<&SymbolOccurrence> = symbol_table.all_occurrences.iter()
                        .filter(|occ| occ.name == symbol.name)
                        .collect();

                    if !all_usages.is_empty() {
                        debug!("Found {} usage(s) of '{}' as S-expression head", all_usages.len(), symbol.name);

                        let parent_locations: Vec<Location> = all_usages
                            .into_iter()
                            .map(|occ| {
                                let parent_range = virtual_doc.map_range_to_parent(occ.range);
                                Location {
                                    uri: virtual_doc.parent_uri.clone(),
                                    range: parent_range,
                                }
                            })
                            .collect();

                        if parent_locations.len() == 1 {
                            return Ok(Some(GotoDefinitionResponse::Scalar(parent_locations.into_iter().next().unwrap())));
                        } else {
                            return Ok(Some(GotoDefinitionResponse::Array(parent_locations)));
                        }
                    }
                    // Fall through to scope-based lookup as fallback
                } else {
                    // Map locations from virtual to parent document
                    let parent_locations: Vec<Location> = matching_locations
                        .into_iter()
                        .map(|loc| {
                            let parent_range = virtual_doc.map_range_to_parent(loc.range);
                            Location {
                                uri: virtual_doc.parent_uri.clone(),
                                range: parent_range,
                            }
                        })
                        .collect();

                    debug!(
                        "Found {} pattern-matched definition(s) for '{}'",
                        parent_locations.len(),
                        symbol.name
                    );

                    if parent_locations.len() == 1 {
                        return Ok(Some(GotoDefinitionResponse::Scalar(parent_locations.into_iter().next().unwrap())));
                    } else {
                        return Ok(Some(GotoDefinitionResponse::Array(parent_locations)));
                    }
                }
            }
        }

        // Try scope-based lookup (for variables, parameters, etc.)
        if let Some(definition) = symbol_table.find_definition(symbol) {
            // Map virtual range to parent range
            let parent_range = virtual_doc.map_range_to_parent(definition.range);

            debug!(
                "Found MeTTa variable definition for '{}' at {:?}",
                symbol.name, parent_range
            );

            return Ok(Some(GotoDefinitionResponse::Scalar(Location {
                uri: virtual_doc.parent_uri.clone(),
                range: parent_range,
            })));
        }

        // Try cross-document lookup using composable resolver (AsyncGlobalVirtualSymbolResolver)
        // This enables goto-definition across all MeTTa virtual documents in the workspace
        debug!("Trying cross-document lookup for MeTTa symbol '{}'", symbol.name);

        use crate::ir::symbol_resolution::ResolutionContext;

        let global_resolver = AsyncGlobalVirtualSymbolResolver::new(self.workspace.clone());
        let resolution_context = ResolutionContext {
            uri: virtual_doc.uri.clone(),
            scope_id: Some(symbol.scope_id),
            ir_node: None,
            language: virtual_doc.language.clone(),
            parent_uri: Some(virtual_doc.parent_uri.clone()),
        };

        let symbol_locations = global_resolver
            .resolve_symbol_async(&symbol.name, &resolution_context)
            .await;

        if !symbol_locations.is_empty() {
            debug!(
                "Found {} cross-document definition(s) for MeTTa symbol '{}' via AsyncGlobalVirtualSymbolResolver",
                symbol_locations.len(),
                symbol.name
            );

            // Map SymbolLocation to LSP Location
            let parent_locations: Vec<Location> = symbol_locations
                .iter()
                .map(|sym_loc| Location {
                    uri: sym_loc.uri.clone(),
                    range: sym_loc.range,
                })
                .collect();

            if parent_locations.len() == 1 {
                return Ok(Some(GotoDefinitionResponse::Scalar(
                    parent_locations.into_iter().next().unwrap(),
                )));
            } else {
                return Ok(Some(GotoDefinitionResponse::Array(parent_locations)));
            }
        }

        debug!("No definition found for MeTTa symbol '{}'", symbol.name);
        Ok(None)
    }

    /// Find the function call SExpr containing the given position
    ///
    /// Searches for the innermost SExpr that contains the position and
    /// could represent a function call (i.e., has an atom as first element)
    fn find_metta_call_at_position(
        &self,
        nodes: &[Arc<MettaNode>],
        position: &IrPosition,
    ) -> Option<MettaNode> {
        use crate::ir::metta_node::compute_positions_with_prev_end;

        debug!("Searching {} top-level IR nodes for call at position L{}:C{}",
            nodes.len(), position.row, position.column);

        // We need to compute positions for all nodes with proper prev_end tracking
        // to get accurate ranges that account for comments and whitespace
        let mut prev_end = IrPosition {
            row: 0,
            column: 0,
            byte: 0,
        };

        for (i, node) in nodes.iter().enumerate() {
            let node_type = match &**node {
                MettaNode::Definition { .. } => "Definition",
                MettaNode::SExpr { .. } => "SExpr",
                MettaNode::Atom { .. } => "Atom",
                MettaNode::If { .. } => "If",
                _ => "Other"
            };

            // Compute positions with prev_end tracking (includes comments/whitespace)
            let (positions, new_prev_end) = compute_positions_with_prev_end(node, prev_end);
            prev_end = new_prev_end;

            let node_ptr = &**node as *const MettaNode as usize;
            let range_info = if let Some((start, end)) = positions.get(&node_ptr) {
                format!("L{}:C{}-L{}:C{}", start.row, start.column, end.row, end.column)
            } else {
                "no-position".to_string()
            };

            debug!("Checking top-level node {} (type: {}, range: {})", i, node_type, range_info);

            // Use the properly computed positions for this node
            if let Some(call) = self.find_metta_call_in_node(node, position, &positions) {
                debug!("Found call in top-level node {}", i);
                return Some(call);
            }
        }
        debug!("No call found in any of the {} top-level nodes", nodes.len());
        None
    }

    /// Recursively search for function call in a node
    fn find_metta_call_in_node(
        &self,
        node: &Arc<MettaNode>,
        position: &IrPosition,
        positions: &std::collections::HashMap<usize, (IrPosition, IrPosition)>,
    ) -> Option<MettaNode> {
        // Use the pre-computed positions (with proper prev_end tracking)
        let node_ptr = &**node as *const MettaNode as usize;
        let (start, end) = match positions.get(&node_ptr) {
            Some(pos) => pos,
            None => {
                debug!("No position info for node");
                return None;
            }
        };

        if !self.position_in_range(position, start, end) {
            return None;
        }

        // If this is an SExpr with an atom as first element, it's a potential function call
        match &**node {
            MettaNode::SExpr { elements, .. } if elements.len() > 0 => {
                debug!("Searching in SExpr with {} elements", elements.len());
                // First check children to find the most specific match
                for elem in elements {
                    if let Some(call) = self.find_metta_call_in_node(elem, position, positions) {
                        return Some(call);
                    }
                }

                // If no child matched more specifically, check if this is a call
                if matches!(&*elements[0], MettaNode::Atom { .. }) {
                    debug!("Found SExpr with Atom as first element - returning as call");
                    return Some((**node).clone());
                }

                None
            }
            MettaNode::Definition { pattern, body, .. } => {
                debug!("Searching in Definition: pattern and body");
                self.find_metta_call_in_node(pattern, position, positions)
                    .or_else(|| self.find_metta_call_in_node(body, position, positions))
            }
            MettaNode::Match { scrutinee, cases, .. } => {
                self.find_metta_call_in_node(scrutinee, position, positions)
                    .or_else(|| {
                        for (pat, res) in cases {
                            if let Some(call) = self.find_metta_call_in_node(pat, position, positions) {
                                return Some(call);
                            }
                            if let Some(call) = self.find_metta_call_in_node(res, position, positions) {
                                return Some(call);
                            }
                        }
                        None
                    })
            }
            MettaNode::If { condition, consequence, alternative, .. } => {
                debug!("Searching in If node: condition, consequence, alternative");
                self.find_metta_call_in_node(condition, position, positions)
                    .or_else(|| self.find_metta_call_in_node(consequence, position, positions))
                    .or_else(|| {
                        if let Some(alt) = alternative {
                            self.find_metta_call_in_node(alt, position, positions)
                        } else {
                            None
                        }
                    })
            }
            MettaNode::Let { bindings, body, .. } => {
                for (var, val) in bindings {
                    if let Some(call) = self.find_metta_call_in_node(var, position, positions) {
                        return Some(call);
                    }
                    if let Some(call) = self.find_metta_call_in_node(val, position, positions) {
                        return Some(call);
                    }
                }
                self.find_metta_call_in_node(body, position, positions)
            }
            MettaNode::Lambda { params, body, .. } => {
                for param in params {
                    if let Some(call) = self.find_metta_call_in_node(param, position, positions) {
                        return Some(call);
                    }
                }
                self.find_metta_call_in_node(body, position, positions)
            }
            MettaNode::TypeAnnotation { expr, type_expr, .. } => {
                self.find_metta_call_in_node(expr, position, positions)
                    .or_else(|| self.find_metta_call_in_node(type_expr, position, positions))
            }
            MettaNode::Eval { expr, .. } => {
                self.find_metta_call_in_node(expr, position, positions)
            }
            _ => None,
        }
    }

    /// Check if a position is within a range
    fn position_in_range(
        &self,
        pos: &IrPosition,
        start: &IrPosition,
        end: &IrPosition,
    ) -> bool {
        (pos.row > start.row || (pos.row == start.row && pos.column >= start.column))
            && (pos.row < end.row || (pos.row == end.row && pos.column <= end.column))
    }

    /// Rename support for MeTTa symbols
    pub(super) async fn rename_metta(
        &self,
        virtual_doc: &Arc<VirtualDocument>,
        virtual_position: LspPosition,
        new_name: &str,
    ) -> LspResult<Option<WorkspaceEdit>> {
        
        use std::collections::HashMap;

        // Get symbol table
        let symbol_table = match virtual_doc.get_or_build_symbol_table() {
            Some(table) => table,
            None => {
                debug!("Failed to build symbol table for MeTTa virtual document");
                return Ok(None);
            }
        };

        // Find symbol at position
        let symbol = match symbol_table.find_symbol_at_position(&virtual_position) {
            Some(sym) => sym,
            None => {
                debug!("No MeTTa symbol at position {:?}", virtual_position);
                return Ok(None);
            }
        };

        // Find all references in the same scope
        let references = symbol_table.find_symbol_references(symbol);

        // Create text edits for all occurrences
        let edits: Vec<TextEdit> = references
            .iter()
            .map(|occ| {
                let parent_range = virtual_doc.map_range_to_parent(occ.range);
                TextEdit {
                    range: parent_range,
                    new_text: new_name.to_string(),
                }
            })
            .collect();

        if edits.is_empty() {
            return Ok(None);
        }

        // Build workspace edit
        let mut changes = HashMap::new();
        changes.insert(virtual_doc.parent_uri.clone(), edits);

        debug!(
            "Renaming MeTTa symbol '{}' to '{}' ({} occurrences)",
            symbol.name,
            new_name,
            changes.values().map(|v| v.len()).sum::<usize>()
        );

        Ok(Some(WorkspaceEdit {
            changes: Some(changes),
            document_changes: None,
            change_annotations: None,
        }))
    }

    /// Find all references to a MeTTa symbol
    pub(super) async fn references_metta(
        &self,
        virtual_doc: &Arc<VirtualDocument>,
        virtual_position: LspPosition,
        include_declaration: bool,
    ) -> LspResult<Option<Vec<Location>>> {
        // Get symbol table
        let symbol_table = match virtual_doc.get_or_build_symbol_table() {
            Some(table) => table,
            None => {
                debug!("Failed to build symbol table for MeTTa virtual document");
                return Ok(None);
            }
        };

        // Find symbol at position
        let symbol = match symbol_table.find_symbol_at_position(&virtual_position) {
            Some(sym) => sym,
            None => {
                debug!("No MeTTa symbol at position {:?}", virtual_position);
                return Ok(None);
            }
        };

        // Find all references in the same scope
        let references = symbol_table.find_symbol_references(symbol);

        // Create locations for all occurrences
        let locations: Vec<Location> = references
            .iter()
            .filter(|occ| {
                // Include or exclude declaration based on parameter
                if include_declaration {
                    true
                } else {
                    !occ.is_definition
                }
            })
            .map(|occ| {
                let parent_range = virtual_doc.map_range_to_parent(occ.range);
                Location {
                    uri: virtual_doc.parent_uri.clone(),
                    range: parent_range,
                }
            })
            .collect();

        if locations.is_empty() {
            debug!("No references found for MeTTa symbol '{}'", symbol.name);
            return Ok(None);
        }

        debug!(
            "Found {} reference(s) for MeTTa symbol '{}' (include_declaration: {})",
            locations.len(),
            symbol.name,
            include_declaration
        );

        Ok(Some(locations))
    }
}
