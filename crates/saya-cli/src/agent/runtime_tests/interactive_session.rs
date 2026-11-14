use super::*;

// The one tool universe, ask-gated. An interactive session queries the
// database and writes files — each via one ask — with no scope flags, over
// the shared RunTools composite and the session's bound workspace.
// ===========================================================================

/// The red test: an interactive session can query the database and write
/// files into the project — each via one ask, with no scope flags anywhere
/// in the composition.
#[tokio::test]
async fn an_interactive_session_queries_and_writes_each_via_one_ask() {
    use crate::interactive::session::universe::SessionUniverse;

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let project = std::env::temp_dir().join(format!(
        "saya-u1-session-project-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(project.join(".git")).unwrap();
    fs::create_dir_all(project.join("src")).unwrap();

    let mut runtime = test_runtime(default_memory());
    runtime.resolved.ai.provider = AiProvider::Ollama;
    runtime.resolved.ai.allow_data_sharing = true;
    let mut registry = ConnectionRegistry::new("primary");
    registry.insert(
        "primary",
        ConnectionEntry {
            connector: Box::new(RowConnector),
            dialect: SqlDialect::Sqlite,
            profile_id: None,
        },
    );
    let inputs = TurnInputs {
        ai: ResolvedAi {
            app_attribution: true,
            provider: AiProvider::Ollama,
            model: "test-model".into(),
            base_url: None,
            api_key: None,
            allow_data_sharing: true,
            temperature: 0.0,
            timeout_seconds: 60,
            idle_timeout_seconds: 90,
            request_timeout_seconds: 180,
            max_output_tokens: None,
            reasoning_effort: None,
            context_byte_budget: 256 * 1024,
            context_window_tokens: None,
            show_thinking: false,
            compaction: saya_config::CompactionMode::Auto,
            compaction_trigger_percent: 50,
            retry_delays_ms: vec![250, 500, 1000],
        },
        provider: Box::new(ThreeCallsProvider),
        registry,
        failures: Vec::new(),
    };
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
        knowledge_log: Arc::new(Mutex::new(Vec::new())),
    };
    // The session's universe, composed over the temp worktree — no scope
    // flags, no pre-authorisation, the ask is the gate.
    let session = Arc::new(
        SessionUniverse::compose(&runtime, None, None, true, &project)
            .expect("the session universe composes"),
    );
    let result = run_prompt_with_inputs(
        &runtime,
        inputs,
        "work with the data and the tree",
        saya_agent::ApprovalPolicy::Ask,
        true,
        true,
        Vec::new(),
        &sink,
        saya_agent::CancellationToken::new(),
        None,
        Some(Arc::new(AskYesDecider)),
        None,
        Some(Arc::clone(&session)),
        saya_agent::AgentMode::Build,
        // No capture hook in these headless turn tests.
        None,
        None,
    )
    .await
    .expect("the turn completes");
    assert_eq!(result.tool_metadata.len(), 3, "three calls ran");
    for metadata in &result.tool_metadata {
        assert_eq!(
            metadata.status, "completed",
            "every ask was answered and every call completed: {result:?}"
        );
    }
    // The file landed in the project, where the user works.
    assert!(
        project.join("src/notes.md").exists(),
        "the write lands in the project tree"
    );
    assert_eq!(
        fs::read_to_string(project.join("src/notes.md")).unwrap(),
        "written"
    );
    assert!(
        project.join("src/second.md").exists(),
        "the second write lands in the project tree too"
    );
    // The database call completed through the connector.
    let events = events.lock().unwrap();
    assert!(
        events.iter().any(|event| matches!(
            event,
            AgentEvent::ToolCompleted { name, .. } if name == "bounded_sql_query"
        )),
        "the database call completed: {events:?}"
    );
    assert!(
        events.iter().any(|event| matches!(
            event,
            AgentEvent::ToolRequested { name, .. } if name == "write"
        )),
        "the write was requested through the loop: {events:?}"
    );
    let _ = fs::remove_dir_all(&project);
}
