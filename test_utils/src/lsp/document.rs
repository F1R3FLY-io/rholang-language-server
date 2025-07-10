use std::sync::RwLock;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc::Sender;

use url::Url;

use ropey::Rope;

use tower_lsp::lsp_types::{Position, TextEdit};

use crate::lsp::events::LspEvent;

/// Represents an LSP-managed document, using Rope for efficient text manipulation.
/// Handles cursor position and emits events on changes.
#[allow(dead_code)]
pub struct LspDocument {
    pub id: u64,
    language_id: String,
    pub url: RwLock<Url>,
    text: RwLock<Rope>,
    pub version: AtomicI32,
    cursor: RwLock<Cursor>,
    event_sender: Sender<LspEvent>,
}

#[derive(Debug)]
struct Cursor {
    line: usize,
    column: usize,
}

#[allow(dead_code)]
impl LspDocument {
    /// Converts an LSP Position to a character offset in the Rope.
    fn lsp_range_to_offset(position: &Position, text: &Rope) -> usize {
        let line = position.line as usize;
        let column = position.character as usize;
        text.line_to_char(line) + column
    }

    /// Creates a new LspDocument from the given parameters.
    pub fn from_path_and_text(
        document_id: u64,
        language_id: String,
        path: String,
        text: String,
        event_sender: Sender<LspEvent>,
    ) -> Self {
        LspDocument {
            id: document_id,
            language_id,
            url: RwLock::new(Url::from_file_path(path.as_str()).expect("Invalid file path")),
            text: RwLock::new(Rope::from_str(&text)),
            version: AtomicI32::new(1),
            cursor: RwLock::new(Cursor { line: 0, column: 0 }),
            event_sender,
        }
    }

    /// Returns the document's URI as a string.
    pub fn uri(&self) -> String {
        self.url.read().expect("Failed to acquire read lock on url").to_string()
    }

