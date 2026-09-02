//! Application state transitions: input, menus, streaming, and clipboard.

mod input_actions;
mod picker;
mod search;
mod streaming;

use super::history::History;
use super::input::InputBuffer;
use super::transcript::{BlockKind, Transcript};
use super::types::{App, MAX_INPUT_ROWS, OverlayState, RequestState};
use crate::config::runtime::RuntimeConfig;
use saya_store::SqliteStateStore;
use std::cell::Cell;
use std::sync::Arc;

impl App {
    pub(crate) fn new(
        profiles: Vec<String>,
        runtime: Arc<RuntimeConfig>,
        state_db: SqliteStateStore,
    ) -> Self {
        let mut transcript = Transcript::new();
        transcript.push(
            BlockKind::System,
            "Welcome to saya. Type a message and press Enter. Type / for commands, \
             Tab to accept a suggestion, ↑/↓ for history, Alt+Enter for a newline, Ctrl+C to quit.",
        );
        Self {
            input: InputBuffer::new(),
            transcript,
            profiles,
            pending: None,
            request: RequestState::default(),
            overlays: OverlayState::default(),
            spinner: 0,
            history: History::load(),
            viewport: Cell::new((0, 0)),
            ctrl_c_armed: false,
            at_refs: Vec::new(),
            pending_clipboard: None,
            clipboard_copy: None,
            session_save: None,
            sql_task: None,
            pending_session_save: None,
            last_query: None,
            runtime,
            state_db,
            should_quit: false,
        }
    }

    /// Whether the UI is busy: an agent request is streaming **or** a direct-SQL
    /// command is running off-thread. The status-bar spinner, the queued-prompt
    /// gate, and the submit-time queue check all read this so a running query is
    /// treated like a streaming agent — visible, and a second command is held
    /// rather than dispatched over the first.
    ///
    /// The cancel paths (Esc, Ctrl+C in `keys.rs`) detach a SQL task *before*
    /// they reach the agent-cancel branch, so broadening this predicate to cover
    /// SQL tasks never makes Esc claim a query was cancelled when it was only
    /// detached. See [`App::detach_sql_task`].
    pub(crate) fn is_busy(&self) -> bool {
        self.request.stream.is_some() || self.sql_task.is_some()
    }

    /// Number of visible text rows the input box should show (clamped).
    pub(crate) fn input_rows(&self) -> usize {
        self.input.lines().len().clamp(1, MAX_INPUT_ROWS)
    }
}

/// The dispatch decision when a second direct-SQL command arrives while one is
/// already running. A pure function over [`App`] state so the result-loss
/// behaviour is testable without a live database (see [`App::admit_second_sql`]).
pub(crate) enum SecondSqlDecision {
    /// No SQL command is in flight — start this one.
    Start,
    /// One is already running — refuse rather than silently drop the first
    /// result. The message is shown to the user.
    Reject(&'static str),
}

impl App {
    /// Decides whether a direct-SQL command may start now. The primary defence
    /// against silent result loss is the queued-prompt gate: `is_busy()` now
    /// covers SQL tasks, so a second command submitted while one runs is held
    /// in `pending` and dispatches only after the first finishes (both results
    /// report). This guard is the backstop: should a `SqlTask` ever reach the
    /// dispatch handler while one is already running, it is refused with a
    /// message instead of replacing the first receiver.
    pub(crate) fn admit_second_sql(&self) -> SecondSqlDecision {
        if self.sql_task.is_some() {
            SecondSqlDecision::Reject(
                "A SQL command is already running — wait for it to finish before starting another.",
            )
        } else {
            SecondSqlDecision::Start
        }
    }

    /// Detaches the in-flight SQL command so the UI moves on without blocking.
    /// The worker thread is not joined and the connector has no cancellation
    /// token wired here, so the query keeps running **server-side**; its result
    /// lands on a dropped channel and is discarded. The message says exactly
    /// that — it never claims the query was cancelled.
    pub(crate) fn detach_sql_task(&mut self) {
        if let Some((_, _, started)) = self.sql_task.take() {
            // Release the status fields the bar reused while the query ran.
            self.request.started = None;
            self.request.activity = None;
            self.transcript.push(
                super::transcript::BlockKind::System,
                format!(
                    "Detached the running query ({}s elapsed) — it may still be running on the \
                     server; its result will be discarded.",
                    started.elapsed().as_secs()
                ),
            );
        }
    }
}

#[cfg(test)]
pub(crate) mod tests_support {
    use crate::config::runtime::RuntimeConfig;
    use crate::interactive::tui::history::History;
    use crate::interactive::tui::input::InputBuffer;
    use crate::interactive::tui::sql_task::{Followup, SqlTask};
    use crate::interactive::tui::transcript::Transcript;
    use crate::interactive::tui::types::{App, OverlayState, RequestState};
    use crate::render::TerminalEvent;
    use saya_store::SqliteStateStore;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::Arc;

