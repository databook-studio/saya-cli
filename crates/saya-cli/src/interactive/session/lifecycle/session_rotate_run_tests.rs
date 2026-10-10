//! `/rotate`'s runner against a real store and a real runtime: what a
//! completed rotation leaves on disk, what a refusal leaves untouched, and
//! what a failed swap keeps held.

use super::{RotateResult, rotate_session_with_id};
use crate::interactive::session::continuation::compaction::manual::{
    apply_with_continuation, plan as compact_plan,
};
use crate::interactive::session::lifecycle::resume::{SessionDefaults, block_on, resume_session};
use crate::interactive::session::runtime::SessionRuntime;
use crate::interactive::session::state::SessionState;
use saya_agent::ApprovalPolicy;
use saya_config::{
    AiProvider, ColorChoice, MemoryMode, OutputFormat, ResolvedAi, ResolvedConfig,
    ResolvedFetchJobs, ResolvedHostCommands, ResolvedInterpreterJobs, ResolvedJobs, ResolvedMemory,
    ResolvedRunnerJobs, ThemeChoice,
};
use saya_store::{FsSessionStore, ParentLink, SessionStore};
use std::path::PathBuf;

const SUMMARY: &str = "older turns discussed orders";
const OLD: &str = "20260101-000000-old0";
const SUCCESSOR: &str = "20260101-000000-new0";

/// Removes the throwaway root when the fixture drops; declared after the
/// session runtime so the locks release before the directory goes.
struct RootGuard(std::path::PathBuf);

impl Drop for RootGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Fixture {
    store: FsSessionStore,
    runtime: crate::config::runtime::RuntimeConfig,
    state: SessionState,
    session: SessionRuntime,
    /// Kept for the drop order: the guard is the last field, so the session
    /// locks release before the directory goes. The sessions root hangs off
    /// it, so a re-acquire re-opens the same tree.
    root: RootGuard,
}

/// The runtime fixture the interactive-level question tests build: a
/// resolved config with no profiles and no endpoints, so composition
/// touches nothing real.
fn test_runtime() -> crate::config::runtime::RuntimeConfig {
    crate::config::runtime::RuntimeConfig {
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
            provenance: Default::default(),
            endpoints: Default::default(),
        },
        connections: Default::default(),
        config_path: None,
        connections_path: None,
        cache_scope: PathBuf::from("/tmp/saya-rotate-run"),
        investigations_root: crate::config::runtime::temp_investigations_root(),
        secret_values: Default::default(),
    }
}

impl Fixture {
    /// A live session (`OLD`) over a nested store root: the tombstone dir
    /// lives at the root's parent, so every fixture owns its own.
    fn build(tag: &str) -> Self {
        Self::build_with_pin(tag, None)
    }

    /// A live session over a bound root: the pin rides the state, so the
    /// recomposed universe composes the host lane (a root binds — no root,
    /// no lane), and a rotation's re-acquire re-binds it for the successor.
    fn build_pinned(tag: &str) -> Self {
        let root = Self::root_for(tag);
        let project = root.join("project");
        std::fs::create_dir_all(&project).expect("the pinned project dir");
        Self::build_with_pin(tag, Some(project.display().to_string()))
    }

    fn root_for(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "saya-rotate-run-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ))
    }

    fn build_with_pin(tag: &str, pin: Option<String>) -> Self {
        let root = Self::root_for(tag);
        std::fs::create_dir_all(&root).expect("rotate fixture root");
        let sessions = root.join("sessions");
        let runtime = test_runtime();
        // `fresh: false` with no explicit statement composes from the pin
        // alone: with a pin the lane composes, without one the universe is
        // unbound and never walks up to the test cwd's worktree.
        let session = SessionRuntime::acquire(
            &runtime,
            None,
            false,
            pin.as_deref(),
            OLD,
            ApprovalPolicy::Ask,
            &sessions,
        )
        .expect("the fixture session acquires");
        let mut state = compacted_state(OLD);
        state.workspace_root = pin;
        Self {
            store: FsSessionStore::new(&sessions),
            runtime,
            state,
            session,
            root: RootGuard(root),
        }
    }

    fn reacquire(&self, id: &str) -> Result<SessionRuntime, String> {
        SessionRuntime::acquire(
            &self.runtime,
            None,
            false,
            None,
            id,
            ApprovalPolicy::Ask,
            &self.root.0.join("sessions"),
        )
    }
}

/// A compacted parent through the production compaction path.
fn compacted_state(id: &str) -> SessionState {
    let mut state = SessionState::new(id, None, "m");
    for index in 0..6 {
        state.record_turn(format!("q{index}"), format!("a{index}"), false, Vec::new());
    }
    let history = state.provider_history();
    let plan = compact_plan(&state.turns, &history).expect("six turns plan");
    apply_with_continuation(&mut state, &plan, SUMMARY, &|_| None).expect("the summary applies");
    state
}

include!("session_rotate_run_tests/rotation_outcomes.rs");

include!("session_rotate_run_tests/successor_posture.rs");
