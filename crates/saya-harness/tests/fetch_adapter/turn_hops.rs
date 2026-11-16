use super::*;

/// A cancel racing the redirect: hop 1 answers 302 after cancelling the
/// turn's token, and the fetch must return cancelled having requested no
/// second hop — the recorded transport calls end at the first URL.
#[tokio::test]
async fn a_cancelled_turn_never_begins_another_redirect_hop() {
    let (_root, workspace) = workspace("cancel-redirect");
    let calls = Arc::new(Mutex::new(Vec::new()));
    let token = CancellationToken::new();
    let tools = adapter(
        RedirectCancelNet {
            calls: Arc::clone(&calls),
            token: token.clone(),
        },
        workspace,
        DownloadBudget::default(),
        FetchLimits::for_tool_lane(),
    );
    let error = tokio::time::timeout(
        ABORT,
        tools.execute_with_context(
            "http_fetch",
            serde_json::json!({"url": url("/doc")}),
            context(&token),
        ),
    )
    .await
    .expect("a cancelled redirect aborts instead of hanging")
    .expect_err("cancellation is an error, never a result");
    let ToolError::Fetch(detail) = error else {
        panic!("the fetch lane's typed error, got: {error:?}")
    };
    assert_eq!(detail, "fetch cancelled", "the lane's cancellation detail");
    assert_eq!(
        calls.lock().expect("calls").clone(),
        vec![url("/doc")],
        "hop 2 must never be requested after cancellation"
    );
}

/// A token cancelled before the call: the fetch refuses at once and never
/// touches the transport.
#[tokio::test]
async fn a_precancelled_turn_makes_no_transport_calls() {
    let (_root, workspace) = workspace("precancelled");
    let (net, calls) = StaticNet::serve(b"unused", 4_096);
    let tools = adapter(
        net,
        workspace,
        DownloadBudget::default(),
        FetchLimits::for_tool_lane(),
    );
    let token = CancellationToken::new();
    token.cancel();
    let error = tokio::time::timeout(
        ABORT,
        tools.execute_with_context(
            "http_fetch",
            serde_json::json!({"url": url("/doc")}),
            context(&token),
        ),
    )
    .await
    .expect("a pre-cancelled fetch refuses at once")
    .expect_err("cancellation is an error, never a result");
    let ToolError::Fetch(detail) = error else {
        panic!("the fetch lane's typed error, got: {error:?}")
    };
    assert_eq!(detail, "fetch cancelled", "the lane's cancellation detail");
    assert!(
        calls.lock().expect("calls").is_empty(),
        "a pre-cancelled turn never touches the transport"
    );
}

/// The regression pin: a token that is never cancelled leaves the fetch
/// path exactly as it was — whole body, contained block, final URL.
#[tokio::test]
async fn an_uncancelled_turn_still_returns_the_whole_body() {
    let (_root, workspace) = workspace("cancel-regression");
    let (net, _calls) = StaticNet::serve(b"an ordinary page", 4_096);
    let tools = adapter(
        net,
        workspace,
        DownloadBudget::default(),
        FetchLimits::for_tool_lane(),
    );
    let value = tokio::time::timeout(
        ABORT,
        tools.execute_with_context(
            "http_fetch",
            serde_json::json!({"url": url("/doc")}),
            context(&CancellationToken::new()),
        ),
    )
    .await
    .expect("an uncancelled fetch completes")
    .expect("the uncancelled path is unchanged");
    let content = value["content"].as_str().expect("the lane's content");
    assert!(
        content.contains("an ordinary page") && content.contains(OPEN) && content.contains(CLOSE),
        "the whole body rides the contained block: {content}"
    );
    assert_eq!(value["url"].as_str().unwrap(), url("/doc"));
}
