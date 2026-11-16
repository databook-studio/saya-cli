use super::*;

// --- test 1: the budget trip pauses fail-safe, the partial is resumable -----

/// The run's download budget trips before the byte that would overrun it is
/// written: a typed pause, the destination still absent, and the partial on
/// disk — with its sidecar recording exactly the bytes it holds — resumes to
/// completion under a fresh wallet.
#[tokio::test]
async fn the_run_budget_trips_typed_and_the_partial_left_resumes() {
    let content = vec![b'a'; 40];
    let (root, workspace) = workspace("budget");
    let site = LocalSite::spawn(vec![("/doc", route(&content, 4))]).await;
    let net = transport(&site);
    let budget = DownloadBudget::new(10);

    let error = http_download(
        &policy(),
        &net,
        &workspace,
        DEST,
        &url("/doc"),
        DownloadLimits::default(),
        &budget,
    )
    .await
    .expect_err("the budget of 10 trips on the third 4-byte chunk");
    assert!(
        matches!(
            error,
            DownloadError::BudgetExhausted {
                limit: 10,
                received: 8
            }
        ),
        "typed pause, not overrun: {error:?}"
    );
    assert!(
        !root.join(DEST).exists(),
        "nothing landed at the destination"
    );
    let partial = std::fs::read(root.join(PART)).expect("the partial exists");
    assert_eq!(partial, content[..8], "exactly the bytes under the budget");
    let sidecar = sidecar_json(&root);
    assert_eq!(sidecar["len"], 8);
    assert_eq!(sidecar["sha256"], digest(&content[..8]));

    // The state a resume can use: a fresh wallet finishes the file.
    let outcome = http_download(
        &policy(),
        &net,
        &workspace,
        DEST,
        &url("/doc"),
        DownloadLimits::default(),
        &DownloadBudget::default(),
    )
    .await
    .expect("the resume completes");
    assert_eq!(outcome.bytes, 40);
    assert_eq!(outcome.sha256, digest(&content));
    assert_eq!(std::fs::read(root.join(DEST)).expect("dest"), content);
    assert!(!root.join(SIDECAR).exists(), "the sidecar is cleared");
}

// --- test 6: the per-file and per-run byte bounds each trip as typed --------

/// The per-file bound trips as a typed error before the overrunning byte is
/// written; the partial below the bound is left in place.
#[tokio::test]
async fn the_file_bound_trips_as_a_typed_error() {
    let content = vec![b'x'; 40];
    let (root, workspace) = workspace("file-bound");
    let site = LocalSite::spawn(vec![("/doc", route(&content, 4))]).await;
    let net = transport(&site);
    let error = http_download(
        &policy(),
        &net,
        &workspace,
        DEST,
        &url("/doc"),
        DownloadLimits {
            max_file_bytes: 8,
            ..DownloadLimits::default()
        },
        &DownloadBudget::default(),
    )
    .await
    .expect_err("over the file bound");
    assert!(
        matches!(error, DownloadError::FileTooLarge { limit: 8 }),
        "{error:?}"
    );
    assert_eq!(
        std::fs::read(root.join(PART)).expect("the partial exists"),
        content[..8],
        "no overrun: exactly the bytes under the bound"
    );
}

/// The per-run budget trips as a typed error on its own, with a generous
/// file bound — the two bounds are independent.
#[tokio::test]
async fn the_run_bound_trips_as_a_typed_error() {
    let content = vec![b'b'; 40];
    let (_root, workspace) = workspace("run-bound");
    let site = LocalSite::spawn(vec![("/doc", route(&content, 4))]).await;
    let net = transport(&site);
    let error = http_download(
        &policy(),
        &net,
        &workspace,
        DEST,
        &url("/doc"),
        DownloadLimits::default(),
        &DownloadBudget::new(8),
    )
    .await
    .expect_err("over the run bound");
    assert!(
        matches!(
            error,
            DownloadError::BudgetExhausted {
                limit: 8,
                received: 8
            }
        ),
        "{error:?}"
    );
}
