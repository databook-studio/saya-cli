/// A job whose own wall clock is already spent must stop with the clock's
/// class — an answering-budget stop — not open a request that can only be cut
/// and spend a stream-cut allowance on it. This is the product half of H2: a
/// job killed at its time limit reports a timeout, not a provider failure.
#[tokio::test]
async fn a_wall_clock_deadline_stops_the_job_with_the_clock_class() {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let provider = HangingEstablishmentProvider {
        calls: calls.clone(),
    };
    let limits = AgentLimits {
        progress: Some(saya_agent::ProgressLimits {
            wall_clock: Some(std::time::Duration::ZERO),
            ..saya_agent::ProgressLimits::default()
        }),
        ..AgentLimits::default()
    };
    let control = saya_agent::AgentRunControl::new(
        limits,
        saya_agent::AnsweringLimits {
            max_requests: Some(8),
            max_tool_calls: Some(8),
            max_reported_tokens: Some(10_000),
        },
        &AllowReadOnlyApproval,
        &saya_agent::NoopEventSink,
        CancellationToken::new(),
    );
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        saya_agent::run_agent_with_control(
            &provider,
            &MockTools {
                calls: Arc::new(Mutex::new(Vec::new())),
            },
            request(),
            definitions(),
            control,
        ),
    )
    .await
    .expect("the spent wall clock, not the test harness, ends the run")
    .expect("a spent wall clock is a budget stop, not a provider error");
    assert!(output.truncated, "the stop returns the partial work");
    assert!(
        output.events.iter().any(|event| matches!(
            event,
            AgentEvent::AnsweringBudgetStopped {
                dimension: saya_agent::BudgetDimension::ElapsedTime
            }
        )),
        "the stop names the elapsed-time budget: {:?}",
        output.events
    );
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "a spent wall clock must not open a request that can only be cut"
    );
    assert!(
        !output
            .events
            .iter()
            .any(|event| matches!(event, AgentEvent::StreamCutRecovered { .. })),
        "no stream-cut allowance is spent on a request that never starts"
    );
}

/// A wall clock that expires while the provider stream is open is the job's
/// budget stopping, not a stream cut: the run returns the partial work under
/// the elapsed-time class instead of a provider failure. This is the real H2
/// path — the deadline is the job's clock, not the provider's cap.
#[tokio::test]
async fn a_wall_clock_expiry_mid_stream_reports_the_clock_class() {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let provider = HangingStreamProvider {
        calls: calls.clone(),
    };
    let limits = AgentLimits {
        progress: Some(saya_agent::ProgressLimits {
            wall_clock: Some(std::time::Duration::from_millis(100)),
            ..saya_agent::ProgressLimits::default()
        }),
        ..AgentLimits::default()
    };
    let control = saya_agent::AgentRunControl::new(
        limits,
        saya_agent::AnsweringLimits {
            max_requests: Some(8),
            max_tool_calls: Some(8),
            max_reported_tokens: Some(10_000),
        },
        &AllowReadOnlyApproval,
        &saya_agent::NoopEventSink,
        CancellationToken::new(),
    );
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        saya_agent::run_agent_with_control(
            &provider,
            &MockTools {
                calls: Arc::new(Mutex::new(Vec::new())),
            },
            request(),
            definitions(),
            control,
        ),
    )
    .await
    .expect("the wall clock, not the test harness, bounds the open stream")
    .expect("a wall clock stop is a budget stop, not a provider error");
    assert!(output.truncated, "the stop returns the partial work");
    assert!(
        output.events.iter().any(|event| matches!(
            event,
            AgentEvent::AnsweringBudgetStopped {
                dimension: saya_agent::BudgetDimension::ElapsedTime
            }
        )),
        "the stop names the elapsed-time budget: {:?}",
        output.events
    );
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the open stream was cut by the job's clock"
    );
    assert!(
        !output
            .events
            .iter()
            .any(|event| matches!(event, AgentEvent::StreamCutRecovered { .. })),
        "the job's clock does not spend a stream-cut continuation"
    );
}
