use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use saya_agent::{LocalStateEffect, ToolEffect};
use saya_config::{
    AiProvider, ColorChoice, ConnectionsFile, MemoryMode, OutputFormat, ResolvedAi, ResolvedConfig,
    ResolvedFetchJobs, ResolvedHostCommands, ResolvedInterpreterJobs, ResolvedJobs, ResolvedMemory,
    ResolvedRunnerJobs, ThemeChoice,
};
use saya_store::SqliteStateStore;

use super::super::history::History;
use super::super::input::InputBuffer;
use super::super::transcript::Transcript;
use super::super::types::{App, OverlayState, RequestState};
use crate::interactive::session_prompt::StatusView;
/// `bounded_sql_query`'s declared effect, carried on the fabricated request
/// events so they match what the loop emits.
pub(crate) fn bounded_sql_query_effect() -> ToolEffect {
    ToolEffect {
        database_data: true,
        external_side_effect: false,
        requires_approval: true,
        local_state: LocalStateEffect::None,
    }
}

/// A minimal `RuntimeConfig` that satisfies the `App` fields `ui::draw` never
/// reads. Built as a struct literal so no config file, env file, or connection
/// file is touched — the only requirement is that the type constructs.
pub(crate) fn unused_runtime() -> Arc<crate::config::runtime::RuntimeConfig> {
    Arc::new(crate::config::runtime::RuntimeConfig {
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
    })
}

/// A lazy `SqliteStateStore` whose pool is never initialized — `ui::draw` never
/// calls `pool()`, so no file is created or read. The path is empty and never
/// touched.
pub(crate) fn unused_store() -> SqliteStateStore {
    SqliteStateStore::new(PathBuf::new())
}

/// An idle `App` with an empty transcript and a fixed profile list. Built
/// directly so no history file is read (`App::new` calls `History::load`).
pub(crate) fn empty_app() -> App {
    App {
        sql_task: None,
        compact_task: None,
        input: InputBuffer::new(),
        transcript: Transcript::new(),
        profiles: vec!["analytics".into(), "billing".into()],
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
        runtime: unused_runtime(),
        state_db: unused_store(),
        session: std::sync::Arc::new(
            crate::interactive::session_universe::SessionUniverse::empty(),
        ),
        should_quit: false,
        pending_trust_answer: None,
    }
}

/// An `empty_app` with the input buffer pre-set, for cursor-mapping tests that
/// need a non-empty input but no transcript turns.
pub(crate) fn empty_app_with_text(text: &str) -> App {
    let mut app = empty_app();
    app.input.set_text(text);
    app
}

/// A stable status bar: profile `analytics`, `ollama/qwen`, `read-only`
/// approval, `build` mode, sharing on. The spinner/elapsed fields are not read when the app
/// is idle, so this is the whole status strip.
pub(crate) fn fixed_status() -> StatusView {
    StatusView {
        profile: "analytics".into(),
        included: Vec::new(),
        model: "qwen".into(),
        approval_mode: "read-only".into(),
        agent_mode: "build".into(),
        workspace_root: Some("/home/user/proj".into()),
        sharing_on: true,
        host_composed: false,
        denied_programs: Vec::new(),
        task_summary: None,
    }
}

/// Draws `app` at `w×h` through the real `ui::draw` and returns the backend's
/// buffer view (one quoted line per screen row, trailing whitespace preserved).
pub(crate) fn render_buffer(app: &App, status: &StatusView, w: u16, h: u16) -> String {
    let backend = ratatui::backend::TestBackend::new(w, h);
    let mut terminal = ratatui::Terminal::new(backend).expect("test backend builds");
    terminal
        .draw(|frame| super::super::ui::draw(frame, app, status))
        .expect("draw completes");
    format!("{}", terminal.backend())
}

/// Draws `app` at `w×h` through the real `ui::draw` and returns the terminal
/// cursor position the render path set with `set_cursor_position`, in
/// `(x, y)` screen cells. This is the seam that pins the input-box cursor
/// mapping end-to-end: a long input must place the cursor at the visual cell
/// of the true insertion point, not pinned against the right border.
pub(crate) fn render_cursor(app: &App, status: &StatusView, w: u16, h: u16) -> (u16, u16) {
    let backend = ratatui::backend::TestBackend::new(w, h);
    let mut terminal = ratatui::Terminal::new(backend).expect("test backend builds");
    terminal
        .draw(|frame| super::super::ui::draw(frame, app, status))
        .expect("draw completes");
    let pos = terminal.backend().cursor_position();
    (pos.x, pos.y)
}

/// A running agent request for tests: a real `Stream` whose receiver never
/// delivers, so `is_busy()` reads true and the bar/draft paths render their
/// streaming shape without a provider.
pub(crate) fn busy_stream() -> super::super::agent::Stream {
    let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
    super::super::agent::Stream {
        rx,
        cancel: saya_agent::CancellationToken::new(),
        prompt: String::new(),
    }
}
