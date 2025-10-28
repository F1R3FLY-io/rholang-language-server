use lsp_types::{Position, Range};
use std::sync::Arc;
use crate::ir::rholang_node::{RholangNode, BinOperator};

/// Represents a part of a concatenation chain - either a string literal or a hole
#[derive(Debug, Clone)]
pub enum ConcatPart {
    /// A string literal that becomes part of the virtual document
    Literal {
        /// The actual string content
        content: String,
        /// Position range in the original Rholang source
        original_range: Range,
    },
    /// A variable or expression that creates a "hole" in the virtual document
    Hole {
        /// Position range of the variable/expr in the Rholang source
        original_range: Range,
    },
}

impl ConcatPart {
    /// Returns the original range in the Rholang source
    pub fn original_range(&self) -> &Range {
        match self {
            ConcatPart::Literal { original_range, .. } => original_range,
            ConcatPart::Hole { original_range } => original_range,
        }
    }

    /// Returns the length in the virtual document (0 for holes)
    pub fn virtual_length(&self) -> usize {
        match self {
            ConcatPart::Literal { content, .. } => content.len(),
            ConcatPart::Hole { .. } => 0,
        }
    }

    /// Returns true if this is a literal part
    pub fn is_literal(&self) -> bool {
        matches!(self, ConcatPart::Literal { .. })
    }

    /// Returns true if this is a hole
    pub fn is_hole(&self) -> bool {
        matches!(self, ConcatPart::Hole { .. })
    }
}

/// Represents a chain of concatenated string parts
#[derive(Debug, Clone)]
pub struct ConcatenationChain {
    /// The parts of the concatenation in order
    pub parts: Vec<ConcatPart>,
    /// The full range encompassing the entire concatenation expression
    pub full_range: Range,
}

impl ConcatenationChain {
    /// Creates a new concatenation chain
    pub fn new(parts: Vec<ConcatPart>, full_range: Range) -> Self {
        ConcatenationChain { parts, full_range }
    }

