/// (a) An uncontended real SQLite schema discovery records exactly one
/// succeeded DatabaseWait sample and one succeeded SchemaDiscovery sample,
/// and no DatabaseExecute sample at all.
#[tokio::test]
async fn uncontended_sqlite_schema_records_one_wait_and_one_discovery_sample() {
    let root = temp_root("schema-uncontended");
    let metrics = Arc::new(LocalMetrics::new());
    let tools = sqlite_tools(&root, Some(Arc::clone(&metrics))).await;
    let result = tools
        .execute_with_context(
            "schema_discovery",
            json!({}),
            tool_context(CancellationToken::new()),
        )
        .await
        .expect("uncontended schema discovery succeeds");
    assert!(
        result.is_object(),
        "the model received a schema: {result:?}"
    );
    let snapshot = metrics.snapshot();
    assert_eq!(
        outcome_in(
            &snapshot,
            AgentMetricPhase::DatabaseWait,
            AgentMetricOutcome::Succeeded
        ),
        1,
        "one succeeded lease wait: {snapshot:?}"
    );
    assert_eq!(
        samples_in(&snapshot, AgentMetricPhase::SchemaDiscovery),
        1,
        "one discovery sample: {snapshot:?}"
    );
    assert_eq!(
        outcome_in(
            &snapshot,
            AgentMetricPhase::SchemaDiscovery,
            AgentMetricOutcome::Succeeded
        ),
        1,
        "the live discovery settled: {snapshot:?}"
    );
    assert_eq!(
        samples_in(&snapshot, AgentMetricPhase::DatabaseExecute),
        0,
        "discovery runs no query tool: {snapshot:?}"
    );
    let _ = std::fs::remove_dir_all(root);
}

/// (b) A schema waiter cancelled while the entry lease is held records one
/// cancelled DatabaseWait sample and no SchemaDiscovery sample at all — the
/// discovery window never opened; the holder's execute still settles.
#[tokio::test]
async fn cancelled_schema_waiter_records_no_discovery_sample() {
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let executions = Arc::new(AtomicUsize::new(0));
    let cancel_requests = Arc::new(AtomicUsize::new(0));
    let metrics = Arc::new(LocalMetrics::new());
    let tools = lease_tools(
        Arc::clone(&started),
        Arc::clone(&release),
        Arc::clone(&executions),
        Arc::clone(&cancel_requests),
        Some(Arc::clone(&metrics)),
    );
    let holder_started = started.notified();
    let holder = {
        let tools = Arc::clone(&tools);
        tokio::spawn(async move {
            tools
                .execute_with_context(
                    "bounded_sql_query",
                    json!({"sql": "SELECT 1"}),
                    tool_context(CancellationToken::new()),
                )
                .await
        })
    };
    holder_started.await;

    let waiter_token = CancellationToken::new();
    let waiter_cancellation = waiter_token.clone();
    let waiter = {
        let tools = Arc::clone(&tools);
        tokio::spawn(async move {
            tools
                .execute_with_context(
                    "schema_discovery",
                    json!({}),
                    tool_context(waiter_cancellation),
                )
                .await
        })
    };
    let mut waiter = waiter;
    assert_pending(&mut waiter, "schema waiter").await;
    waiter_token.cancel();
    waiter
        .await
        .expect("waiter task joins")
        .expect_err("cancelled schema waiter is refused");
    let after_cancel = metrics.snapshot();
    assert_eq!(
        outcome_in(
            &after_cancel,
            AgentMetricPhase::DatabaseWait,
            AgentMetricOutcome::Cancelled
        ),
        1,
        "the cancelled lease wait is counted once: {after_cancel:?}"
    );
    assert_eq!(
        outcome_in(
            &after_cancel,
            AgentMetricPhase::DatabaseWait,
            AgentMetricOutcome::Succeeded
        ),
        1,
        "the holder's wait is the only succeeded one: {after_cancel:?}"
    );
    assert_eq!(
        samples_in(&after_cancel, AgentMetricPhase::SchemaDiscovery),
        0,
        "no discovery sample when the lease was never granted: {after_cancel:?}"
    );
    release.notify_waiters();
    holder
        .await
        .expect("holder task joins")
        .expect("holder query succeeds");
    let settled = metrics.snapshot();
    assert_eq!(
        samples_in(&settled, AgentMetricPhase::SchemaDiscovery),
        0,
        "still no discovery sample after the holder settles: {settled:?}"
    );
}
