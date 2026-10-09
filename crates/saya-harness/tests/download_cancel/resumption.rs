use super::transports::*;
use super::*;

/// Two 8-byte chunks are served and fully written and claimed; the third
/// wait is held and the turn cancels there. The partial on disk holds
/// exactly the served bytes, the sidecar records exactly that length and
/// digest, the destination is untouched, and the budget holds the served
/// count — then the retry (same wallet, a cooperative ranged fake) completes
/// to the whole-file digest, with every byte claimed exactly once.
#[tokio::test]
async fn cancelling_mid_body_leaves_an_exact_resumable_partial_and_a_retry_completes() {
    let content: Vec<u8> = (0..40u8).collect();
    let (root, workspace) = workspace("mid-body");
    let budget = DownloadBudget::default();
    let served = 16usize;

    let held = Arc::new(Notify::new());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let net = StalledNet {
        content: Arc::new(content.clone()),
        chunk: 8,
        stall_after: 2,
        held: Arc::clone(&held),
        calls: Arc::clone(&calls),
    };
    let token = CancellationToken::new();
    let task = spawn_download(net, workspace.clone(), budget.clone(), token.clone());
    held.notified().await;
    token.cancel();
    let error = tokio::time::timeout(ABORT, task)
        .await
        .expect("a cancelled body wait aborts instead of hanging")
        .expect("the download task joins")
        .expect_err("cancellation is an error, never a result");
    assert!(matches!(error, DownloadError::Cancelled), "{error:?}");
    assert_eq!(
        std::fs::read(root.join(PART)).expect("the partial exists"),
        content[..served],
        "exactly the served bytes, nothing more"
    );
    let sidecar = sidecar_json(&root);
    assert_eq!(
        sidecar["len"], served as u64,
        "the sidecar records exactly the served length: {sidecar}"
    );
    assert_eq!(
        sidecar["sha256"],
        digest(&content[..served]),
        "the sidecar digests exactly the served bytes"
    );
    assert!(
        !root.join(DEST).exists(),
        "the destination is never touched by a cancelled download"
    );
    assert_eq!(
        budget.consumed(),
        served as u64,
        "each served byte was claimed from the wallet exactly once"
    );

    let retry_calls = Arc::new(Mutex::new(Vec::new()));
    let ranges = Arc::new(Mutex::new(Vec::new()));
    let retry = RangedNet {
        content: Arc::new(content.clone()),
        chunk: 8,
        calls: Arc::clone(&retry_calls),
        ranges: Arc::clone(&ranges),
    };
    let outcome = tokio::time::timeout(
        ABORT,
        http_download(
            &policy(),
            &retry,
            &workspace,
            DEST,
            &url("/doc"),
            DownloadLimits::default(),
            &budget,
        ),
    )
    .await
    .expect("the retry must not hang")
    .expect("the retry completes");
    assert_eq!(outcome.bytes, content.len() as u64);
    assert_eq!(outcome.sha256, digest(&content));
    assert_eq!(
        std::fs::read(root.join(DEST)).expect("the destination"),
        content,
        "the retry completed the whole file"
    );
    assert!(!root.join(SIDECAR).exists(), "the sidecar is cleared");
    assert_eq!(
        budget.consumed(),
        content.len() as u64,
        "each byte was claimed exactly once across both legs"
    );
    assert_eq!(
        ranges.lock().expect("ranges").clone(),
        vec![Some(served as u64)],
        "the retry asked for exactly the recorded offset"
    );
}
