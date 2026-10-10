/// A minimal `RuntimeConfig` carrying only what `run_prompt_with_inputs` reads
/// (`resolved.memory`, `max_rows`, `max_iterations`). The provider and registry
/// are injected via `TurnInputs`, so the live-build fields are never used.
fn test_runtime(memory: ResolvedMemory) -> RuntimeConfig {
    RuntimeConfig {
        resolved: ResolvedConfig {
            coding: Default::default(),
            instructions: Default::default(),
            profile_name: None,
            profile: None,
            investigation_budgets: Default::default(),
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
            max_rows: 100,
            read_only: true,
            max_iterations: 4,
            candidates: 1,
            jobs: ResolvedJobs {
                wall_clock_seconds: None,
                tokens_per_endpoint: BTreeMap::new(),
                turns: Some(4),
                tool_calls: None,
                fetch: ResolvedFetchJobs::default(),
                interpreter: ResolvedInterpreterJobs::default(),
                runner: ResolvedRunnerJobs::default(),
            },
            query_timeout_seconds: 5,
            output_format: OutputFormat::Text,
            output_color: ColorChoice::Auto,
            ui_theme: ThemeChoice::Auto,
            memory,
            host_commands: ResolvedHostCommands::default(),
            session_deny: Default::default(),
            ignored_project_overrides: Vec::new(),
            provenance: Default::default(),
            endpoints: BTreeMap::new(),
        },
        connections: Default::default(),
        config_path: None,
        connections_path: None,
        cache_scope: PathBuf::from("/tmp/saya-runtime-p1b"),
        investigations_root: crate::config::runtime::temp_investigations_root(),
        secret_values: BTreeMap::new(),
    }
}

fn default_memory() -> ResolvedMemory {
    ResolvedMemory {
        mode: MemoryMode::Off,
        max_contracts: 5,
        max_claims_per_contract: 12,
        max_context_bytes: 16384,
    }
}

/// `assisted` memory: permits candidate writes and recalls active + candidate knowledge.
fn assisted_memory() -> ResolvedMemory {
    ResolvedMemory {
        mode: MemoryMode::Assisted,
        max_contracts: 5,
        max_claims_per_contract: 12,
        max_context_bytes: 16384,
    }
}

/// A sink that records every event in order. `knowledge_log`, when shared with
/// the provider, is the ordering oracle: the sink pushes `"knowledge"` when it
/// sees `KnowledgeSupplied`, the provider pushes `"provider"` when it is
/// called, and the order in the log proves the emit preceded the request.
struct RecordingSink {
    events: Arc<Mutex<Vec<AgentEvent>>>,
    knowledge_log: Arc<Mutex<Vec<&'static str>>>,
}

#[async_trait]
impl AgentEventSink for RecordingSink {
    async fn emit(&self, event: AgentEvent) {
        if matches!(event, AgentEvent::KnowledgeSupplied { .. }) {
            self.knowledge_log.lock().unwrap().push("knowledge");
        }
        self.events.lock().unwrap().push(event);
    }
}

/// A provider that returns one text answer, recording `"provider"` into the
/// shared log at call time so the ordering test can prove the emit came first.
/// Every request it receives is recorded into `seen_requests`, shared with the
/// test, so assertions read the exact messages the composition sent — the
/// TA-04 test below asserts on them: a recalled claim is user-side data,
/// never a system message.
struct AnswerProvider {
    answer: &'static str,
    log: Arc<Mutex<Vec<&'static str>>>,
    seen_requests: Arc<Mutex<Vec<ChatRequest>>>,
}

#[async_trait]
impl ChatProvider for AnswerProvider {
    fn name(&self) -> &str {
        "answer"
    }
    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        self.log.lock().unwrap().push("provider");
        self.seen_requests.lock().unwrap().push(request);
        Ok(ChatResponse::new(ChatMessage::text(
            "assistant",
            self.answer,
        )))
    }
}

// ===========================================================================
// Test 1: a turn that supplies claims emits exactly one KnowledgeSupplied,
// naming those claims.
// ===========================================================================
