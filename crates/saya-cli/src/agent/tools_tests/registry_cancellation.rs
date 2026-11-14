/// A cancellation while waiting for an entry lease cannot target the active
/// holder's connector operation. The holder is released by an independent
/// source event and settles normally; the waiter never starts SQL or calls the
/// connector's cancellation hook.
#[tokio::test]
async fn cancelled_registry_waiter_leaves_active_owner_running() {
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let executions = Arc::new(AtomicUsize::new(0));
    let cancel_requests = Arc::new(AtomicUsize::new(0));
    let mut registry = ConnectionRegistry::new("primary");
    registry.insert(
        "primary",
        ConnectionEntry {
            connector: Box::new(LeaseConnector {
                started: started.clone(),
                release: release.clone(),
                executions: executions.clone(),
                cancel_requests: cancel_requests.clone(),
            }),
            dialect: SqlDialect::Sqlite,
            profile_id: None,
        },
    );
    let tools = Arc::new(DatabaseTools::with_registry(registry, 100, true, None));
    let holder_context = saya_agent::ToolExecutionContext {
        result_cap: 1024,
        cancellation: saya_agent::CancellationToken::new(),
    };
    let holder_tools = tools.clone();
    let started_wait = started.notified();
    let holder = tokio::spawn(async move {
        holder_tools
            .execute_with_context(
                "bounded_sql_query",
                serde_json::json!({"sql": "SELECT 1"}),
                holder_context,
            )
            .await
    });
    started_wait.await;

    let waiter_token = saya_agent::CancellationToken::new();
    let waiter_tools = tools.clone();
    let waiter_context = saya_agent::ToolExecutionContext {
        result_cap: 1024,
        cancellation: waiter_token.clone(),
    };
    let waiter = waiter_tools.execute_with_context(
        "bounded_sql_query",
        serde_json::json!({"sql": "SELECT 1"}),
        waiter_context,
    );
    tokio::pin!(waiter);
    poll_fn(|cx| match waiter.as_mut().poll(cx) {
        Poll::Pending => Poll::Ready(()),
        Poll::Ready(result) => panic!("waiter settled before reaching the entry lease: {result:?}"),
    })
    .await;
    waiter_token.cancel();
    let waiter_result = waiter.await;
    assert!(waiter_result.is_err(), "cancelled waiter is refused");
    assert_eq!(executions.load(Ordering::SeqCst), 1, "waiter starts no SQL");
    assert_eq!(
        cancel_requests.load(Ordering::SeqCst),
        0,
        "waiter cannot cancel the holder's connector operation"
    );

    release.notify_waiters();
    assert!(
        holder.await.expect("holder task joins").is_ok(),
        "active owner stays alive until its independent source settles"
    );
}
