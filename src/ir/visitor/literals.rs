//! Visitor methods for literal value nodes
//!
//! This module provides the LiteralVisitor trait with methods for visiting
//! literal value nodes in the Rholang IR (bool, long, string, URI, nil, unit).

use std::sync::Arc;
use super::super::rholang_node::{RholangNode, Metadata};
use super::super::semantic_node::NodeBase;

/// Visitor methods for literal value constructs
///
/// All methods have default implementations that return the original node unchanged.
/// Override specific methods to transform literal nodes.
pub trait LiteralVisitor {
    /// Main dispatcher for recursion (required by trait composition)
    fn visit_node(&self, node: &Arc<RholangNode>) -> Arc<RholangNode>;

    /// Visits a boolean literal node (BoolLiteral)
    ///
    /// # Arguments
    /// * `node` - The original BoolLiteral node
    /// * `base` - Metadata including position and text
    /// * `value` - The boolean value (true or false)
    /// * `metadata` - Optional node metadata
    ///
    /// # Returns
    /// The original node by default; override to transform
    ///
    /// # Examples
    /// For `true` or `false`, returns unchanged unless overridden
    fn visit_bool_literal(
        &self,
        node: &Arc<RholangNode>,
        _base: &NodeBase,
        _value: bool,
        _metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        Arc::clone(node)
    }

    /// Visits a long integer literal node (LongLiteral)
    ///
    /// # Arguments
    /// * `node` - The original LongLiteral node
    /// * `base` - Metadata including position and text
    /// * `value` - The 64-bit integer value
    /// * `metadata` - Optional node metadata
    ///
    /// # Returns
    /// The original node by default; override to transform
    ///
    /// # Examples
    /// For `123` or `-456`, returns unchanged unless overridden
    fn visit_long_literal(
        &self,
        node: &Arc<RholangNode>,
        _base: &NodeBase,
        _value: i64,
        _metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        Arc::clone(node)
    }

    /// Visits a string literal node (StringLiteral)
    ///
    /// # Arguments
    /// * `node` - The original StringLiteral node
    /// * `base` - Metadata including position and text
    /// * `value` - The string value
    /// * `metadata` - Optional node metadata
    ///
    /// # Returns
    /// The original node by default; override to transform
    ///
    /// # Examples
    /// For `"hello"` or `"world"`, returns unchanged unless overridden
    fn visit_string_literal(
        &self,
        node: &Arc<RholangNode>,
        _base: &NodeBase,
        _value: &str,
        _metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        Arc::clone(node)
    }

    /// Visits a URI literal node (UriLiteral)
    ///
    /// # Arguments
    /// * `node` - The original UriLiteral node
    /// * `base` - Metadata including position and text
    /// * `value` - The URI string
    /// * `metadata` - Optional node metadata
    ///
    /// # Returns
    /// The original node by default; override to transform
    ///
    /// # Examples
    /// For `\`http://example.com\``, returns unchanged unless overridden
    fn visit_uri_literal(
        &self,
        node: &Arc<RholangNode>,
        _base: &NodeBase,
        _value: &str,
        _metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        Arc::clone(node)
    }

    /// Visits a nil node (Nil)
    ///
    /// # Arguments
    /// * `node` - The original Nil node
    /// * `base` - Metadata including position and text
    /// * `metadata` - Optional node metadata
    ///
    /// # Returns
    /// The original node by default; override to transform
    ///
    /// # Examples
    /// For `Nil`, returns unchanged unless overridden
    fn visit_nil(
        &self,
        node: &Arc<RholangNode>,
        _base: &NodeBase,
        _metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        Arc::clone(node)
    }

    /// Visits a unit value node (Unit)
    ///
    /// # Arguments
    /// * `node` - The original Unit node
    /// * `base` - Metadata including position and text
    /// * `metadata` - Optional node metadata
    ///
    /// # Returns
    /// The original node by default; override to transform
    ///
    /// # Examples
    /// For `()`, returns unchanged unless overridden
    fn visit_unit(
        &self,
        node: &Arc<RholangNode>,
        _base: &NodeBase,
        _metadata: &Option<Arc<Metadata>>,
    ) -> Arc<RholangNode> {
        Arc::clone(node)
    }
}
