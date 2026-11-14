/// A loop budget-stop is kept once: the token's Budget reason must not append
/// a second typed event to an output that already carries its own.
#[tokio::test]
async fn a_retained_output_already_budget_stopped_is_not_marked_twice() {
    let token = CancellationToken::new();
    let sql = "SELECT a FROM t WHERE id = 1";
    let runner = ScriptedRunner::new(vec![
        Outcome::Ok(budget_stopped_output(sql)),
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
    let sink = RecordingSink {
        events: Arc::new(Mutex::new(Vec::new())),
    };

    let out = orchestrate_test(2, &runner, &executor, &sink, &token)
        .await
        .expect("the retained answer returns");

    assert!(out.truncated, "the loop's own partial marking stands");
    let stops = out
        .events
        .iter()
        .filter(|event| matches!(event, AgentEvent::AnsweringBudgetStopped { .. }))
        .count();
    assert_eq!(
        stops, 1,
        "the loop's budget-stop event is kept, not duplicated: {:?}",
        out.events
    );
}

/// A legacy truncation is never relabelled as a configured-budget stop: the
/// token's Budget reason must not fabricate a typed event onto an output that
/// was already truncated for some other reason.
#[tokio::test]
async fn a_legacy_truncated_output_is_not_relabelled_as_a_budget_stop() {
    let token = CancellationToken::new();
    let sql = "SELECT a FROM t WHERE id = 1";
    let runner = ScriptedRunner::new(vec![
        Outcome::Ok(legacy_truncated_output(sql)),
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
    let sink = RecordingSink {
        events: Arc::new(Mutex::new(Vec::new())),
    };

    let out = orchestrate_test(2, &runner, &executor, &sink, &token)
        .await
        .expect("the retained answer returns");

    assert!(out.truncated, "the legacy truncation stands");
    assert!(
        !out.events
            .iter()
            .any(|event| matches!(event, AgentEvent::AnsweringBudgetStopped { .. })),
        "a legacy truncation is not relabelled as a configured-budget stop: {:?}",
        out.events
    );
}
