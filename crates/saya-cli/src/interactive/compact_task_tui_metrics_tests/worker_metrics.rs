use super::*;

#[test]
fn the_tui_worker_records_the_session_collectors_succeeded_sample() {
    let mut app = idle_app();
    app.session_metrics = Some(Arc::new(LocalMetrics::new()));
    let seen = Arc::new(Mutex::new(0_usize));
    let (release_tx, release_rx) = mpsc::channel();
    let (base_url, source) = counting_source(Arc::clone(&seen), release_rx);
    app.runtime = Arc::new(runtime_for_source(base_url, Some(64)));
    let mut state = openai_turns(4);
    compact_task::start(&mut app, &state);
    wait_for_worker(&mut app, &mut state);
    release_tx.send(()).expect("release source");
    source.join().expect("source thread joins");
    let snapshot = app.session_metrics.expect("the collector rides").snapshot();
    let cell = compaction_cell(&snapshot);
    assert_eq!(cell.samples, 1, "one compaction sample: {snapshot:?}");
    let ok = outcome_index(AgentMetricOutcome::Succeeded).expect("succeeded has a slot");
    assert_eq!(
        cell.outcomes[ok], 1,
        "the sample is Succeeded: {snapshot:?}"
    );
    assert_eq!(
        started_count(&snapshot),
        1,
        "the summariser call started once"
    );
    assert!(
        app.transcript
            .blocks()
            .iter()
            .any(|block| block.text.contains("turns into a summary")),
        "the transcript announces the compaction"
    );
}

#[test]
fn a_dead_provider_records_one_failed_sample_through_the_tui_worker() {
    let mut app = idle_app();
    app.session_metrics = Some(Arc::new(LocalMetrics::new()));
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a port");
    let address = listener.local_addr().expect("the closed port's address");
    drop(listener);
    app.runtime = Arc::new(runtime_for_source(format!("http://{address}/v1"), Some(64)));
    let mut state = openai_turns(4);
    compact_task::start(&mut app, &state);
    wait_for_worker(&mut app, &mut state);
    let snapshot = app.session_metrics.expect("the collector rides").snapshot();
    let cell = compaction_cell(&snapshot);
    assert_eq!(cell.samples, 1, "one compaction sample: {snapshot:?}");
    let failed = outcome_index(AgentMetricOutcome::Failed).expect("failed has a slot");
    assert_eq!(
        cell.outcomes[failed], 1,
        "the sample is Failed: {snapshot:?}"
    );
}

#[test]
fn a_collector_less_tui_worker_matches_the_recorded_worker() {
    let seen = Arc::new(Mutex::new(0_usize));
    let (release_tx, release_rx) = mpsc::channel();
    let (base_url, source) = counting_source(Arc::clone(&seen), release_rx);
    let runtime = Arc::new(runtime_for_source(base_url, Some(64)));
    let mut recorded = idle_app();
    recorded.session_metrics = Some(Arc::new(LocalMetrics::new()));
    recorded.runtime = Arc::clone(&runtime);
    let mut bare = idle_app();
    bare.runtime = Arc::clone(&runtime);
    let mut recorded_state = openai_turns(4);
    let mut bare_state = openai_turns(4);
    compact_task::start(&mut recorded, &recorded_state);
    compact_task::start(&mut bare, &bare_state);
    wait_for_worker(&mut recorded, &mut recorded_state);
    wait_for_worker(&mut bare, &mut bare_state);
    release_tx.send(()).expect("release source");
    source.join().expect("source thread joins");
    let blocks = |app: &App| {
        app.transcript
            .blocks()
            .iter()
            .map(|block| block.text.to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        blocks(&recorded),
        blocks(&bare),
        "the transcript is identical"
    );
    assert_eq!(
        recorded_state.provider_history(),
        bare_state.provider_history(),
        "the live conversation is identical"
    );
    assert_eq!(
        *seen.lock().expect("request count lock"),
        2,
        "each worker called once"
    );
}
