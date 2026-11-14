#[tokio::test]
async fn empty_slice_returns_without_panicking() {
    let executor = ScriptedExecutor::new(HashMap::new());
    let decision = decide(&[], &executor, D, &CancellationToken::new()).await;

    assert_eq!(decision.winner, None);
    assert_eq!(decision.votes, 0);
    assert!(!decision.tied);
    assert!(executor.calls().is_empty());
}

#[tokio::test]
async fn single_none_nomination_returns_no_winner() {
    let executor = ScriptedExecutor::new(HashMap::new());
    let decision = decide(&[None], &executor, D, &CancellationToken::new()).await;

    assert_eq!(decision.winner, None);
    assert_eq!(decision.votes, 0);
    assert!(executor.calls().is_empty());
}

#[tokio::test]
async fn all_none_nominations_return_no_winner() {
    let executor = ScriptedExecutor::new(HashMap::new());
    let decision = decide(&[None, None, None], &executor, D, &CancellationToken::new()).await;

    assert_eq!(decision.winner, None);
    assert_eq!(decision.votes, 0);
    assert!(executor.calls().is_empty());
}

#[tokio::test]
async fn single_successful_candidate_wins_without_a_probe() {
    let mut results = HashMap::new();
    results.insert(
        "SELECT 1 AS a".to_string(),
        rows_result(&["a"], vec![json!([1])]),
    );
    let executor = ScriptedExecutor::new(results);

    let decision = decide(
        &[nominated("SELECT 1 AS a")],
        &executor,
        D,
        &CancellationToken::new(),
    )
    .await;

    assert_eq!(decision.winner, Some(0));
    assert_eq!(decision.votes, 1);
    assert_eq!(decision.margin, 1);
    assert!(!decision.tied);
    // A single candidate is a decisive vote; no probe runs.
    assert_eq!(executor.calls().len(), 1);
    assert_eq!(decision.fanout, vec![None]);
}
