use super::*;

#[tokio::test]
async fn a_failed_request_settles_as_an_empty_batch_and_the_step_retries() {
    let run = approved_run("count-settle").await;
    let provider = ScriptProvider::new(vec![Turn::Fail, Turn::Answer("recovered")]);
    let approval = AllowApproval;
    let toolsets = vec![toolset(Arc::new(RecordingTools::default()), Vec::new())];
    let (sink, driver) = driver_and_sink(&run, &provider, &approval, &toolsets, false);
    let mut budget = Budgets::default();
    budget.turns = Some(3);
    let plan = RunPlan::new(vec![step_with_budget(
        "fail once then answer",
        Some(budget),
    )])
    .unwrap();

    driver
        .run_step(&sink, &plan, 0, &run.workspace())
        .await
        .unwrap();

    assert_eq!(provider.requests().len(), 2);
    assert_eq!(
        count_pairs_for_step(&Journal::open(&run.run_dir).read().unwrap(), 0),
        vec![
            (Some(0), Some(0)),
            (Some(1), None),
            (None, Some(0)),
            (Some(1), None),
            (None, Some(0)),
        ],
        "the failed request closes as an empty batch before the retry admits"
    );
    assert_eq!(sink.state(), RunState::Completed);
    let _ = fs::remove_dir_all(run.root);
}

#[tokio::test]
async fn a_provider_failing_every_attempt_exhausts_normally_not_count_evidence() {
    let run = approved_run("count-exhaust").await;
    let provider = ScriptProvider::new(vec![Turn::Fail, Turn::Fail, Turn::Fail]);
    let approval = AllowApproval;
    let toolsets = vec![toolset(Arc::new(RecordingTools::default()), Vec::new())];
    let (sink, driver) = driver_and_sink(&run, &provider, &approval, &toolsets, false);
    let mut budget = Budgets::default();
    budget.turns = Some(3);
    let plan = RunPlan::new(vec![step_with_budget("always fails", Some(budget))]).unwrap();

    let error = driver
        .run_step(&sink, &plan, 0, &run.workspace())
        .await
        .unwrap_err();

    assert!(
        matches!(error, EpisodeError::StepExhausted { .. }),
        "the normal exhausted path, never CountEvidence: {error:?}"
    );
    assert_eq!(sink.state(), RunState::Paused);
    assert_eq!(
        count_pairs_for_step(&Journal::open(&run.run_dir).read().unwrap(), 0),
        vec![
            (Some(0), Some(0)),
            (Some(1), None),
            (None, Some(0)),
            (Some(1), None),
            (None, Some(0)),
            (Some(1), None),
            (None, Some(0)),
        ],
        "each failed attempt's request closes as an empty batch"
    );
    let _ = fs::remove_dir_all(run.root);
}
