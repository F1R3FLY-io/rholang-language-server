use indoc::indoc;

pub mod common;
use crate::common::lsp_client::LspClient;

with_lsp_client!(test_diagnostics, (|client: &LspClient|{
    let doc = client.open_document("/path/to/source.rho", indoc! {r#"
        new x {
            x!("Hello World!")
        }
    "#}).unwrap();
    let diagnostic_params = client.await_diagnostics(&doc).unwrap();
    assert!(diagnostic_params.uri.to_string() == doc.uri());
    assert!(diagnostic_params.diagnostics.len() == 1);
    let diagnostic = &diagnostic_params.diagnostics[0];
    let range = &diagnostic.range;
    let start = &range.start;
    let start_line = start.line as usize;
    let start_column = start.character as usize;
    let end = &range.end;
    let end_line = end.line as usize;
    let end_column = end.character as usize;
    assert!(start_line == 0);
    assert!(start_column == 6);
    assert!(end_line == 0);
    assert!(end_column == 7);
    assert!(diagnostic.message == "Error: coop.rchain.rholang.interpreter.errors$SyntaxError: syntax error(): { at 1:7-1:8".to_string());
}));
