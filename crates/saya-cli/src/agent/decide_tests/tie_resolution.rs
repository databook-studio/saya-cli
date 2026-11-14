#[tokio::test]
async fn tie_broken_when_one_group_cleared_and_others_flagged() {
    let (flagged, cleared, probe_flagged, probe_cleared) = tied_join_candidates();

    let mut results = HashMap::new();
    // The two nominated answers disagree, so the vote ties.
    results.insert(flagged.clone(), rows_result(&["total"], vec![json!([100])]));
    results.insert(cleared.clone(), rows_result(&["total"], vec![json!([200])]));
    // Flagged group: join multiplied rows (joined 10 > base 5).
    register_probe(&mut results, &probe_flagged, 10, 5);
    // Cleared group: no fan-out (joined 5 == base 5).
    register_probe(&mut results, &probe_cleared, 5, 5);
    let executor = ScriptedExecutor::new(results);

    let decision = decide(
        &[nominated(&flagged), nominated(&cleared)],
        &executor,
        D,
        &CancellationToken::new(),
    )
    .await;

    // The cleared group (candidate 1) wins; the vote is still reported tied.
    assert_eq!(decision.winner, Some(1));
    assert!(decision.tied);
    assert!(decision.probe_broke_tie);
    assert_eq!(decision.fanout, vec![Some(true), Some(false)]);
}

#[tokio::test]
async fn tie_not_broken_when_no_group_is_cleared() {
    // Both groups flagged: no cleared group, so no winner.
    let (flagged, cleared, probe_flagged, probe_cleared) = tied_join_candidates();
    let mut results = HashMap::new();
    results.insert(flagged.clone(), rows_result(&["total"], vec![json!([100])]));
    results.insert(cleared.clone(), rows_result(&["total"], vec![json!([200])]));
    register_probe(&mut results, &probe_flagged, 10, 5);
    register_probe(&mut results, &probe_cleared, 9, 5); // also flagged
    let executor = ScriptedExecutor::new(results);

    let decision = decide(
        &[nominated(&flagged), nominated(&cleared)],
        &executor,
        D,
        &CancellationToken::new(),
    )
    .await;

    assert_eq!(decision.winner, None);
    assert!(decision.tied);
    assert!(!decision.probe_broke_tie);
    assert_eq!(decision.fanout, vec![Some(true), Some(true)]);
}

#[tokio::test]
async fn tie_not_broken_when_several_groups_are_cleared() {
    // Two cleared groups: ambiguous, so no winner.
    let (flagged, cleared, probe_flagged, probe_cleared) = tied_join_candidates();
    let mut results = HashMap::new();
    results.insert(flagged.clone(), rows_result(&["total"], vec![json!([100])]));
    results.insert(cleared.clone(), rows_result(&["total"], vec![json!([200])]));
    register_probe(&mut results, &probe_flagged, 5, 5); // cleared
    register_probe(&mut results, &probe_cleared, 5, 5); // also cleared
    let executor = ScriptedExecutor::new(results);

    let decision = decide(
        &[nominated(&flagged), nominated(&cleared)],
        &executor,
        D,
        &CancellationToken::new(),
    )
    .await;

    assert_eq!(decision.winner, None);
    assert!(decision.tied);
    assert!(!decision.probe_broke_tie);
    assert_eq!(decision.fanout, vec![Some(false), Some(false)]);
}

#[tokio::test]
async fn tie_not_broken_when_a_probe_statement_fails() {
    // A probe statement that fails to execute is NO SIGNAL — recorded as
    // None, never as fan-out — so it cannot be read as flagged. Here the
    // would-be-cleared group's `base_rows` fails (the known "no such column"
    // case), so its evidence is None and the tie stays unbroken.
    let (flagged, cleared, probe_flagged, probe_cleared) = tied_join_candidates();
    let mut results = HashMap::new();
    results.insert(flagged.clone(), rows_result(&["total"], vec![json!([100])]));
    results.insert(cleared.clone(), rows_result(&["total"], vec![json!([200])]));
    register_probe(&mut results, &probe_flagged, 10, 5); // flagged
    // Cleared group: joined runs, but base_rows is absent from the map so the
    // executor returns None for it — a failed statement, treated as no signal.
    results.insert(probe_cleared.joined_rows.clone(), count_result(5));
    // base_rows deliberately NOT registered → run() returns None.
    let executor = ScriptedExecutor::new(results);

    let decision = decide(
        &[nominated(&flagged), nominated(&cleared)],
        &executor,
        D,
        &CancellationToken::new(),
    )
    .await;

    // The failed probe is None — neither flagged nor cleared — so it must not
    // be read as fan-out, and the tie cannot be broken.
    assert_eq!(decision.fanout, vec![Some(true), None]);
    assert_eq!(decision.winner, None);
    assert!(decision.tied);
    assert!(!decision.probe_broke_tie);
}

#[tokio::test]
async fn tie_not_broken_when_probe_cannot_be_built() {
    // One tied candidate is a plain aggregate with no join, so `fanout_probe`
    // returns None for it. Its evidence is None; even though the other group
    // is cleared, "every other group flagged" fails (this one is None), so the
    // tie stays unbroken.
    let join_sql =
        "SELECT SUM(o.amount) AS total FROM orders o JOIN lines l ON l.order_id = o.id".to_string();
    let plain_sql = "SELECT 1 AS total".to_string();
    let probe_join = fanout_probe(&join_sql, D).expect("join candidate yields a probe");
    assert!(
        fanout_probe(&plain_sql, D).is_none(),
        "plain aggregate has no probe"
    );

    let mut results = HashMap::new();
    results.insert(
        join_sql.clone(),
        rows_result(&["total"], vec![json!([100])]),
    );
    results.insert(
        plain_sql.clone(),
        rows_result(&["total"], vec![json!([200])]),
    );
    register_probe(&mut results, &probe_join, 5, 5); // cleared
    let executor = ScriptedExecutor::new(results);

    let decision = decide(
        &[nominated(&join_sql), nominated(&plain_sql)],
        &executor,
        D,
        &CancellationToken::new(),
    )
    .await;

    assert_eq!(decision.fanout, vec![Some(false), None]);
    assert_eq!(decision.winner, None);
    assert!(decision.tied);
    assert!(!decision.probe_broke_tie);
}
