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
use crate::interactive::session_runtime::SessionRuntime;
use crate::interactive::session_state::SessionState;
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

/// One table, one walk: every mutating slash command abandons the pending
/// clarification (the next begin mints a different question and the old
/// token is cancelled); every no-op or rejected command preserves it (the
/// next begin re-begins the SAME question, token live). The rows share one
/// real session and evolve its state in order; each row normalizes the
/// lifecycle to PendingClarification first.
#[test]
fn a_context_change_abandons_the_pending_question_and_a_noop_preserves_it() {
    let table: Vec<Row> = vec![
        // ---- ABANDON: each real mutation of the answering context ----
        (
            "/connect demo selects the profile",
            Box::new(|h, label| {
                let pending = h.pend(label);
                let action = h.run(SlashCommand::Connect("demo".into()));
                assert_message(&action, label);
                assert_abandoned(h, &pending, label);
            }),
        ),
        (
            "/include demo adds the profile to the answering set",
            Box::new(|h, label| {
                let pending = h.pend(label);
                let action = h.run(SlashCommand::Include("demo".into()));
                assert_message(&action, label);
                assert_abandoned(h, &pending, label);
            }),
        ),
        (
            "/exclude demo removes the profile from the answering set",
            Box::new(|h, label| {
                let pending = h.pend(label);
                let action = h.run(SlashCommand::Exclude("demo".into()));
                assert_message(&action, label);
                assert_abandoned(h, &pending, label);
            }),
        ),
        (
            "/provider openai switches the provider",
            Box::new(|h, label| {
                let pending = h.pend(label);
                let action = h.run(SlashCommand::Provider(Some("openai".into())));
                assert_message(&action, label);
                assert_abandoned(h, &pending, label);
            }),
        ),
        (
            "/model m2 switches the model",
            Box::new(|h, label| {
                let pending = h.pend(label);
                let action = h.run(SlashCommand::Model(Some("m2".into())));
                assert_message(&action, label);
                assert_abandoned(h, &pending, label);
            }),
        ),
        (
            "/mode plan switches the task posture",
            Box::new(|h, label| {
                let pending = h.pend(label);
                let action = h.run(SlashCommand::Mode(Some(AgentMode::Plan)));
                assert_message(&action, label);
                assert_abandoned(h, &pending, label);
            }),
        ),
        (
            "/privacy on flips data sharing",
            Box::new(|h, label| {
                let pending = h.pend(label);
                let action = h.run(SlashCommand::Privacy(Some(true)));
                assert_message(&action, label);
                assert_abandoned(h, &pending, label);
            }),
        ),
        (
            "/approval read-only is a non-escalating mode change",
            Box::new(|h, label| {
                let pending = h.pend(label);
                let action = h.run(SlashCommand::Approvals(Some(ApprovalPolicy::ReadOnly)));
                assert_message(&action, label);
                assert_abandoned(h, &pending, label);
            }),
        ),
        (
            "/tasks clear empties the task list",
            Box::new(|h, label| {
                let pending = h.pend(label);
                h.state.task_list = SessionTaskList::new(vec![
                    SessionTask::with_note(
                        "context-key task",
                        TaskStatus::InProgress,
                        None::<&str>,
                    )
                    .expect("a valid task"),
                ])
                .expect("a valid task list");
                let action = h.run(SlashCommand::Tasks(Some("clear".into())));
                assert_message(&action, label);
                assert_abandoned(h, &pending, label);
            }),
        ),
        (
            "/clear wipes the conversation history",
            Box::new(|h, label| {
                let pending = h.pend(label);
                let action = h.run(SlashCommand::Clear);
                assert_message(&action, label);
                assert_abandoned(h, &pending, label);
            }),
        ),
        // ---- PRESERVE: a command that changes nothing ----
        (
            "/help is a usage query",
            Box::new(|h, label| {
                let pending = h.pend(label);
                let action = h.run(SlashCommand::Help(None));
                assert_message(&action, label);
                assert_preserved(h, &pending, label);
            }),
        ),
        (
            "/model with no argument only reports",
            Box::new(|h, label| {
                let pending = h.pend(label);
                let action = h.run(SlashCommand::Model(None));
                assert_message(&action, label);
                assert_preserved(h, &pending, label);
            }),
        ),
        (
            "/provider with no argument only reports",
            Box::new(|h, label| {
                let pending = h.pend(label);
                let action = h.run(SlashCommand::Provider(None));
                assert_message(&action, label);
                assert_preserved(h, &pending, label);
            }),
        ),
        (
            "/mode with no argument only reports",
            Box::new(|h, label| {
                let pending = h.pend(label);
                let action = h.run(SlashCommand::Mode(None));
                assert_message(&action, label);
                assert_preserved(h, &pending, label);
            }),
        ),
        (
            "/usage is a no-op diagnostic",
            Box::new(|h, label| {
                let pending = h.pend(label);
                let action = h.run(SlashCommand::Usage);
                assert_message(&action, label);
                assert_preserved(h, &pending, label);
            }),
        ),
        (
            "setting the model to its current value changes nothing",
            Box::new(|h, label| {
                let pending = h.pend(label);
                let action = h.run(SlashCommand::Model(Some("m2".into())));
                assert_message(&action, label);
                assert_preserved(h, &pending, label);
            }),
        ),
        (
            "a rejected change (unknown provider) errors without mutating",
            Box::new(|h, label| {
                let pending = h.pend(label);
                let action = h.run(SlashCommand::Provider(Some("nosuch-provider".into())));
                let SessionAction::Error(message) = action else {
                    panic!("{label}: an unsupported provider is refused, got {action:?}");
                };
                assert!(
                    message.contains("Unsupported provider"),
                    "{label}: the refusal names the problem: {message}"
                );
                assert_preserved(h, &pending, label);
            }),
        ),
    ];
    let mut harness = Harness::new("table");
    for (label, step) in &table {
        step(&mut harness, label);
    }
    let root = harness.sessions_root.clone();
    drop(harness);
    let _ = std::fs::remove_dir_all(&root);
}

