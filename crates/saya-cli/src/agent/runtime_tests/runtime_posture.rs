use super::*;

// Test 11: runtime turn with mode = off builds ConfiguredOff receipt and
// emits KnowledgeOutcome::Off.
// ===========================================================================

// ===========================================================================
// Test 12: runtime turn with closed privacy gate builds PrivacyGateClosed
// receipt and emits KnowledgeOutcome::Skipped.
// ===========================================================================

// ===========================================================================

#[tokio::test]
async fn runtime_turn_with_recall_off_emits_knowledge_outcome_off() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
        knowledge_log: log.clone(),
    };
    let provider = AnswerProvider {
        answer: "done",
        log: log.clone(),
        seen_requests: Arc::new(Mutex::new(Vec::new())),
    };
    let identity = identity_for("analytics");
    let inputs = TurnInputs {
        ai: ResolvedAi {
            provider: AiProvider::Ollama,
            model: "test-model".into(),
            base_url: None,
            api_key: None,
            allow_data_sharing: true,
            temperature: 0.0,
            timeout_seconds: 60,
            idle_timeout_seconds: 90,
            max_output_tokens: 4096,
            max_output_tokens_is_default: true,
            context_byte_budget: 256 * 1024,
            context_window_tokens: None,
            show_thinking: false,
            compaction: saya_config::CompactionMode::Auto,
            retry_delays_ms: vec![250, 500, 1000],
        },
        provider: Box::new(provider),
        registry: registry_for("analytics", &identity),
        failures: Vec::new(),
    };
    let mut memory = default_memory();
    memory.mode = saya_config::MemoryMode::Off;
    let runtime = test_runtime(memory);
    run_prompt_with_inputs(
        &runtime,
        inputs,
        "orders by month",
        saya_agent::ApprovalPolicy::ReadOnly,
        false,
        false,
        Vec::new(),
        &sink,
        saya_agent::CancellationToken::new(),
        None,
        None,
        None,
        None,
        saya_agent::AgentMode::Build,
        // No capture hook in these headless turn tests (C1).
        None,
        None,
    )
    .await
    .unwrap();

    let captured = events.lock().unwrap();
    let outcome = captured
        .iter()
        .find_map(|e| match e {
            AgentEvent::KnowledgeSupplied { outcome, .. } => Some(*outcome),
            _ => None,
        })
        .expect("KnowledgeSupplied present");
    assert_eq!(outcome, KnowledgeOutcome::Off);
}

/// Slice 3 — the session-aware system prompt reaches the provider: a session
/// turn names its connection and the session's bound root in the system
/// message, while a session-less turn says no workspace is bound. The
/// provider records the system message it saw, so this asserts the wiring —
/// not just the rendering — end to end through `run_prompt_with_inputs`.
#[tokio::test]
async fn session_turn_system_prompt_names_connection_and_workspace_root() {
    use crate::interactive::session::universe::SessionUniverse;

    let answer_seen: Arc<Mutex<Option<Option<String>>>> = Arc::new(Mutex::new(None));
    struct CapturingProvider {
        answer_seen: Arc<Mutex<Option<Option<String>>>>,
    }
    #[async_trait]
    impl ChatProvider for CapturingProvider {
        fn name(&self) -> &str {
            "capturing"
        }
        async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
            *self.answer_seen.lock().unwrap() = Some(
                request
                    .messages
                    .iter()
                    .find(|m| m.role == "system")
                    .map(|m| m.content.clone()),
            );
            Ok(ChatResponse::new(ChatMessage::text("assistant", "done")))
        }
    }

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let project =
        std::env::temp_dir().join(format!("saya-facts-project-{}-{stamp}", std::process::id()));
    fs::create_dir_all(project.join(".git")).unwrap();
    let state_dir =
        std::env::temp_dir().join(format!("saya-facts-state-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&state_dir).unwrap();

    let runtime = test_runtime(default_memory());
    let identity = identity_for("analytics");
    let seen = Arc::clone(&answer_seen);
    let inputs = TurnInputs {
        ai: ResolvedAi {
            provider: AiProvider::Ollama,
            model: "test-model".into(),
            base_url: None,
            api_key: None,
            allow_data_sharing: true,
            temperature: 0.0,
            timeout_seconds: 60,
            idle_timeout_seconds: 90,
            max_output_tokens: 4096,
            max_output_tokens_is_default: true,
            context_byte_budget: 256 * 1024,
            context_window_tokens: None,
            show_thinking: false,
            compaction: saya_config::CompactionMode::Auto,
            retry_delays_ms: vec![250, 500, 1000],
        },
        provider: Box::new(CapturingProvider {
            answer_seen: Arc::clone(&answer_seen),
        }),
        registry: registry_for("analytics", &identity),
        failures: Vec::new(),
    };
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events,
        knowledge_log: Arc::new(Mutex::new(Vec::new())),
    };
    let session = Arc::new(
        SessionUniverse::compose(&runtime, None, None, true, &project, &state_dir)
            .expect("the session universe composes"),
    );
    run_prompt_with_inputs(
        &runtime,
        inputs,
        "orders by month",
        saya_agent::ApprovalPolicy::ReadOnly,
        false,
        false,
        Vec::new(),
        &sink,
        saya_agent::CancellationToken::new(),
        None,
        None,
        None,
        Some(Arc::clone(&session)),
        saya_agent::AgentMode::Build,
        // No capture hook in these headless turn tests (C1).
        None,
        None,
    )
    .await
    .expect("turn completes");
    let system = seen
        .lock()
        .unwrap()
        .clone()
        .flatten()
        .expect("a system message");
    assert!(
        system.contains("Session facts"),
        "the session turn carries the facts section: {system}"
    );
    assert!(
        system.contains("analytics"),
        "the facts name the connection in scope: {system}"
    );
    assert!(
        system.contains(&project.canonicalize().unwrap().display().to_string()),
        "the facts name the session's bound root: {system}"
    );
    let _ = fs::remove_dir_all(&project);
    let _ = fs::remove_dir_all(&state_dir);
}

#[path = "runtime_posture/privacy_gate.rs"]
mod privacy_gate;
