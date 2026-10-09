use super::*;

fn one_request_budget() -> Budgets {
    let mut budget = Budgets::default();
    budget.turns = Some(1);
    budget
}

#[tokio::test]
async fn a_new_step_initializes_a_fresh_count_grant() {
    let run = approved_run("count-next-step").await;
    let provider = ScriptProvider::new(vec![Turn::Answer("step one"), Turn::Answer("step two")]);
    let approval = AllowApproval;
    let tools: Arc<dyn ToolExecutor> = Arc::new(RecordingTools::default());
    let toolsets = vec![
        toolset(Arc::clone(&tools), Vec::new()),
        toolset(tools, Vec::new()),
    ];
    let (sink, driver) = driver_and_sink(&run, &provider, &approval, &toolsets, false);
    let plan = RunPlan::new(vec![
        step_with_budget("first independent grant", Some(one_request_budget())),
        step_with_budget("second independent grant", Some(one_request_budget())),
    ])
    .unwrap();

    for step in 0..plan.steps.len() {
        driver
            .run_step(&sink, &plan, step, &run.workspace())
            .await
            .unwrap();
    }

    let events = Journal::open(&run.run_dir).read().unwrap();
    let expected = vec![(Some(0), Some(0)), (Some(1), None), (None, Some(0))];
    assert_eq!(count_pairs_for_step(&events, 0), expected);
    assert_eq!(count_pairs_for_step(&events, 1), expected);
    assert_eq!(provider.requests().len(), 2);
    assert_eq!(sink.state(), RunState::Completed);
    let _ = fs::remove_dir_all(run.root);
}
