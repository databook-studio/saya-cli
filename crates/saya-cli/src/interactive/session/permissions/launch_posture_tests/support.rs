use crate::interactive::session::runtime::SessionRuntime;
use crate::interactive::session::state::SessionState;
use crate::interactive::session_host::HostLaunch;
use saya_agent::ApprovalPolicy;
use saya_config::{
    AiProvider, ColorChoice, CompactionMode, MemoryMode, OutputFormat, ResolvedAi, ResolvedConfig,
    ResolvedFetchJobs, ResolvedHostCommands, ResolvedInterpreterJobs, ResolvedJobs, ResolvedMemory,
    ResolvedRunnerJobs, ResolvedSessionDeny, ThemeChoice,
};
use saya_store::JournalEvent;
use std::path::Path;

pub(super) const ID: &str = "20260101-000000-lnc0";
pub(super) const DENIED: &str = "curl";

/// Removes the throwaway root when the fixture drops; declared after the
/// session runtime so the lock releases before the directory goes.
pub(super) struct RootGuard(std::path::PathBuf);

impl Drop for RootGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub(super) struct Fixture {
    pub(super) state: SessionState,
    pub(super) session: SessionRuntime,
    /// Kept last so the session's lock releases before the directory goes.
    _root: RootGuard,
}

impl Fixture {
    /// The recorded shape every launch kind loads: the two header facts are
    /// `#[serde(skip)]`, so any state arrives as the defaults (lane off, no
    /// deny list) no matter what the previous process ran under.
    pub(super) fn state() -> SessionState {
        SessionState::new(ID, None, "test-model")
    }

    /// A fresh-shaped launch: `fresh: true` with the explicit statement, then
    /// the launch recomposition `run()` performs before deriving the posture.
    /// The launch carries the `--deny curl` refusal.
    pub(super) fn build_fresh(tag: &str) -> Self {
        let root = root_for(tag);
        std::fs::create_dir_all(&root).expect("the fixture root");
        let sessions = root.join("sessions");
        let runtime = test_runtime(&[]);
        let project = project_dir(&root);
        let mut session = SessionRuntime::acquire(
            &runtime,
            Some(Path::new(&project)),
            true,
            None,
            ID,
            ApprovalPolicy::Ask,
            &sessions,
        )
        .expect("the fresh fixture session acquires");
        let launch = HostLaunch::from_deny_for_tests(vec![DENIED.to_owned()]);
        session
            .recompose_with_launch(&runtime, None, Some(&launch))
            .expect("the fresh launch recomposes");
        Self {
            state: Self::state(),
            session,
            _root: RootGuard(root),
        }
    }

    /// A resumed-shaped launch (`--resume`): `fresh: false`, the recorded pin
    /// rides the state, and the launch statement is gone — the universe is
    /// what `acquire` composed, deny list from the user-layer config alone.
    pub(super) fn build_resumed(tag: &str, pin: Option<String>, deny: &[&str]) -> Self {
        let root = root_for(tag);
        std::fs::create_dir_all(&root).expect("the fixture root");
        let sessions = root.join("sessions");
        let runtime = test_runtime(deny);
        let session = SessionRuntime::acquire(
            &runtime,
            None,
            false,
            pin.as_deref(),
            ID,
            ApprovalPolicy::Ask,
            &sessions,
        )
        .expect("the resumed fixture session acquires");
        let mut state = Self::state();
        state.workspace_root = pin;
        Self {
            state,
            session,
            _root: RootGuard(root),
        }
    }

    /// A `--continue`-shaped launch: `fresh: false` re-opening the most
    /// recent session — the same non-fresh composition path `--resume` takes.
    pub(super) fn build_continued(tag: &str, pin: Option<String>, deny: &[&str]) -> Self {
        Self::build_resumed(tag, pin, deny)
    }
}

pub(super) fn root_for(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "saya-launch-posture-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ))
}

pub(super) fn project_dir(root: &std::path::Path) -> String {
    let project = root.join("project");
    std::fs::create_dir_all(&project).expect("the fixture project dir");
    project.display().to_string()
}

/// The runtime fixture: no profiles and no endpoints, so composition touches
/// nothing real; `deny` is the user-layer `[session_commands] deny` the
/// composer folds into every universe.
fn test_runtime(deny: &[&str]) -> crate::config::runtime::RuntimeConfig {
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
                compaction: CompactionMode::Auto,
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
            session_deny: ResolvedSessionDeny {
                programs: deny.iter().map(|name| (*name).to_owned()).collect(),
            },
            ignored_project_overrides: Vec::new(),
            provenance: Default::default(),
            endpoints: Default::default(),
        },
        connections: Default::default(),
        config_path: None,
        connections_path: None,
        cache_scope: std::path::PathBuf::from("/tmp/saya-launch-posture"),
        investigations_root: crate::config::runtime::temp_investigations_root(),
        secret_values: Default::default(),
    }
}

/// The `DenyList` journal events, in order — the deny list the launch wrote.
pub(super) fn deny_journal_events(session: &SessionRuntime) -> Vec<Vec<String>> {
    session
        .journal()
        .read()
        .expect("the journal reads")
        .into_iter()
        .filter_map(|event| match event {
            JournalEvent::DenyList { programs } => Some(programs),
            _ => None,
        })
        .collect()
}
