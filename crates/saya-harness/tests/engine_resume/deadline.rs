use super::*;

#[tokio::test]
async fn resume_deadline_waits_for_context_tool_settlement_before_its_pause() {
    let clock = saya_harness::engine::ElapsedClock::arm_now(Duration::from_secs(1)).unwrap();
    let plan = RunPlan::new(vec![step("hold until the resumed source stops")]).unwrap();
    let run = crashed_run(
        "held-resume-deadline",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
            clock.event(),
        ],
        RunStatus::Executing,
        plan,
    )
    .await;
    let provider = ScriptProvider::new(vec![
        Turn::Tools(vec![tool_call("held_probe")]),
        Turn::Answer("done"),
    ]);
    let (started_tx, started_rx) = oneshot::channel();
    let (stopped_tx, stopped_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let toolsets = vec![StepToolset {
        executor: Arc::new(HeldResumeTool {
            started: std::sync::Mutex::new(Some(started_tx)),
            stopped: std::sync::Mutex::new(Some(stopped_tx)),
            release: std::sync::Mutex::new(Some(release_rx)),
        }),
        definitions: vec![ToolDefinition {
            name: "held_probe".into(),
            description: "bounded held source".into(),
            read_only: true,
            parameters: serde_json::json!({"type": "object"}),
            effect: ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: false,
                local_state: LocalStateEffect::None,
            },
            concurrency: saya_agent::ToolConcurrency::Serial,
            completion: None,
        }],
    }];
    let cancellation = CancellationToken::default();
    let mut inputs = run.inputs();
    inputs.incomplete_policy = saya_harness::engine::IncompletePolicy::Retry;
    inputs.collaborators.provider = &provider;
    inputs.collaborators.cancellation = cancellation.clone();
    inputs.collaborators.toolsets = &toolsets;
    inputs.wall_clock = Some(Duration::from_secs(1));
    let active = resume(&run.run_dir, inputs);
    tokio::pin!(active);

    let source_token = tokio::time::timeout(Duration::from_secs(2), async {
        tokio::select! {
            started = started_rx => started.unwrap(),
            result = &mut active => panic!("resume ended before the held source started: {result:?}"),
        }
    })
    .await
    .expect("the resumed context-aware tool starts");
    let baseline = run.journal();
    assert!(!matches!(
        baseline.last(),
        Some(
            RunEvent::Paused {
                reason: PauseReason::WallClockExceeded
            } | RunEvent::Cancelled
        )
    ));
    let observed = tokio::time::timeout(Duration::from_secs(2), async {
        tokio::select! {
            stopped = stopped_rx => stopped.unwrap(),
            result = &mut active => panic!("resume returned before the held source saw stop: {result:?}"),
        }
    })
    .await
    .expect("the independent resume timer reaches the actual tool token");
    assert_eq!(
        observed,
        CancellationReason::Budget(saya_agent::BudgetDimension::ElapsedTime)
    );
    assert_eq!(source_token.reason(), Some(observed));
    assert_eq!(
        run.journal(),
        baseline,
        "no terminal lifecycle is written while the source is held"
    );

    release_tx.send(()).unwrap();
    let result = tokio::time::timeout(Duration::from_secs(2), &mut active)
        .await
        .expect("resume awaits the same source before recording its stop")
        .unwrap_err();
    assert!(matches!(result, ResumeError::Episode { .. }));
    let events = run.journal();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event,
                RunEvent::Paused {
                    reason: PauseReason::WallClockExceeded
                }
            ))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, RunEvent::Cancelled))
            .count(),
        0
    );
}
