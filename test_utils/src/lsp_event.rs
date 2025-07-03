#[derive(Debug)]
pub enum LspEvent {
    FileOpened {
        document_id: u64,
        uri: String,
        text: String,
    },
    FileClosed {
        document_id: u64,
        uri: String,
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
    Shutdown,
    Exit,
}
