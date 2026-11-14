#[tokio::test]
async fn one_candidate_runs_once_and_emits_no_consensus_event() {
    let runner = ScriptedRunner::new(vec![Outcome::Ok(output_with_sql(
        "SELECT a FROM t WHERE id = 1",
    ))]);
    let executor = ScriptedExecutor {
        results: HashMap::new(),
    };
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
    };

    let out = orchestrate_test(1, &runner, &executor, &sink, &CancellationToken::new())
        .await
        .expect("single attempt succeeds");

    // Proved by counting: exactly one attempt ran …
    assert_eq!(runner.calls(), 1, "the agent ran exactly once");
    // … and no consensus event was emitted (the orchestration never decided).
    assert_eq!(
        consensus_count(&events.lock().expect("events lock")),
        0,
        "no consensus event for a single candidate"
    );
    // The single attempt's output is returned unchanged.
    assert_eq!(
        out.answer_sql.as_deref(),
        Some("SELECT a FROM t WHERE id = 1")
    );
}

// ===========================================================================
// 2. candidates = 3, all attempts agreeing → one event, votes: 3, tied: false,
// and the winning SQL present.
// ===========================================================================

#[tokio::test]
async fn three_candidates_all_agree_emit_one_event_with_three_votes() {
    let sql = "SELECT a FROM t WHERE id = 1";
    let runner = ScriptedRunner::new(vec![
        Outcome::Ok(output_with_sql(sql)),
        Outcome::Ok(output_with_sql(sql)),
        Outcome::Ok(output_with_sql(sql)),
    ]);
    // Every attempt nominated the same SQL, so the executor returns the same
    // result for it — one fingerprint group of three.
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

    assert_eq!(runner.calls(), 3, "all three attempts ran");
    let captured = events.lock().expect("events lock");
    assert_eq!(consensus_count(&captured), 1, "exactly one consensus event");
    let event = one_consensus(&captured).expect("the consensus event");
    assert_consensus(event, Some(sql), 3, 3, 3, false);
    // The winning attempt's output is returned.
    assert_eq!(out.answer_sql.as_deref(), Some(sql));
}

// ===========================================================================
// 3. candidates = 3 with a disagreement → the majority wins.
// ===========================================================================

#[tokio::test]
async fn three_candidates_disagree_and_the_majority_wins() {
    let majority = "SELECT a FROM t WHERE id = 1";
    let minority = "SELECT a FROM t WHERE id = 2";
    let runner = ScriptedRunner::new(vec![
        Outcome::Ok(output_with_sql(majority)),
        Outcome::Ok(output_with_sql(majority)),
        Outcome::Ok(output_with_sql(minority)),
    ]);
    let mut results = HashMap::new();
    results.insert(majority.to_string(), rows_result(&["a"], vec![json!([1])]));
    results.insert(minority.to_string(), rows_result(&["a"], vec![json!([2])]));
    let executor = ScriptedExecutor { results };
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
    };

    let out = orchestrate_test(3, &runner, &executor, &sink, &CancellationToken::new())
        .await
        .expect("attempts succeed");

    let captured = events.lock().expect("events lock");
    let event = one_consensus(&captured).expect("one consensus event");
    assert_consensus(event, Some(majority), 3, 3, 2, false);
    // The majority attempt's output is returned, not the minority's.
    assert_eq!(out.answer_sql.as_deref(), Some(majority));
}

// ===========================================================================
// 4. attempts that all disagree → event emitted with tied: true and sql: None.
// ===========================================================================

#[tokio::test]
async fn all_attempts_disagree_emit_tied_event_with_no_sql() {
    let a = "SELECT a FROM t WHERE id = 1";
    let b = "SELECT a FROM t WHERE id = 2";
    let c = "SELECT a FROM t WHERE id = 3";
    let runner = ScriptedRunner::new(vec![
        Outcome::Ok(output_with_sql(a)),
        Outcome::Ok(output_with_sql(b)),
        Outcome::Ok(output_with_sql(c)),
    ]);
    let mut results = HashMap::new();
    results.insert(a.to_string(), rows_result(&["a"], vec![json!([1])]));
    results.insert(b.to_string(), rows_result(&["a"], vec![json!([2])]));
    results.insert(c.to_string(), rows_result(&["a"], vec![json!([3])]));
    let executor = ScriptedExecutor { results };
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
    };

    let out = orchestrate_test(3, &runner, &executor, &sink, &CancellationToken::new())
        .await
        .expect("attempts succeed");

    let captured = events.lock().expect("events lock");
    let event = one_consensus(&captured).expect("a consensus event is emitted even with no winner");
    assert_consensus(event, None, 3, 3, 1, true);
    // With no winner, the last successful attempt's output is returned so the
    // caller always has an answer to show.
    assert_eq!(out.answer_sql.as_deref(), Some(c));
}

// ===========================================================================
// 5. one attempt erroring → the others still decide.
// ===========================================================================

#[tokio::test]
async fn one_attempt_erroring_lets_the_others_decide() {
    let sql = "SELECT a FROM t WHERE id = 1";
    let runner = ScriptedRunner::new(vec![
        Outcome::Ok(output_with_sql(sql)),
        Outcome::Err(AgentRuntimeError::Agent("attempt 2 failed".into())),
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
        .expect("the two successful attempts decide");

    assert_eq!(
        runner.calls(),
        3,
        "every attempt ran; the error did not abort the rest"
    );
    let captured = events.lock().expect("events lock");
    // 3 attempts ran, 2 voted (the errored attempt nominated nothing).
    let event = one_consensus(&captured).expect("the survivors still decide");
    assert_consensus(event, Some(sql), 3, 2, 2, false);
    assert_eq!(out.answer_sql.as_deref(), Some(sql));
}

// ===========================================================================
// 6. every attempt erroring → the last error is returned.
// ===========================================================================

#[tokio::test]
async fn every_attempt_erroring_returns_the_last_error() {
    let runner = ScriptedRunner::new(vec![
        Outcome::Err(AgentRuntimeError::Agent("first failure".into())),
        Outcome::Err(AgentRuntimeError::Provider("second failure".into())),
        Outcome::Err(AgentRuntimeError::Database("third failure".into())),
    ]);
    let executor = ScriptedExecutor {
        results: HashMap::new(),
    };
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
    };

    let error = orchestrate_test(3, &runner, &executor, &sink, &CancellationToken::new())
        .await
        .expect_err("all attempts failed");

    assert_eq!(runner.calls(), 3, "every attempt ran before giving up");
    // The last error is surfaced, not the first.
    assert!(
        matches!(error, AgentRuntimeError::Database(ref m) if m == "third failure"),
        "the last error is returned: {error:?}"
    );
    // No consensus event when nothing succeeded.
    assert_eq!(
        consensus_count(&events.lock().expect("events lock")),
        0,
        "no consensus event when every attempt errored"
    );
}
