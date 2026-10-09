/// Two turn steps — a `bounded_sql_query` call (object activity) followed by
/// a non-trivial answer — so the gate admits extraction the same way
/// `TurnAndExtractionProvider`'s callers above do.
fn breaker_turn_steps(answer: &'static str) -> Vec<ChatResponse> {
    vec![
        ChatResponse::new(ChatMessage {
            role: "assistant".into(),
            content: String::new(),
            tool_calls: vec![ToolCall {
                id: "call-1".into(),
                name: "bounded_sql_query".into(),
                arguments: serde_json::json!({
                    "connection": "analytics",
                    "sql": "SELECT id, status FROM catalog.public.orders",
                }),
            }],
            tool_call_id: None,
        }),
        ChatResponse::new(ChatMessage::text("assistant", answer)),
    ]
}

/// The `TurnInputs` every breaker test shares: only the provider varies.
fn breaker_inputs(provider: Box<dyn ChatProvider>, identity: &ProfileIdentity) -> TurnInputs {
    TurnInputs {
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
        provider,
        registry: registry_for("analytics", identity),
        failures: Vec::new(),
    }
}

/// Runs one breaker-test turn against the shared harness shape, returning the
/// captured events and the turn's answer.
#[allow(clippy::too_many_arguments)]
async fn run_breaker_turn(
    runtime: &RuntimeConfig,
    provider: Box<dyn ChatProvider>,
    identity: &ProfileIdentity,
    store: SqliteStateStore,
    session: Option<Arc<crate::interactive::session::universe::SessionUniverse>>,
) -> (Vec<AgentEvent>, String) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
        knowledge_log: Arc::new(Mutex::new(Vec::new())),
    };
    let inputs = breaker_inputs(provider, identity);
    let out = run_prompt_with_inputs(
        runtime,
        inputs,
        "table orders has alias orders",
        saya_agent::ApprovalPolicy::ReadOnly,
        false,
        false,
        Vec::new(),
        &sink,
        saya_agent::CancellationToken::new(),
        Some(store),
        None,
        None,
        session,
        saya_agent::AgentMode::Build,
        // No capture hook in these headless turn tests (C1).
        None,
        None,
    )
    .await
    .expect("turn completes (fail-soft)");
    let captured = events.lock().unwrap().clone();
    (captured, out.answer)
}

fn disabled_event(events: &[AgentEvent]) -> Option<(&str, u32)> {
    events.iter().find_map(|event| match event {
        AgentEvent::KnowledgeLearningDisabled { model, misses } => Some((model.as_str(), *misses)),
        _ => None,
    })
}

fn skipped_reasons(events: &[AgentEvent]) -> Vec<LearningSkipReason> {
    events
        .iter()
        .filter_map(|event| match event {
            AgentEvent::KnowledgeLearningSkipped { reason } => Some(*reason),
            _ => None,
        })
        .collect()
}

/// A provider that answers the turn normally, then — for the extraction
/// call — waits briefly before returning a valid proposal. Tokio's paused
/// clock (`#[tokio::test(start_paused = true)]`) is not reachable here: the
/// workspace's `tokio` dependency does not enable the `test-util` feature
/// (Cargo.toml is outside this packet's owned paths), so this uses a short
/// real sleep instead of simulating minutes. What is asserted is unchanged
/// either way: the runtime awaits the call to completion — no
/// `tokio::time::timeout` races it — and the proposal it returns is stored.
struct SlowExtractionProvider {
    turn_step: Mutex<usize>,
    turn_steps: Vec<ChatResponse>,
}

#[async_trait]
impl ChatProvider for SlowExtractionProvider {
    fn name(&self) -> &str {
        "slow-extraction-provider"
    }
    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let is_extraction = request
            .messages
            .first()
            .map(|m| m.content.contains("precision schema knowledge extractor"))
            .unwrap_or(false);
        if is_extraction {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            Ok(ChatResponse::new(ChatMessage::text(
                "assistant",
                r#"{"proposals": [{"object_id": "T0", "slot": "table.alias", "value": "orders", "origin": "user_explicit"}]}"#,
            )))
        } else {
            let mut step = self.turn_step.lock().unwrap();
            let idx = *step;
            *step += 1;
            if idx < self.turn_steps.len() {
                Ok(self.turn_steps[idx].clone())
            } else {
                Ok(ChatResponse::new(ChatMessage::text("assistant", "done")))
            }
        }
    }
}
