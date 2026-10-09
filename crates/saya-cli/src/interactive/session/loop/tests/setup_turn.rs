#[cfg(test)]
mod setup_turn_tests {
    use super::*;
    use saya_agent::ApprovalPolicy;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use saya_config::{
        AiProvider, ColorChoice, ConnectionsFile, MemoryMode, OutputFormat, ResolvedAi,
        ResolvedConfig, ResolvedFetchJobs, ResolvedHostCommands, ResolvedInterpreterJobs,
        ResolvedJobs, ResolvedMemory, ResolvedRunnerJobs, ThemeChoice,
    };

    /// The explicit runtime the interactive test modules hand-build (the
    /// `session_headless_receipt_tests` pattern): no profile configured, no
    /// provider reachable — the refusal must fire before any connector.
    fn handoff_runtime() -> RuntimeConfig {
        RuntimeConfig {
            resolved: ResolvedConfig {
                provenance: Default::default(),
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
                    tokens_per_endpoint: Default::default(),
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
                endpoints: BTreeMap::new(),
            },
            connections: ConnectionsFile::default(),
            config_path: None,
            connections_path: None,
            cache_scope: PathBuf::new(),
            investigations_root: crate::config::runtime::temp_investigations_root(),
            secret_values: BTreeMap::new(),
        }
    }

    /// `/setup` in a headless turn is refused with the `saya setup` pointer
    /// and ends errored — `--turn-file` exits 5. The refusal text is emitted
    /// through `emit_action`'s private `print!` (the receipt-test limit), so
    /// the wording is pinned on the shared constant and the outcome through
    /// the real per-turn entry is the proof here.
    #[test]
    fn setup_turn_refuses_with_the_saya_setup_pointer_and_ends_errored() {
        const REFUSAL: &str = crate::interactive::session::commands::SETUP_TUI_ONLY_ERROR;
        assert!(
            REFUSAL.contains("full-screen terminal") && REFUSAL.contains("`saya setup`"),
            "the refusal names the surface and the command to run: {REFUSAL}"
        );
        let root = std::env::temp_dir().join(format!(
            "saya-setup-turn-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("the setup-turn root");
        let runtime = handoff_runtime();
        let store = FsSessionStore::new(root.join("session-store"));
        let state_db = SqliteStateStore::new(PathBuf::new());
        let state = &mut SessionState::new("setup-turn", None, "test-model");
        let session = &mut SessionRuntime::acquire(
            &runtime,
            Some(&root),
            true,
            None,
            "setup-turn",
            ApprovalPolicy::Ask,
            &root.join("session-dirs"),
        )
        .expect("the setup-turn session acquires");
        let mut ctx = TurnContext {
            state: &mut *state,
            runtime: Arc::new(runtime),
            store: &store,
            state_db: &state_db,
            format: RenderFormat::Text,
            terminal: false,
            session: &mut *session,
            trusted_echo: None,
            last_turn: None,
            save_refused: false,
            launch_options: GlobalOptions::default(),
        };
        let outcome = handle_line_verbatim("/setup", &mut ctx).expect("the turn runs");
        assert!(
            matches!(outcome, TurnOutcome::Errored),
            "the headless /setup turn ends errored — never completed"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