/// Removes the throwaway root when the fixture drops (the chain-test
/// precedent): declared after the session runtime so the lock releases
/// before the directory goes.
struct RootGuard(PathBuf);

impl Drop for RootGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// One fixture for the real TUI dispatch: the exact pieces the live loop
/// passes (`loop_tick::pending`'s shape), backed by a throwaway root.
struct TuiHarness {
    app: App,
    state: SessionState,
    session: SessionRuntime,
    store: FsSessionStore,
    runtime: RuntimeConfig,
    /// Held, never read: the guard's purpose is its lifetime — dropping the
    /// fixture removes the throwaway root (the `_lock` precedent).
    _root: RootGuard,
}

impl TuiHarness {
    fn build(tag: &str) -> Self {
        let root = temp_sessions_root(tag);
        let mut runtime = unused_runtime();
        // The composed investigations root: anything the dispatch runs
        // writes under this fixture's own root, never the machine's data
        // directory.
        runtime.investigations_root = root.join("investigations");
        let session = SessionRuntime::acquire(
            &runtime,
            Some(&root),
            true,
            None,
            "context-key-tui",
            ApprovalPolicy::Ask,
            &root.join("sessions"),
        )
        .expect("the TUI test session acquires");
        Self {
            app: idle_app(),
            state: SessionState::new("context-key-tui", None, "m1"),
            session,
            store: FsSessionStore::new(root.join("session-store")),
            runtime,
            _root: RootGuard(root),
        }
    }

    /// Normalizes the lifecycle to PendingClarification, the same way the
    /// table's harness does.
    fn pend(&mut self, label: &str) -> QuestionScope {
        self.session.abandon_question();
        let scope = self
            .session
            .begin_question(&self.runtime)
            .unwrap_or_else(|error| panic!("{label}: the question begins: {error}"));
        self.session.settle_question(&scope, true);
        scope
    }

    /// The real `dispatch`, called with the fixture's pieces exactly as the
    /// live loop does.
    fn dispatch_line(&mut self, line: &str) {
        dispatch(
            line,
            &mut self.app.transcript,
            &self.app.profiles,
            &mut self.state,
            &self.runtime,
            &self.store,
            &self.app.state_db,
            RenderFormat::Text,
            &mut self.app.last_query,
            &self.app.captured,
            self.app.agent_captures.gap,
            &mut self.session,
        );
    }

    fn last_system_block(&self) -> Option<String> {
        self.app
            .transcript
            .blocks()
            .iter()
            .rev()
            .find(|block| block.kind == BlockKind::System)
            .map(|block| block.text.clone())
    }
}

/// `/mode plan` through the REAL TUI dispatch: the answer lands in the
/// transcript unchanged and the pending question is abandoned — the old
/// token is cancelled and the next begin mints a different question.
#[test]
fn a_context_change_through_the_tui_dispatch_abandons_the_pending_question() {
    let mut fx = TuiHarness::build("tui-abandon");
    let pending = fx.pend("tui abandon");
    fx.dispatch_line("/mode plan");
    let message = fx.last_system_block().expect("the mode answer is said");
    assert!(
        message.contains("Mode: plan"),
        "the dispatch output is unchanged: {message}"
    );
    assert!(
        pending.cancellation().is_cancelled(),
        "the context change through the TUI dispatch cancels the pending token"
    );
    let next = fx
        .session
        .begin_question(&fx.runtime)
        .expect("the next question begins");
    assert!(
        !next.is_same_question(&pending),
        "the next begin mints a different question after the TUI context change"
    );
}

/// `/help` through the REAL TUI dispatch changes nothing: the pending
/// clarification survives with its token live.
#[test]
fn a_noop_through_the_tui_dispatch_preserves_the_pending_question() {
    let mut fx = TuiHarness::build("tui-preserve");
    let pending = fx.pend("tui preserve");
    fx.dispatch_line("/help");
    let message = fx.last_system_block().expect("the help answer is said");
    assert!(
        message.contains("Slash commands:"),
        "help answers through the dispatch: {message}"
    );
    let next = fx
        .session
        .begin_question(&fx.runtime)
        .expect("the next question begins");
    assert!(
        next.is_same_question(&pending),
        "a no-op through the TUI dispatch re-begins the same pending question"
    );
    assert!(
        !next.cancellation().is_cancelled(),
        "the preserved question's token stays live"
    );
}
