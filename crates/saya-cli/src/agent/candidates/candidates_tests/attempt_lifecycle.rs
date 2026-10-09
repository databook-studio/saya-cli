#[tokio::test]
async fn cancellation_between_attempts_stops_early() {
    let token = CancellationToken::new();
    let sql = "SELECT a FROM t WHERE id = 1";
    // Three attempts queued, but the runner cancels the token once the second
    // call completes — the check before the third attempt must stop the loop.
    let runner = ScriptedRunner::new(vec![
        Outcome::Ok(output_with_sql(sql)),
        Outcome::Ok(output_with_sql(sql)),
        Outcome::Ok(output_with_sql(sql)),
    ])
    .cancel_after(2, token.clone());
    let mut results = HashMap::new();
    results.insert(sql.to_string(), rows_result(&["a"], vec![json!([1])]));
    let executor = ScriptedExecutor { results };
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
    };

    orchestrate_test(3, &runner, &executor, &sink, &token)
        .await
        .expect("partial run completes");

    // Only two attempts ran: the cancellation between the second and third
    // stopped the loop before the third could start.
    assert_eq!(
        runner.calls(),
        2,
        "cancellation between attempts stopped the loop early (2 of 3 ran)"
    );
    assert_eq!(
        consensus_count(&events.lock().expect("events lock")),
        0,
        "a stopped question never starts vote or consensus work"
    );
}

#[tokio::test]
async fn selected_candidate_retains_all_settled_attempt_usage() {
    let sql = "SELECT a FROM t WHERE id = 1";
    let runner = ScriptedRunner::new(vec![
        Outcome::Ok(output_with_usage(sql, 2, 3)),
        Outcome::Ok(output_with_usage(sql, 5, 7)),
    ]);
    let executor = ScriptedExecutor {
        results: HashMap::from([(sql.to_string(), rows_result(&["a"], vec![json!([1])]))]),
    };
    let sink = RecordingSink {
        events: Arc::new(Mutex::new(Vec::new())),
    };

    let cancellation = CancellationToken::new();
    let scope = test_scope(cancellation.clone());
    let receipts = AnsweringReceiptSink::new(&sink);
    receipts
        .emit(AgentEvent::usage(
            saya_agent::UsageCall::Answer,
            TokenUsage::new(2, 3),
        ))
        .await;
    receipts
        .emit(AgentEvent::usage(
            saya_agent::UsageCall::Answer,
            TokenUsage::new(5, 7),
        ))
        .await;
    let output = orchestrate(2, &runner, &executor, D, &receipts, &cancellation, &scope)
        .await
        .expect("two settled candidates decide");

    assert_eq!(
        output.usage.input_tokens, 7,
        "both paid input receipts survive"
    );
    assert_eq!(
        output.usage.output_tokens, 10,
        "both paid output receipts survive"
    );
}

// ===========================================================================
// Extra: an attempt that succeeds but designates no SQL still runs and
// contributes no nomination — the survivors decide, and the event reports the
// voting attempts honestly. Pins the None-nomination handling end to end.
// ===========================================================================

#[tokio::test]
async fn an_attempt_without_designated_sql_contributes_no_nomination() {
    let sql = "SELECT a FROM t WHERE id = 1";
    let runner = ScriptedRunner::new(vec![
        Outcome::Ok(output_with_sql(sql)),
        Outcome::Ok(output_without_sql("no-designation")),
        Outcome::Ok(output_with_sql(sql)),
    ]);
    let mut results = HashMap::new();
    results.insert(sql.to_string(), rows_result(&["a"], vec![json!([1])]));
    let executor = ScriptedExecutor { results };
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
    };

    let out = orchestrate_test(3, &runner, &executor, &sink, &CancellationToken::new())
        .await
        .expect("attempts succeed");

    let captured = events.lock().expect("events lock");
    let event = one_consensus(&captured).expect("the two designating attempts decide");
    // 3 attempts ran; only 2 nominated (and voted) — the no-designation one
    // neither nominated nor voted.
    assert_consensus(event, Some(sql), 3, 2, 2, false);
    assert_eq!(out.answer_sql.as_deref(), Some(sql));
}
