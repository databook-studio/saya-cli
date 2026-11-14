/// (c) A waiter cancelled while the lease is held records one cancelled
/// DatabaseWait sample and no DatabaseExecute sample at all; the holder's
/// execute still settles once its independent release fires.
#[tokio::test]
async fn cancelled_waiter_records_no_execute_sample() {
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
                    "bounded_sql_query",
                    json!({"sql": "SELECT 1"}),
                    tool_context(waiter_cancellation),
                )
                .await
        })
    };
    let mut waiter = waiter;
    assert_pending(&mut waiter, "waiter").await;
    waiter_token.cancel();
    waiter
        .await
        .expect("waiter task joins")
        .expect_err("cancelled waiter is refused");
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
        samples_in(&after_cancel, AgentMetricPhase::DatabaseExecute),
        0,
        "a cancelled waiter records no execute sample: {after_cancel:?}"
    );
    assert_eq!(
        executions.load(Ordering::SeqCst),
        1,
        "the waiter starts no SQL"
    );
    assert_eq!(
        cancel_requests.load(Ordering::SeqCst),
        0,
        "the waiter cannot cancel the holder's source operation"
    );

    release.notify_waiters();
    holder
        .await
        .expect("holder task joins")
        .expect("holder query succeeds");
    let settled = metrics.snapshot();
    assert_eq!(
        outcome_in(
            &settled,
            AgentMetricPhase::DatabaseExecute,
            AgentMetricOutcome::Succeeded
        ),
        1,
        "the holder's execute alone settles: {settled:?}"
    );
}

/// (d) A connector whose execute fails records one DatabaseExecute sample
/// with the failed outcome, beside the succeeded lease wait.
#[tokio::test]
async fn failing_connector_execute_records_a_failed_execute_sample() {
    let metrics = Arc::new(LocalMetrics::new());
    let mut registry = ConnectionRegistry::new("primary");
    registry.insert(
        "primary",
        ConnectionEntry {
            connector: Box::new(FailingConnector),
            dialect: SqlDialect::Sqlite,
            profile_id: None,
        },
    );
    let tools = DatabaseTools::with_registry(registry, 100, true, None)
        .with_metrics(Some(Arc::clone(&metrics)));
    let result = tools
        .execute_with_context(
            "bounded_sql_query",
            json!({"sql": "SELECT 1"}),
            tool_context(CancellationToken::new()),
        )
        .await;
    assert!(result.is_err(), "the failing connector is reported");
    let snapshot = metrics.snapshot();
    assert_eq!(
        outcome_in(
            &snapshot,
            AgentMetricPhase::DatabaseWait,
            AgentMetricOutcome::Succeeded
        ),
        1,
        "the lease was acquired before the execute: {snapshot:?}"
    );
    assert_eq!(
        samples_in(&snapshot, AgentMetricPhase::DatabaseExecute),
        1,
        "one execute sample: {snapshot:?}"
    );
    assert_eq!(
        outcome_in(
            &snapshot,
            AgentMetricPhase::DatabaseExecute,
            AgentMetricOutcome::Failed
        ),
        1,
        "the connector error settles the execute as failed: {snapshot:?}"
    );
}
