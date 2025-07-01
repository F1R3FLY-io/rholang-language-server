use ropey::Rope;
use tower_lsp::lsp_types::{Position, TextDocumentContentChangeEvent, Url};
use tracing::error;

pub use crate::models::{LspDocument, LspDocumentState, VersionedChanges};

pub fn lsp_range_to_offset(position: &Position, text: &Rope) -> usize {
    let line = position.line as usize;
    let char = position.character as usize;
    text.line_to_char(line) + char
}

impl LspDocumentState {
    pub fn apply(
        &mut self,
        changes: Vec<TextDocumentContentChangeEvent>,
        version: i32,
    ) -> Result<(), String> {
        if version > self.version {
            for change in &changes {
                if let Some(range) = change.range {
                    let start = lsp_range_to_offset(&range.start, &self.text);
                    let end = lsp_range_to_offset(&range.end, &self.text);
                    self.text.remove(start..end);
                    self.text.insert(start, &change.text);
                } else {
                    self.text = Rope::from_str(&change.text);
                }
            }
            self.history.changes.push(VersionedChanges { version, changes });
        } else {
            self.text = Rope::from_str(self.history.text.as_str());
            let mut iter = self.history.changes.iter();
            let mut pivot = 0;
            for pending in iter.by_ref().take_while(|pending| pending.version < version) {
                for change in &pending.changes {
                    if let Some(range) = change.range {
                        let start = lsp_range_to_offset(&range.start, &self.text);
                        let end = lsp_range_to_offset(&range.end, &self.text);
                        self.text.remove(start..end);
                        self.text.insert(start, &change.text);
                    } else {
                        self.text = Rope::from_str(&change.text);
                    }
                }
                pivot += 1;
            }
            for change in &changes {
                if let Some(range) = change.range {
                    let start = lsp_range_to_offset(&range.start, &self.text);
                    let end = lsp_range_to_offset(&range.end, &self.text);
                    self.text.remove(start..end);
                    self.text.insert(start, &change.text);
                } else {
                    self.text = Rope::from_str(&change.text);
                }
            }
            for pending in iter {
                for change in &pending.changes {
                    if let Some(range) = change.range {
                        let start = lsp_range_to_offset(&range.start, &self.text);
                        let end = lsp_range_to_offset(&range.end, &self.text);
                        self.text.remove(start..end);
                        self.text.insert(start, &change.text);
                    } else {
                        self.text = Rope::from_str(&change.text);
                    }
                }
            }
            self.history.changes.insert(pivot, VersionedChanges { version, changes });
        }
        self.version = self.history.changes.last().expect("Failed to store change").version;
        Ok(())
    }
}

impl LspDocument {
    pub async fn uri(&self) -> Url {
        self.state.read().await.uri.clone()
    }

    pub async fn text(&self) -> String {
        self.state.read().await.text.to_string()
    }

    pub async fn version(&self) -> i32 {
        self.state.read().await.version
    }

    pub async fn num_lines(&self) -> usize {
        self.state.read().await.text.len_lines()
    }

    pub async fn last_line(&self) -> usize {
        self.num_lines().await - 1
    }

    pub async fn num_columns(&self, line: usize) -> usize {
        self.state.read().await.text.line(line).len_chars()
    }

    pub async fn last_column(&self, line: usize) -> usize {
        self.state.read().await.text.line(line).len_chars() - 1
    }

    pub async fn last_linecol(&self) -> (usize, usize) {
        let state = self.state.read().await;
        let text = &state.text;
        let last_line = text.len_lines() - 1;
        let last_column = text.line(last_line).len_chars() - 1;
        (last_line, last_column)
    }

    pub async fn apply(
        &self,
        changes: Vec<TextDocumentContentChangeEvent>,
        version: i32,
    ) -> Option<String> {
        let mut state = self.state.write().await;
        match state.apply(changes, version) {
            Ok(_) => Some(state.text.to_string()),
            Err(message) => {
                error!("Failed to apply changes: {}", message);
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::LspDocumentHistory;
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

        let result = doc.apply(changes, 1).await;
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

        let result = doc.apply(changes, 1).await;
        assert!(result.is_some(), "Apply should succeed");
        assert_eq!(result.unwrap(), "hello there", "Text should be updated");
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

        // Apply with version 0 (current version), should succeed
        let _ = doc.apply(changes.clone(), 1).await;
        // Apply again with version -1 (outdated), should do nothing
        let result = doc.apply(changes, -1).await;
        assert!(result.is_some(), "Apply should succeed but not change text");
        assert_eq!(doc.text().await, "new text", "Text should remain from previous change");
        assert_eq!(doc.version().await, 1, "Version should not revert");
    }
}
