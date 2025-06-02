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
