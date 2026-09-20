//! Test fixture builders: a minimal idle `App` plus in-flight SQL task stubs.

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
        compact_task: None,
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
        wide_table: Default::default(),
        run_panel: None,
        runtime: Arc::new(unused_runtime()),
        state_db: SqliteStateStore::new(PathBuf::new()),
        session: std::sync::Arc::new(
            crate::interactive::session_universe::SessionUniverse::empty(),
        ),
        should_quit: false,
        pending_trust_answer: None,
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
        ResolvedConfig, ResolvedFetchJobs, ResolvedHostCommands, ResolvedInterpreterJobs,
        ResolvedJobs, ResolvedMemory, ResolvedRunnerJobs, ThemeChoice,
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
                tokens_per_endpoint: BTreeMap::new(),
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
            endpoints: BTreeMap::new(),
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
