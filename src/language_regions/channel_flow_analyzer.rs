//! Channel flow analysis for detecting embedded language regions
//!
//! Tracks data flow through channels to detect embedded languages.
//! For example, if a variable receives a MeTTa compiler channel,
//! any strings sent to that variable should be treated as MeTTa code.
//!
//! Example:
//! ```rholang
//! new metta in {
//!   @"rho:metta:compile"!(*metta) |
//!   metta!("(= factorial ...)") // <- Detected as MeTTa via flow analysis
//! }
//! ```

use tree_sitter::{Node as TSNode, Tree};
use ropey::Rope;
use tracing::{debug, trace};

use super::{LanguageRegion, RegionSource};

/// Tracks which variables are bound to language compiler channels
#[derive(Debug, Clone)]
struct ChannelBinding {
    /// Variable name
    var_name: String,
    /// Language the channel compiles/evaluates
    language: String,
    /// Scope depth where this binding exists
    scope_depth: usize,
}

/// Represents a string sent to an intermediate channel
#[derive(Debug, Clone)]
struct PendingSend {
    /// Channel name the string was sent to
    channel_name: String,
    /// String content
    content: String,
    /// Start byte position
    start_byte: usize,
    /// End byte position
    end_byte: usize,
    /// Start line
    start_line: usize,
    /// Start column
    start_column: usize,
}

/// Tracks which variables receive from which channels
#[derive(Debug, Clone)]
struct VariableSource {
    /// Variable name
    var_name: String,
    /// Channel it receives from
    source_channel: String,
    /// Scope depth
    scope_depth: usize,
}

/// Analyzer for detecting embedded languages via channel flow
pub struct ChannelFlowAnalyzer {
    /// Active channel bindings (variable -> language)
    bindings: Vec<ChannelBinding>,
    /// Pending string sends to intermediate channels
    pending_sends: Vec<PendingSend>,
    /// Variable sources (variable -> channel it receives from)
    variable_sources: Vec<VariableSource>,
    /// Current scope depth
    scope_depth: usize,
}

impl ChannelFlowAnalyzer {
    /// Creates a new channel flow analyzer
    pub fn new() -> Self {
        Self {
            bindings: Vec::new(),
            pending_sends: Vec::new(),
            variable_sources: Vec::new(),
            scope_depth: 0,
        }
    }

    /// Analyzes a Rholang AST to detect embedded languages via channel flow
    ///
    /// # Arguments
    /// * `source` - The source text
    /// * `tree` - The Tree-Sitter parse tree
    /// * `rope` - The rope representation
    ///
    /// # Returns
    /// Vector of detected language regions
    pub fn analyze(source: &str, tree: &Tree, _rope: &Rope) -> Vec<LanguageRegion> {
        let mut analyzer = Self::new();
        let mut regions = Vec::new();

        let root = tree.root_node();
        analyzer.analyze_node(&root, source, &mut regions);

        debug!("Channel flow analysis found {} regions", regions.len());
        regions
    }

