#![cfg(target_os = "macos")]
#[path = "lsp_process_fixtures.rs"]
mod lsp_process_fixtures;
#[allow(clippy::duplicate_mod)] // intentional: this test group links its own private copy of the shared fixture (own statics)
#[path = "extension_support.rs"]
mod support;
use saya_agent::CancellationToken;
use saya_harness::coding::extensions::{
    ExtensionError, ExtensionProcessEvent, ExtensionProcessObserver, lsp::LspClient,
};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct FailingSettledObserver {
    spawned: Mutex<Option<u32>>,
    settled_attempts: Mutex<usize>,
}

impl ExtensionProcessObserver for FailingSettledObserver {
    fn record(&self, event: ExtensionProcessEvent) -> Result<(), ExtensionError> {
        match event {
            ExtensionProcessEvent::Spawned { pid } => *self.spawned.lock().unwrap() = Some(pid),
            ExtensionProcessEvent::Settled(_) => {
                *self.settled_attempts.lock().unwrap() += 1;
                return Err(ExtensionError::Observer);
            }
            ExtensionProcessEvent::Uncertain { .. } => {}
        }
        Ok(())
    }
}

#[tokio::test]
async fn failed_diagnostic_exchange_does_not_hide_mandatory_observer_failure() {
    let _slot = lsp_process_fixtures::lifecycle_test_slot().await;
    let fixture = support::Fixture::new();
    let document = fixture.root.join("source.rs");
    std::fs::write(&document, "fn main() {}\n").unwrap();
    let observer = Arc::new(FailingSettledObserver::default());
    // The response id is a real protocol exchange failure after `didOpen`.
    // The failing observer then rejects the mandatory settled receipt.
    let script = lsp_process_fixtures::after_open([lsp_process_fixtures::response(2)]);
    let token = CancellationToken::new();
    let mut client = LspClient::connect(
        fixture.config(&script).with_observer(observer.clone()),
        &token,
    )
    .await
    .unwrap();

    let exchange = client
        .diagnostics(&document, "rust", 1, "fn main() {}\n", &token)
        .await
        .expect_err("the malformed diagnostics exchange is the first failure");
    let cleanup = client
        .shutdown()
        .await
        .expect_err("a failed settled observer remains fatal after the exchange");
    let pid = observer
        .spawned
        .lock()
        .unwrap()
        .expect("a real server spawned");

    assert!(matches!(exchange, ExtensionError::Protocol));
    assert!(matches!(cleanup, ExtensionError::Observer));
    assert_eq!(*observer.settled_attempts.lock().unwrap(), 1);
    assert_eq!(unsafe { libc::kill(pid as libc::pid_t, 0) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
}

#[tokio::test]
async fn initialize_exchange_and_observer_failure_return_the_safe_composite() {
    let _slot = lsp_process_fixtures::lifecycle_test_slot().await;
    let fixture = support::Fixture::new();
    let observer = Arc::new(FailingSettledObserver::default());
    let token = CancellationToken::new();
    assert!(matches!(
        LspClient::connect(
            fixture
                .config(&lsp_process_fixtures::initialize_with_response(
                    lsp_process_fixtures::response(9),
                ))
                .with_observer(observer.clone()),
            &token,
        )
        .await,
        Err(ExtensionError::ExchangeAndObserver)
    ));
    assert_eq!(*observer.settled_attempts.lock().unwrap(), 1);
}

#[tokio::test]
async fn shutdown_protocol_exchange_precedes_a_sticky_mandatory_observer_failure() {
    let _slot = lsp_process_fixtures::lifecycle_test_slot().await;
    let fixture = support::Fixture::new();
    let document = fixture.root.join("source.rs");
    std::fs::write(&document, "fn main() {}\n").unwrap();
    let observer = Arc::new(FailingSettledObserver::default());
    let uri = url::Url::from_file_path(&document).unwrap().to_string();
    let script = lsp_process_fixtures::after_open_with_shutdown(
        [lsp_process_fixtures::diagnostics(&uri, 1, "current")],
        lsp_process_fixtures::response(9),
    );
    let token = CancellationToken::new();
    let mut client = LspClient::connect(
        fixture.config(&script).with_observer(observer.clone()),
        &token,
    )
    .await
    .unwrap();

    client
        .diagnostics(&document, "rust", 1, "fn main() {}\n", &token)
        .await
        .expect("the diagnostic response is complete before shutdown");
    let (shutdown, settlement) = client.shutdown_parts().await;
    assert!(matches!(shutdown, Err(ExtensionError::Protocol)));
    assert!(matches!(settlement, Err(ExtensionError::Observer)));
    let (retry_shutdown, retry_settlement) = client.shutdown_parts().await;
    assert!(retry_shutdown.is_ok());
    assert!(matches!(retry_settlement, Err(ExtensionError::Observer)));
    let pid = observer.spawned.lock().unwrap().expect("server spawned");
    assert_eq!(*observer.settled_attempts.lock().unwrap(), 1);
    assert_eq!(unsafe { libc::kill(pid as libc::pid_t, 0) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
}

#[tokio::test]
async fn lsp_document_outside_pinned_root_is_refused_before_did_open() {
    let _slot = lsp_process_fixtures::lifecycle_test_slot().await;
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
    let _slot = lsp_process_fixtures::lifecycle_test_slot().await;
    let fixture = support::Fixture::new();
    let document = fixture.root.join("source.rs");
    std::fs::write(&document, "fn main() {}\n").unwrap();
    let uri = url::Url::from_file_path(&document).unwrap().to_string();
    let stale_uri = url::Url::from_file_path(fixture.root.join("other.rs"))
        .unwrap()
        .to_string();
    let script = lsp_process_fixtures::after_open([
        lsp_process_fixtures::diagnostics(&stale_uri, 7, "wrong document"),
        lsp_process_fixtures::diagnostics(&uri, 6, "stale version"),
        lsp_process_fixtures::diagnostics(&uri, 7, "current diagnostic"),
        lsp_process_fixtures::response(2),
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
    let _slot = lsp_process_fixtures::lifecycle_test_slot().await;
    let fixture = support::Fixture::new();
    let script = lsp_process_fixtures::after_initialize();
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
    let _slot = lsp_process_fixtures::lifecycle_test_slot().await;
    let fixture = support::Fixture::new();
    let document = fixture.root.join("source.rs");
    std::fs::write(&document, "fn main() {}\n").unwrap();
    let observer = std::sync::Arc::new(lsp_process_fixtures::ObservedProcesses::default());
    let script = lsp_process_fixtures::initialize_then_idle();
    let start = CancellationToken::new();
    let mut client = LspClient::connect(
        fixture.config(&script).with_observer(observer.clone()),
        &start,
    )
    .await
    .unwrap();
    let cancelled = CancellationToken::new();
    let trigger = cancelled.clone();
    let cancellation = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        trigger.cancel();
    });

    assert!(matches!(
        client
            .diagnostics(&document, "rust", 1, "fn main() {}\n", &cancelled)
            .await,
        Err(ExtensionError::Cancelled)
    ));
    cancellation.await.unwrap();
    lsp_process_fixtures::assert_joined_and_reaped(&observer);
}

#[tokio::test]
async fn lsp_early_server_exit_is_typed_and_settled() {
    let _slot = lsp_process_fixtures::lifecycle_test_slot().await;
    let fixture = support::Fixture::new();
    let observer = std::sync::Arc::new(lsp_process_fixtures::ObservedProcesses::default());
    let token = CancellationToken::new();

    assert!(matches!(
        LspClient::connect(
            fixture.config("exit 0").with_observer(observer.clone()),
            &token
        )
        .await,
        Err(ExtensionError::Process)
    ));
    lsp_process_fixtures::assert_joined_and_reaped(&observer);
}

#[tokio::test]
async fn lsp_server_proposed_outside_edit_is_refused_without_touching_file() {
    let _slot = lsp_process_fixtures::lifecycle_test_slot().await;
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
    let observer = std::sync::Arc::new(lsp_process_fixtures::ObservedProcesses::default());
    let token = CancellationToken::new();
    let mut client = LspClient::connect(
        fixture.config(&script).with_observer(observer.clone()),
        &token,
    )
    .await
    .unwrap();
    assert!(matches!(
        client
            .diagnostics(&document, "rust", 1, "fn main() {}", &token)
            .await,
        Err(ExtensionError::Protocol)
    ));
    assert_eq!(std::fs::read_to_string(original).unwrap(), "user content");
    lsp_process_fixtures::assert_joined_and_reaped(&observer);
}
