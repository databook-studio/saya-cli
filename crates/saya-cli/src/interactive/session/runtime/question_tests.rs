//! `settle_question` settles by the completing scope's identity: only the
//! scope `begin_question` handed out may settle itself, so a late completion
//! of a stale question can never restore or change the live question's
//! state. One table drives a real `SessionRuntime` and real `QuestionScope`
//! values through every transition.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use saya_agent::ApprovalPolicy;
use saya_config::{
    AiProvider, ColorChoice, MemoryMode, OutputFormat, ResolvedAi, ResolvedConfig,
    ResolvedFetchJobs, ResolvedHostCommands, ResolvedInterpreterJobs, ResolvedJobs, ResolvedMemory,
    ResolvedRunnerJobs, ThemeChoice,
};
use saya_store::{FsSessionStore, SessionStore, SqliteStateStore};

use super::super::super::tui::loop_tick::tick_resume;
use super::super::super::tui::types::App;
use super::super::SessionState;
use super::super::universe::SessionUniverse;
use super::{QuestionScope, SessionRuntime};
use crate::interactive::session::lifecycle::resume::{SessionDefaults, block_on, resume_session};

/// A named sessions root so parallel tests never share state directories.
fn temp_sessions_root_named(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("saya-session-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

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
        cache_scope: PathBuf::from("/tmp/saya-session-question"),
        investigations_root: crate::config::runtime::temp_investigations_root(),
        secret_values: Default::default(),
    }
}

struct Harness {
    runtime: crate::config::runtime::RuntimeConfig,
    session: SessionRuntime,
    sessions_root: PathBuf,
    /// Every scope the walk has begun, in order: the stale rows settle one
    /// of these by index rather than minting a synthetic one.
    begun: Vec<QuestionScope>,
}

impl Harness {
    fn new() -> Self {
        Self::build("question", "question-scope-walk")
    }

    fn build(tag: &str, id: &str) -> Self {
        let sessions_root = temp_sessions_root_named(tag);
        let runtime = test_runtime();
        let session = SessionRuntime::acquire(
            &runtime,
            None,
            false,
            None,
            id,
            ApprovalPolicy::Ask,
            &sessions_root,
        )
        .expect("a real session runtime acquires");
        Self {
            runtime,
            session,
            sessions_root,
            begun: Vec::new(),
        }
    }

    fn begin(&mut self, label: &str) -> QuestionScope {
        let scope = self
            .session
            .begin_question(&self.runtime)
            .unwrap_or_else(|error| panic!("{label}: the question begins: {error}"));
        self.begun.push(scope.clone());
        scope
    }

    fn begin_refused(&mut self, label: &str) {
        let error = match self.session.begin_question(&self.runtime) {
            Ok(_) => panic!("{label}: the overlapping begin refuses while a question is active"),
            Err(error) => error,
        };
        assert!(
            error.contains("already active"),
            "{label}: the refusal names the live question: {error}"
        );
    }
}

/// One table row: the label said on failure, and the step it drives.
type Row = (&'static str, Box<dyn Fn(&mut Harness, &str)>);
/// A bounded wait for the background save `tick_resume` queues, so the
/// fixture root is removed only after the writer is done with it.
fn wait_for_background_save(store_root: &Path) {
    for _ in 0..200 {
        if std::fs::read_dir(store_root).is_ok_and(|entries| entries.count() > 0) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
#[path = "question_tests/picker_resume.rs"]
mod picker_resume;
#[path = "question_tests/reacquire.rs"]
mod reacquire;
#[path = "question_tests/saved_state.rs"]
mod saved_state;
#[path = "question_tests/transitions.rs"]
mod transitions;
