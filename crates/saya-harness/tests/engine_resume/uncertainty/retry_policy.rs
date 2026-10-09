use super::*;

/// Explicit retry begins only the incomplete suffix: a completed prefix stays
/// authoritative even when the resumed operator opts into retrying uncertainty.
#[tokio::test]
async fn explicit_retry_runs_only_the_incomplete_step() {
    let mut capabilities = Capabilities::default();
    capabilities.workspace_write = true;
    let plan = RunPlan::new(vec![
        StepSpec::new(
            "completed write",
            capabilities.clone(),
            None,
            Vec::new(),
            None,
        )
        .unwrap(),
        StepSpec::new("incomplete write", capabilities, None, Vec::new(), None).unwrap(),
    ])
    .unwrap();
    let run = crashed_run(
        "retry-incomplete",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
            RunEvent::StepCompleted { step: 0 },
            RunEvent::StepStarted { step: 1 },
        ],
        RunStatus::Executing,
        plan,
    )
    .await;
    let mut inputs = run.inputs();
    inputs.incomplete_policy = saya_harness::engine::IncompletePolicy::Retry;

    let outcome = resume(&run.run_dir, inputs).await.unwrap();

    assert_eq!(
        outcome,
        ResumeOutcome::Resumed {
            first_step: 1,
            state: RunState::Completed
        }
    );
    assert_eq!(run.stubs.provider.count(), 1);
    let events = run.journal();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, RunEvent::StepStarted { step: 0 }))
            .count(),
        1,
        "the durable completed prefix must not repeat"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, RunEvent::StepStarted { step: 1 }))
            .count(),
        2,
        "the explicit retry is represented by a new StepStarted"
    );
}

/// This establishes retry eligibility for a database-data-only definition.
/// The SQL tool is not called; the provider returns a direct answer.
#[tokio::test]
async fn a_database_data_only_definition_does_not_require_retry_opt_in() {
    let plan = RunPlan::new(vec![step("inspect the data")]).unwrap();
    let mut run = crashed_run(
        "fresh-observation",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
        ],
        RunStatus::Executing,
        plan,
    )
    .await;
    run.toolsets[0].definitions.push(ToolDefinition {
        name: "bounded_sql_query".into(),
        description: String::new(),
        read_only: false,
        parameters: serde_json::json!({"type": "object"}),
        effect: ToolEffect {
            database_data: true,
            external_side_effect: false,
            requires_approval: true,
            local_state: LocalStateEffect::None,
        },
        concurrency: saya_agent::ToolConcurrency::Serial,
        completion: None,
    });

    let outcome = resume(&run.run_dir, run.inputs()).await.unwrap();

    assert_eq!(
        outcome,
        ResumeOutcome::Resumed {
            first_step: 0,
            state: RunState::Completed
        }
    );
    assert_eq!(run.stubs.provider.count(), 1);
}

#[tokio::test]
async fn read_only_and_approval_flags_do_not_hide_declared_external_effects() {
    let mut run = crashed_run(
        "read-only-external-effect",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
        ],
        RunStatus::Executing,
        RunPlan::new(vec![step("inspect and publish")]).unwrap(),
    )
    .await;
    run.toolsets[0].definitions.push(ToolDefinition {
        name: "publish_report".into(),
        description: String::new(),
        read_only: true,
        parameters: serde_json::json!({"type": "object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: true,
            requires_approval: false,
            local_state: LocalStateEffect::None,
        },
        concurrency: saya_agent::ToolConcurrency::Serial,
        completion: None,
    });

    let outcome = resume(&run.run_dir, run.inputs()).await.unwrap();

    assert_eq!(
        outcome,
        ResumeOutcome::IncompleteEffects {
            step: 0,
            goal: "inspect and publish".into(),
            effects: vec![saya_harness::engine::ResumeEffect::ExternalSideEffect],
        }
    );
    assert_eq!(run.stubs.provider.count(), 0);
}
