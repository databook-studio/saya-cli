fn temp_dir(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "saya-session-universe-{label}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

/// A worktree-shaped project: `.git` present, so the walk binds its top.
fn worktree(label: &str) -> PathBuf {
    let project = temp_dir(label);
    fs::create_dir_all(project.join(".git")).unwrap();
    project
}

fn session_runtime(
    runner: Option<(Vec<String>, Option<PathBuf>)>,
) -> crate::config::runtime::RuntimeConfig {
    crate::config::runtime::RuntimeConfig {
        resolved: ResolvedConfig {
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
                runner: match runner {
                    Some((allow, program_dir)) => ResolvedRunnerJobs {
                        allow,
                        program_dir,
                        timeout_seconds: 300,
                    },
                    None => ResolvedRunnerJobs::default(),
                },
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
            endpoints: Default::default(),
        },
        connections: Default::default(),
        config_path: None,
        connections_path: None,
        cache_scope: PathBuf::from("/tmp/saya-session-universe"),
        investigations_root: crate::config::runtime::temp_investigations_root(),
        secret_values: Default::default(),
    }
}

fn compose(
    runtime: &crate::config::runtime::RuntimeConfig,
    project: &Path,
    state_dir: &Path,
) -> SessionUniverse {
    compose_result(runtime, project, state_dir).expect("composition succeeds on a plain worktree")
}

fn compose_result(
    runtime: &crate::config::runtime::RuntimeConfig,
    cwd: &Path,
    state_dir: &Path,
) -> Result<SessionUniverse, String> {
    SessionUniverse::compose(runtime, None, None, true, cwd, state_dir)
}

const SESSION_WRITE_TOOLS: [&str; 7] = [
    "workspace_write",
    "workspace_edit",
    "scratch_sql",
    "scratch_import",
    "http_fetch",
    "http_download",
    "run_program",
];

fn advertised(
    universe: &SessionUniverse,
    agent_mode: AgentMode,
    mode: ApprovalPolicy,
    can_obtain_approval: bool,
) -> Vec<String> {
    universe
        .definitions(agent_mode, mode, can_obtain_approval, true, false, false)
        .into_iter()
        .map(|definition| definition.name)
        .collect()
}
