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

/// Whether this host can prove the write-denied sandbox the `read-only`
/// mode's only shell lane needs. macOS's seatbelt expresses the denial;
/// Linux's Landlock is allow-only and refuses it, so a read-only session has
/// no shell lane there — the product's own `shell_policy::prove` answer,
/// not a platform guess.
fn read_only_sandbox_proves(project: &Path) -> bool {
    crate::agent::tools::shell_policy::prove(project, ApprovalPolicy::ReadOnly).is_some()
}

/// Whether this host can prove the sandbox an `ask` session's headless lane
/// needs when no operator can answer. Same product answer, different policy:
/// macOS proves it, Linux's allow-only Landlock does not.
fn ask_sandbox_proves(project: &Path) -> bool {
    crate::agent::tools::shell_policy::prove(project, ApprovalPolicy::Ask).is_some()
}

fn session_runtime() -> crate::config::runtime::RuntimeConfig {
    crate::config::runtime::RuntimeConfig {
        resolved: ResolvedConfig {
            hooks: Default::default(),
            web_search: Default::default(),
            mcp_servers: Default::default(),
            permissions: Default::default(),
            tools: Default::default(),
            coding: Default::default(),
            agents: Default::default(),
            instructions: Default::default(),
            profile_name: None,
            profile: None,
            limits: Default::default(),
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
            max_rows: 100,
            read_only: true,
            candidates: 1,
            jobs: ResolvedJobs::default(),
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
    _state_dir: &Path,
) -> SessionUniverse {
    compose_result(runtime, project, _state_dir).expect("composition succeeds on a plain worktree")
}

fn compose_result(
    runtime: &crate::config::runtime::RuntimeConfig,
    cwd: &Path,
    _state_dir: &Path,
) -> Result<SessionUniverse, String> {
    SessionUniverse::compose(runtime, None, None, true, cwd)
}

const SESSION_WRITE_TOOLS: [&str; 4] = [
    "write",
    "edit",
    "http_fetch",
    "http_download",
];

/// The names `universe` advertises under `agent_mode` and `mode`, read off the
/// session's own gate — the same cell the turn's `definitions` call reads, so a
/// test that approves a plan sees the advertisement the next turn would get.
fn advertised(
    universe: &SessionUniverse,
    agent_mode: AgentMode,
    mode: ApprovalPolicy,
    can_obtain_approval: bool,
) -> Vec<String> {
    universe
        .definitions(
            agent_mode,
            universe.plan_gate_open(),
            mode,
            can_obtain_approval,
            true,
            false,
            false,
            &Default::default(),
        )
        .into_iter()
        .map(|definition| definition.name)
        .collect()
}
