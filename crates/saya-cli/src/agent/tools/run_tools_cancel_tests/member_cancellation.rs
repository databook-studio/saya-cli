use super::*;

/// The composite forwards the turn's token to the fetch member: a fetch
/// waiting on request/headers returns the lane's typed cancellation instead
/// of hanging to its own 30 s budget.
#[tokio::test]
async fn composite_forwards_the_turn_token_to_fetch() {
    let entered = Arc::new(Notify::new());
    let (_root, fetch) = fetch_member(
        "fetch",
        HeldHeadersNet {
            entered: Arc::clone(&entered),
        },
    );
    let composite = RunTools::compose(Arc::new(DatabaseTools::new(None, 100, true)), Some(fetch));
    let token = CancellationToken::new();
    let task = tokio::spawn({
        let token = token.clone();
        async move {
            composite
                .execute_with_context("http_fetch", json!({"url": url("/doc")}), context(&token))
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
    match error {
        ToolError::Fetch(detail) => {
            assert_eq!(detail, "fetch cancelled", "the lane's cancellation detail")
        }
        other => panic!("the fetch lane's typed error, got: {other:?}"),
    }
}

/// The regression pin: an uncancelled turn keeps the fetch lane working
/// through the composite — the contained block arrives exactly as before the
/// forwarding existed.
#[tokio::test]
async fn an_uncancelled_turn_keeps_fetch_working_through_the_composite() {
    let (_fetch_root, fetch) = fetch_member(
        "regression-fetch",
        CannedNet {
            body: b"an ordinary page",
        },
    );
    let composite = RunTools::compose(Arc::new(DatabaseTools::new(None, 100, true)), Some(fetch));
    let value = tokio::time::timeout(
        ABORT,
        composite.execute_with_context(
            "http_fetch",
            json!({"url": url("/doc")}),
            context(&CancellationToken::new()),
        ),
    )
    .await
    .expect("an uncancelled fetch completes")
    .unwrap();
    let content = value["content"].as_str().unwrap();
    assert!(
        content.contains("an ordinary page") && content.contains(OPEN) && content.contains(CLOSE),
        "the whole body rides the contained block: {content}"
    );
    assert_eq!(value["url"].as_str().unwrap(), url("/doc"));
}
