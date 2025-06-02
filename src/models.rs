use ropey::Rope;
use tower_lsp::lsp_types::{TextDocumentContentChangeEvent, Url};
use std::cmp;

#[derive(Debug)]
pub struct VersionedChanges {
    pub version: i32,
    pub changes: Vec<TextDocumentContentChangeEvent>,
}

impl PartialEq for VersionedChanges {
    fn eq(&self, other: &Self) -> bool {
        self.version == other.version
    }
}

impl Eq for VersionedChanges {}

impl PartialOrd for VersionedChanges {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(other.version.cmp(&self.version))
    }
}

impl Ord for VersionedChanges {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        other.version.cmp(&self.version)
    }
}

#[derive(Debug)]
pub struct LspDocumentHistory {
    pub text: String,
    pub changes: Vec<VersionedChanges>,
}

#[derive(Debug)]
pub struct LspDocumentState {
    pub uri: Url,
    pub text: Rope,
    pub version: i32,
    pub history: LspDocumentHistory,
}

#[derive(Debug)]
pub struct LspDocument {
    pub id: u32,
    pub state: tokio::sync::RwLock<LspDocumentState>,
}
