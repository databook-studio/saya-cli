/// The failed receipt is the final cumulative snapshot, not a sum of stream
/// snapshots. Reset precedes the typed retry progress and all progress uses a
/// one-based actual-attempt count out of the four allowed calls.
#[tokio::test]
async fn retry_uses_latest_failed_snapshot_and_unambiguous_progress() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let output = run_agent_with_sink(
        &LatestUsageThenSuccess {
            attempts: Mutex::new(0),
        },
        &NoTools {
            calls: Arc::new(Mutex::new(0)),
        },
        request(),
        Vec::new(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &RecordingSink(events.clone()),
        CancellationToken::new(),
    )
    .await
    .expect("the second attempt answers");
    assert_eq!(output.answer, "kept");
    assert_eq!(output.usage, TokenUsage::new(18, 7));
    let events = events.lock().unwrap();
    assert_eq!(
        events
            .iter()
            .filter_map(|event| match event {
                AgentEvent::FailedAttemptUsage { usage } => Some(*usage),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![TokenUsage::new(7, 2)]
    );
    let reset = events
        .iter()
        .position(|event| matches!(event, AgentEvent::TurnReset))
        .expect("a retry resets discarded text");
    assert!(matches!(
        events.get(reset + 1),
        Some(AgentEvent::ProviderRecovery {
            phase: ProviderRecoveryPhase::Retrying,
            reason: ProviderRecoveryReason::StreamEnded,
            attempt: 2,
            limit: 4,
        })
    ));
}
