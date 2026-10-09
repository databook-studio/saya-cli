use super::*;

#[tokio::test]
async fn an_incomplete_workspace_step_requires_explicit_retry() {
    let mut capabilities = Capabilities::default();
    capabilities.workspace_write = true;
    let risky = StepSpec::new("write the report", capabilities, None, Vec::new(), None).unwrap();
    let run = crashed_run(
        "uncertain-workspace",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
        ],
        RunStatus::Executing,
        RunPlan::new(vec![risky]).unwrap(),
    )
    .await;
    let expected = ResumeOutcome::IncompleteEffects {
        step: 0,
        goal: "write the report".into(),
        effects: vec![saya_harness::engine::ResumeEffect::WorkspaceWrite],
    };

    let first = resume(&run.run_dir, run.inputs()).await.unwrap();

    assert_eq!(first, expected);
    assert_eq!(run.stubs.provider.count(), 0);
    assert_eq!(run.tool_calls.load(Ordering::Relaxed), 0);
    let first_journal = run.journal();
    assert_eq!(
        first_journal.last(),
        Some(&RunEvent::Paused {
            reason: PauseReason::ProcessDeath
        })
    );
    assert!(!first_journal.contains(&RunEvent::StepCompleted { step: 0 }));

    let repeated = resume(&run.run_dir, run.inputs()).await.unwrap();

    assert_eq!(repeated, expected);
    assert_eq!(run.stubs.provider.count(), 0);
    assert_eq!(run.tool_calls.load(Ordering::Relaxed), 0);
    assert_eq!(run.journal(), first_journal);
}

#[tokio::test]
async fn an_incomplete_failed_scratch_step_is_also_uncertain() {
    let mut capabilities = Capabilities::default();
    capabilities.scratch = true;
    let plan = RunPlan::new(vec![
        StepSpec::new("stage the data", capabilities, None, Vec::new(), None).unwrap(),
    ])
    .unwrap();
    let run = crashed_run(
        "failed-scratch",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepFailed { step: 0 },
            RunEvent::Paused {
                reason: PauseReason::ProcessDeath,
            },
        ],
        RunStatus::Paused,
        plan,
    )
    .await;

    let outcome = resume(&run.run_dir, run.inputs()).await.unwrap();

    assert_eq!(
        outcome,
        ResumeOutcome::IncompleteEffects {
            step: 0,
            goal: "stage the data".into(),
            effects: vec![saya_harness::engine::ResumeEffect::Scratch],
        }
    );
    assert_eq!(run.stubs.provider.count(), 0);
    assert_eq!(
        run.journal().last(),
        Some(&RunEvent::Paused {
            reason: PauseReason::ProcessDeath
        })
    );
}

#[tokio::test]
async fn an_unavailable_step_toolset_fails_closed_as_unknown() {
    let mut run = crashed_run(
        "unknown-toolset",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
        ],
        RunStatus::Executing,
        RunPlan::new(vec![step("unknown capability")]).unwrap(),
    )
    .await;
    run.toolsets.clear();

    let outcome = resume(&run.run_dir, run.inputs()).await.unwrap();

    assert_eq!(
        outcome,
        ResumeOutcome::IncompleteEffects {
            step: 0,
            goal: "unknown capability".into(),
            effects: vec![saya_harness::engine::ResumeEffect::Unknown],
        }
    );
    assert_eq!(run.stubs.provider.count(), 0);
}
