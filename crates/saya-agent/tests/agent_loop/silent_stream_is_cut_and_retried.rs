/// A stream that delivers one delta and then only keep-alives has stalled: the
/// loop cuts it under the content-progress window and retries the turn, rather
/// than waiting for the request's total deadline (or forever).
///
/// The clock is paused, so the assertion is the product's two-minute window
/// elapsing, not the test's machine being slow: the loop only advances when it
/// is idle, which is exactly the stalled read.
#[tokio::test(start_paused = true)]
async fn a_stream_that_stops_after_content_is_cut_and_retried() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = SilentAfterContentProvider {
        requests: requests.clone(),
    };
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
    };
    let output = run_agent_with_sink(
        &provider,
        &MockTools {
            calls: Arc::new(Mutex::new(Vec::new())),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &sink,
        CancellationToken::new(),
    )
    .await
    .expect("the retried turn completes");
    assert_eq!(
        output.answer, "The answer is 42.",
        "the retried attempt's answer is the run's answer"
    );
    assert_eq!(
        requests.lock().unwrap().len(),
        2,
        "one stalled attempt and one retry"
    );
    let seen = events.lock().unwrap().clone();
    assert!(
        seen.iter().any(|event| matches!(
            event,
            AgentEvent::ProviderRecovery {
                phase: saya_agent::ProviderRecoveryPhase::Retrying,
                reason: saya_agent::ProviderRecoveryReason::StreamStalled,
                attempt: 2,
                ..
            }
        )),
        "the stall is typed, and the loop retries under it: {seen:#?}"
    );
    assert!(
        !seen.iter().any(|event| matches!(
            event,
            AgentEvent::ProviderRecovery {
                phase: saya_agent::ProviderRecoveryPhase::NotRetried,
                ..
            }
        )),
        "a stall is retryable: {seen:#?}"
    );
}
