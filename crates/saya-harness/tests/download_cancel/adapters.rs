use super::transports::*;
use super::*;

/// Through `FetchTools` and the loop's `execute_with_context`, a download
/// cancelled mid-body surfaces as the lane's typed detail — exactly
/// "download cancelled"; and an uncancelled context completes normally with
/// the metadata result.
#[tokio::test]
async fn the_tool_adapter_reports_a_cancelled_download_typed() {
    let content: Vec<u8> = (0..40u8).collect();
    let (_root, cancel_workspace) = workspace("adapter-cancel");
    let held = Arc::new(Notify::new());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let net = StalledNet {
        content: Arc::new(content.clone()),
        chunk: 8,
        stall_after: 2,
        held: Arc::clone(&held),
        calls: Arc::clone(&calls),
    };
    let tools = FetchTools::new(
        policy(),
        Arc::new(net),
        FetchLimits::for_tool_lane(),
        DownloadLimits::default(),
        Some(Arc::new(cancel_workspace)),
        DownloadBudget::default(),
    );
    let token = CancellationToken::new();
    let task = tokio::spawn({
        let token = token.clone();
        async move {
            tools
                .execute_with_context(
                    "http_download",
                    serde_json::json!({"url": url("/doc"), "destination": DEST}),
                    ToolExecutionContext {
                        result_cap: FetchLimits::tool_lane_cap(),
                        cancellation: token,
                    },
                )
                .await
        }
    });
    held.notified().await;
    token.cancel();
    let error = tokio::time::timeout(ABORT, task)
        .await
        .expect("a cancelled download aborts instead of hanging")
        .expect("the tool task joins")
        .expect_err("cancellation is an error, never a result");
    let ToolError::Fetch(text) = error else {
        panic!("the download lane's typed error, got: {error:?}")
    };
    assert_eq!(text, "download cancelled", "the lane's cancellation detail");

    // The positive control: an uncancelled context completes normally.
    let (_ok_root, ok_workspace) = workspace("adapter-ok");
    let ok_calls = Arc::new(Mutex::new(Vec::new()));
    let ok_ranges = Arc::new(Mutex::new(Vec::new()));
    let ok_net = RangedNet {
        content: Arc::new(content.clone()),
        chunk: 8,
        calls: Arc::clone(&ok_calls),
        ranges: Arc::clone(&ok_ranges),
    };
    let ok_tools = FetchTools::new(
        policy(),
        Arc::new(ok_net),
        FetchLimits::for_tool_lane(),
        DownloadLimits::default(),
        Some(Arc::new(ok_workspace)),
        DownloadBudget::default(),
    );
    let result = tokio::time::timeout(
        ABORT,
        ok_tools.execute_with_context(
            "http_download",
            serde_json::json!({"url": url("/doc"), "destination": OK_DEST}),
            ToolExecutionContext {
                result_cap: FetchLimits::tool_lane_cap(),
                cancellation: CancellationToken::new(),
            },
        ),
    )
    .await
    .expect("an uncancelled download must not hang")
    .expect("the uncancelled download completes");
    assert_eq!(result["destination"], OK_DEST);
    assert_eq!(result["bytes"], 40);
    assert_eq!(result["sha256"], digest(&content));
}

/// The regression leg: an uncancelled `http_download` through the unchanged
/// 7-parameter wrapper still downloads, promotes, and clears the sidecar.
#[tokio::test]
async fn an_uncancelled_download_through_the_old_wrapper_still_completes() {
    let content: Vec<u8> = (0..40u8).collect();
    let (root, workspace) = workspace("wrapper");
    let budget = DownloadBudget::default();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let ranges = Arc::new(Mutex::new(Vec::new()));
    let net = RangedNet {
        content: Arc::new(content.clone()),
        chunk: 8,
        calls: Arc::clone(&calls),
        ranges: Arc::clone(&ranges),
    };
    let outcome = tokio::time::timeout(
        ABORT,
        http_download(
            &policy(),
            &net,
            &workspace,
            DEST,
            &url("/doc"),
            DownloadLimits::default(),
            &budget,
        ),
    )
    .await
    .expect("the wrapper must not hang")
    .expect("the uncancelled download completes");
    assert_eq!(outcome.bytes, content.len() as u64);
    assert_eq!(outcome.sha256, digest(&content));
    assert_eq!(
        std::fs::read(root.join(DEST)).expect("the destination"),
        content
    );
    assert!(!root.join(SIDECAR).exists(), "the sidecar is cleared");
}
