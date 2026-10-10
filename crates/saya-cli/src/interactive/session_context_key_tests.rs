//! One context key for the answering session: a slash command that changes
//! the answering context abandons the question scope; a command that changes
//! nothing leaves the pending clarification in place. One table drives real
//! `SessionState::apply` values against a real `SessionRuntime` (the D1
//! question-scope fixture shape); the surfaces are checked through their real
//! dispatch beside the table.

use std::path::PathBuf;

use saya_config::{
    AiProvider, ColorChoice, MemoryMode, OutputFormat, ResolvedAi, ResolvedConfig,
    ResolvedFetchJobs, ResolvedHostCommands, ResolvedInterpreterJobs, ResolvedJobs, ResolvedMemory,
    ResolvedRunnerJobs, ThemeChoice,
};
use saya_types::{SessionTask, SessionTaskList, TaskStatus};

use saya_agent::{AgentMode, ApprovalPolicy};

use crate::agent::question_scope::QuestionScope;
use crate::config::runtime::RuntimeConfig;
use crate::interactive::SessionAction;
use crate::interactive::session::runtime::SessionRuntime;
use crate::interactive::session::state::SessionState;
use crate::interactive::tui::application::tests_support::{idle_app, unused_runtime};
use crate::interactive::tui::transcript::BlockKind;
use crate::interactive::tui::types::App;
use crate::render::RenderFormat;
use crate::slash::SlashCommand;
use saya_store::FsSessionStore;

use super::dispatch;

fn temp_sessions_root(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "saya-context-key-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("context-key test root");
    root
}

fn test_runtime() -> RuntimeConfig {
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
        cache_scope: PathBuf::from("/tmp/saya-context-key"),
        investigations_root: crate::config::runtime::temp_investigations_root(),
        secret_values: Default::default(),
    }
}

/// One real `SessionRuntime` (the D1 fixture shape) beside one real
/// `SessionState`: the pair the slash-command dispatch actually holds.
struct Harness {
    runtime: RuntimeConfig,
    session: SessionRuntime,
    state: SessionState,
    sessions_root: PathBuf,
}

impl Harness {
    fn new(tag: &str) -> Self {
        let sessions_root = temp_sessions_root(tag);
        let runtime = test_runtime();
        let session = SessionRuntime::acquire(
            &runtime,
            None,
            false,
            None,
            "context-key-walk",
            ApprovalPolicy::Ask,
            &sessions_root,
        )
        .expect("a real session runtime acquires");
        let mut state = SessionState::new("context-key", None, "m1");
        // One seeded line of history: a pending clarification rides a session
        // with turns, and `/clear` must change the key's history marker when
        // it wipes the conversation.
        state.record("user", "seeded turn for the history marker");
        Self {
            runtime,
            session,
            state,
            sessions_root,
        }
    }

    /// Normalizes the lifecycle to PendingClarification: whatever is live is
    /// abandoned, a fresh question begins, and it settles as a clarification
    /// with its token live.
    fn pend(&mut self, label: &str) -> QuestionScope {
        self.session.abandon_question();
        let scope = self
            .session
            .begin_question(&self.runtime)
            .unwrap_or_else(|error| panic!("{label}: the question begins: {error}"));
        self.session.settle_question(&scope, true);
        scope
    }

    fn begin(&mut self, label: &str) -> QuestionScope {
        self.session
            .begin_question(&self.runtime)
            .unwrap_or_else(|error| panic!("{label}: the question begins: {error}"))
    }

    /// Runs one slash command exactly as both surfaces do: the key before,
    /// the real `apply`, then the shared abandon-when-changed step.
    fn run(&mut self, command: SlashCommand) -> SessionAction {
        let available = ["demo".to_string()];
        let before = self.state.context_key();
        let action = self.state.apply(command, &available);
        self.state
            .abandon_question_when_context_changed(&before, &mut self.session);
        action
    }
}

/// The abandon assertion: the pending question's token is cancelled and the
/// next begin mints a different question.
fn assert_abandoned(h: &mut Harness, pending: &QuestionScope, label: &str) {
    assert!(
        pending.cancellation().is_cancelled(),
        "{label}: the context change must cancel the pending question's token"
    );
    let next = h.begin(label);
    assert!(
        !next.is_same_question(pending),
        "{label}: the next begin must mint a different question after the context change"
    );
}

/// The preserve assertion: the next begin re-begins the SAME pending
/// question, its token still live.
fn assert_preserved(h: &mut Harness, pending: &QuestionScope, label: &str) {
    let next = h.begin(label);
    assert!(
        next.is_same_question(pending),
        "{label}: a command that changes nothing must re-begin the same pending question"
    );
    assert!(
        !next.cancellation().is_cancelled(),
        "{label}: the preserved question's token stays live"
    );
}

fn assert_message(action: &SessionAction, label: &str) {
    assert!(
        matches!(action, SessionAction::Message(_)),
        "{label}: the command answers with a message, got {action:?}"
    );
}

/// One table row: the label said on failure, and the step it drives.
type Row = (&'static str, Box<dyn Fn(&mut Harness, &str)>);

include!("session_context_key_tests/context_actions.rs");

include!("session_context_key_tests/tui_support.rs");
include!("session_context_key_tests/tui_dispatch.rs");

include!("session_context_key_tests/reload.rs");