    /// A minimal idle `App` built as a struct literal so no history file is
    /// read and no config/connection file is touched. Only the fields the
    /// behaviours under test read need to be meaningful.
    pub(crate) fn idle_app() -> App {
        App {
            sql_task: None,
            input: InputBuffer::new(),
            transcript: Transcript::new(),
            profiles: Vec::new(),
            pending: None,
            request: RequestState::default(),
            overlays: OverlayState::default(),
            spinner: 0,
            history: History::with_path_disabled(PathBuf::new()),
            viewport: std::cell::Cell::new((0, 0)),
            ctrl_c_armed: false,
            at_refs: Vec::new(),
            pending_clipboard: None,
            clipboard_copy: None,
            session_save: None,
            pending_session_save: None,
            last_query: None,
            runtime: Arc::new(unused_runtime()),
            state_db: SqliteStateStore::new(PathBuf::new()),
            should_quit: false,
        }
    }

    /// An idle app with a direct-SQL command already in flight, exactly the
    /// state the dispatch loop produces when a query is running: the task is
    /// set and the status fields the bar reuses (`started`, `activity`) are
    /// populated so the spinner + "running query" label render.
    pub(crate) fn running_app_with_sql_task() -> App {
        let mut app = idle_app();
        let started = std::time::Instant::now();
        app.sql_task = Some(in_flight_task_at(started));
        app.request.started = Some(started);
        app.request.activity = Some("query".into());
        app
    }

    /// An idle app with a direct-SQL command already in flight, exactly the
    /// state the dispatch loop produces when a query is running.
    pub(crate) fn idle_app_with_sql_task() -> App {
        let mut app = idle_app();
        app.sql_task = Some(in_flight_task());
        app
    }

    pub(crate) fn unused_runtime() -> RuntimeConfig {
        use saya_config::{
            AiProvider, ColorChoice, ConnectionsFile, MemoryMode, OutputFormat, ResolvedAi,
            ResolvedConfig, ResolvedMemory,
        };
        RuntimeConfig {
            resolved: ResolvedConfig {
                profile_name: None,
                profile: None,
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
                    context_byte_budget: 256 * 1024,
                    show_thinking: false,
                },
                max_rows: 100,
                read_only: true,
                max_iterations: 4,
                query_timeout_seconds: 5,
                output_format: OutputFormat::Text,
                output_color: ColorChoice::Auto,
                memory: ResolvedMemory {
                    mode: MemoryMode::Off,
                    max_contracts: 5,
                    max_claims_per_contract: 12,
                    max_context_bytes: 16384,
                },
                ignored_project_overrides: Vec::new(),
            },
            connections: ConnectionsFile::default(),
            config_path: None,
            connections_path: None,
            cache_scope: PathBuf::new(),
            secret_values: BTreeMap::new(),
        }
    }

    /// A dummy in-flight task: a real mpsc receiver (never sent to) plus a
    /// `SqlTask` and a dispatch instant, exactly the shape `app.sql_task`
    /// holds when a query is running.
    pub(crate) fn in_flight_task() -> (
        std::sync::mpsc::Receiver<TerminalEvent>,
        super::super::sql_task::SqlTask,
        std::time::Instant,
    ) {
        in_flight_task_at(std::time::Instant::now())
    }

