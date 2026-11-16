/// (a) An uncontended real SQLite query through the operation owner records
/// exactly one DatabaseWait and one DatabaseExecute sample, both succeeded.
#[tokio::test]
async fn uncontended_sqlite_query_records_one_wait_and_one_execute_sample() {
    let root = temp_root("uncontended");
    let metrics = Arc::new(LocalMetrics::new());
    let tools = sqlite_tools(&root, Some(Arc::clone(&metrics))).await;
    let result = tools
        .execute_with_context(
            "bounded_sql_query",
            json!({"sql": "SELECT id FROM orders"}),
            tool_context(CancellationToken::new()),
        )
        .await
        .expect("uncontended query succeeds");
    assert!(
        result.is_object(),
        "the model received a query result: {result:?}"
    );
    let snapshot = metrics.snapshot();
    assert_eq!(
        samples_in(&snapshot, AgentMetricPhase::DatabaseWait),
        1,
        "one wait sample: {snapshot:?}"
    );
    assert_eq!(
        outcome_in(
            &snapshot,
            AgentMetricPhase::DatabaseWait,
            AgentMetricOutcome::Succeeded
        ),
        1,
        "the uncontended acquire succeeded: {snapshot:?}"
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
            AgentMetricOutcome::Succeeded
        ),
        1,
        "the settled query succeeded: {snapshot:?}"
    );
    let _ = std::fs::remove_dir_all(root);
}

/// (b) A waiter held at the entry lease records its DatabaseWait sample only
/// once the holder releases, and its DatabaseExecute sample only once its own
/// source settles — the two intervals never cover each other.
#[tokio::test]
async fn contended_waiter_records_wait_only_after_the_lease_is_granted() {
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
    let waiter = {
        let tools = Arc::clone(&tools);
        tokio::spawn(async move {
            tools
                .execute_with_context(
                    "bounded_sql_query",
                    json!({"sql": "SELECT 1"}),
                    tool_context(waiter_token),
                )
                .await
        })
    };
    let mut waiter = waiter;
    assert_pending(&mut waiter, "waiter").await;
    let before_release = metrics.snapshot();
    assert_eq!(
        outcome_in(
            &before_release,
            AgentMetricPhase::DatabaseWait,
            AgentMetricOutcome::Succeeded
        ),
        1,
        "only the holder's wait is recorded while the waiter pends: {before_release:?}"
    );
    assert_eq!(
        samples_in(&before_release, AgentMetricPhase::DatabaseExecute),
        0,
        "no execute settles before any release: {before_release:?}"
    );

    release.notify_waiters();
    wait_for_executions(&executions, 2).await;
    let after_grant = metrics.snapshot();
    assert_eq!(
        outcome_in(
            &after_grant,
            AgentMetricPhase::DatabaseWait,
            AgentMetricOutcome::Succeeded
        ),
        2,
        "the waiter's wait sample appears only after the lease is granted: {after_grant:?}"
    );
    assert_eq!(
        outcome_in(
            &after_grant,
            AgentMetricPhase::DatabaseExecute,
            AgentMetricOutcome::Succeeded
        ),
        1,
        "only the holder's execute has settled: {after_grant:?}"
    );
    assert_pending(&mut waiter, "waiter inside execute").await;

    release.notify_waiters();
    holder
        .await
        .expect("holder task joins")
        .expect("holder query succeeds");
    let waiter_result = waiter
        .await
        .expect("waiter task joins")
        .expect("waiter query succeeds after its lease");
    assert!(waiter_result.is_object(), "waiter received a result");
    let settled = metrics.snapshot();
    assert_eq!(
        outcome_in(
            &settled,
            AgentMetricPhase::DatabaseWait,
            AgentMetricOutcome::Succeeded
        ),
        2,
        "each operation recorded exactly one succeeded wait: {settled:?}"
    );
    assert_eq!(
        outcome_in(
            &settled,
            AgentMetricPhase::DatabaseExecute,
            AgentMetricOutcome::Succeeded
        ),
        2,
        "each operation recorded exactly one succeeded execute: {settled:?}"
    );
    assert_eq!(executions.load(Ordering::SeqCst), 2);
    assert_eq!(cancel_requests.load(Ordering::SeqCst), 0);
}
