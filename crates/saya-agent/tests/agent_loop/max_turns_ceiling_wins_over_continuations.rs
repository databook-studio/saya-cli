/// The ceilings compose: `max_turns: Some(2)` against a provider that
/// truncates forever stops at the turn ceiling through the salvage path —
/// two turns plus the salvage call — not after three continuations.
#[tokio::test]
async fn max_turns_ceiling_wins_over_continuations() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = continuation_script(&requests, true, "partial", Vec::new());
    let limits = AgentLimits {
        max_turns: Some(2),
        ..AgentLimits::default()
    };
    let output = run_agent_with_sink(
        &provider,
        &MockTools {
            calls: Arc::new(Mutex::new(Vec::new())),
        },
        request(),
        definitions(),
        limits,
        &AllowReadOnlyApproval,
        &NoopSink,
        CancellationToken::new(),
    )
    .await
    .expect("the turn ceiling salvages instead of erroring");
    assert!(
        output.truncated,
        "the run stopped at the turn ceiling through salvage"
    );
    assert_eq!(
        requests.lock().unwrap().len(),
        3,
        "two turns plus the salvage call — the tighter ceiling wins"
    );
}
