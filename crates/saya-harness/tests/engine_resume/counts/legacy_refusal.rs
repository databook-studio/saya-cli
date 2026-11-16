//! The legacy journal's count budget: a started step with a finite count
//! budget and no initialization row is refused before the provider runs,
//! and uncertain effects outrank the missing count evidence.

use super::rig::{bounded_step, count_rows_for_step, inputs};
use super::stubs::{ResumeTurn, ScriptedAnswerProvider};
use super::*;
use saya_types::Budgets;

#[tokio::test]
async fn legacy_started_finite_count_budget_refuses_unknown_before_provider() {
    let plan = RunPlan::new(vec![bounded_step("legacy incomplete step", 2, None)]).unwrap();
    let run = crashed_run(
        "legacy-counts",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
        ],
        RunStatus::Executing,
        plan,
    )
    .await;
    let provider = ScriptedAnswerProvider::new(vec![ResumeTurn::Answer("must not run")]);

    let error = resume(&run.run_dir, inputs(&run, &provider, None))
        .await
        .unwrap_err();

    assert!(
        error.to_string().contains("count evidence"),
        "unexpected refusal: {error}"
    );
    assert_eq!(provider.count(), 0);
    assert!(count_rows_for_step(&run.journal(), 0).is_empty());

    let mut capabilities = Capabilities::default();
    capabilities.workspace_write = true;
    let mut budget = Budgets::default();
    budget.turns = Some(2);
    let effectful_plan = RunPlan::new(vec![
        StepSpec::new(
            "uncertain effect outranks missing count evidence",
            capabilities,
            Some(budget),
            Vec::new(),
            None,
        )
        .unwrap(),
    ])
    .unwrap();
    let effectful = crashed_run(
        "legacy-counts-uncertain",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
        ],
        RunStatus::Executing,
        effectful_plan,
    )
    .await;
    let provider = ScriptedAnswerProvider::new(vec![ResumeTurn::Answer("must not run")]);
    assert!(matches!(
        resume(&effectful.run_dir, inputs(&effectful, &provider, None)).await,
        Ok(ResumeOutcome::IncompleteEffects { step: 0, .. })
    ));
    assert_eq!(
        provider.count(),
        0,
        "H1 effect uncertainty retains precedence"
    );
}
