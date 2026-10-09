//! The carry the public resume performs on the same incomplete step: the
//! already-admitted request's counts stand in the source-owned journal, a
//! held completion proves the first response is never admitted twice, and
//! the budget-stopped resume admits only the remaining request.

use super::rig::{bounded_step, count_rows_for_step, inputs};
use super::stubs::{CountedTool, HoldComplete, ResumeTurn, ScriptedAnswerProvider};
use super::*;
use saya_agent::AgentEventSink;

#[tokio::test]
async fn source_counts_carry_through_public_resume_of_same_incomplete_step() {
    let plan = RunPlan::new(vec![
        step("completed prefix"),
        bounded_step("bounded step", 2, Some(1)),
    ])
    .unwrap();
    let mut run = crashed_run(
        "count-resume",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
            RunEvent::StepCompleted { step: 0 },
        ],
        RunStatus::Executing,
        plan,
    )
    .await;
    let tool = Arc::new(CountedTool::default());
    let executor: Arc<dyn ToolExecutor> = tool.clone();
    run.toolsets[1] = StepToolset {
        executor,
        definitions: vec![ToolDefinition {
            name: "probe".into(),
            description: "probe".into(),
            read_only: true,
            parameters: serde_json::json!({"type": "object"}),
            effect: saya_agent::ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: false,
                local_state: LocalStateEffect::None,
            },
            concurrency: saya_agent::ToolConcurrency::Serial,
            completion: None,
        }],
    };
    let provider = ScriptedAnswerProvider::new(vec![
        ResumeTurn::Answer("first admitted response"),
        ResumeTurn::Tool(ToolCall {
            id: "probe-1".into(),
            name: "probe".into(),
            arguments: serde_json::json!({}),
        }),
        ResumeTurn::Answer("must not be admitted"),
    ]);
    let (started, reached) = oneshot::channel();
    let stream: Arc<dyn AgentEventSink> = Arc::new(HoldComplete(Mutex::new(Some(started))));

    {
        let active = resume(&run.run_dir, inputs(&run, &provider, Some(stream)));
        tokio::pin!(active);
        tokio::select! {
            result = &mut active => panic!("the first response should be held before step completion: {result:?}"),
            result = reached => result.expect("the actual episode reaches its completion event"),
        }
    }

    assert_eq!(
        count_rows_for_step(&run.journal(), 1),
        vec![(Some(0), Some(0)), (Some(1), None), (None, Some(0))],
        "a completed zero-call batch is known zero in source-owned journal rows"
    );

    let error = resume(&run.run_dir, inputs(&run, &provider, None))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ResumeError::Episode {
            source: saya_harness::engine::EpisodeError::BudgetStopped { .. }
        }
    ));
    assert_eq!(
        provider.count(),
        2,
        "resume admits only the remaining request"
    );
    assert_eq!(
        tool.0.load(Ordering::SeqCst),
        1,
        "the already-admitted tool runs once"
    );
    assert_eq!(
        count_rows_for_step(&run.journal(), 1),
        vec![
            (Some(0), Some(0)),
            (Some(1), None),
            (None, Some(0)),
            (Some(1), None),
            (None, Some(1)),
        ],
        "the repeated initialization is a no-op and the resumed delta carries forward"
    );
    assert!(matches!(
        run.journal().last(),
        Some(RunEvent::Paused {
            reason: saya_types::PauseReason::BudgetExhausted
        })
    ));
}