    /// Increments and returns the new document version.
    fn bump_version(&self) -> i32 {
        self.version.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Emits an LSP event.
    fn emit(&self, event: LspEvent) -> Result<(), String> {
        self.event_sender
            .send(event)
            .map_err(|e| format!("Failed to emit event: {}", e))
    }

    /// Notifies the server that the document has been opened.
    pub fn open(&self) -> Result<(), String> {
        let full_text = self
            .text
            .read()
            .expect("Failed to acquire read lock on text")
            .to_string();
        self.emit(LspEvent::FileOpened {
            document_id: self.id,
            uri: self.uri(),
            text: full_text,
        })
    }

    /// Notifies the server that the document has been closed.
    pub fn close(&self) -> Result<(), String> {
        self.emit(LspEvent::FileClosed {
            document_id: self.id,
            uri: self.uri(),
        })
    }

    /// Returns the document's file path.
    pub fn path(&self) -> String {
        self.url.read().expect("Failed to acquire read lock on url").path().to_string()
    }

    /// Returns the current cursor position as a character offset.
    pub fn position(&self) -> usize {
        let cursor = self.cursor.read().expect("Failed to acquire read lock on cursor");
        let text = self.text.read().expect("Failed to acquire read lock on text");
        text.line_to_char(cursor.line) + cursor.column
    }

    /// Returns the current cursor position (1-indexed line and column).
    pub fn cursor(&self) -> (usize, usize) {
        // Translate index conventions from 0-index to 1-index:
        let cursor = self.cursor.read().expect("Failed to acquire read lock on cursor");
        (cursor.line + 1, cursor.column + 1)
    }

    /// Moves the cursor to the specified 1-indexed line and column.
    pub fn move_cursor(&self, line: usize, column: usize) {
        // Translate index conventions from 1-index to 0-index
        let mut cursor = self.cursor.write().expect("Failed to acquire write lock on cursor");
        cursor.line = line - 1;
        cursor.column = column - 1;
    }

    /// Inserts text at the current cursor position, updates the cursor, and emits a change event.
    pub fn insert_text(&self, text: String) -> Result<(), String> {
        let (from_line, from_column) = {
            let cursor = self.cursor.read().expect("Failed to acquire read lock on cursor");
            (cursor.line, cursor.column)
        };
        let to_line = from_line;
        let to_column = from_column;
        let mut position = self.position();
        {
            let mut self_text = self.text.write().expect("Failed to acquire write lock on text");
            self_text.insert(position, &text);
        }
        position += text.chars().count();
        {
            let mut cursor = self.cursor.write().expect("Failed to acquire write lock on cursor");
            let self_text = self.text.read().expect("Failed to acquire read lock on text");
            cursor.line = self_text.char_to_line(position);
            cursor.column = position - self_text.line_to_char(cursor.line);
        }
        let version = self.bump_version();
        self.emit(LspEvent::TextChanged {
            document_id: self.id,
            uri: self.uri(),
            version,
            from_line,
            from_column,
            to_line,
            to_column,
            text,
        })
    }

    /// Applies a list of text edits to the document.
    /// Edits are sorted in reverse order to avoid offset issues.
    pub fn apply(&self, mut edits: Vec<TextEdit>) {
        // Re-order the edits so they will be applied in reverse in case there
        // are more than one:
        edits.sort_by(|a, b| {
            let pos_a = a.range.start;
            let pos_b = b.range.start;
            pos_b.line.cmp(&pos_a.line).then(pos_b.character.cmp(&pos_a.character))
        });
        let mut text = self.text.write().expect("Failed to acquire write lock on text");
        for edit in edits {
            let range = edit.range;
            let start_position = &range.start;
            let start_line = start_position.line as usize;
            let start_column = start_position.character as usize;
            let end_position = &range.end;
            let end_line = end_position.line as usize;
            let end_column = end_position.character as usize;
            let start = text.line_to_char(start_line) + start_column;
            let end = text.line_to_char(end_line) + end_column;
            text.remove(start..end);
            text.insert(start, &edit.new_text);
        }
    }

    /// Returns the full text of the document.
    pub fn text(&self) -> Result<String, String> {
        let full_text = self
            .text
            .read()
            .expect("Failed to acquire read lock on text")
            .to_string();
        Ok(full_text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use tower_lsp::lsp_types::{Range, TextEdit};

    #[test]
    fn test_insert_text() {
        let (sender, _receiver) = mpsc::channel();
        let doc = LspDocument::from_path_and_text(1, "test".to_string(), "/test".to_string(), "hello".to_string(), sender);
        doc.move_cursor(1, 6); // After 'o'
        doc.insert_text(" world".to_string()).unwrap();
        assert_eq!(doc.text().unwrap(), "hello world");
        assert_eq!(doc.cursor(), (1, 12)); // Cursor moved after insert
    }

    #[test]
    fn test_apply_edits() {
        let (sender, _) = mpsc::channel();
        let doc = LspDocument::from_path_and_text(1, "test".to_string(), "/test".to_string(), "hello".to_string(), sender);
        let edit = TextEdit {
            range: Range {
                start: Position { line: 0, character: 5 },
                end: Position { line: 0, character: 5 },
            },
            new_text: " world".to_string(),
        };
        doc.apply(vec![edit]);
        assert_eq!(doc.text().unwrap(), "hello world");
    }

    #[test]
    fn test_apply_multiple_edits() {
        let (sender, _) = mpsc::channel();
        let doc = LspDocument::from_path_and_text(1, "test".to_string(), "/test".to_string(), "hello".to_string(), sender);
        let edits = vec![
            TextEdit {
                range: Range {
                    start: Position { line: 0, character: 0 },
                    end: Position { line: 0, character: 0 },
                },
                new_text: "say ".to_string(),
            },
            TextEdit {
                range: Range {
                    start: Position { line: 0, character: 5 },
                    end: Position { line: 0, character: 5 },
                },
                new_text: " world".to_string(),
            },
        ];
        doc.apply(edits);
        assert_eq!(doc.text().unwrap(), "say hello world");
    }
}
