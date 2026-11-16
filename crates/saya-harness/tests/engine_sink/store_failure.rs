use super::*;

#[tokio::test]
async fn a_store_failure_pauses_rather_than_continuing() {
    // A store that can never open — its parent path is a regular file, the
    // `store_open_failed` recipe — so every mirror fails fast, permanently.
    let root = temp_root("store-failure");
    fs::write(root.join("blocker"), b"x").unwrap();
    let failing: Arc<dyn RunStore> =
        Arc::new(SqliteStateStore::new(root.join("blocker/state.sqlite3")));
    let run_id = RunId::parse("run-store-failure").unwrap();
    let run_dir = root.join("run");
    fs::create_dir_all(&run_dir).unwrap();
    let sink = EngineEventSink::new(
        run_id,
        RunState::Approved,
        Journal::open(&run_dir),
        failing,
        SinkBudgets {
            wall_clock: None,
            token_ceiling: None,
            download_budget: None,
            carried_usage: UsageTotals::default(),
        },
        Instant::now,
    );

    // The first mirror fails while the run is mid-flight (approved →
    // executing). The sink must pause with a diagnostic — never continue
    // executing with the store refusing writes.
    let error = sink.record(TransitionEvent::Begin).await.unwrap_err();
    assert!(
        matches!(error, EngineSinkError::Store { .. }),
        "expected a store error, got {error:?}"
    );
    assert_eq!(
        sink.state(),
        RunState::Paused,
        "the run must pause on a store failure"
    );
    assert_eq!(
        Journal::open(&run_dir).read().unwrap(),
        vec![RunEvent::Paused {
            reason: PauseReason::StoreUnavailable
        }],
        "the store-unavailable pause must be journaled, with its reason"
    );
    assert!(
        matches!(
            sink.take_diagnostic().as_deref(),
            Some(EngineSinkError::Store { .. })
        ),
        "the store failure must be held as a diagnostic, not swallowed"
    );

    // The gate holds: from paused, completing is refused by the machine, and
    // the refusal writes nothing to the journal.
    let refused = sink.record(TransitionEvent::Complete).await.unwrap_err();
    assert!(
        matches!(refused, EngineSinkError::Transition { .. }),
        "expected a transition refusal from paused, got {refused:?}"
    );
    assert_eq!(Journal::open(&run_dir).read().unwrap().len(), 1);

    let _ = fs::remove_dir_all(root);
}
