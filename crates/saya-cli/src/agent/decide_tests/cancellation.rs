#[tokio::test]
async fn cancelled_before_any_call_returns_no_winner() {
    let token = CancellationToken::new();
    token.cancel();
    let executor = ScriptedExecutor::new(HashMap::new());

    let decision = decide(
        &[nominated("SELECT 1 AS a"), nominated("SELECT 2 AS a")],
        &executor,
        D,
        &token,
    )
    .await;

    assert_eq!(decision.winner, None);
    assert!(!decision.tied);
    // The check precedes the first executor call, so nothing ran.
    assert!(executor.calls().is_empty());
}

#[tokio::test]
async fn cancelled_before_a_candidate_call_returns_no_winner() {
    // The first candidate runs and the executor cancels the token; the check
    // before the second candidate then returns early.
    let token = CancellationToken::new();
    let mut results = HashMap::new();
    results.insert(
        "SELECT 1 AS a".to_string(),
        rows_result(&["a"], vec![json!([1])]),
    );
    let executor = ScriptedExecutor::new(results).cancel_after(1, token.clone());

    let decision = decide(
        &[nominated("SELECT 1 AS a"), nominated("SELECT 2 AS a")],
        &executor,
        D,
        &token,
    )
    .await;

    assert_eq!(decision.winner, None);
    assert!(!decision.tied);
    // Only the first candidate ran; the second was never reached.
    assert_eq!(executor.calls().len(), 1);
}

#[tokio::test]
async fn cancelled_during_probe_phase_returns_no_winner() {
    // Two tied join candidates. The executor cancels the token during the
    // first probe's joined-rows call; the check before the base-rows call then
    // returns early with no winner, rather than presenting a considered pick.
    let (flagged, cleared, probe_flagged, _probe_cleared) = tied_join_candidates();
    let token = CancellationToken::new();
    let mut results = HashMap::new();
    results.insert(flagged.clone(), rows_result(&["total"], vec![json!([100])]));
    results.insert(cleared.clone(), rows_result(&["total"], vec![json!([200])]));
    // Register only the first probe's joined statement (count call #3).
    results.insert(probe_flagged.joined_rows.clone(), count_result(10));
    let executor = ScriptedExecutor::new(results).cancel_after(3, token.clone());

    let decision = decide(
        &[nominated(&flagged), nominated(&cleared)],
        &executor,
        D,
        &token,
    )
    .await;

    assert_eq!(decision.winner, None);
    // Three calls: two nominations + the first probe's joined count, then the
    // cancellation check before the base count returns early.
    assert_eq!(executor.calls().len(), 3);
}
