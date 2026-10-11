#![cfg(unix)]
#[path = "extension_support.rs"]
mod support;
#[path = "x03_support.rs"]
mod x03_support;
use saya_agent::CancellationToken;
use saya_harness::coding::extensions::{ExtensionError, lsp::LspClient};

#[tokio::test]
async fn lsp_document_outside_pinned_root_is_refused_before_did_open() {
    let fixture = support::Fixture::new();
    let outside = support::Fixture::new();
    let path = outside.root.join("source.rs");
    std::fs::write(&path, "fn main() {}\n").unwrap();
    let original = std::fs::read_to_string(&path).unwrap();
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
    assert_eq!(std::fs::read_to_string(path).unwrap(), original);
    drop(client);
}

#[tokio::test]
async fn lsp_uses_only_current_document_diagnostics() {
    let fixture = support::Fixture::new();
    let document = fixture.root.join("source.rs");
    std::fs::write(&document, "fn main() {}\n").unwrap();
    let uri = url::Url::from_file_path(&document).unwrap().to_string();
    let stale_uri = url::Url::from_file_path(fixture.root.join("other.rs"))
        .unwrap()
        .to_string();
    let script = x03_support::after_open([
        x03_support::diagnostics(&stale_uri, 7, "wrong document"),
        x03_support::diagnostics(&uri, 6, "stale version"),
        x03_support::diagnostics(&uri, 7, "current diagnostic"),
        x03_support::response(2),
    ]);
    let token = CancellationToken::new();
    let mut client = LspClient::connect(fixture.config(&script), &token)
        .await
        .unwrap();

    let diagnostics = client
        .diagnostics(&document, "rust", 7, "fn main() {}\n", &token)
        .await
        .unwrap();

    assert_eq!(diagnostics.document_uri, uri);
    assert_eq!(diagnostics.version, 7);
    assert_eq!(diagnostics.diagnostics.len(), 1);
    assert_eq!(diagnostics.diagnostics[0].message, "current diagnostic");
    client.shutdown().await.unwrap();
}

#[tokio::test]
async fn lsp_shutdown_returns_a_joined_process_receipt() {
    let fixture = support::Fixture::new();
    let script = x03_support::after_initialize();
    let token = CancellationToken::new();
    let client = LspClient::connect(fixture.config(&script), &token)
        .await
        .unwrap();

    let receipt = client.shutdown_with_receipt().await.unwrap();

    assert!(receipt.joined);
    assert_ne!(receipt.pid, 0);
    assert!(receipt.exit_code.is_some() || receipt.signal.is_some());
    assert!(receipt.group_kill_requested);
}

#[tokio::test]
async fn lsp_cancelled_diagnostics_settle_the_started_process() {
    let fixture = support::Fixture::new();
    let document = fixture.root.join("source.rs");
    std::fs::write(&document, "fn main() {}\n").unwrap();
    let observer = std::sync::Arc::new(x03_support::ObservedProcesses::default());
    let script = x03_support::initialize_then_idle();
    let start = CancellationToken::new();
    let mut client = LspClient::connect(
        fixture.config(&script).with_observer(observer.clone()),
        &start,
    )
    .await
    .unwrap();
    let cancelled = CancellationToken::new();
    cancelled.cancel();

    assert!(matches!(
        client
            .diagnostics(&document, "rust", 1, "fn main() {}\n", &cancelled)
            .await,
        Err(ExtensionError::Cancelled)
    ));
    x03_support::assert_joined_and_reaped(&observer);
}

#[tokio::test]
async fn lsp_early_server_exit_is_typed_and_settled() {
    let fixture = support::Fixture::new();
    let observer = std::sync::Arc::new(x03_support::ObservedProcesses::default());
    let token = CancellationToken::new();

    assert!(matches!(
        LspClient::connect(
            fixture.config("exit 0").with_observer(observer.clone()),
            &token
        )
        .await,
        Err(ExtensionError::Process)
    ));
    x03_support::assert_joined_and_reaped(&observer);
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
