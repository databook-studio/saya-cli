use super::*;

/// A torn final line — the crash that matters most — replays to the last
/// whole event: the half-written event is dropped, the run resumes past it.
#[tokio::test]
async fn a_torn_final_line_replays_to_the_last_whole_event() {
    let run = crashed_run(
        "torn",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
            RunEvent::StepCompleted { step: 0 },
        ],
        RunStatus::Executing,
        two_step_plan(),
    )
    .await;
    let torn = "{\"type\":\"step_started\",\"step\":1";
    fs::write(
        run.run_dir.join("events.ndjson"),
        format!(
            "{}\n{torn}",
            run.journal()
                .iter()
                .map(|event| serde_json::to_string(event).unwrap())
                .collect::<Vec<_>>()
                .join("\n")
        ),
    )
    .unwrap();

    let outcome = resume(&run.run_dir, run.inputs()).await.unwrap();

    assert_eq!(
        outcome,
        ResumeOutcome::Resumed {
            first_step: 1,
            state: RunState::Completed
        },
        "the torn line must not error the run nor hide step 1's incompleteness"
    );
    assert_eq!(
        run.journal(),
        vec![
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
            RunEvent::StepCompleted { step: 0 },
            RunEvent::Paused {
                reason: PauseReason::ProcessDeath
            },
            RunEvent::StepStarted { step: 1 },
            RunEvent::StepCompleted { step: 1 },
            RunEvent::Completed,
        ],
        "the torn line must be dropped, not replayed"
    );
}

/// Seeding rides the repaired record: a torn trailing usage line — the
/// half-written event a crash left — never counts toward the carried spend.
/// The journal shows 100 tokens spent and a torn line that would have added
/// 60; counting it would push the carried spend past the 150 ceiling and
/// pause the resumed run at its first tick. The repaired record carries only
/// the whole event, so the run resumes and completes.
#[tokio::test]
async fn a_torn_usage_line_never_counts_toward_the_carried_spend() {
    let plan = RunPlan::new(vec![step("the only step")]).unwrap();
    let run = crashed_run(
        "torn-usage",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
            journaled_usage(100),
        ],
        RunStatus::Executing,
        plan,
    )
    .await;
    let torn = "{\"type\":\"usage\",\"endpoint\":\"orchestrator\",\"tokens\":60";
    fs::write(
        run.run_dir.join("events.ndjson"),
        format!(
            "{}\n{torn}",
            run.journal()
                .iter()
                .map(|event| serde_json::to_string(event).unwrap())
                .collect::<Vec<_>>()
                .join("\n")
        ),
    )
    .unwrap();
    let provider = UsageProvider::default();

    let outcome = resume(&run.run_dir, budgeted_inputs(&run, &provider, Some(150))).await;

    // The carried spend is the repaired record's 100 — the torn 60 never
    // counted — so the resumed turn's 10 keep the run under the ceiling.
    assert_eq!(
        outcome.as_ref().unwrap(),
        &ResumeOutcome::Resumed {
            first_step: 0,
            state: RunState::Completed
        },
        "counting the torn line would have paused the run at 170 against 150"
    );
    assert_eq!(
        run.journal(),
        vec![
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
            journaled_usage(100),
            RunEvent::Paused {
                reason: PauseReason::ProcessDeath
            },
            RunEvent::StepStarted { step: 0 },
            journaled_usage(10),
            RunEvent::StepCompleted { step: 0 },
            RunEvent::Completed,
        ]
    );
    assert_eq!(provider.count(), 1);
}
