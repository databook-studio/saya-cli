#[tokio::test]
async fn a_settled_clarification_stops_further_attempts_and_never_begins_a_vote_segment() {
    let first = "SELECT a FROM t WHERE id = 1";
    let second = "SELECT a FROM t WHERE id = 2";
    // Attempt 1 settles on a clarification; attempts 2 and 3 are queued and
    // observable (by the runner's call count) if the loop ever starts them.
    let runner = ScriptedRunner::new(vec![
        Outcome::Ok(clarification_output(first)),
        Outcome::Ok(output_with_sql(second)),
        Outcome::Ok(output_with_sql(second)),
    ]);
    let executor = CountingExecutor::new();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
    };

    let out = orchestrate_test(3, &runner, &executor, &sink, &CancellationToken::new())
        .await
        .expect("the settled clarification is a finished outcome, not an error");

    assert_eq!(
        runner.calls(),
        1,
        "attempt 2 never started once the clarification settled"
    );
    assert_eq!(
        executor.calls(),
        0,
        "no vote segment began: the executor is never called"
    );
    assert_eq!(
        consensus_count(&events.lock().expect("events lock")),
        0,
        "no consensus may be fabricated for a paused question"
    );
    // The returned output IS the paused attempt's, unchanged.
    assert_eq!(out.answer_sql.as_deref(), Some(first));
    assert!(
        out.events
            .iter()
            .any(|event| matches!(event, AgentEvent::ClarificationNeeded { .. })),
        "the paused attempt's evidence is preserved: {:?}",
        out.events
    );
    assert!(
        !out.truncated,
        "a settled clarification is not a truncation: {:?}",
        out.truncated
    );
}

// ===========================================================================
// A settled clarification after an errored attempt: the error did not stop the
// loop, the clarification does — and the returned paused output keeps the
// earlier paid usage summed truthfully.
// ===========================================================================

#[tokio::test]
async fn a_settled_clarification_after_an_errored_attempt_returns_the_paused_output_with_paid_usage()
 {
    let sql = "SELECT a FROM t WHERE id = 1";
    let runner = ScriptedRunner::new(vec![
        Outcome::Err(AgentRuntimeError::Agent("attempt 1 failed".into())),
        Outcome::Ok(clarification_output(sql)),
    ]);
    let executor = CountingExecutor::new();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
    };

    let cancellation = CancellationToken::new();
    let scope = test_scope(cancellation.clone());
    let receipts = AnsweringReceiptSink::new(&sink);
    receipts
        .emit(AgentEvent::usage(
            saya_agent::UsageCall::Answer,
            TokenUsage::new(4, 2),
        ))
        .await;
    let out = orchestrate(2, &runner, &executor, D, &receipts, &cancellation, &scope)
        .await
        .expect("the settled clarification is returned");

    assert_eq!(
        runner.calls(),
        2,
        "the errored attempt did not stop the loop; the clarification did"
    );
    assert_eq!(
        executor.calls(),
        0,
        "no vote segment began: the executor is never called"
    );
    assert_eq!(
        consensus_count(&events.lock().expect("events lock")),
        0,
        "no consensus may be fabricated for a paused question"
    );
    assert_eq!(out.answer_sql.as_deref(), Some(sql));
    assert!(
        out.events
            .iter()
            .any(|event| matches!(event, AgentEvent::ClarificationNeeded { .. })),
        "the paused attempt's evidence is preserved: {:?}",
        out.events
    );
    assert_eq!(
        out.usage.input_tokens, 4,
        "the earlier paid usage survives on the paused output"
    );
    assert_eq!(out.usage.output_tokens, 2, "same, for output tokens");
    assert!(
        !out.truncated,
        "a settled clarification is not a truncation: {:?}",
        out.truncated
    );
}
