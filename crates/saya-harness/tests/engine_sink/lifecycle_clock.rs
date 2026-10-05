//! Lifecycle transitions under a failing elapsed clock: a pause or a
//! terminal transition holds the clock error as a diagnostic and records
//! anyway — a user Cancel must land — while Begin and Resume refuse rather
//! than work against a clock that cannot be observed.

use super::*;
use saya_harness::engine::ElapsedClockError;

/// A sink whose elapsed clock fails on every observation: the armed origin
/// sits past any real system time, so raising the clock errors `Backwards`.
async fn failing_clock_sink(
    root: &Path,
    label: &str,
    state: RunState,
) -> (EngineEventSink, PathBuf) {
    let (store, run_id) = seeded_store(root, label, RunStatus::Executing).await;
    if state == RunState::Paused {
        store
            .set_run_status(&run_id, RunStatus::Paused, None)
            .await
            .unwrap();
    }
    let run_dir = root.join("run");
    fs::create_dir_all(&run_dir).unwrap();
    let elapsed =
        saya_harness::engine::ElapsedClock::arm(Duration::from_secs(30), u64::MAX, Instant::now())
            .unwrap();
    let sink = EngineEventSink::new(
        run_id,
        state,
        Journal::open(&run_dir),
        store,
        SinkBudgets {
            wall_clock: Some(Duration::from_secs(30)),
            token_ceiling: None,
            download_budget: None,
            carried_usage: UsageTotals::default(),
        },
        Instant::now,
    )
    .with_elapsed_clock(elapsed);
    (sink, run_dir)
}

#[tokio::test]
async fn a_clock_error_holds_a_diagnostic_and_still_records_the_cancel() {
    let root = temp_root("cancel-clock");
    let (sink, run_dir) = failing_clock_sink(&root, "cancel-clock", RunState::Executing).await;

    let state = sink.record(TransitionEvent::Cancel).await.unwrap();

    assert_eq!(state, RunState::Cancelled);
    assert!(
        Journal::open(&run_dir)
            .read()
            .unwrap()
            .contains(&RunEvent::Cancelled),
        "the user's cancel must be journaled even with a broken clock"
    );
    assert!(matches!(
        sink.take_diagnostic().as_deref(),
        Some(EngineSinkError::ElapsedClock {
            source: ElapsedClockError::Backwards
        })
    ));
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn a_clock_error_holds_a_diagnostic_and_still_records_the_completion() {
    let root = temp_root("complete-clock");
    let (sink, run_dir) = failing_clock_sink(&root, "complete-clock", RunState::Executing).await;

    let state = sink.record(TransitionEvent::Complete).await.unwrap();

    assert_eq!(state, RunState::Completed);
    assert!(
        Journal::open(&run_dir)
            .read()
            .unwrap()
            .contains(&RunEvent::Completed),
        "the completion must be journaled even with a broken clock"
    );
    assert!(matches!(
        sink.take_diagnostic().as_deref(),
        Some(EngineSinkError::ElapsedClock {
            source: ElapsedClockError::Backwards
        })
    ));
    let _ = fs::remove_dir_all(root);
}

/// Begin and Resume keep refusing: no work starts or continues against a
/// clock that cannot be observed.
#[tokio::test]
async fn a_clock_error_still_refuses_a_resume() {
    let root = temp_root("resume-clock");
    let (sink, run_dir) = failing_clock_sink(&root, "resume-clock", RunState::Paused).await;

    let error = sink.record(TransitionEvent::Resume).await.unwrap_err();

    assert!(matches!(
        error,
        EngineSinkError::ElapsedClock {
            source: ElapsedClockError::Backwards
        }
    ));
    assert!(
        Journal::open(&run_dir).read().unwrap().is_empty(),
        "a refused resume records nothing"
    );
    let _ = fs::remove_dir_all(root);
}
