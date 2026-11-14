//! The progress guards on a run step: a step that repeats one call with the
//! same result, or makes no progress for its declared turns, stops the loop
//! and pauses the run. The model still gets its final turn, so the step's
//! best state is written before the pause.

use super::*;

/// One step whose budget declares only the repeat guard; two identical
/// results are noted, the third stops, and the final turn consumes the
/// scripted answer.
#[tokio::test]
async fn a_repeating_step_pauses_the_run() {
    let run = approved_run("progress-repeat").await;
    let provider = ScriptProvider::new(vec![
        Turn::Tools(vec![call("probe")]),
        Turn::Tools(vec![call("probe")]),
        Turn::Tools(vec![call("probe")]),
        Turn::Answer("final"),
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
    budget.no_progress_turns = Some(0);
    budget.repeat_limit = Some(2);
    let plan = RunPlan::new(vec![step_with_budget("probe", Some(budget))]).unwrap();

    assert!(matches!(
        driver.run_step(&sink, &plan, 0, &workspace).await,
        Err(EpisodeError::BudgetStopped {
            reason: PauseReason::BudgetExhausted,
            ..
        })
    ));
    assert_eq!(
        provider.requests().len(),
        4,
        "three tool turns plus the final turn"
    );
    assert_eq!(sink.state(), RunState::Paused);
    let _ = fs::remove_dir_all(run.root);
}

/// A step that spends its declared no-progress turns stops and pauses; the
/// repeat guard is off, so the identical calls are not what stopped it.
#[tokio::test]
async fn a_step_without_progress_pauses_the_run() {
    let run = approved_run("progress-none").await;
    let provider = ScriptProvider::new(vec![
        Turn::Tools(vec![call("probe")]),
        Turn::Tools(vec![call("probe")]),
        Turn::Answer("final"),
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
    budget.no_progress_turns = Some(2);
    budget.repeat_limit = Some(0);
    let plan = RunPlan::new(vec![step_with_budget("probe", Some(budget))]).unwrap();

    assert!(matches!(
        driver.run_step(&sink, &plan, 0, &workspace).await,
        Err(EpisodeError::BudgetStopped {
            reason: PauseReason::BudgetExhausted,
            ..
        })
    ));
    assert_eq!(
        provider.requests().len(),
        3,
        "two no-progress turns plus the final turn"
    );
    assert_eq!(sink.state(), RunState::Paused);
    let _ = fs::remove_dir_all(run.root);
}
