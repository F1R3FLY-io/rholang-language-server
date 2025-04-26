use std::sync::{Arc, Mutex, RwLock};
use std::sync::atomic::{AtomicI32, Ordering};

use url::Url;

use ropey::Rope;

use tower_lsp::lsp_types::{
    Position,
    TextEdit,
};

#[derive(Debug)]
pub enum LspDocumentEvent {
    FileOpened {
        document_id: u64,
        uri: String,
        text: String,
    },
    TextChanged {
        document_id: u64,
        uri: String,
        version: i32,
        from_line: usize,
        from_column: usize,
        to_line: usize,
        to_column: usize,
        text: String,
    },
}

pub trait LspDocumentEventHandler {
    fn handle_lsp_document_event(&self, event: &LspDocumentEvent);
}

pub struct LspDocumentEventManager {
    handler: Arc<dyn LspDocumentEventHandler>,
}

impl LspDocumentEventManager {
    pub fn new(handler: Arc<dyn LspDocumentEventHandler>) -> Arc<Self> {
        Arc::new(LspDocumentEventManager { handler })
    }

    pub fn emit_lsp_document_event(&self, event: LspDocumentEvent) {
        self.handler.handle_lsp_document_event(&event);
    }
}

#[derive(Debug)]
struct Cursor {
    line: usize,
    column: usize,
}

#[allow(dead_code)]
pub struct LspDocument {
    pub id: u64,
    language_id: String,
    url: RwLock<Url>,
    text: RwLock<Rope>,
    pub version: AtomicI32,
    cursor: RwLock<Cursor>,
    event_manager: Mutex<Option<Arc<LspDocumentEventManager>>>,
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
            event_manager: Mutex::new(None),
        }
    }

    pub fn set_event_manager(&self, event_manager: Arc<LspDocumentEventManager>) {
        let mut lock = self.event_manager.lock().unwrap();
        *lock = Some(event_manager);
    }

    pub fn uri(&self) -> String {
        let url = self.url.read().unwrap();
        url.to_string()
    }

    fn bump_version(&self) -> i32 {
        self.version.fetch_add(1, Ordering::SeqCst)
    }

    pub fn open(&self) -> Result<(), String> {
        if let Some(event_manager) = &*self.event_manager.lock().unwrap() {
            let full_text = self.text.read().unwrap().to_string();
            event_manager.emit_lsp_document_event(LspDocumentEvent::FileOpened{
                document_id: self.id,
                uri: self.uri(),
                text: full_text,
            });
            Ok(())
        } else {
            Err("event_manager has not been set!".to_string())
        }
    }

    pub fn path(&self) -> String {
        let url = self.url.read().unwrap();
        url.path().to_string()
    }

    pub fn position(&self) -> usize {
        let cursor = self.cursor.read().unwrap();
        let text = self.text.read().unwrap();
        text.line_to_char(cursor.line) + cursor.column
    }

    pub fn cursor(&self) -> (usize, usize) {
        // Translate index conventions from 0-index to 1-index:
        let cursor = self.cursor.read().unwrap();
        (cursor.line + 1, cursor.column + 1)
    }

    pub fn move_cursor(&self, line: usize, column: usize) {
        // Translate index conventions from 1-index to 0-index
        let mut cursor = self.cursor.write().unwrap();
        cursor.line = line - 1;
        cursor.column = column - 1;
    }

    pub fn insert_text(&self, text: String) -> Result<(), String> {
        let (from_line, from_column) = {
            let cursor = self.cursor.read().unwrap();
            (cursor.line, cursor.column)
        };
        let to_line = from_line;
        let to_column = from_column;
        let mut position = self.position();
        {
            let mut self_text = self.text.write().unwrap();
            self_text.insert(position, &text);
        }
        position += text.chars().count();
        {
            let mut cursor = self.cursor.write().unwrap();
            let self_text = self.text.read().unwrap();
            cursor.line = self_text.char_to_line(position);
            cursor.column = position - self_text.line_to_char(cursor.line);
        }
        let version = self.bump_version();
        if let Some(event_manager) = &*self.event_manager.lock().unwrap() {
            event_manager.emit_lsp_document_event(LspDocumentEvent::TextChanged {
                document_id: self.id,
                uri: self.uri(),
                version,
                from_line,
                from_column,
                to_line,
                to_column,
                text,
            });
            Ok(())
        } else {
            Err("event_manager has not been set!".to_string())
        }
    }

    pub fn apply(&self, mut edits: Vec<TextEdit>) -> () {
        // Re-order the edits so they will be applied in reverse in case there
        // are more than one:
        edits.sort_by(|a, b| {
            let pos_a = a.range.start;
            let pos_b = b.range.start;
            pos_b.line.cmp(&pos_a.line).then(pos_b.character.cmp(&pos_a.character))
        });
        let mut text = self.text.write().unwrap();
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
}
