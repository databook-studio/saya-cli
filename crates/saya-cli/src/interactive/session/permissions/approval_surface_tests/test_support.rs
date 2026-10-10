use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use saya_agent::{AgentMode, ApprovalPolicy};

use crate::interactive::session::universe::SessionUniverse;

// ---------------------------------------------------------------------------
// harness
// ---------------------------------------------------------------------------

pub(super) fn temp_dir(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "saya-approval-surface-{label}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

/// A worktree-shaped project: `.git` present, so the walk binds its top.
pub(super) fn worktree(label: &str) -> PathBuf {
    let project = temp_dir(label);
    fs::create_dir_all(project.join(".git")).unwrap();
    project
}

fn session_runtime() -> crate::config::runtime::RuntimeConfig {
    crate::config::runtime::RuntimeConfig {
        resolved: saya_config::ResolvedConfig {
            coding: Default::default(),
            instructions: Default::default(),
            profile_name: None,
            profile: None,
            investigation_budgets: Default::default(),
            ai: saya_config::ResolvedAi {
                provider: saya_config::AiProvider::Ollama,
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
            jobs: saya_config::ResolvedJobs {
                wall_clock_seconds: None,
                tokens_per_endpoint: Default::default(),
                turns: Some(4),
                tool_calls: None,
                fetch: saya_config::ResolvedFetchJobs::default(),
                interpreter: saya_config::ResolvedInterpreterJobs::default(),
                runner: saya_config::ResolvedRunnerJobs::default(),
            },
            query_timeout_seconds: 5,
            output_format: saya_config::OutputFormat::Text,
            output_color: saya_config::ColorChoice::Auto,
            ui_theme: saya_config::ThemeChoice::Auto,
            memory: saya_config::ResolvedMemory {
                mode: saya_config::MemoryMode::Off,
                max_contracts: 5,
                max_claims_per_contract: 12,
                max_context_bytes: 16384,
            },
            host_commands: saya_config::ResolvedHostCommands::default(),
            session_deny: Default::default(),
            ignored_project_overrides: Vec::new(),
            provenance: Default::default(),
            endpoints: Default::default(),
        },
        connections: Default::default(),
        config_path: None,
        connections_path: None,
        cache_scope: PathBuf::from("/tmp/saya-approval-surface"),
        investigations_root: crate::config::runtime::temp_investigations_root(),
        secret_values: Default::default(),
    }
}

pub(super) fn compose(project: &Path, state_dir: &Path) -> SessionUniverse {
    SessionUniverse::compose(&session_runtime(), None, None, true, project, state_dir)
        .expect("composition succeeds on a plain worktree")
}

/// The write-shaped members the plain worktree composition carries: no
/// runner composes (nothing staged a program dir), so no `run_program` —
/// the honest shape, not a hidden capability. The unstated host lane does
/// compose over a bound root, so `run_command` rides.
pub(super) const WRITE_SHAPED: [&str; 7] = [
    "workspace_write",
    "workspace_edit",
    "scratch_sql",
    "scratch_import",
    "http_fetch",
    "http_download",
    "run_command",
];

pub(super) fn assert_host_tool_for_platform(names: &BTreeSet<String>) {
    if cfg!(windows) {
        assert!(
            !names.contains("run_command"),
            "Windows keeps the unavailable host lane out of the surface: {names:?}"
        );
    } else {
        assert!(
            names.contains("run_command"),
            "a composed Unix host lane advertises run_command: {names:?}"
        );
    }
}

pub(super) fn advertised_set(
    universe: &SessionUniverse,
    agent_mode: AgentMode,
    mode: ApprovalPolicy,
    can_obtain_approval: bool,
) -> BTreeSet<String> {
    universe
        .definitions(agent_mode, mode, can_obtain_approval, true, false, false)
        .into_iter()
        .map(|definition| definition.name)
        .collect()
}
