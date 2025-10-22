use std::cmp::Ordering;

use ropey::Rope;

use tower_lsp::lsp_types::{Position, TextDocumentContentChangeEvent, Url};

use tree_sitter::Tree;

use crate::tree_sitter::{parse_code, update_tree};

pub use crate::lsp::models::{LspDocument, LspDocumentState, VersionedChanges};

/// Converts an LSP position to a byte offset in the Rope.
fn position_to_byte_offset(position: &Position, text: &Rope) -> usize {
    let line = position.line as usize;
    let char = position.character as usize;
    text.line_to_char(line) + char
}

impl PartialEq for VersionedChanges {
    fn eq(&self, other: &Self) -> bool {
        self.version == other.version
    }
}

impl Eq for VersionedChanges {}

impl PartialOrd for VersionedChanges {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(other.version.cmp(&self.version))
    }
}

impl Ord for VersionedChanges {
    fn cmp(&self, other: &Self) -> Ordering {
        other.version.cmp(&self.version)
    }
}

impl LspDocumentState {
    /// Applies a list of content changes to the document state, updating the text and syntax tree incrementally.
    /// Returns the updated text and tree if the version is newer, otherwise an error.
    pub fn apply(
        &mut self,
        changes: Vec<TextDocumentContentChangeEvent>,
        version: i32
    ) -> Result<(String, Tree), String> {
        if version <= self.version {
            return Err(format!("Version {} not newer than {}", version, self.version));
        }
        let mut tree = parse_code(&self.text.to_string());
        for change in &changes {
            if let Some(range) = change.range {
                let start = position_to_byte_offset(&range.start, &self.text);
                let end = position_to_byte_offset(&range.end, &self.text);
                self.text.remove(start..end);
                self.text.insert(start, &change.text);
                tree = update_tree(&tree, &self.text.to_string(), start, end, change.text.len());
            } else {
                self.text = Rope::from_str(&change.text);
                tree = parse_code(&self.text.to_string());
            }
        }
        self.history.changes.push(VersionedChanges { version, changes });
        self.version = version;
        Ok((self.text.to_string(), tree))
    }
}

impl LspDocument {
    /// Returns the URI of the document.
    pub async fn uri(&self) -> Url {
        self.state.read().await.uri.clone()
    }

    /// Returns the current text of the document as a string.
    pub async fn text(&self) -> String {
        self.state.read().await.text.to_string()
    }

    /// Returns the current version of the document.
    pub async fn version(&self) -> i32 {
        self.state.read().await.version
    }

    /// Returns the number of lines in the document.
    pub async fn num_lines(&self) -> usize {
        self.state.read().await.text.len_lines()
    }

    /// Returns the index of the last line in the document.
    pub async fn last_line(&self) -> usize {
        self.num_lines().await - 1
    }

    /// Returns the number of characters in the specified line.
    pub async fn num_columns(&self, line: usize) -> usize {
        self.state.read().await.text.line(line).len_chars()
    }

    /// Returns the index of the last column in the specified line.
    pub async fn last_column(&self, line: usize) -> usize {
        let num_chars = self.state.read().await.text.line(line).len_chars();
        if num_chars > 0 { num_chars - 1 } else { 0 }
    }

    /// Returns the position (line, column) of the end of the document.
    pub async fn last_linecol(&self) -> (usize, usize) {
        let state = self.state.read().await;
        let text = &state.text;
        let last_line = text.len_lines() - 1;
        let num_chars = text.line(last_line).len_chars();
        let last_column = if num_chars > 0 { num_chars - 1 } else { 0 };
        (last_line, last_column)
    }

    /// Applies changes to the document, updating text and tree.
    pub async fn apply(
        &self,
        changes: Vec<TextDocumentContentChangeEvent>,
        version: i32
    ) -> Option<(String, Tree)> {
        let mut state = self.state.write().await;
        state.apply(changes, version).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::models::LspDocumentHistory;
    use std::sync::Arc;
    use tokio::sync::RwLock;
    use tower_lsp::lsp_types::{Range, TextDocumentContentChangeEvent};

    /// Helper to create a test LspDocument.
    fn create_test_document(uri: &str, text: &str) -> Arc<LspDocument> {
        Arc::new(LspDocument {
            id: 1,
            state: RwLock::new(LspDocumentState {
                uri: Url::parse(uri).unwrap(),
                text: Rope::from_str(text),
                version: 0,
                history: LspDocumentHistory {
                    text: text.to_string(),
                    changes: vec![],
                },
            }),
        })
    }

    #[tokio::test]
    async fn test_apply_full_change() {
        // Test replacing entire document text
        let doc = create_test_document("file:///test.rho", "initial text");
        let changes = vec![TextDocumentContentChangeEvent {
            range: None,
            range_length: None,
            text: "new text".to_string(),
        }];

        let result = doc.apply(changes, 1).await.map(|(text, _)| text);
        assert!(result.is_some(), "Apply should succeed");
        assert_eq!(result.unwrap(), "new text", "Text should be updated");
        assert_eq!(doc.version().await, 1, "Version should be updated");
    }

    #[tokio::test]
    async fn test_apply_incremental_change() {
        // Test replacing a portion of the document text
        let doc = create_test_document("file:///test.rho", "hello world");
        let changes = vec![TextDocumentContentChangeEvent {
            range: Some(Range {
                start: Position { line: 0, character: 6 },
                end: Position { line: 0, character: 11 },
            }),
            range_length: None,
            text: "there".to_string(),
        }];

        let result = doc.apply(changes, 1).await.map(|(text, _)| text);
        assert!(result.is_some(), "Apply should succeed");
        assert_eq!(result.unwrap(), "hello there", "Text should be updated");
        assert_eq!(doc.version().await, 1, "Version should be updated");
    }

    #[tokio::test]
    async fn test_apply_multiple_incremental() {
        // Test applying multiple incremental changes sequentially
        let doc = create_test_document("file:///test.rho", "hello world");
        let changes = vec![
            TextDocumentContentChangeEvent {
                range: Some(Range {
                    start: Position { line: 0, character: 6 },
                    end: Position { line: 0, character: 11 },
                }),
                range_length: None,
                text: "rust".to_string(),
            },
            TextDocumentContentChangeEvent {
                range: Some(Range {
                    start: Position { line: 0, character: 0 },
                    end: Position { line: 0, character: 5 },
                }),
                range_length: None,
                text: "hi".to_string(),
            },
        ];

        let result = doc.apply(changes, 1).await.map(|(text, _)| text);
        assert!(result.is_some(), "Apply should succeed");
        assert_eq!(result.unwrap(), "hi rust", "Text should be updated after multiple changes");
        assert_eq!(doc.version().await, 1, "Version should be updated");
    }

    #[tokio::test]
    async fn test_apply_outdated_version() {
        // Test applying changes with an outdated version (should fail)
        let doc = create_test_document("file:///test.rho", "initial text");
        let changes = vec![TextDocumentContentChangeEvent {
            range: None,
            range_length: None,
            text: "new text".to_string(),
        }];

        // Apply with version 1 (newer than current version 0), should succeed
        let _ = doc.apply(changes.clone(), 1).await;
        // Apply again with version -1 (outdated), should fail and not change text
        let result = doc.apply(changes, -1).await;
        assert!(result.is_none(), "Apply should fail for outdated version");
        assert_eq!(doc.text().await, "new text", "Text should remain from previous change");
        assert_eq!(doc.version().await, 1, "Version should not change");
    }
}
