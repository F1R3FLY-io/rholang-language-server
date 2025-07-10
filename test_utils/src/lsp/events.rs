/// Events related to LSP document lifecycle and changes.
#[derive(Debug)]
pub enum LspEvent {
    /// Emitted when a file is opened.
    FileOpened {
        document_id: u64,
        uri: String,
        text: String,
    },
    /// Emitted when a file is closed.
    FileClosed {
        document_id: u64,
        uri: String,
    },
    /// Emitted when text in a document changes.
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
    /// Emitted when the server is shut down.
    Shutdown,
    /// Emitted when the client exits.
    Exit,
}
