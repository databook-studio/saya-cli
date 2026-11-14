/// The observed failure: the gateway cut the stream after ~62k tokens of
/// reasoning, and the loop re-sent the byte-identical request four times
/// (~21 minutes). A cut that delivered content is typed as its own cause and
/// answered with one bounded continuation per remaining allowance — a closed
/// user-role note naming the window, never a replay of what the cut delivered.
#[tokio::test]
async fn a_cut_after_content_is_typed_and_never_resent_identically() {
    use saya_agent::{
        ProviderContinuationPhase as Continuation, ProviderRecoveryPhase as Recovery,
        ProviderRecoveryReason as Reason,
    };
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = CuttingReasoningProvider {
        deltas: CUT_REASONING_DELTAS,
        visible: false,
        requests: requests.clone(),
    };
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
    };
    let error = run_agent_with_sink(
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
    .unwrap_err();
    assert!(
        matches!(
            error,
            AgentError::Provider(saya_agent::ProviderError::StreamCut {
                estimated_tokens: 0,
                ..
            })
        ),
        "a cut that delivered content is typed as a stream cut: {error:?}"
    );

    let captured = requests.lock().unwrap();
    let bodies: Vec<_> = captured
        .iter()
        .map(|request| serde_json::to_value(request).unwrap())
        .collect();
    let first = &bodies[0];
    assert_eq!(
        bodies.len(),
        3,
        "two consecutive cuts are recovered; the third consecutive cut is terminal"
    );
    assert_eq!(
        bodies.iter().filter(|body| *body == first).count(),
        1,
        "the identical request is never re-sent after a cut that delivered content"
    );
    let ceilings: Vec<_> = events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match event {
            AgentEvent::StreamCutRecovered { ceiling_after, .. } => Some(*ceiling_after),
            _ => None,
        })
        .collect();
    assert!(
        !ceilings.is_empty() && ceilings.iter().all(|ceiling| *ceiling == 0),
        "a reasoning-only cut is not a measured output window: {ceilings:?}"
    );
    let note = &captured[1].messages.last().expect("a note was pushed");
    assert_eq!(
        note.role, "user",
        "the continuation note is a user-role message"
    );
    assert!(
        note.content
            .contains("cut off by the request time limit after 0s"),
        "the note names the window the provider allowed: {}",
        note.content
    );
    assert!(
        !note.content.contains("one tool call") && !note.content.contains("smaller steps"),
        "a durable one-call order outlives compaction: {}",
        note.content
    );
    assert!(
        !captured.iter().any(|request| request
            .messages
            .iter()
            .any(|message| message.content.contains("xxxx"))),
        "the cut attempt's reasoning never re-enters request history"
    );

    let seen = events.lock().unwrap().clone();
    let recoveries: Vec<_> = seen
        .iter()
        .filter_map(|event| match event {
            AgentEvent::ProviderRecovery {
                phase,
                reason,
                attempt,
                limit,
            } => Some((*phase, *reason, *attempt, *limit)),
            _ => None,
        })
        .collect();
    assert_eq!(
        recoveries,
        vec![(Recovery::NotRetried, Reason::StreamCut, 1, 9); 3],
        "every cut is reported under its own reason, never as a retry: {seen:?}"
    );
    let phases: Vec<_> = seen
        .iter()
        .filter_map(|event| match event {
            AgentEvent::ProviderContinuation {
                phase,
                index,
                limit,
            } => Some((*phase, *index, *limit)),
            _ => None,
        })
        .collect();
    assert_eq!(
        phases,
        vec![
            (Continuation::Continuing, 1, 3),
            (Continuation::Continuing, 2, 3),
            (Continuation::Exhausted, 3, 3),
        ],
        "two consecutive cuts are admitted, then the third exhausts the bound: {seen:?}"
    );
    assert_eq!(
        seen.iter()
            .filter(|event| matches!(event, AgentEvent::TurnReset))
            .count(),
        0,
        "nothing is reset, because nothing is retried"
    );
    // The adaptation is never silent: the operator sees the cut's clock, the
    // estimate, and the ceiling the run continues under.
    let cuts: Vec<_> = seen
        .iter()
        .filter_map(|event| match event {
            AgentEvent::StreamCutRecovered {
                elapsed_secs,
                estimated_tokens,
                ceiling_after,
            } => Some((*elapsed_secs, *estimated_tokens, *ceiling_after)),
            _ => None,
        })
        .collect();
    assert_eq!(
        cuts,
        vec![(0, 0, 0); 2],
        "a reasoning-only cut reports no visible window and does not lower the ceiling: {seen:?}"
    );
}

/// The guard: a cut that delivered no content bytes at all is still a plain
/// retryable `StreamEnded`. A flaky gateway still recovers on the backoff.
#[tokio::test]
async fn a_cut_before_any_content_still_retries() {
    use saya_agent::ProviderRecoveryReason as Reason;
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = CuttingReasoningProvider {
        deltas: 0,
        visible: false,
        requests: requests.clone(),
    };
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
    };
    let error = run_agent_with_sink(
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
    .unwrap_err();
    assert!(
        matches!(
            error,
            AgentError::Provider(saya_agent::ProviderError::StreamEnded)
        ),
        "a cut with nothing delivered keeps the retryable cause: {error:?}"
    );
    assert_eq!(
        requests.lock().unwrap().len(),
        9,
        "one attempt plus the eight scheduled retries"
    );
    let seen = events.lock().unwrap().clone();
    let reasons: Vec<_> = seen
        .iter()
        .filter_map(|event| match event {
            AgentEvent::ProviderRecovery { reason, .. } => Some(*reason),
            _ => None,
        })
        .collect();
    assert!(
        reasons.len() == 9 && reasons.iter().all(|reason| *reason == Reason::StreamEnded),
        "a content-free cut keeps the stream-ended retry reason: {seen:?}"
    );
}
