use super::*;

/// The step's budget becomes the loop's `AgentLimits`, and
/// `SAYA_AGENT_MAX_TURNS` set in the environment changes nothing.
#[tokio::test]
async fn the_step_budget_becomes_the_limits_and_the_environment_changes_nothing() {
    unsafe { std::env::set_var("SAYA_AGENT_MAX_TURNS", "99") };

    // A two-request ceiling with a model that always wants a tool: the
    // salvage request is counted too, so it cannot start after both requests
    // have been admitted, regardless of the environment-derived value.
    let run = approved_run("budget-turns").await;
    let provider = ScriptProvider::new(vec![
        Turn::Tools(vec![call("probe")]),
        Turn::Tools(vec![call("probe")]),
        Turn::Answer("salvaged"),
    ]);
    let approval = AllowApproval;
    let tools = Arc::new(RecordingTools::default());
    let toolsets = vec![toolset(
        tools.clone(),
        vec![def("probe", LocalStateEffect::None)],
    )];
    let (sink, driver) = driver_and_sink(&run, &provider, &approval, &toolsets, false);
    let workspace = run.workspace();
    let mut budget = Budgets::default();
    budget.turns = Some(2);
    budget.tool_calls = Some(99);
    let plan = RunPlan::new(vec![step_with_budget("probe twice", Some(budget))]).unwrap();

    assert!(matches!(
        driver.run_step(&sink, &plan, 0, &workspace).await,
        Err(EpisodeError::BudgetStopped { .. })
    ));

    assert_eq!(
        provider.requests().len(),
        2,
        "the step's request ceiling includes salvage and ignores the environment"
    );
    assert!(
        provider
            .requests()
            .iter()
            .all(|request| !request.tools.is_empty()),
        "the refused third request cannot be a salvage call"
    );
    assert_eq!(tools.calls().len(), 2);
    assert_eq!(sink.state(), RunState::Paused);
    let _ = fs::remove_dir_all(run.root);

    // The tool-call ceiling stops the projected second effect and prevents
    // the follow-up salvage request from starting after that budget stop.
    let run = approved_run("budget-tools").await;
    let provider = ScriptProvider::new(vec![
        Turn::Tools(vec![call("probe")]),
        Turn::Tools(vec![call("probe")]),
        Turn::Answer("salvaged"),
    ]);
    let approval = AllowApproval;
    let tools = Arc::new(RecordingTools::default());
    let toolsets = vec![toolset(
        tools.clone(),
        vec![def("probe", LocalStateEffect::None)],
    )];
    let (sink, driver) = driver_and_sink(&run, &provider, &approval, &toolsets, false);
    let workspace = run.workspace();
    let mut budget = Budgets::default();
    budget.tool_calls = Some(1);
    let plan = RunPlan::new(vec![step_with_budget("probe once", Some(budget))]).unwrap();

    assert!(matches!(
        driver.run_step(&sink, &plan, 0, &workspace).await,
        Err(EpisodeError::BudgetStopped { .. })
    ));

    assert_eq!(
        provider.requests().len(),
        2,
        "the tool ceiling must bind regardless of the environment"
    );
    assert_eq!(tools.calls().len(), 1, "the over-limit effect never starts");
    assert_eq!(sink.state(), RunState::Paused);
    unsafe { std::env::remove_var("SAYA_AGENT_MAX_TURNS") };
    let _ = fs::remove_dir_all(run.root);
}
