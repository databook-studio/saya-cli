#[tokio::test]
async fn a_compacting_run_records_one_succeeded_sample_and_one_start() {
    let seen = Arc::new(Mutex::new(0_usize));
    let (release_tx, release_rx) = mpsc::channel();
    let (base_url, source) = counting_source(Arc::clone(&seen), release_rx);
    let runtime = runtime_for_source(base_url, Some(64));
    let mut state = openai_turns(4);
    let metrics = LocalMetrics::new();
    let result = run_with_trigger(&runtime, &mut state, &universe(), false, Some(&metrics)).await;
    release_tx.send(()).expect("release source");
    source.join().expect("source thread joins");
    assert!(
        !result.failed,
        "the compaction succeeds: {}",
        result.message
    );
    let snapshot = metrics.snapshot();
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
    assert_eq!(
        *seen.lock().expect("request count lock"),
        1,
        "exactly one provider request was made"
    );
}

#[tokio::test]
async fn a_refusing_provider_records_one_failed_sample() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a port");
    let address = listener.local_addr().expect("the closed port's address");
    drop(listener);
    let runtime = runtime_for_source(format!("http://{address}/v1"), Some(64));
    let mut state = openai_turns(4);
    let metrics = LocalMetrics::new();
    let result = run_with_trigger(&runtime, &mut state, &universe(), false, Some(&metrics)).await;
    assert!(
        result.failed,
        "a dead provider fails the compaction: {}",
        result.message
    );
    let snapshot = metrics.snapshot();
    let cell = compaction_cell(&snapshot);
    assert_eq!(cell.samples, 1, "one compaction sample: {snapshot:?}");
    let failed = outcome_index(AgentMetricOutcome::Failed).expect("failed has a slot");
    assert_eq!(
        cell.outcomes[failed], 1,
        "the sample is Failed: {snapshot:?}"
    );
    assert_eq!(started_count(&snapshot), 1, "the summariser call did start");
}

#[tokio::test]
async fn a_short_conversation_records_one_skipped_sample_and_no_start() {
    let seen = Arc::new(Mutex::new(0_usize));
    let (release_tx, release_rx) = mpsc::channel();
    let (base_url, source) = counting_source(Arc::clone(&seen), release_rx);
    let runtime = runtime_for_source(base_url, Some(64));
    let mut state = openai_turns(2);
    let metrics = LocalMetrics::new();
    let result = run_with_trigger(&runtime, &mut state, &universe(), false, Some(&metrics)).await;
    release_tx.send(()).expect("release source");
    source.join().expect("source thread joins");
    assert!(
        !result.failed && result.summary.is_none(),
        "a short conversation does no work: {}",
        result.message
    );
    let snapshot = metrics.snapshot();
    let cell = compaction_cell(&snapshot);
    assert_eq!(cell.samples, 1, "one compaction sample: {snapshot:?}");
    let skipped = outcome_index(AgentMetricOutcome::Skipped).expect("skipped has a slot");
    assert_eq!(
        cell.outcomes[skipped], 1,
        "the sample is Skipped: {snapshot:?}"
    );
    assert_eq!(
        started_count(&snapshot),
        0,
        "the summariser call never started"
    );
    assert_eq!(
        *seen.lock().expect("request count lock"),
        0,
        "a no-work result never reaches the provider"
    );
}

#[tokio::test]
async fn a_collector_less_run_matches_the_recorded_run() {
    let seen = Arc::new(Mutex::new(0_usize));
    let (release_tx, release_rx) = mpsc::channel();
    let (base_url, source) = counting_source(Arc::clone(&seen), release_rx);
    let runtime = runtime_for_source(base_url, Some(64));
    let mut bare = openai_turns(4);
    let without = run_with_trigger(&runtime, &mut bare, &universe(), false, None).await;
    let mut tracked = openai_turns(4);
    let metrics = LocalMetrics::new();
    let with = run_with_trigger(&runtime, &mut tracked, &universe(), false, Some(&metrics)).await;
    release_tx.send(()).expect("release source");
    source.join().expect("source thread joins");
    assert_eq!(without.message, with.message, "the message is identical");
    assert_eq!(without.failed, with.failed, "the outcome is identical");
    assert_eq!(
        without.compacted_turns, with.compacted_turns,
        "the compacted-turn count is identical"
    );
    assert_eq!(
        *seen.lock().expect("request count lock"),
        2,
        "each run called once"
    );
}
