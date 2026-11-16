use super::*;

const STREAM_ITEM_LIMIT: usize = 65_536;

#[tokio::test]
async fn empty_deltas_hit_the_response_item_limit_without_retry_or_effects() {
    let mut oversized = vec![Ok(ProviderEvent::Usage(TokenUsage::new(40, 10)))];
    oversized.extend((0..=STREAM_ITEM_LIMIT).map(|_| Ok(ProviderEvent::TextDelta(String::new()))));
    oversized.push(Ok(ProviderEvent::Done));
    let (provider, requests) =
        ScriptedProvider::new(vec![oversized, turn("retry must not happen", Vec::new())]);
    let cancellation = CancellationToken::new();
    let sink = RecordingSink::new(cancellation.clone());
    let executor = RecordingExecutor::default();

    let error = saya_agent::run_agent_with_sink(
        &provider,
        &executor,
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &sink,
        cancellation,
    )
    .await
    .expect_err("an over-item response is a typed terminal refusal");

    assert!(matches!(
        error,
        AgentError::Provider(ProviderError::StreamItemLimit {
            usage: Some(usage)
        }) if *usage == TokenUsage::new(40, 10)
    ));
    assert_eq!(
        requests.lock().unwrap().len(),
        1,
        "the failed response is not retried"
    );
    assert!(executor.calls.lock().unwrap().is_empty());
    let emitted = sink.events.lock().unwrap();
    assert!(
        emitted.len() <= 3,
        "empty deltas created events: {emitted:?}"
    );
    assert!(!emitted.iter().any(|event| matches!(
        event,
        AgentEvent::AssistantText { text } if text.is_empty()
    )));
    assert_eq!(
        emitted
            .iter()
            .filter(|event| matches!(
                event,
                AgentEvent::FailedAttemptUsage { usage }
                    if *usage == TokenUsage::new(40, 10)
            ))
            .count(),
        1,
        "known usage before the item cap is retained once"
    );
    assert!(emitted.iter().any(|event| matches!(
        event,
        AgentEvent::ProviderRecovery { phase, reason, .. }
            if *phase == saya_agent::ProviderRecoveryPhase::NotRetried
                && format!("{reason:?}") == "StreamItemLimit"
    )));
}
