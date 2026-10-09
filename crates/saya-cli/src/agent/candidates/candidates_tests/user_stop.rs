/// A User stop leaves the retained output untouched — no partial marking, no
/// typed event; the ask adapter classifies a user stop from the token itself.
#[tokio::test]
async fn a_user_stop_leaves_the_retained_output_untouched() {
    let token = CancellationToken::new();
    let sql = "SELECT a FROM t WHERE id = 1";
    let runner = ScriptedRunner::new(vec![
        Outcome::Ok(output_with_sql(sql)),
        Outcome::Ok(output_with_sql(sql)),
    ])
    .cancel_after(1, token.clone());
    let mut results = HashMap::new();
    results.insert(sql.to_string(), rows_result(&["a"], vec![json!([1])]));
    let executor = ScriptedExecutor { results };
    let sink = RecordingSink {
        events: Arc::new(Mutex::new(Vec::new())),
    };

    let out = orchestrate_test(2, &runner, &executor, &sink, &token)
        .await
        .expect("the retained answer returns");

    assert!(
        !out.truncated,
        "a user stop does not mark the answer partial"
    );
    assert!(
        !out.events
            .iter()
            .any(|event| matches!(event, AgentEvent::AnsweringBudgetStopped { .. })),
        "a user stop never fabricates a budget-stop event: {:?}",
        out.events
    );
}
