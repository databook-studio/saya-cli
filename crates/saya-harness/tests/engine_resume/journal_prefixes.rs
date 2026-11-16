use super::*;

/// Every prefix of the driven two-step run's journal, resumed. The full
/// journal is seven events; for each prefix the table says where resume
/// must continue, how many provider requests that is, and what the journal
/// gains — completed steps are never re-run, never re-journalled.
#[tokio::test]
async fn a_crash_at_every_journal_position_resumes_at_the_first_incomplete_step() {
    let run = driven_run("prefixes").await;
    let raw = fs::read_to_string(run.run_dir.join("events.ndjson")).unwrap();
    let lines: Vec<&str> = raw.split_inclusive('\n').collect();
    assert_eq!(lines.len(), 7, "the driven fixture journal: {raw}");
    let full = run.journal();

    let ss = |step: usize| RunEvent::StepStarted { step };
    let sc = |step: usize| RunEvent::StepCompleted { step };
    let pd = RunEvent::Paused {
        reason: PauseReason::ProcessDeath,
    };
    let done = RunState::Completed;
    let cases: Vec<(usize, ResumeOutcome, usize, Vec<RunEvent>)> = vec![
        (0, ResumeOutcome::NoRun, 0, vec![]),
        (1, ResumeOutcome::Unapproved, 0, vec![]),
        (
            2,
            ResumeOutcome::Resumed {
                first_step: 0,
                state: done,
            },
            2,
            vec![ss(0), sc(0), ss(1), sc(1), RunEvent::Completed],
        ),
        (
            3,
            ResumeOutcome::Resumed {
                first_step: 0,
                state: done,
            },
            2,
            vec![pd.clone(), ss(0), sc(0), ss(1), sc(1), RunEvent::Completed],
        ),
        (
            4,
            ResumeOutcome::Resumed {
                first_step: 1,
                state: done,
            },
            1,
            vec![pd.clone(), ss(1), sc(1), RunEvent::Completed],
        ),
        (
            5,
            ResumeOutcome::Resumed {
                first_step: 1,
                state: done,
            },
            1,
            vec![pd.clone(), ss(1), sc(1), RunEvent::Completed],
        ),
        (
            6,
            ResumeOutcome::Settled { state: done },
            0,
            vec![pd, RunEvent::Completed],
        ),
        (7, ResumeOutcome::Settled { state: done }, 0, vec![]),
    ];

    for (prefix, outcome, requests, appended) in cases {
        // The store row a real crash leaves: at or behind the journal, never
        // ahead of it — the journal is written before every mirror.
        let status = match prefix {
            0 | 1 => RunStatus::Planned,
            2 => RunStatus::Approved,
            3..=6 => RunStatus::Executing,
            _ => RunStatus::Completed,
        };
        let crashed = crashed_run(
            &format!("prefix-{prefix}"),
            &full[..prefix],
            status,
            run.plan.clone(),
        )
        .await;

        let result = resume(&crashed.run_dir, crashed.inputs()).await;

        let continuing = match &outcome {
            ResumeOutcome::Resumed { first_step, .. } => format!("continue at step {first_step}"),
            ResumeOutcome::Settled { .. } => "settle the run".into(),
            other => format!("report {other:?}"),
        };
        assert_eq!(
            result.as_ref().unwrap(),
            &outcome,
            "prefix {prefix}: resume must {continuing}"
        );
        assert_eq!(
            crashed.stubs.provider.count(),
            requests,
            "prefix {prefix}: exactly the remaining steps' work, no more"
        );
        let expected = [full[..prefix].to_vec(), appended].concat();
        assert_eq!(
            crashed.journal(),
            expected,
            "prefix {prefix}: the journal must hold the prefix plus exactly the resumed events"
        );
        let recorded = crashed
            .store
            .get_run(&crashed.run_id)
            .await
            .unwrap()
            .unwrap();
        let expected_status = if prefix < 2 {
            RunStatus::Planned
        } else {
            RunStatus::Completed
        };
        assert_eq!(
            recorded.status, expected_status,
            "prefix {prefix}: the store must mirror the run's final state"
        );
        if (2..=6).contains(&prefix) {
            let steps = RunStore::list_steps(&*crashed.store, &crashed.run_id)
                .await
                .unwrap();
            assert_eq!(steps.len(), 2, "prefix {prefix}: both steps mirrored");
            assert!(
                steps.iter().all(|step| step.status == RunStepStatus::Done),
                "prefix {prefix}: every driven step is done in the store"
            );
        }
    }
}
