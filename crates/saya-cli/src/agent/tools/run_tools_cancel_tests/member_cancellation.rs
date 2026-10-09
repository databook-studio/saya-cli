use super::*;

/// The composite forwards the turn's token to the scratch member: the runaway
/// statement is interrupted, the cancelled detail surfaces, the elapsed time
/// is well under the 30 s query ceiling, and the released connection still
/// serves a follow-up statement through the same composite.
#[tokio::test]
async fn composite_forwards_the_turn_token_to_scratch() {
    let (_root, scratch) = scratch_member("scratch");
    let composite = scratch_composite(&scratch);
    let token = CancellationToken::new();
    let canceller = {
        let token = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            token.cancel();
        })
    };
    let started = Instant::now();
    let error = tokio::time::timeout(
        GUARD,
        composite.execute_with_context("scratch_sql", json!({"sql": RUNAWAY}), context(&token)),
    )
    .await
    .expect("the guard: the turn token must interrupt the runaway statement")
    .expect_err("a cancelled call is an error, never a result");
    let elapsed = started.elapsed();
    canceller.await.unwrap();
    match error {
        ToolError::QueryFailedDetail(detail) => {
            assert!(
                detail.contains("cancelled"),
                "the cancelled detail: {detail}"
            );
            assert!(
                !detail.contains("timed out"),
                "never the ceiling path: {detail}"
            );
        }
        other => panic!("cancellation must surface through the composite, got {other:?}"),
    }
    assert!(
        elapsed < Duration::from_secs(5),
        "the cancel must land well under the 30 s query ceiling, took {elapsed:?}"
    );
    // The interrupt-and-join released the connection: the composite still serves.
    let answer = tokio::time::timeout(
        GUARD,
        composite.execute_with_context(
            "scratch_sql",
            json!({"sql": "SELECT 42 AS answer"}),
            context(&CancellationToken::new()),
        ),
    )
    .await
    .expect("the follow-up guard")
    .unwrap();
    assert_eq!(answer["rows"], json!([[42]]));
}

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
    let composite = RunTools::compose(
        Arc::new(DatabaseTools::new(None, 100, true)),
        None,
        Some(fetch),
        None,
    );
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

/// The regression pin: an uncancelled turn keeps both lanes working through
/// the composite — the scratch statement's rows and the fetch's contained
/// block arrive exactly as before the forwarding existed.
#[tokio::test]
async fn an_uncancelled_turn_keeps_scratch_and_fetch_working_through_the_composite() {
    let (_root, scratch) = scratch_member("regression-scratch");
    let composite = scratch_composite(&scratch);
    let answer = tokio::time::timeout(
        GUARD,
        composite.execute_with_context(
            "scratch_sql",
            json!({"sql": "SELECT 41 + 1 AS answer"}),
            context(&CancellationToken::new()),
        ),
    )
    .await
    .expect("the guard")
    .unwrap();
    assert_eq!(answer["rows"], json!([[42]]));
    let (_fetch_root, fetch) = fetch_member(
        "regression-fetch",
        CannedNet {
            body: b"an ordinary page",
        },
    );
    let composite = RunTools::compose(
        Arc::new(DatabaseTools::new(None, 100, true)),
        None,
        Some(fetch),
        None,
    );
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
