use std::sync::RwLock;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc::Sender;

use url::Url;

use ropey::Rope;

use tower_lsp::lsp_types::{
    Position,
    TextEdit,
};

use crate::lsp_event::LspEvent;

#[derive(Debug)]
struct Cursor {
    line: usize,
    column: usize,
}

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

#[allow(dead_code)]
impl LspDocument {
    fn lsp_range_to_offset(position: &Position, text: &Rope) -> usize {
        let line = position.line as usize;
        let column = position.character as usize;
        text.line_to_char(line) + column
    }

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
            cursor: RwLock::new(Cursor {
                line: 0,
                column: 0,
            }),
            event_sender,
        }
    }

    pub fn uri(&self) -> String {
        let url = self.url.read()
            .expect("Failed to acquire read lock on url");
        url.to_string()
    }

    fn bump_version(&self) -> i32 {
        self.version.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn emit(&self, event: LspEvent) -> Result<(), String> {
        self.event_sender.send(event)
            .map_err(|e| format!("Failed to emit event: {}", e))?;
        Ok(())
    }

    pub fn open(&self) -> Result<(), String> {
        let full_text = self.text.read()
            .expect("Failed to acquire read lock on text")
            .to_string();
        self.emit(LspEvent::FileOpened {
            document_id: self.id,
            uri: self.uri(),
            text: full_text,
        })
    }

    pub fn close(&self) -> Result<(), String> {
        self.emit(LspEvent::FileClosed {
            document_id: self.id,
            uri: self.uri(),
        })
    }

    pub fn path(&self) -> String {
        let url = self.url.read()
            .expect("Failed to acquire read lock on url");
        url.path().to_string()
    }

    pub fn position(&self) -> usize {
        let cursor = self.cursor.read()
            .expect("Failed to acquire read lock on cursor");
        let text = self.text.read()
            .expect("Failed to acquire read lock on text");
        text.line_to_char(cursor.line) + cursor.column
    }

    pub fn cursor(&self) -> (usize, usize) {
        // Translate index conventions from 0-index to 1-index:
        let cursor = self.cursor.read()
            .expect("Failed to acquire read lock on cursor");
        (cursor.line + 1, cursor.column + 1)
    }

    pub fn move_cursor(&self, line: usize, column: usize) {
        // Translate index conventions from 1-index to 0-index
        let mut cursor = self.cursor.write()
            .expect("Failed to acquire write lock on cursor");
        cursor.line = line - 1;
        cursor.column = column - 1;
    }

    /// Inserts text at the current cursor position and notifies the server.
    /// The document version is incremented, and the new version is used in the event.
    pub fn insert_text(&self, text: String) -> Result<(), String> {
        let (from_line, from_column) = {
            let cursor = self.cursor.read()
                .expect("Failed to acquire read lock on cursor");
            (cursor.line, cursor.column)
        };
        let to_line = from_line;
        let to_column = from_column;
        let mut position = self.position();
        {
            let mut self_text = self.text.write()
                .expect("Failed to acquire write lock on text");
            self_text.insert(position, &text);
        }
        position += text.chars().count();
        {
            let mut cursor = self.cursor.write()
                .expect("Failed to acquire write lock on cursor");
            let self_text = self.text.read()
                .expect("Failed to acquire read lock on text");
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

    pub fn apply(&self, mut edits: Vec<TextEdit>) -> () {
        // Re-order the edits so they will be applied in reverse in case there
        // are more than one:
        edits.sort_by(|a, b| {
            let pos_a = a.range.start;
            let pos_b = b.range.start;
            pos_b.line.cmp(&pos_a.line).then(pos_b.character.cmp(&pos_a.character))
        });
        let mut text = self.text.write()
            .expect("Failed to acquire write lock on text");
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

    pub fn text(&self) -> Result<String, String> {
        let full_text = self.text.read()
            .expect("Failed to acquire read lock on text")
            .to_string();
        Ok(full_text)
    }
}
