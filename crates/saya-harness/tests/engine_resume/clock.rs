use super::*;

#[tokio::test]
async fn configured_resume_refuses_a_legacy_journal_without_an_elapsed_origin() {
    let run = crashed_run(
        "clock-legacy",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
        ],
        RunStatus::Approved,
        RunPlan::new(vec![step("the only step")]).unwrap(),
    )
    .await;
    let mut inputs = run.inputs();
    inputs.wall_clock = Some(Duration::from_secs(30));

    let error = resume(&run.run_dir, inputs).await.unwrap_err();

    assert!(matches!(
        error,
        ResumeError::ElapsedClock {
            source: saya_harness::engine::ElapsedClockError::MissingOrigin
        }
    ));
    assert_eq!(run.stubs.provider.count(), 0);
    assert_eq!(run.tool_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn an_expired_carried_clock_pauses_before_provider_or_tool_work() {
    let origin = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        - 10_000;
    let run = crashed_run(
        "clock-expired",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::WallClockObserved {
                origin_unix_ms: origin,
                high_water_unix_ms: origin,
            },
        ],
        RunStatus::Approved,
        RunPlan::new(vec![step("the only step")]).unwrap(),
    )
    .await;
    let mut inputs = run.inputs();
    inputs.wall_clock = Some(Duration::from_millis(1));

    let outcome = resume(&run.run_dir, inputs).await.unwrap();

    assert_eq!(
        outcome,
        ResumeOutcome::Resumed {
            first_step: 0,
            state: RunState::Paused,
        }
    );
    assert_eq!(run.stubs.provider.count(), 0);
    assert_eq!(run.tool_calls.load(Ordering::Relaxed), 0);
    assert!(matches!(
        run.journal().last(),
        Some(RunEvent::Paused {
            reason: PauseReason::WallClockExceeded
        })
    ));

    let mut repeated_inputs = run.inputs();
    repeated_inputs.wall_clock = Some(Duration::from_millis(1));
    let repeated = resume(&run.run_dir, repeated_inputs).await.unwrap();
    assert_eq!(
        repeated,
        ResumeOutcome::Resumed {
            first_step: 0,
            state: RunState::Paused,
        }
    );
    assert_eq!(run.stubs.provider.count(), 0);
    let clock_marks: Vec<_> = run
        .journal()
        .into_iter()
        .filter_map(|event| match event {
            RunEvent::WallClockObserved {
                origin_unix_ms,
                high_water_unix_ms,
            } => Some((origin_unix_ms, high_water_unix_ms)),
            _ => None,
        })
        .collect();
    assert!(clock_marks.len() >= 3);
    assert!(
        clock_marks
            .iter()
            .all(|(origin, _)| *origin == clock_marks[0].0)
    );
    assert!(clock_marks.windows(2).all(|pair| pair[1].1 >= pair[0].1));
}

#[tokio::test]
async fn a_torn_elapsed_origin_is_repaired_then_refused_as_missing() {
    use std::io::Write;

    let run = crashed_run(
        "clock-torn-origin",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
        ],
        RunStatus::Approved,
        RunPlan::new(vec![step("the only step")]).unwrap(),
    )
    .await;
    let mut journal_file = fs::OpenOptions::new()
        .append(true)
        .open(run.run_dir.join("events.ndjson"))
        .unwrap();
    let serialized = serde_json::to_vec(&RunEvent::WallClockObserved {
        origin_unix_ms: 1_000,
        high_water_unix_ms: 1_000,
    })
    .unwrap();
    journal_file
        .write_all(&serialized[..serialized.len() / 2])
        .unwrap();
    let mut inputs = run.inputs();
    inputs.wall_clock = Some(Duration::from_secs(30));

    let error = resume(&run.run_dir, inputs).await.unwrap_err();

    assert!(matches!(
        error,
        ResumeError::ElapsedClock {
            source: saya_harness::engine::ElapsedClockError::MissingOrigin
        }
    ));
    assert_eq!(run.stubs.provider.count(), 0);
    assert_eq!(
        run.journal(),
        vec![
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
        ]
    );
}