    /// Analyzes a single AST node
    fn analyze_node<'a>(
        &mut self,
        node: &TSNode<'a>,
        source: &'a str,
        regions: &mut Vec<LanguageRegion>,
    ) {
        trace!("Analyzing node kind: {} at depth {}", node.kind(), self.scope_depth);

        match node.kind() {
            // New statements may bind unforgeable names to variables
            "new" => {
                trace!("Found new node");
                self.scope_depth += 1;
                self.analyze_new(node, source);
                self.analyze_children(node, source, regions);
                self.exit_scope();
            }

            // Enter new scope for blocks and contracts
            "block" | "contract" => {
                self.scope_depth += 1;
                self.analyze_children(node, source, regions);
                self.exit_scope();
            }

            // Input (for) statements may bind channels to variables
            "input" => {
                trace!("Found input node");
                self.analyze_input(node, source, regions);
                self.analyze_children(node, source, regions);
            }

            // Send statements may use bound channel variables
            "send" => {
                trace!("Found send node");
                self.analyze_send(node, source, regions);
                // Don't recurse into children - we've handled them
            }

            // Recursively analyze other nodes
            _ => {
                self.analyze_children(node, source, regions);
            }
        }
    }

    /// Analyzes all children of a node
    fn analyze_children<'a>(
        &mut self,
        node: &TSNode<'a>,
        source: &'a str,
        regions: &mut Vec<LanguageRegion>,
    ) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.analyze_node(&child, source, regions);
        }
    }

    /// Analyzes a new statement to detect unforgeable name bindings
    ///
    /// Pattern: `new mettaCompile(\`rho:metta:compile\`) in { ... }`
    fn analyze_new<'a>(&mut self, new_node: &TSNode<'a>, source: &'a str) {
        // New node structure: new name_decls in { proc }
        // We need to find name_decls that bind to language compiler channels

        for child in new_node.children(&mut new_node.walk()) {
            if child.kind() == "name_decls" {
                self.analyze_name_decls(&child, source);
            }
        }
    }

    /// Analyzes name declarations to find compiler channel bindings
    fn analyze_name_decls<'a>(&mut self, name_decls_node: &TSNode<'a>, source: &'a str) {
        // Each name_decl can be: var or var(unforgeable_name)
        for name_decl in name_decls_node.children(&mut name_decls_node.walk()) {
            if name_decl.kind() == "name_decl" {
                self.analyze_name_decl(&name_decl, source);
            }
        }
    }

    /// Analyzes a single name declaration (e.g., "mettaCompile(\`rho:metta:compile\`)")
    fn analyze_name_decl<'a>(&mut self, name_decl_node: &TSNode<'a>, source: &'a str) {
        trace!("analyze_name_decl: node kind = {}", name_decl_node.kind());

        // Extract variable name and unforgeable name (if present)
        let mut var_name: Option<String> = None;
        let mut uri_literal: Option<String> = None;

        for child in name_decl_node.children(&mut name_decl_node.walk()) {
            trace!("  name_decl child: {}", child.kind());

            match child.kind() {
                "var" => {
                    if let Ok(name) = child.utf8_text(source.as_bytes()) {
                        var_name = Some(name.to_string());
                        trace!("  Found var: {}", name);
                    }
                }
                "uri_literal" => {
                    if let Ok(uri) = child.utf8_text(source.as_bytes()) {
                        // Remove the backticks
                        let uri_content = uri.trim_matches('`');
                        uri_literal = Some(uri_content.to_string());
                        trace!("  Found uri_literal: {}", uri_content);
                    }
                }
                _ => {}
            }
        }

        // If we have both a variable name and a URI literal pointing to a compiler channel
        if let (Some(var), Some(uri)) = (var_name, uri_literal) {
            if let Some(language) = Self::channel_to_language(&uri) {
                trace!(
                    "Found new binding: {} <- {} (language: {})",
                    var,
                    uri,
                    language
                );

                self.bindings.push(ChannelBinding {
                    var_name: var,
                    language,
                    scope_depth: self.scope_depth,
                });
            }
        }
    }

    /// Analyzes an input (for) statement to detect channel bindings
    ///
    /// Pattern: `for(var <- @"rho:metta:compile") { ... }`
    fn analyze_input<'a>(&mut self, input_node: &TSNode<'a>, source: &'a str, regions: &mut Vec<LanguageRegion>) {
        // Input node structure: for (receipts) { proc }
        // We need to find receipts that bind from language compiler channels

        for child in input_node.children(&mut input_node.walk()) {
            if child.kind() == "receipts" {
                self.analyze_receipts(&child, source, regions);
            }
        }
    }

    /// Analyzes receipts to find channel bindings
    fn analyze_receipts<'a>(&mut self, receipts_node: &TSNode<'a>, source: &'a str, regions: &mut Vec<LanguageRegion>) {
        // Each receipt is a binding like: var <- channel
        for receipt in receipts_node.children(&mut receipts_node.walk()) {
            self.analyze_receipt(&receipt, source, regions);
        }
    }

    /// Analyzes a single receipt (e.g., "x <- @\"rho:metta:compile\"")
    fn analyze_receipt<'a>(&mut self, receipt_node: &TSNode<'a>, source: &'a str, regions: &mut Vec<LanguageRegion>) {
        trace!("analyze_receipt: node kind = {}", receipt_node.kind());

        // Receipt contains bind nodes as children
        for child in receipt_node.children(&mut receipt_node.walk()) {
            trace!("  Receipt child: {}", child.kind());

            if matches!(
                child.kind(),
                "linear_bind" | "repeated_bind" | "peek_bind"
            ) {
                trace!("  Found bind node: {}", child.kind());
                self.analyze_bind(&child, source, regions);
            }
        }
    }

    /// Analyzes a bind node
    fn analyze_bind<'a>(&mut self, bind_node: &TSNode<'a>, source: &'a str, regions: &mut Vec<LanguageRegion>) {
        trace!("analyze_bind called");
        // Get the variable names being bound
        let var_names = self.extract_bound_variables(bind_node, source);
        trace!("  Bound variables: {:?}", var_names);

        // Get the source channel (either a compiler channel or an intermediate channel)
        if let Some(channel_name) = self.extract_source_channel(bind_node, source) {
            trace!("  Source channel: {}", channel_name);

            // Check if this is a language compiler channel (either by URI or by variable binding)
            let language = Self::channel_to_language(&channel_name)
                .or_else(|| self.get_binding(&channel_name));

            if let Some(language) = language {
                trace!(
                    "Found compiler channel binding: {:?} <- {} (language: {})",
                    var_names,
                    channel_name,
                    language
                );

                // Record bindings for all variables
                for var_name in var_names.clone() {
                    self.bindings.push(ChannelBinding {
                        var_name,
                        language: language.clone(),
                        scope_depth: self.scope_depth,
                    });
                }

                // Also check if the source channel expression contains variable arguments
                // This handles: for (@state <- mettaCompile!?(code))
                for child in bind_node.children(&mut bind_node.walk()) {
                    if child.kind() == "input" || child.kind() == "simple_source" || child.kind() == "send_receive_source" {
                        self.check_source_expression_for_variables(&child, source, &language, regions);
                    }
                }
            }

            // Also track variable sources for data flow analysis (even for non-compiler channels)
            // This handles patterns like: for (@code <- codeFile) { mettaCompile!(code) }
            for var_name in var_names {
                trace!("  Tracking variable source: {} <- {}", var_name, channel_name);
                self.variable_sources.push(VariableSource {
                    var_name,
                    source_channel: channel_name.clone(),
                    scope_depth: self.scope_depth,
                });
            }
        } else {
            trace!("  No channel name found");
        }
    }

    /// Extracts variable names from a bind node
    fn extract_bound_variables<'a>(
        &self,
        bind_node: &TSNode<'a>,
        source: &'a str,
    ) -> Vec<String> {
        let mut vars = Vec::new();

        // Look for "names" field
        for child in bind_node.children(&mut bind_node.walk()) {
            if child.kind() == "names" {
                vars.extend(self.extract_names_from_pattern(&child, source));
            }
        }

        vars
    }

    /// Extracts variable names from a names pattern
    fn extract_names_from_pattern<'a>(
        &self,
        names_node: &TSNode<'a>,
        source: &'a str,
    ) -> Vec<String> {
        let mut names = Vec::new();

        for child in names_node.children(&mut names_node.walk()) {
            if child.kind() == "var" {
                if let Ok(text) = child.utf8_text(source.as_bytes()) {
                    names.push(text.to_string());
                }
            } else if child.kind() == "quote" {
                // Handle @code pattern
                names.extend(self.extract_names_from_quote(&child, source));
            }
        }

        names
    }

    /// Extracts variable names from a quote pattern (@var)
    fn extract_names_from_quote<'a>(
        &self,
        quote_node: &TSNode<'a>,
        source: &'a str,
    ) -> Vec<String> {
        let mut names = Vec::new();

        for child in quote_node.children(&mut quote_node.walk()) {
            if child.kind() == "var" {
                if let Ok(text) = child.utf8_text(source.as_bytes()) {
                    names.push(text.to_string());
                }
            }
        }

        names
    }

    /// Extracts the source channel from a bind node
    fn extract_source_channel<'a>(
        &self,
        bind_node: &TSNode<'a>,
        source: &'a str,
    ) -> Option<String> {
        // Look for "input" field which contains the channel
        for child in bind_node.children(&mut bind_node.walk()) {
            if child.kind() == "input" || child.kind() == "simple_source" || child.kind() == "send_receive_source" {
                return self.extract_channel_name(&child, source);
            }
        }

        None
    }

    /// Extracts channel name from a channel expression
    fn extract_channel_name<'a>(&self, channel_node: &TSNode<'a>, source: &'a str) -> Option<String> {

        // Handle quote expressions: @"rho:metta:compile"
        if channel_node.kind() == "quote" {
            for child in channel_node.children(&mut channel_node.walk()) {
                if child.kind() == "string_literal" {
                    if let Ok(text) = child.utf8_text(source.as_bytes()) {
                        // Remove quotes
                        let content = text.trim_matches('"');
                        return Some(content.to_string());
                    }
                }
            }
        }

        // Handle variable names directly: codeFile
        if channel_node.kind() == "var" {
            if let Ok(text) = channel_node.utf8_text(source.as_bytes()) {
                return Some(text.to_string());
            }
        }

        // Handle send expressions: mettaCompile!?(code)
        // The channel is the first child of the send node
        if channel_node.kind() == "send" {
            let mut cursor = channel_node.walk();
            if cursor.goto_first_child() {
                return self.extract_channel_name(&cursor.node(), source);
            }
        }

        // Recursively check children
        for child in channel_node.children(&mut channel_node.walk()) {
            if let Some(name) = self.extract_channel_name(&child, source) {
                return Some(name);
            }
        }

        None
    }

    /// Maps a channel name to a language
    fn channel_to_language(channel_name: &str) -> Option<String> {
        match channel_name {
            "rho:metta:compile" | "rho:metta:eval" | "rho:metta:repl" => Some("metta".to_string()),
            _ => None,
        }
    }

    /// Analyzes a send statement to detect sends to bound channel variables
    /// and track data flow through intermediate channels
    fn analyze_send<'a>(
        &mut self,
        send_node: &TSNode<'a>,
        source: &'a str,
        regions: &mut Vec<LanguageRegion>,
    ) {
        // Get the channel being sent to
        let mut cursor = send_node.walk();
        if !cursor.goto_first_child() {
            return;
        }

        let channel_node = cursor.node();

        // Check if this is a variable reference
        if channel_node.kind() == "var" {
            if let Ok(var_name) = channel_node.utf8_text(source.as_bytes()) {
                // Case 1: Send to a variable bound to a compiler channel
                if let Some(language) = self.get_binding(var_name) {
                    trace!("Found send to bound channel variable: {} ({})", var_name, language);

                    // Check what's being sent
                    self.analyze_send_arguments(send_node, source, var_name, &language, regions);
                } else {
                    // Case 2: Send to an intermediate channel (track pending sends)
                    self.track_pending_sends(send_node, source, var_name);
                }
            }
        }
    }

    /// Analyzes send arguments - handles both direct strings and variable forwards
    fn analyze_send_arguments<'a>(
        &mut self,
        send_node: &TSNode<'a>,
        source: &'a str,
        target_var: &str,
        language: &str,
        regions: &mut Vec<LanguageRegion>,
    ) {
        // Find the inputs node
        for child in send_node.children(&mut send_node.walk()) {
            if child.kind() == "inputs" {
                for input_child in child.children(&mut child.walk()) {
                    match input_child.kind() {
                        // Direct string send
                        "string_literal" => {
                            if let Ok(text) = input_child.utf8_text(source.as_bytes()) {
                                let content = Self::extract_string_content(text);

                                regions.push(LanguageRegion {
                                    language: language.to_string(),
                                    start_byte: input_child.start_byte() + 1,
                                    end_byte: input_child.end_byte() - 1,
                                    start_line: input_child.start_position().row,
                                    start_column: input_child.start_position().column,
                                    source: RegionSource::ChannelFlow,
                                    content,
                                concatenation_chain: None,
                                });
                            }
                        }
                        // Variable forward (e.g., mettaCompile!(code))
                        "var" => {
                            if let Ok(var_name) = input_child.utf8_text(source.as_bytes()) {
                                trace!("Variable forward detected: {} -> {}", var_name, target_var);
                                // Check if this variable has a source channel with pending sends
                                self.check_variable_forward(var_name, language, regions);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    /// Checks if a variable forward should create regions from pending sends
    fn check_variable_forward(&self, var_name: &str, language: &str, regions: &mut Vec<LanguageRegion>) {
        // Find if this variable receives from a channel
        if let Some(source_channel) = self.get_variable_source(var_name) {
            trace!("Variable {} receives from channel {}", var_name, source_channel);

            // Check if we have pending sends to that channel
            for pending in &self.pending_sends {
                if pending.channel_name == source_channel {
                    trace!("Creating region from pending send via data flow: {} -> {}", source_channel, var_name);

                    regions.push(LanguageRegion {
                        language: language.to_string(),
                        start_byte: pending.start_byte,
                        end_byte: pending.end_byte,
                        start_line: pending.start_line,
                        start_column: pending.start_column,
                        source: RegionSource::ChannelFlow,
                        content: pending.content.clone(),
                        concatenation_chain: None,
                    });
                }
            }
        }
    }

    /// Checks if a source expression contains variables that should be linked
    /// Handles: for (@state <- mettaCompile!?(code))
    fn check_source_expression_for_variables<'a>(
        &self,
        source_expr: &TSNode<'a>,
        source: &'a str,
        language: &str,
        regions: &mut Vec<LanguageRegion>,
    ) {
        trace!("check_source_expression_for_variables: node kind = {}", source_expr.kind());

        // Look for "var" nodes in the source expression
        for child in source_expr.children(&mut source_expr.walk()) {
            trace!("  Checking child: {}", child.kind());
            if child.kind() == "var" {
                // Extract variable name
                if let Ok(var_name) = child.utf8_text(source.as_bytes()) {
                    trace!("  Found variable in source expression: {}", var_name);
                    // Check if this variable forwards data
                    self.check_variable_forward(var_name, language, regions);
                }
            }
            // Recursively check children
            self.check_source_expression_for_variables(&child, source, language, regions);
        }
    }

    /// Gets the source channel for a variable (if any)
    fn get_variable_source(&self, var_name: &str) -> Option<String> {
        for source in self.variable_sources.iter().rev() {
            if source.var_name == var_name {
                return Some(source.source_channel.clone());
            }
        }
        None
    }

    /// Tracks string sends to intermediate channels for data flow analysis
    fn track_pending_sends<'a>(&mut self, send_node: &TSNode<'a>, source: &'a str, channel_name: &str) {
        // Find the inputs node and extract string literals
        for child in send_node.children(&mut send_node.walk()) {
            if child.kind() == "inputs" {
                for input_child in child.children(&mut child.walk()) {
                    if input_child.kind() == "string_literal" {
                        if let Ok(text) = input_child.utf8_text(source.as_bytes()) {
                            let content = Self::extract_string_content(text);

                            trace!("Tracking pending send to {}: {}", channel_name, &content[..content.len().min(40)]);

                            self.pending_sends.push(PendingSend {
                                channel_name: channel_name.to_string(),
                                content,
                                start_byte: input_child.start_byte() + 1,
                                end_byte: input_child.end_byte() - 1,
                                start_line: input_child.start_position().row,
                                start_column: input_child.start_position().column,
                            });
                        }
                    }
                }
            }
        }
    }

    /// Gets the language binding for a variable (if any)
    fn get_binding(&self, var_name: &str) -> Option<String> {
        // Search from most recent to oldest (for shadowing)
        for binding in self.bindings.iter().rev() {
            if binding.var_name == var_name {
                return Some(binding.language.clone());
            }
        }
        None
    }

    /// Extracts string literals from a send node and marks them as language regions
    fn extract_string_regions<'a>(
        &self,
        send_node: &TSNode<'a>,
        source: &'a str,
        language: &str,
        regions: &mut Vec<LanguageRegion>,
    ) {
        // Find the inputs node
        for child in send_node.children(&mut send_node.walk()) {
            if child.kind() == "inputs" {
                // Extract all string literals
                for input_child in child.children(&mut child.walk()) {
                    if input_child.kind() == "string_literal" {
                        if let Ok(text) = input_child.utf8_text(source.as_bytes()) {
                            let content = Self::extract_string_content(text);

                            regions.push(LanguageRegion {
                                language: language.to_string(),
                                start_byte: input_child.start_byte() + 1, // Skip quote
                                end_byte: input_child.end_byte() - 1,     // Skip quote
                                start_line: input_child.start_position().row,
                                start_column: input_child.start_position().column,
                                source: RegionSource::ChannelFlow,
                                content,
                            concatenation_chain: None,
                            });
                        }
                    }
                }
            }
        }
    }

    /// Extracts content from a string literal
    fn extract_string_content(string_with_quotes: &str) -> String {
        if string_with_quotes.len() < 2 {
            return String::new();
        }

        let content = &string_with_quotes[1..string_with_quotes.len() - 1];

        content
            .replace("\\\"", "\"")
            .replace("\\\\", "\\")
            .replace("\\n", "\n")
            .replace("\\r", "\r")
            .replace("\\t", "\t")
    }

    /// Exits the current scope, removing bindings and tracking data from that scope
    fn exit_scope(&mut self) {
        self.bindings.retain(|b| b.scope_depth < self.scope_depth);
        self.variable_sources.retain(|v| v.scope_depth < self.scope_depth);
        // Note: We keep pending_sends across scopes as they represent concrete string sends
        // that might be consumed in outer scopes
        self.scope_depth -= 1;
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree_sitter::parse_code;

    fn print_tree(node: &TSNode, source: &str, depth: usize) {
        let indent = "  ".repeat(depth);
        let text = node.utf8_text(source.as_bytes()).unwrap_or("");
        let preview = if text.len() > 40 {
            format!("{}...", &text[..40])
        } else {
            text.to_string()
        };
        eprintln!("{}{} [{}]", indent, node.kind(), preview.replace("\n", "\\n"));

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            print_tree(&child, source, depth + 1);
        }
    }

    #[test]
    fn test_new_unforgeable_binding() {
        let source = r#"
new mettaCompile(`rho:metta:compile`) in {
  mettaCompile!("(= test 123)")
}
"#;

        let tree = parse_code(source);
        let rope = Rope::from_str(source);

        let regions = ChannelFlowAnalyzer::analyze(source, &tree, &rope);

        assert_eq!(regions.len(), 1, "Should detect one region via new binding");
        assert_eq!(regions[0].language, "metta");
        assert_eq!(regions[0].source, RegionSource::ChannelFlow);
        assert!(regions[0].content.contains("test"));
    }

    #[test]
    fn test_simple_channel_flow() {
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

        assert_eq!(regions.len(), 1, "Should detect one region via flow analysis");
        assert_eq!(regions[0].language, "metta");
        assert_eq!(regions[0].source, RegionSource::ChannelFlow);
        assert!(regions[0].content.contains("factorial"));
    }

    #[test]
    fn test_multiple_sends_to_same_channel() {
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
    fn test_scoping() {
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
    fn test_no_detection_without_binding() {
        let source = r#"
new metta in {
  metta!("(= factorial 42)")
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

    #[test]
    fn test_eval_channel() {
        let source = r#"
new eval in {
  for (eval <- @"rho:metta:eval") {
    eval!("(+ 1 2 3)")
  }
}
"#;

        let tree = parse_code(source);
        let rope = Rope::from_str(source);

        let regions = ChannelFlowAnalyzer::analyze(source, &tree, &rope);

        assert_eq!(regions.len(), 1, "Should detect eval channel");
        assert!(regions[0].content.contains("+ 1 2 3"));
    }

    #[test]
    fn test_robot_planning_pattern() {
        // Test the pattern from robot_planning.rho:
        // 1. new mettaCompile(`rho:metta:compile`)
        // 2. codeFile!("... MeTTa code ...")
        // 3. for (@code <- codeFile) { mettaCompile!(code) }
        //
        // This test verifies that we can detect sends to variables
        // bound via new with unforgeable names
        let source = r#"
new mettaCompile(`rho:metta:compile`) in {
  new codeFile in {
    codeFile!("(= factorial (lambda (n) 42))") |
    for (@code <- codeFile) {
      mettaCompile!(code)
    }
  }
}
"#;

        let tree = parse_code(source);
        let rope = Rope::from_str(source);

        let regions = ChannelFlowAnalyzer::analyze(source, &tree, &rope);

        // We now support full data flow analysis!
        // 1. The string is sent to codeFile (tracked as pending send)
        // 2. The code variable receives from codeFile (tracked as variable source)
        // 3. When code is forwarded to mettaCompile, we link the pending send
        assert_eq!(
            regions.len(),
            1,
            "Should detect indirect flow through intermediate channels via data flow analysis"
        );
        assert!(regions[0].content.contains("factorial"));
        assert_eq!(regions[0].language, "metta");
    }

    #[test]
    fn test_direct_send_to_new_bound_variable() {
        // This pattern SHOULD work: direct send to a variable bound via new
        let source = r#"
new mettaCompile(`rho:metta:compile`) in {
  mettaCompile!("(= test 123)")
}
"#;

        let tree = parse_code(source);
        let rope = Rope::from_str(source);

        let regions = ChannelFlowAnalyzer::analyze(source, &tree, &rope);

        assert_eq!(regions.len(), 1, "Should detect direct send to new-bound variable");
        assert!(regions[0].content.contains("test"));
    }
}