    /// Reconstructs the virtual document content (with holes removed)
    pub fn to_virtual_content(&self) -> String {
        self.parts
            .iter()
            .filter_map(|part| match part {
                ConcatPart::Literal { content, .. } => Some(content.as_str()),
                ConcatPart::Hole { .. } => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }

    /// Returns the number of literal parts (excluding holes)
    pub fn literal_count(&self) -> usize {
        self.parts.iter().filter(|p| p.is_literal()).count()
    }

    /// Returns the number of holes
    pub fn hole_count(&self) -> usize {
        self.parts.iter().filter(|p| p.is_hole()).count()
    }

    /// Returns true if the chain contains any holes
    pub fn has_holes(&self) -> bool {
        self.parts.iter().any(|p| p.is_hole())
    }

    /// Returns the total length of the virtual document (sum of literal lengths)
    pub fn virtual_length(&self) -> usize {
        self.parts.iter().map(|p| p.virtual_length()).sum()
    }
}

/// Position mapping for virtual documents with holes
#[derive(Debug, Clone)]
pub struct HoledPositionMap {
    /// The concatenation chain this map is based on
    chain: Arc<ConcatenationChain>,
}

impl HoledPositionMap {
    /// Creates a new position map for a concatenation chain
    pub fn new(chain: Arc<ConcatenationChain>) -> Self {
        HoledPositionMap { chain }
    }

    /// Maps a position in the virtual document to the original Rholang source
    ///
    /// Returns None if the position is out of bounds or falls in a hole
    pub fn virtual_to_original(&self, virtual_pos: Position) -> Option<Position> {
        // For now, we assume single-line virtual documents (common for MeTTa snippets)
        // If we need multi-line support, this will need to be extended

        let virtual_offset = virtual_pos.character as usize;
        let mut current_virtual_offset = 0;

        for part in &self.chain.parts {
            match part {
                ConcatPart::Literal { content, original_range } => {
                    let part_len = content.len();
                    if virtual_offset < current_virtual_offset + part_len {
                        // Position falls within this literal
                        let offset_in_part = virtual_offset - current_virtual_offset;

                        // Calculate position in original source
                        // Assuming single-line string literals for now
                        let original_char = original_range.start.character + offset_in_part as u32;

                        return Some(Position {
                            line: original_range.start.line,
                            character: original_char,
                        });
                    }
                    current_virtual_offset += part_len;
                }
                ConcatPart::Hole { .. } => {
                    // Holes don't contribute to virtual document position
                    continue;
                }
            }
        }

        None
    }

    /// Maps a position in the original Rholang source to the virtual document
    ///
    /// Returns None if the position doesn't fall within a literal part (i.e., it's in a hole)
    pub fn original_to_virtual(&self, original_pos: Position) -> Option<Position> {
        let mut current_virtual_offset = 0;

        for part in &self.chain.parts {
            match part {
                ConcatPart::Literal { content, original_range } => {
                    // Check if position falls within this literal's range
                    if Self::position_in_range(original_pos, original_range) {
                        // Calculate offset within this literal
                        let offset_in_part = if original_pos.line == original_range.start.line {
                            (original_pos.character - original_range.start.character) as usize
                        } else {
                            // Multi-line case - would need more sophisticated handling
                            // For now, return None for multi-line literals
                            return None;
                        };

                        let virtual_char = current_virtual_offset + offset_in_part;

                        return Some(Position {
                            line: 0, // Virtual documents are single-line for now
                            character: virtual_char as u32,
                        });
                    }
                    current_virtual_offset += content.len();
                }
                ConcatPart::Hole { original_range } => {
                    // If position falls in a hole, return None
                    if Self::position_in_range(original_pos, original_range) {
                        return None;
                    }
                    // Holes don't contribute to virtual offset
                }
            }
        }

        None
    }

    /// Helper to check if a position falls within a range
    fn position_in_range(pos: Position, range: &Range) -> bool {
        if pos.line < range.start.line || pos.line > range.end.line {
            return false;
        }

        if pos.line == range.start.line && pos.character < range.start.character {
            return false;
        }

        if pos.line == range.end.line && pos.character >= range.end.character {
            return false;
        }

        true
    }

    /// Returns the underlying concatenation chain
    pub fn chain(&self) -> &ConcatenationChain {
        &self.chain
    }
}

/// Extracts a concatenation chain from an IR node
///
/// Returns Some(ConcatenationChain) if the node is a concatenation of strings,
/// or None if it's not a concatenation or doesn't contain string literals
pub fn extract_concatenation_chain(node: &Arc<RholangNode>) -> Option<ConcatenationChain> {
    match node.as_ref() {
        RholangNode::BinOp { op, left, right, base, .. } => {
            if matches!(op, BinOperator::Concat) {
                // Recursively extract parts from left and right
                let mut parts = Vec::new();

                extract_concat_parts(left, &mut parts);
                extract_concat_parts(right, &mut parts);

                if !parts.is_empty() {
                    // Calculate full range from first to last part
                    let full_range = Range {
                        start: parts.first()?.original_range().start,
                        end: parts.last()?.original_range().end,
                    };

                    return Some(ConcatenationChain::new(parts, full_range));
                }
            }
        }
        _ => {}
    }

    None
}

/// Helper function to recursively extract concatenation parts
fn extract_concat_parts(node: &Arc<RholangNode>, parts: &mut Vec<ConcatPart>) {
    match node.as_ref() {
        RholangNode::BinOp { op, left, right, .. } => {
            if matches!(op, BinOperator::Concat) {
                // Recursively process left and right
                extract_concat_parts(left, parts);
                extract_concat_parts(right, parts);
                return;
            }
            // Non-concat binary operators are treated as holes
            let range = compute_node_range(node);
            parts.push(ConcatPart::Hole {
                original_range: range,
            });
        }
        RholangNode::StringLiteral { value, .. } => {
            // This is a string literal
            let range = compute_node_range(node);
            parts.push(ConcatPart::Literal {
                content: value.clone(),
                original_range: range,
            });
        }
        _ => {
            // All other nodes are treated as holes
            let range = compute_node_range(node);
            parts.push(ConcatPart::Hole {
                original_range: range,
            });
        }
    }
}

/// Helper to compute the range of any RholangNode
///
/// NOTE: This currently returns a placeholder range. In production use, ranges should be
/// computed using the document's position map via `compute_absolute_positions()`.
/// This function exists to support standalone testing and will be enhanced when integrated
/// with the LSP document system.
fn compute_node_range(_node: &RholangNode) -> Range {
    // TODO: Integrate with position mapping system
    // When used in LSP context, this should take a positions HashMap parameter
    // and use it to compute the absolute Range from the node's relative position

    // For now, return a placeholder range
    // This will be properly implemented when extract_concatenation_chain is called
    // from the semantic detector with access to the document's position map
    Range {
        start: Position { line: 0, character: 0 },
        end: Position { line: 0, character: 0 },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_concat_part_virtual_length() {
        let literal = ConcatPart::Literal {
            content: "hello".to_string(),
            original_range: Range {
                start: Position { line: 0, character: 0 },
                end: Position { line: 0, character: 5 },
            },
        };
        assert_eq!(literal.virtual_length(), 5);

        let hole = ConcatPart::Hole {
            original_range: Range {
                start: Position { line: 0, character: 10 },
                end: Position { line: 0, character: 15 },
            },
        };
        assert_eq!(hole.virtual_length(), 0);
    }

    #[test]
    fn test_concatenation_chain_to_virtual_content() {
        let parts = vec![
            ConcatPart::Literal {
                content: "!(get_neighbors ".to_string(),
                original_range: Range {
                    start: Position { line: 0, character: 0 },
                    end: Position { line: 0, character: 16 },
                },
            },
            ConcatPart::Hole {
                original_range: Range {
                    start: Position { line: 0, character: 20 },
                    end: Position { line: 0, character: 28 },
                },
            },
            ConcatPart::Literal {
                content: ")".to_string(),
                original_range: Range {
                    start: Position { line: 0, character: 32 },
                    end: Position { line: 0, character: 33 },
                },
            },
        ];

        let chain = ConcatenationChain::new(
            parts,
            Range {
                start: Position { line: 0, character: 0 },
                end: Position { line: 0, character: 33 },
            },
        );

        assert_eq!(chain.to_virtual_content(), "!(get_neighbors )");
        assert_eq!(chain.literal_count(), 2);
        assert_eq!(chain.hole_count(), 1);
        assert!(chain.has_holes());
        assert_eq!(chain.virtual_length(), 17);
    }

    #[test]
    fn test_holed_position_map_virtual_to_original() {
        let parts = vec![
            ConcatPart::Literal {
                content: "hello ".to_string(),
                original_range: Range {
                    start: Position { line: 5, character: 10 },
                    end: Position { line: 5, character: 16 },
                },
            },
            ConcatPart::Hole {
                original_range: Range {
                    start: Position { line: 5, character: 20 },
                    end: Position { line: 5, character: 24 },
                },
            },
            ConcatPart::Literal {
                content: " world".to_string(),
                original_range: Range {
                    start: Position { line: 5, character: 28 },
                    end: Position { line: 5, character: 34 },
                },
            },
        ];

        let chain = Arc::new(ConcatenationChain::new(
            parts,
            Range {
                start: Position { line: 5, character: 10 },
                end: Position { line: 5, character: 34 },
            },
        ));

        let map = HoledPositionMap::new(chain);

        // Test mapping positions in first literal
        let virtual_pos = Position { line: 0, character: 0 };
        let original = map.virtual_to_original(virtual_pos).unwrap();
        assert_eq!(original.line, 5);
        assert_eq!(original.character, 10);

        let virtual_pos = Position { line: 0, character: 5 };
        let original = map.virtual_to_original(virtual_pos).unwrap();
        assert_eq!(original.line, 5);
        assert_eq!(original.character, 15);

        // Test mapping positions in second literal (after hole)
        let virtual_pos = Position { line: 0, character: 6 };
        let original = map.virtual_to_original(virtual_pos).unwrap();
        assert_eq!(original.line, 5);
        assert_eq!(original.character, 28);
    }

    #[test]
    fn test_holed_position_map_original_to_virtual() {
        let parts = vec![
            ConcatPart::Literal {
                content: "hello ".to_string(),
                original_range: Range {
                    start: Position { line: 5, character: 10 },
                    end: Position { line: 5, character: 16 },
                },
            },
            ConcatPart::Hole {
                original_range: Range {
                    start: Position { line: 5, character: 20 },
                    end: Position { line: 5, character: 24 },
                },
            },
            ConcatPart::Literal {
                content: " world".to_string(),
                original_range: Range {
                    start: Position { line: 5, character: 28 },
                    end: Position { line: 5, character: 34 },
                },
            },
        ];

        let chain = Arc::new(ConcatenationChain::new(
            parts,
            Range {
                start: Position { line: 5, character: 10 },
                end: Position { line: 5, character: 34 },
            },
        ));

        let map = HoledPositionMap::new(chain);

        // Test mapping from first literal
        let original_pos = Position { line: 5, character: 10 };
        let virt_pos = map.original_to_virtual(original_pos).unwrap();
        assert_eq!(virt_pos.line, 0);
        assert_eq!(virt_pos.character, 0);

        // Test mapping from hole (should return None)
        let original_pos = Position { line: 5, character: 22 };
        assert!(map.original_to_virtual(original_pos).is_none());

        // Test mapping from second literal
        let original_pos = Position { line: 5, character: 30 };
        let virt_pos = map.original_to_virtual(original_pos).unwrap();
        assert_eq!(virt_pos.line, 0);
        assert_eq!(virt_pos.character, 8); // 6 chars from first literal + 2 from second
    }
}