    /// `in_flight_task` with an explicit dispatch instant, so a test can place
    /// the same `Instant` in both `app.sql_task` and `app.request.started`.
    pub(crate) fn in_flight_task_at(
        started: std::time::Instant,
    ) -> (
        std::sync::mpsc::Receiver<TerminalEvent>,
        super::super::sql_task::SqlTask,
        std::time::Instant,
    ) {
        let (_tx, rx) = std::sync::mpsc::channel();
        let task = SqlTask {
            profile: Some("analytics".into()),
            sql: "SELECT 1".into(),
            followup: Followup::Sql {
                connection: Some("analytics".into()),
            },
        };
        (rx, task, started)
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::{idle_app, in_flight_task};
    use super::*;

    #[test]
    fn idle_app_admits_a_sql_command() {
        let app = idle_app();
        assert!(matches!(app.admit_second_sql(), SecondSqlDecision::Start));
        assert!(!app.is_busy(), "idle app is not busy");
    }

    /// The result-loss root cause: `is_busy()` ignored SQL tasks, so the
    /// `!is_busy()` queued-prompt gate stayed open while a query ran and a
    /// second command dispatched, replacing `app.sql_task` and dropping the
    /// first receiver. With `is_busy()` covering SQL tasks the gate now holds
    /// the second command — the first is preserved and reports its result.
    #[test]
    fn is_busy_covers_a_running_sql_task_so_the_gate_holds_a_second_command() {
        let mut app = idle_app();
        assert!(!app.is_busy(), "idle is not busy");
        app.sql_task = Some(in_flight_task());
        // A running query now reads as busy, so the `!is_busy()` gate will not
        // dispatch a queued line over it.
        assert!(app.is_busy(), "a running SQL task must count as busy");
        // The first task is still tracked — nothing replaced it.
        assert!(app.sql_task.is_some());
    }

    /// Backstop guard: should a `SqlTask` reach the dispatch handler while one
    /// is already running, it is refused (first preserved, message shown) —
    /// never a silent replacement.
    #[test]
    fn second_sql_command_at_the_handler_is_rejected_not_silently_dropped() {
        let mut app = idle_app();
        app.sql_task = Some(in_flight_task());
        assert!(app.is_busy());

        match app.admit_second_sql() {
            SecondSqlDecision::Reject(msg) => {
                assert!(
                    msg.to_lowercase().contains("running"),
                    "reject message should mention a running command: {msg}"
                );
                assert!(
                    !msg.to_lowercase().contains("cancel"),
                    "reject must not claim cancellation: {msg}"
                );
            }
            SecondSqlDecision::Start => panic!(
                "a second SQL command at the handler must be rejected, not started (silent result loss)"
            ),
        }
        // Refusing does not drop the running task.
        assert!(app.sql_task.is_some());
    }

    #[test]
    fn detach_sql_task_clears_the_task_and_says_it_is_still_running() {
        let mut app = idle_app();
        app.sql_task = Some(in_flight_task());
        app.detach_sql_task();
        // The UI no longer tracks the query (spinner stops, gate reopens).
        assert!(app.sql_task.is_none(), "detach clears the in-flight task");
        assert!(!app.is_busy());
        // The message must be honest: it is still running server-side, NOT cancelled.
        let last = app
            .transcript
            .blocks()
            .last()
            .expect("a message was posted");
        let text = last.text.to_lowercase();
        assert!(
            text.contains("running"),
            "message says still running: {text}"
        );
        assert!(
            !text.contains("cancel"),
            "detach must not claim cancellation: {text}"
        );
        assert!(
            text.contains("discarded"),
            "message says the result is discarded: {text}"
        );
    }

    #[test]
    fn detach_sql_task_is_a_noop_when_nothing_is_running() {
        let mut app = idle_app();
        app.detach_sql_task();
        assert!(app.sql_task.is_none());
        // No spurious message.
        assert!(app.transcript.blocks().is_empty());
    }

    /// A running direct-SQL command is visible. The status bar
    /// must render a spinner and a "running query" label while a query is in
    /// flight, so the user can tell "working" from "hung". Renders through the
    /// real `ui::draw` (the same path the snapshot tests use) onto a
    /// `TestBackend` and inspects the buffer.
    #[test]
    fn running_sql_task_is_visible_in_the_status_bar() {
        use super::tests_support::running_app_with_sql_task;
        use crate::interactive::session_prompt::StatusView;

        // The braille spinner frames `ui::panels::SPINNER` cycles through. That
        // constant is private to `ui`, so mirror the frames here to assert one
        // is on screen without reaching across the privacy boundary.
        const SPINNER_CHARS: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

        let app = running_app_with_sql_task();
        let status = StatusView {
            profile: "analytics".into(),
            included: Vec::new(),
            provider: "ollama".into(),
            model: "qwen".into(),
            approval_mode: "read-only".into(),
            privacy_on: true,
        };
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).expect("test backend builds");
        terminal
            .draw(|frame| crate::interactive::tui::ui::draw(frame, &app, &status))
            .expect("draw completes");
        let buffer = format!("{}", terminal.backend());

        // The status bar shows the running-query label (not "thinking") so a
        // SQL task is distinguishable from an agent stream, and a spinner
        // frame is present so the state animates rather than freezing.
        assert!(
            buffer.contains("running query"),
            "status bar should show 'running query' while a SQL task runs:\n{buffer}"
        );
        assert!(
            !buffer.contains("thinking"),
            "a SQL task must not be labelled 'thinking':\n{buffer}"
        );
        assert!(
            SPINNER_CHARS.iter().any(|c| buffer.contains(c)),
            "a spinner frame should be present:\n{buffer}"
        );
    }
}
