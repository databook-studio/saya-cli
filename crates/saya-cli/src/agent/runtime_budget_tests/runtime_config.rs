fn test_ai(context_byte_budget: usize) -> ResolvedAi {
    ResolvedAi {
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
        context_byte_budget,
        context_window_tokens: None,
        show_thinking: false,
        compaction: saya_config::CompactionMode::Auto,
        retry_delays_ms: vec![250, 500, 1000],
    }
}

/// A minimal `RuntimeConfig` carrying only what the turn reads. The provider
/// and registry arrive via `TurnInputs`; memory is off, so recall and the
/// post-turn extraction are skipped and only the loop's limits are in play.
fn test_runtime(context_byte_budget: usize) -> RuntimeConfig {
    RuntimeConfig {
        resolved: ResolvedConfig {
            coding: Default::default(),
            instructions: Default::default(),
            profile_name: None,
            profile: None,
            investigation_budgets: Default::default(),
            ai: test_ai(context_byte_budget),
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
            memory: ResolvedMemory {
                mode: MemoryMode::Off,
                max_contracts: 5,
                max_claims_per_contract: 12,
                max_context_bytes: 16384,
            },
            host_commands: ResolvedHostCommands::default(),
            session_deny: Default::default(),
            ignored_project_overrides: Vec::new(),
            provenance: Default::default(),
            endpoints: BTreeMap::new(),
        },
        connections: Default::default(),
        config_path: None,
        connections_path: None,
        cache_scope: PathBuf::from("/tmp/saya-runtime-budget-tests"),
        investigations_root: crate::config::runtime::temp_investigations_root(),
        secret_values: BTreeMap::new(),
    }
}
