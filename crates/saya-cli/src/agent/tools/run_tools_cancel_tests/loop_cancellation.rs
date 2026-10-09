use super::*;

/// Through the real loop over the real composite: the turn's token cancelled
/// mid-scratch stops the run — the follow-up `workspace_write` canary never
/// lands, and the provider is asked only once. The batch is conservatively
/// serial (both tools declare `Serial`), so once the cancelled scratch call
/// settles, the loop's own `check_cancelled` ends the run before the canary
/// call is ever reached.
#[tokio::test]
async fn a_cancelled_turn_runs_no_follow_up_effect_through_the_real_loop() {
    let ws_root = TempDir::new("loop-ws");
    let workspace = Arc::new(Workspace::open(&ws_root.0).unwrap());
    let (_scratch_root, scratch) = scratch_member("loop-scratch");
    let database = DatabaseTools::with_registry(
        crate::connection::ConnectionRegistry::new("primary"),
        100,
        true,
        None,
    )
    .with_workspace(Some(Arc::clone(&workspace)));
    let composite = RunTools::compose(Arc::new(database), Some(scratch), None, None);
    let calls = vec![
        ToolCall {
            id: "c1".into(),
            name: "scratch_sql".into(),
            arguments: json!({"sql": RUNAWAY}),
        },
        ToolCall {
            id: "c2".into(),
            name: "workspace_write".into(),
            arguments: json!({"path": "canary.txt", "content": "canary"}),
        },
    ];
    let requests = Arc::new(std::sync::Mutex::new(0));
    let provider = TwoCallsProvider {
        calls,
        turn: std::sync::Mutex::new(0),
        requests: Arc::clone(&requests),
    };
    let definitions = vec![ScratchSql::definition(), workspace_write_definition()];
    let token = CancellationToken::new();
    let canceller = {
        let token = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            token.cancel();
        })
    };
    let started = Instant::now();
    let outcome = tokio::time::timeout(
        GUARD,
        run_agent_with_sink(
            &provider,
            &composite,
            request(),
            definitions,
            AgentLimits {
                permit_workspace_writes: true,
                ..AgentLimits::default()
            },
            &AllowReadOnlyApproval,
            &NoopEventSink,
            token,
        ),
    )
    .await
    .expect("the guard: the turn token must reach the scratch member and stop the run");
    canceller.await.unwrap();
    let elapsed = started.elapsed();
    assert!(
        matches!(outcome, Err(AgentError::Cancelled)),
        "the run must end cancelled, got {outcome:?}"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "the run must end cancelled quickly, took {elapsed:?}"
    );
    assert!(
        !ws_root.0.join("canary.txt").exists(),
        "the cancelled turn must not run the follow-up write"
    );
    assert_eq!(
        *requests.lock().unwrap(),
        1,
        "the provider is asked only once"
    );
}
