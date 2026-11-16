#[tokio::test]
async fn a_budget_stop_before_verification_marks_the_retained_output_partial_with_a_typed_event() {
    let token = CancellationToken::new();
    let sql = "SELECT a FROM t WHERE id = 1";
    // Attempt 1 settles naturally; the elapsed budget latches the token the
    // moment it completes — before any verification could begin.
    let runner = ScriptedRunner::new(vec![
        Outcome::Ok(output_with_sql(sql)),
        Outcome::Ok(output_with_sql(sql)),
    ])
    .cancel_after_with(
        1,
        token.clone(),
        CancellationReason::Budget(BudgetDimension::ElapsedTime),
    );
    let mut results = HashMap::new();
    results.insert(sql.to_string(), rows_result(&["a"], vec![json!([1])]));
    let executor = ScriptedExecutor { results };
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
    };

    let out = orchestrate_test(2, &runner, &executor, &sink, &token)
        .await
        .expect("the retained answer returns");

    assert_eq!(
        runner.calls(),
        1,
        "the stop between attempts kept attempt 2 from starting"
    );
    assert!(
        out.truncated,
        "the retained answer is partial, not a natural completion"
    );
    assert!(
        out.events.iter().any(|event| matches!(
            event,
            AgentEvent::AnsweringBudgetStopped {
                dimension: BudgetDimension::ElapsedTime
            }
        )),
        "the retained events carry the typed budget stop for the real dimension: {:?}",
        out.events
    );
    assert_eq!(
        consensus_count(&events.lock().expect("events lock")),
        0,
        "no consensus may be fabricated over a stopped run"
    );
}

// ===========================================================================
// A budget stop during the vote's verification work: the decision dies with
// the stop, the consensus emit is skipped, and the retained answer comes back
// partial with the typed event — not as a natural completion.
// ===========================================================================

#[tokio::test]
async fn a_budget_stop_during_the_vote_returns_the_answer_as_partial_with_no_consensus() {
    let token = CancellationToken::new();
    let sql = "SELECT a FROM t WHERE id = 1";
    let runner = ScriptedRunner::new(vec![
        Outcome::Ok(output_with_sql(sql)),
        Outcome::Ok(output_with_sql(sql)),
    ]);
    let mut results = HashMap::new();
    results.insert(sql.to_string(), rows_result(&["a"], vec![json!([1])]));
    let executor = StopDuringVoteExecutor {
        calls: AtomicUsize::new(0),
        results,
        reason: CancellationReason::Budget(BudgetDimension::ElapsedTime),
        token: token.clone(),
    };
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
    };

    let out = orchestrate_test(2, &runner, &executor, &sink, &token)
        .await
        .expect("the retained answer returns");

    assert_eq!(runner.calls(), 2, "both attempts settled naturally");
    assert!(executor.calls() >= 1, "the vote ran its verification work");
    assert!(
        out.truncated,
        "the retained answer is partial, not a natural completion"
    );
    assert!(
        out.events.iter().any(|event| matches!(
            event,
            AgentEvent::AnsweringBudgetStopped {
                dimension: BudgetDimension::ElapsedTime
            }
        )),
        "the retained events carry the typed budget stop: {:?}",
        out.events
    );
    assert_eq!(
        consensus_count(&events.lock().expect("events lock")),
        0,
        "a stopped question never emits a consensus"
    );
}
