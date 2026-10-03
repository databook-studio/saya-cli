use super::*;
use saya_harness::engine::{ElapsedClock, ElapsedClockError};

#[tokio::test]
async fn streamed_text_bounds_clock_observations_by_elapsed_seconds_and_boundaries() {
    let root = temp_root("clock-cadence");
    let (store, run_id) = seeded_store(&root, "clock-cadence", RunStatus::Planned).await;
    let run_dir = root.join("run");
    fs::create_dir_all(&run_dir).unwrap();
    let journal = Journal::open(&run_dir);
    journal.append(&RunEvent::RunStarted).unwrap();
    let started = Instant::now();
    let origin = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let elapsed = ElapsedClock::arm(Duration::from_secs(30), origin, started).unwrap();
    let sink = EngineEventSink::new(
        run_id,
        RunState::Planned,
        journal.clone(),
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
    sink.record(TransitionEvent::Approve { scopes: vec![] })
        .await
        .unwrap();
    sink.record(TransitionEvent::Begin).await.unwrap();

    let before_tool = journal
        .read()
        .unwrap()
        .iter()
        .filter(|event| matches!(event, RunEvent::WallClockObserved { .. }))
        .count();
    std::thread::sleep(Duration::from_millis(10));
    AgentEventSink::emit(
        &sink,
        AgentEvent::ToolRequested {
            name: "workspace_read".into(),
            arguments: serde_json::Value::Null,
            effect: None,
        },
    )
    .await;
    let after_tool = journal
        .read()
        .unwrap()
        .iter()
        .filter(|event| matches!(event, RunEvent::WallClockObserved { .. }))
        .count();
    assert_eq!(
        after_tool,
        before_tool + 1,
        "tool requests force a raised clock mark"
    );
    let after_tool_high_water = journal
        .read()
        .unwrap()
        .iter()
        .filter_map(|event| match event {
            RunEvent::WallClockObserved {
                high_water_unix_ms, ..
            } => Some(*high_water_unix_ms),
            _ => None,
        })
        .next_back()
        .unwrap();

    std::thread::sleep(Duration::from_millis(1_100));
    for _ in 0..200 {
        AgentEventSink::emit(&sink, AgentEvent::AssistantText { text: "x".into() }).await;
    }

    let elapsed_seconds = started.elapsed().as_secs();
    let text_marks = journal
        .read()
        .unwrap()
        .iter()
        .filter_map(|event| match event {
            RunEvent::WallClockObserved {
                high_water_unix_ms, ..
            } => Some(*high_water_unix_ms),
            _ => None,
        })
        .collect::<Vec<_>>();
    let clock_count = text_marks.len();
    assert!(
        text_marks.len() > after_tool,
        "the streamed text burst must persist a periodic clock observation"
    );
    assert!(
        *text_marks.last().unwrap() > after_tool_high_water,
        "the periodic observation must raise the durable high-water mark"
    );
    assert!(
        u64::try_from(clock_count).unwrap() <= elapsed_seconds + 3,
        "200 streamed text events over {elapsed_seconds}s wrote {clock_count} clock marks"
    );
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn an_active_clock_error_is_diagnosed_while_the_run_pauses() {
    let root = temp_root("clock-diagnostic");
    let (store, run_id) = seeded_store(&root, "clock-diagnostic", RunStatus::Executing).await;
    let run_dir = root.join("run");
    fs::create_dir_all(&run_dir).unwrap();
    let elapsed = ElapsedClock::arm(Duration::from_secs(30), u64::MAX, Instant::now()).unwrap();
    let sink = EngineEventSink::new(
        run_id,
        RunState::Executing,
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

    AgentEventSink::emit(
        &sink,
        AgentEvent::AssistantText {
            text: "tick".into(),
        },
    )
    .await;

    assert_eq!(sink.state(), RunState::Paused);
    assert!(matches!(
        sink.take_diagnostic().as_deref(),
        Some(EngineSinkError::ElapsedClock {
            source: ElapsedClockError::Backwards
        })
    ));
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn a_failed_clock_append_before_a_tool_request_pauses_with_the_journal_error() {
    let root = temp_root("clock-write-failure");
    let (store, run_id) = seeded_store(&root, "clock-write-failure", RunStatus::Planned).await;
    let run_dir = root.join("run");
    fs::create_dir_all(&run_dir).unwrap();
    let journal = Journal::open(&run_dir);
    journal.append(&RunEvent::RunStarted).unwrap();
    let started = Instant::now();
    let origin = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let elapsed = ElapsedClock::arm(Duration::from_secs(30), origin, started).unwrap();
    let sink = EngineEventSink::new(
        run_id,
        RunState::Planned,
        journal.clone(),
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
    sink.record(TransitionEvent::Approve { scopes: vec![] })
        .await
        .unwrap();
    sink.record(TransitionEvent::Begin).await.unwrap();
    fs::remove_dir_all(&run_dir).unwrap();
    fs::write(&run_dir, b"block journal writes").unwrap();
    std::thread::sleep(Duration::from_millis(10));

    AgentEventSink::emit(
        &sink,
        AgentEvent::ToolRequested {
            name: "workspace_write".into(),
            arguments: serde_json::Value::Null,
            effect: None,
        },
    )
    .await;

    assert_eq!(sink.state(), RunState::Paused);
    assert!(matches!(
        sink.take_diagnostic().as_deref(),
        Some(EngineSinkError::Journal { .. })
    ));
    fs::remove_file(&run_dir).unwrap();
    let _ = fs::remove_dir_all(root);
}
