use super::transports::*;
use super::*;

/// A download admitted on an already-cancelled turn refuses before anything
/// happens: no transport call, no part file, no sidecar, no destination.
#[tokio::test]
async fn a_precancelled_download_writes_nothing() {
    let content: Vec<u8> = (0..40u8).collect();
    let (root, workspace) = workspace("precancel");
    let budget = DownloadBudget::default();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let ranges = Arc::new(Mutex::new(Vec::new()));
    let net = RangedNet {
        content: Arc::new(content),
        chunk: 8,
        calls: Arc::clone(&calls),
        ranges: Arc::clone(&ranges),
    };
    let token = CancellationToken::new();
    token.cancel();
    let policy = policy();
    let error = tokio::time::timeout(
        ABORT,
        http_download_cancellable(DownloadCall {
            policy: &policy,
            transport: &net,
            workspace: &workspace,
            destination: DEST,
            expected_destination_sha256: None,
            url: &url("/doc"),
            limits: DownloadLimits::default(),
            budget: &budget,
            cancellation: &token,
        }),
    )
    .await
    .expect("a pre-cancelled download must not hang")
    .expect_err("a pre-cancelled download refuses");
    assert!(matches!(error, DownloadError::Cancelled), "{error:?}");
    assert!(
        calls.lock().expect("calls").is_empty(),
        "a pre-cancelled download makes no transport call: {:?}",
        calls.lock().expect("calls")
    );
    assert!(
        !root.join(DEST).exists() && !root.join(PART).exists() && !root.join(SIDECAR).exists(),
        "nothing was written: no part file, no sidecar, no destination"
    );
}

/// The download is inside `get`, waiting for the response head, when the
/// turn cancels: the wait aborts typed, the transport saw the one call and
/// nothing after, and the session never opened — nothing was written.
#[tokio::test]
async fn cancelling_a_header_wait_writes_nothing() {
    let (root, workspace) = workspace("header-wait");
    let entered = Arc::new(Notify::new());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let net = HeldGetNet {
        entered: Arc::clone(&entered),
        calls: Arc::clone(&calls),
    };
    let budget = DownloadBudget::default();
    let token = CancellationToken::new();
    let task = spawn_download(net, workspace, budget, token.clone());
    entered.notified().await;
    token.cancel();
    let error = tokio::time::timeout(ABORT, task)
        .await
        .expect("a cancelled header wait aborts instead of hanging")
        .expect("the download task joins")
        .expect_err("cancellation is an error, never a result");
    assert!(matches!(error, DownloadError::Cancelled), "{error:?}");
    assert_eq!(
        calls.lock().expect("calls").clone(),
        vec![url("/doc")],
        "the transport was entered once and never again"
    );
    assert!(
        !root.join(DEST).exists() && !root.join(PART).exists() && !root.join(SIDECAR).exists(),
        "a cancelled header wait wrote nothing: the session never opened"
    );
}
