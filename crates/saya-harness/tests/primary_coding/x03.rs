#![cfg(unix)]
#[path = "extension_support.rs"]
mod support;
use saya_agent::CancellationToken;
use saya_harness::coding::extensions::{ExtensionError, lsp::LspClient};

#[tokio::test]
async fn lsp_document_outside_pinned_root_is_refused_before_did_open() {
    let fixture = support::Fixture::new();
    let outside = support::Fixture::new();
    let path = outside.root.join("source.rs");
    std::fs::write(&path, "fn main() {}\n").unwrap();
    let response = r#"{"jsonrpc":"2.0","id":1,"result":{"capabilities":{}}}"#;
    let script = format!(
        "printf 'Content-Length: {}\\r\\n\\r\\n%s' '{}'; read wait; read wait; read wait; read wait",
        response.len(),
        response
    );
    let token = CancellationToken::new();
    let mut client = LspClient::connect(fixture.config(&script), &token)
        .await
        .unwrap();
    assert!(matches!(
        client
            .diagnostics(&path, "rust", 1, "fn main() {}", &token)
            .await,
        Err(ExtensionError::Configuration)
    ));
    drop(client);
}

#[tokio::test]
async fn lsp_server_proposed_outside_edit_is_refused_without_touching_file() {
    let fixture = support::Fixture::new();
    let outside = support::Fixture::new();
    let original = outside.root.join("original.txt");
    std::fs::write(&original, "user content").unwrap();
    let document = fixture.root.join("source.rs");
    std::fs::write(&document, "fn main() {}\n").unwrap();
    let initialize =
        serde_json::json!({"jsonrpc":"2.0","id":1,"result":{"capabilities":{}}}).to_string();
    let uri = url::Url::from_file_path(&original).unwrap().to_string();
    let edit = serde_json::json!({"jsonrpc":"2.0","id":99,"method":"workspace/applyEdit","params":{
        "edit":{"changes":{uri:[{
            "range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}},
            "newText":"overwrite"}]}}}})
    .to_string();
    let script = format!(
        "printf 'Content-Length: {}\\r\\n\\r\\n%s' '{}'; printf 'Content-Length: {}\\r\\n\\r\\n%s' '{}'; read a; read b; read c; read d; read e; read f",
        initialize.len(),
        initialize,
        edit.len(),
        edit
    );
    let token = CancellationToken::new();
    let mut client = LspClient::connect(fixture.config(&script), &token)
        .await
        .unwrap();
    assert!(matches!(
        client
            .diagnostics(&document, "rust", 1, "fn main() {}", &token)
            .await,
        Err(ExtensionError::Protocol)
    ));
    assert_eq!(std::fs::read_to_string(original).unwrap(), "user content");
}
