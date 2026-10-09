use super::*;

fn count_pairs(events: &[RunEvent]) -> Vec<(Option<u64>, Option<u64>)> {
    events
        .iter()
        .filter_map(|event| match event {
            RunEvent::Usage {
                tokens: None,
                turns,
                tool_calls,
                cached_input_tokens: None,
                cache_creation_input_tokens: None,
                ..
            } if turns.is_some() || tool_calls.is_some() => Some((*turns, *tool_calls)),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn retry_uses_only_remaining_step_request_and_tool_grants() {
    let run = approved_run("count-retry").await;
    let first = call("probe");
    let duplicate = ToolCall {
        id: first.id.clone(),
        name: "probe".into(),
        arguments: serde_json::json!({}),
    };
    let provider = ScriptProvider::new(vec![
        Turn::Tools(vec![first, duplicate]),
        Turn::Tools(vec![call("probe")]),
        Turn::Answer("must not be admitted"),
    ]);
    let tools = Arc::new(RecordingTools::default());
    let executor: Arc<dyn ToolExecutor> = tools.clone();
    let approval = AllowApproval;
    let toolsets = vec![toolset(
        executor,
        vec![def("probe", LocalStateEffect::None)],
    )];
    let (sink, driver) = driver_and_sink(&run, &provider, &approval, &toolsets, false);
    let mut budget = Budgets::default();
    budget.turns = Some(2);
    budget.tool_calls = Some(1);
    let plan = RunPlan::new(vec![step_with_budget(
        "keep the remaining grant",
        Some(budget),
    )])
    .unwrap();

    let error = driver
        .run_step(&sink, &plan, 0, &run.workspace())
        .await
        .unwrap_err();

    assert!(matches!(error, EpisodeError::BudgetStopped { .. }));
    assert_eq!(
        provider.requests().len(),
        2,
        "the third request exceeds the step grant"
    );
    assert!(
        tools.calls().is_empty(),
        "an exhausted tool grant starts no effect"
    );
    assert_eq!(sink.state(), RunState::Paused);
    assert_eq!(
        count_pairs(&Journal::open(&run.run_dir).read().unwrap()),
        vec![
            (Some(0), Some(0)),
            (Some(1), None),
            (None, Some(2)),
            (Some(1), None),
            (None, Some(1)),
        ],
        "the retry's source observations append to the same step totals"
    );
    let _ = fs::remove_dir_all(run.root);
}
