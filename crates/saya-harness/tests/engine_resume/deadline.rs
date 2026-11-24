use super::*;

/// The ceilings an attempt may arm, shortest first. The elapsed clock is armed
/// into the crashed run's journal and read back when resume starts, so the
/// setup in between spends part of the ceiling — on a loaded host enough of it
/// that a short ceiling expires before the held source starts, which says
/// nothing about the settlement under test. An attempt that ends that way
/// returns `false` and the test arms the next, longer ceiling; every assertion
/// of an attempt whose source did start holds at full strength.
const CEILINGS: [Duration; 4] = [
    Duration::from_millis(250),
    Duration::from_secs(1),
    Duration::from_secs(4),
    Duration::from_secs(16),
];

/// How long a wait between two events that follow within milliseconds may take
/// before a test calls it a hang. The slowest of these gaps, the resume's setup
/// before its source starts, took a second on a host running several build
/// paths; only a hang takes ten.
const HANG_BOUND: Duration = Duration::from_secs(10);

#[tokio::test]
async fn resume_deadline_waits_for_context_tool_settlement_before_its_pause() {
    for ceiling in CEILINGS {
        if held_source_attempt(ceiling).await {
            return;
        }
    }
    panic!("the resumed source did not start within any of {CEILINGS:?}");
}

/// One attempt at the property with `ceiling` armed on the resumed run's
/// elapsed clock. `false` means the ceiling expired before the held source
/// started; every other outcome asserts the settlement property in full.
async fn held_source_attempt(ceiling: Duration) -> bool {
    let clock = saya_harness::engine::ElapsedClock::arm_now(ceiling).unwrap();
    let plan = RunPlan::new(vec![step("hold until the resumed source stops")]).unwrap();
    let run = crashed_run(
        &format!("held-resume-deadline-{}", ceiling.as_millis()),
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
    inputs.wall_clock = Some(ceiling);
    let active = resume(&run.run_dir, inputs);
    tokio::pin!(active);

    let source_token = tokio::time::timeout(ceiling + HANG_BOUND, async {
        tokio::select! {
            started = started_rx => Some(started.unwrap()),
            result = &mut active => {
                if expired_before_start(&result) {
                    None
                } else {
                    panic!("the resumed run ended before the held source started: {result:?}")
                }
            }
        }
    })
    .await
    .expect("the resumed source starts");
    let Some(source_token) = source_token else {
        // Windows cannot delete an open database file; close the store first
        // so the abandoned attempt leaves nothing behind for the next one.
        run.store.close().await;
        return false;
    };
    let baseline = run.journal();
    assert!(!matches!(
        baseline.last(),
        Some(
            RunEvent::Paused {
                reason: PauseReason::WallClockExceeded
            } | RunEvent::Cancelled
        )
    ));
    let observed = tokio::time::timeout(ceiling + HANG_BOUND, async {
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
    let result = tokio::time::timeout(HANG_BOUND, &mut active)
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
    true
}

/// Whether the resume ended because its ceiling was already spent before the
/// held source could start: the runner's setup time, not the settlement under
/// test. The attempt then retries with a longer ceiling, as its sibling in
/// `tests/engine_episode/deadline/` does.
fn expired_before_start(result: &Result<ResumeOutcome, ResumeError>) -> bool {
    match result {
        // The ceiling was spent by the time resume armed its clock, so the run
        // paused before any step work.
        Ok(ResumeOutcome::Resumed {
            state: RunState::Paused,
            ..
        }) => true,
        // The deadline tripped in the driver, before the source started.
        Err(ResumeError::Episode {
            source:
                EpisodeError::BudgetStopped {
                    reason: PauseReason::WallClockExceeded,
                    ..
                },
        }) => true,
        _ => false,
    }
}
