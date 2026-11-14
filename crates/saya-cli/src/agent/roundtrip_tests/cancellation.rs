#[tokio::test]
async fn cancelled_before_restate_returns_no_divergence() {
    // The token is already cancelled, so the check before the first model call
    // returns early. Nothing runs.
    let token = CancellationToken::new();
    token.cancel();
    let restater = ScriptedRestater::new()
        .restate_returns(Some("should not be reached".into()))
        .judge_returns(Some(Verdict {
            agrees: false,
            reason: "should not be reached".into(),
        }));

    let rt = round_trip("q", "SELECT 1 AS a", &shape(&["a"], 0), &restater, &token).await;

    assert!(
        restater.log().is_empty(),
        "no model call should run when cancelled up front"
    );
    assert!(rt.restatement.is_none());
    assert!(rt.verdict.is_none());
    assert!(!rt.diverged);
}

#[tokio::test]
async fn cancelled_before_judge_returns_no_divergence() {
    // restate runs (call 1) and trips the token; the check before judge then
    // returns early. A cancelled round-trip is never a considered one, so it
    // carries no restatement and no verdict even though restate produced one.
    let token = CancellationToken::new();
    let restater = ScriptedRestater::new()
        .restate_returns(Some("count of orders per customer".into()))
        .judge_returns(Some(Verdict {
            agrees: false,
            reason: "should not be reached".into(),
        }))
        .cancel_after(1, token.clone());

    let rt = round_trip(
        "how many orders did each customer place?",
        "SELECT customer_id, COUNT(*) AS n FROM orders GROUP BY customer_id",
        &shape(&["customer_id", "n"], 5),
        &restater,
        &token,
    )
    .await;

    let log = restater.log();
    assert_eq!(log.len(), 1, "restate ran, judge did not");
    assert!(matches!(log[0], Call::Restate { .. }));
    assert!(
        rt.restatement.is_none(),
        "a cancelled round-trip carries no restatement"
    );
    assert!(rt.verdict.is_none());
    assert!(!rt.diverged, "cancellation is never a divergence");
}
