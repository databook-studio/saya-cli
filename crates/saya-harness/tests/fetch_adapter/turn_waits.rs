use super::*;

/// A cancelled turn aborts a fetch stuck waiting for request/headers: the
/// lane's typed cancellation instead of a hang, and the transport saw the
/// turn's one call and nothing after.
#[tokio::test]
async fn a_cancelled_turn_aborts_a_fetch_waiting_on_headers() {
    let (_root, workspace) = workspace("cancel-headers");
    let entered = Arc::new(Notify::new());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let tools = adapter(
        HeldHeadersNet {
            entered: Arc::clone(&entered),
            calls: Arc::clone(&calls),
        },
        workspace,
        DownloadBudget::default(),
        FetchLimits::for_tool_lane(),
    );
    let token = CancellationToken::new();
    let task = tokio::spawn({
        let token = token.clone();
        async move {
            tools
                .execute_with_context(
                    "http_fetch",
                    serde_json::json!({"url": url("/doc")}),
                    context(&token),
                )
                .await
        }
    });
    entered.notified().await; // the fetch is inside `get`, now waiting
    token.cancel();
    let error = tokio::time::timeout(ABORT, task)
        .await
        .expect("a cancelled fetch aborts instead of hanging")
        .expect("the fetch task joins")
        .expect_err("cancellation is an error, never a result");
    let ToolError::Fetch(detail) = error else {
        panic!("the fetch lane's typed error, got: {error:?}")
    };
    assert_eq!(detail, "fetch cancelled", "the lane's cancellation detail");
    assert_eq!(
        calls.lock().expect("calls").clone(),
        vec![url("/doc")],
        "the cancelled turn's one transport call"
    );
}

/// The same through the body read: one chunk arrives, the next wait is
/// pending, the cancel lands there, and the fetch returns cancelled.
#[tokio::test]
async fn a_cancelled_turn_aborts_a_fetch_waiting_on_body_chunks() {
    let (_root, workspace) = workspace("cancel-body");
    let first_chunk_served = Arc::new(Notify::new());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let tools = adapter(
        HeldBodyNet {
            first_chunk_served: Arc::clone(&first_chunk_served),
            calls: Arc::clone(&calls),
        },
        workspace,
        DownloadBudget::default(),
        FetchLimits::for_tool_lane(),
    );
    let token = CancellationToken::new();
    let task = tokio::spawn({
        let token = token.clone();
        async move {
            tools
                .execute_with_context(
                    "http_fetch",
                    serde_json::json!({"url": url("/doc")}),
                    context(&token),
                )
                .await
        }
    });
    first_chunk_served.notified().await; // one chunk read; the next wait is pending
    token.cancel();
    let error = tokio::time::timeout(ABORT, task)
        .await
        .expect("a cancelled fetch aborts instead of hanging")
        .expect("the fetch task joins")
        .expect_err("cancellation is an error, never a result");
    let ToolError::Fetch(detail) = error else {
        panic!("the fetch lane's typed error, got: {error:?}")
    };
    assert_eq!(detail, "fetch cancelled", "the lane's cancellation detail");
    assert_eq!(
        calls.lock().expect("calls").clone(),
        vec![url("/doc")],
        "the cancelled turn's one transport call"
    );
}
