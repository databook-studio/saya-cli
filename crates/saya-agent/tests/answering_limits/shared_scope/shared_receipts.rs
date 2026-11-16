use super::*;

#[tokio::test]
async fn shared_scope_failed_and_successful_receipts_reach_one_token_ceiling() {
    let cancellation = CancellationToken::new();
    let scope = AnsweringScope::new(
        answering_limits(Some(4), None, Some(8)),
        cancellation.clone(),
    );
    let sink = RecordingSink::new(cancellation.clone());
    let first = ScriptedProvider::new(vec![vec![
        Ok(ProviderEvent::Usage(TokenUsage::new(9, 0))),
        Ok(ProviderEvent::Usage(TokenUsage::new(2, 1))),
        Ok(ProviderEvent::Done),
    ]])
    .0;
    let error = run_in_scope(
        &first,
        &RecordingExecutor::default(),
        &sink,
        &scope,
        answering_limits(Some(4), None, Some(80)),
    )
    .await
    .expect_err("the first provider attempt ended without an answer");
    assert!(matches!(
        error,
        AgentError::Provider(ProviderError::InvalidResponse)
    ));

    let (second, second_requests) = ScriptedProvider::new(vec![turn_with_usage(
        "settled answer",
        Vec::new(),
        TokenUsage::new(3, 3),
    )]);
    let output = run_in_scope(
        &second,
        &RecordingExecutor::default(),
        &sink,
        &scope,
        answering_limits(Some(4), None, Some(80)),
    )
    .await
    .expect("the completed answer that reaches the cap is retained");

    assert_eq!(second_requests.lock().unwrap().len(), 1);
    assert_eq!(output.answer, "settled answer");
    assert_eq!(output.usage, TokenUsage::new(3, 3));
    assert!(
        !output.truncated,
        "a settled natural answer remains complete"
    );
    assert!(
        !output
            .events
            .iter()
            .any(|event| matches!(event, AgentEvent::AnsweringBudgetStopped { .. }))
    );
    assert_eq!(
        cancellation.reason(),
        Some(CancellationReason::Budget(BudgetDimension::ReportedTokens))
    );
    let failed_receipts: Vec<_> = sink
        .events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match event {
            AgentEvent::FailedAttemptUsage { usage } => Some(*usage),
            _ => None,
        })
        .collect();
    assert_eq!(failed_receipts, [TokenUsage::new(2, 1)]);

    let (third, third_requests) = ScriptedProvider::new(vec![turn("must not run", Vec::new())]);
    let error = run_in_scope(
        &third,
        &RecordingExecutor::default(),
        &sink,
        &scope,
        answering_limits(Some(99), None, Some(99)),
    )
    .await
    .expect_err("a stopped token scope cannot be widened by a later call");
    assert!(matches!(
        error,
        AgentError::Limit("answering budget stopped")
    ));
    assert!(third_requests.lock().unwrap().is_empty());
    assert_eq!(request_count(&sink.observations.lock().unwrap()), 2);
}

#[tokio::test]
async fn shared_scope_keeps_unknown_usage_distinct_from_reported_zero() {
    let cancellation = CancellationToken::new();
    let scope = AnsweringScope::new(
        answering_limits(Some(3), None, Some(0)),
        cancellation.clone(),
    );
    let sink = RecordingSink::new(cancellation);
    let unknown = ScriptedProvider::new(vec![turn("unknown receipt", Vec::new())]).0;
    run_in_scope(
        &unknown,
        &RecordingExecutor::default(),
        &sink,
        &scope,
        answering_limits(Some(3), None, None),
    )
    .await
    .expect("unknown usage does not consume a reported-token budget");
    assert!(!scope.cancellation_token().is_cancelled());

    let reported = ScriptedProvider::new(vec![turn_with_usage(
        "reported zero",
        Vec::new(),
        TokenUsage::default(),
    )])
    .0;
    let output = run_in_scope(
        &reported,
        &RecordingExecutor::default(),
        &sink,
        &scope,
        answering_limits(Some(3), None, None),
    )
    .await
    .expect("reported zero is a settled receipt under the explicit zero ceiling");
    assert_eq!(output.answer, "reported zero");
    assert_eq!(
        scope.cancellation_token().reason(),
        Some(CancellationReason::Budget(BudgetDimension::ReportedTokens))
    );

    let (later, later_requests) = ScriptedProvider::new(vec![turn("must not run", Vec::new())]);
    let error = run_in_scope(
        &later,
        &RecordingExecutor::default(),
        &sink,
        &scope,
        answering_limits(Some(99), None, None),
    )
    .await
    .expect_err("reported zero closes this question's token scope");
    assert!(matches!(
        error,
        AgentError::Limit("answering budget stopped")
    ));
    assert!(later_requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_new_scope_starts_a_distinct_question_budget() {
    let first_cancellation = CancellationToken::new();
    let first_scope = AnsweringScope::new(
        answering_limits(Some(1), None, None),
        first_cancellation.clone(),
    );
    let first_sink = RecordingSink::new(first_cancellation);
    let first = ScriptedProvider::new(vec![turn("first question", Vec::new())]).0;
    run_in_scope(
        &first,
        &RecordingExecutor::default(),
        &first_sink,
        &first_scope,
        answering_limits(Some(1), None, None),
    )
    .await
    .expect("first question completes");

    let second_cancellation = CancellationToken::new();
    let second_scope = AnsweringScope::new(
        answering_limits(Some(1), None, None),
        second_cancellation.clone(),
    );
    let second_sink = RecordingSink::new(second_cancellation);
    let second = ScriptedProvider::new(vec![turn("new question", Vec::new())]).0;
    let output = run_in_scope(
        &second,
        &RecordingExecutor::default(),
        &second_sink,
        &second_scope,
        answering_limits(Some(1), None, None),
    )
    .await
    .expect("a separately constructed scope is a fresh question");
    assert_eq!(output.answer, "new question");
}
