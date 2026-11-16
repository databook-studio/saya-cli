use std::path::PathBuf;

use saya_agent::ApprovalPolicy;
use saya_config::{
    AiProvider, ColorChoice, MemoryMode, OutputFormat, ResolvedAi, ResolvedConfig,
    ResolvedHostCommands, ResolvedJobs, ResolvedMemory, ThemeChoice,
};

use super::super::SessionRuntime;
use crate::interactive::session::state::SessionState;

/// A throwaway sessions root, named per test so two parallel tests never
/// share a state directory.
fn root_for(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "saya-mode-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("the fixture root");
    root
}

/// The fixture runtime: no profiles, no endpoints, nothing composed that
/// could touch the machine. A hand-built `RuntimeConfig` is the established
/// shape for a session-level test (`recovery_acquire_tests`, `question_tests`).
fn test_runtime() -> crate::config::runtime::RuntimeConfig {
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
        cache_scope: PathBuf::from("/tmp/saya-mode"),
        investigations_root: crate::config::runtime::temp_investigations_root(),
        secret_values: Default::default(),
    }
}

/// An approval surface that approves everything put to it: the stand-in for
/// the modal or the terminal prompt when a test drives a session's own ask
/// path — the plan gate, which has no prompt of its own.
pub(super) struct Approves;

#[async_trait::async_trait]
impl saya_agent::ApprovalDecider for Approves {
    async fn approve(
        &self,
        _tool: &saya_agent::ToolDefinition,
        _arguments: &serde_json::Value,
    ) -> bool {
        true
    }
}

/// Removes the throwaway root when the fixture drops; declared after the
/// session runtime so the lock releases before the directory goes.
struct RootGuard(PathBuf);

impl Drop for RootGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// One acquired session, with the state a launch would have handed it.
pub(super) struct Fixture {
    pub(super) state: SessionState,
    pub(super) session: SessionRuntime,
    _root: RootGuard,
}

impl Fixture {
    /// A session running under `approval`, with the process's bypass gate
    /// set as the caller states it — the one field `/mode` and the key cycle
    /// read.
    pub(super) fn acquire(tag: &str, approval: ApprovalPolicy, allow_bypass: bool) -> Self {
        let root = root_for(tag);
        let sessions = root.join("sessions");
        std::fs::create_dir_all(&sessions).expect("the sessions root");
        let session = SessionRuntime::acquire(
            &test_runtime(),
            None,
            false,
            None,
            "20260101-000000-mode",
            approval,
            &sessions,
        )
        .expect("the fixture session acquires");
        let mut state = SessionState::new("20260101-000000-mode", None, "test-model");
        state.approval_mode = crate::interactive::session::commands::approval_name(approval);
        state.allow_bypass = allow_bypass;
        Self {
            state,
            session,
            _root: RootGuard(root),
        }
    }
}
