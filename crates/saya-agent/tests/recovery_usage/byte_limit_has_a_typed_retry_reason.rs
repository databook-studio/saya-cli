#[tokio::test]
async fn byte_limit_has_a_typed_retry_reason() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let output = run_agent_with_sink(
        &ByteLimitThenSuccess {
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
    .expect("the retry answers");
    assert_eq!(output.answer, "kept");
    assert!(events.lock().unwrap().iter().any(|event| matches!(
        event,
        AgentEvent::ProviderRecovery {
            phase: ProviderRecoveryPhase::Retrying,
            reason: ProviderRecoveryReason::StreamByteLimit,
            attempt: 2,
            limit: 4,
        }
    )));
}
