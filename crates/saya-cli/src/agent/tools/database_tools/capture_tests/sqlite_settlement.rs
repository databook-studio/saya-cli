use super::*;

/// A real SQLite query is kept alive through cancellation: the same token the
/// loop receives reaches `RunTools` and the connector, the native future
/// settles before the owner returns, and the second requested effect never
/// starts or promotes a capture/completion.
#[tokio::test]
async fn cancelled_run_tools_query_waits_for_real_sqlite_settlement() {
    const SLOW: &str = "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM cnt \
        WHERE x < 500000000) SELECT count(*) FROM cnt";
    struct Provider {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl ChatProvider for Provider {
        fn name(&self) -> &str {
            "cancelled-real-sqlite"
        }

        async fn complete(
            &self,
            _: ChatRequest,
        ) -> Result<ChatResponse, saya_agent::ProviderError> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if call == 0 {
                Ok(ChatResponse::new(ChatMessage {
                    role: "assistant".into(),
                    content: String::new(),
                    tool_calls: vec![
                        ToolCall {
                            id: "slow".into(),
                            name: "bounded_sql_query".into(),
                            arguments: serde_json::json!({"sql": SLOW}),
                        },
                        ToolCall {
                            id: "must-not-run".into(),
                            name: "result_shape".into(),
                            arguments: serde_json::json!({"sql": "SELECT 1"}),
                        },
                    ],
                    tool_call_id: None,
                }))
            } else {
                Ok(ChatResponse::new(ChatMessage::text(
                    "assistant",
                    "unexpected",
                )))
            }
        }
    }

    let root = std::env::temp_dir().join(format!(
        "saya-agent-cancel-{}-{}",
        std::process::id(),
        FIXTURE_ID.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).expect("isolated SQLite fixture directory");
    let path = root.join("agent.sqlite3");
    let seed = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true),
    )
    .await
    .expect("synthetic SQLite database creates");
    seed.close().await;
    let sqlite = SqliteConnector::open(
        &path,
        false,
        ConnectorOptions {
            query_timeout_seconds: 5,
            ..Default::default()
        },
    )
    .await
    .expect("real SQLite connector opens");
    sqlite.connect().await.expect("real SQLite connects");
    let (first_pending, started) = oneshot::channel();
    let trace = Arc::new(ExecutionTrace {
        first_pending: AsyncMutex::new(Some(first_pending)),
        settled: AtomicBool::new(false),
        execute_calls: AtomicUsize::new(0),
        cancel_calls: AtomicUsize::new(0),
        cancel_outcome: AsyncMutex::new(None),
    });
    let mut registry = ConnectionRegistry::new("primary");
    registry.insert(
        "primary",
        ConnectionEntry {
            connector: Box::new(TracedSqlite {
                inner: sqlite,
                trace: trace.clone(),
            }),
            dialect: SqlDialect::Sqlite,
            profile_id: None,
        },
    );
    let (hook, captures) = observing_hook();
    let database = DatabaseTools::with_registry(registry, 1, true, None).with_capture(
        Some(hook),
        saya_agent::AgentLimits::default().context_byte_budget,
    );
    let tools = RunTools::compose(Arc::new(database), None, None, None);
    let events: AgentEvents = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink(events.clone());
    let cancellation = saya_agent::CancellationToken::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = Provider {
        calls: calls.clone(),
    };
    let run = run_agent_with_sink(
        &provider,
        &tools,
        AgentRequest {
            prompt: "run the slow query".into(),
            profile_names: vec!["primary".into()],
            model: "cancelled-real-sqlite".into(),
            system_prompt: None,
            history: Vec::new(),
            context_blocks: Vec::new(),
        },
        DatabaseTools::definitions(true, false, false, false, true),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &sink,
        cancellation.clone(),
    );
    tokio::pin!(run);
    tokio::select! {
        result = &mut run => panic!("agent settled before real SQLite reached Pending: {result:?}"),
        started = started => started.expect("real SQLite execute future reached Pending"),
    }
    cancellation.cancel();
    assert!(matches!(run.await, Err(saya_agent::AgentError::Cancelled)));
    assert!(
        trace.settled.load(Ordering::Acquire),
        "agent owner returns only after the real SQLite future settles"
    );
    assert_eq!(trace.cancel_calls.load(Ordering::Relaxed), 1);
    assert_eq!(
        trace.execute_calls.load(Ordering::Relaxed),
        1,
        "cancellation reaches no next requested SQL/tool effect"
    );
    assert_eq!(
        *trace.cancel_outcome.lock().await,
        Some(saya_connectors::CancelRequestOutcome::LocalInterruptRequested)
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "no next provider/tool turn"
    );
    assert!(captures.lock().unwrap().is_empty(), "no successful capture");
    assert!(
        events
            .lock()
            .unwrap()
            .iter()
            .all(|event| !matches!(event, saya_agent::AgentEvent::ToolCompleted { .. })),
        "cancellation does not promote ToolCompleted"
    );
    assert_eq!(
        events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| matches!(event, saya_agent::AgentEvent::ToolRequested { .. }))
            .count(),
        1,
        "the loop does not request the follow-up tool after cancellation"
    );
    std::fs::remove_dir_all(root).expect("synthetic fixture is removed");
}
