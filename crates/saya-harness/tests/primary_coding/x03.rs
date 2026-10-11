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
