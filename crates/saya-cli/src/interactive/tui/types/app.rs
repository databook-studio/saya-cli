//! The interactive application state.

use super::super::history::History;
use super::super::input::InputBuffer;
use super::super::transcript::Transcript;
use super::overlays::OverlayState;
use super::request::RequestState;
use super::tasks::{ClipboardCopy, CompactOutcome, LastQuery, SessionSave, WideTableView};
use crate::config::runtime::RuntimeConfig;
use saya_store::{RedactedSession, SqliteStateStore};
use std::cell::Cell;
use std::sync::Arc;

/// Interactive application state.
pub(crate) struct App {
    pub(crate) input: InputBuffer,
    pub(crate) transcript: Transcript,
    pub(crate) profiles: Vec<String>,
    pub(crate) pending: Option<String>,
    pub(crate) request: RequestState,
    pub(crate) overlays: OverlayState,
    pub(crate) spinner: usize,
    pub(crate) history: History,
    /// Transcript viewport (width, height) captured during the last render,
    /// so key-driven scrolling can clamp to the real size.
    pub(crate) viewport: Cell<(u16, u16)>,
    /// Set after a first Ctrl+C on an empty line; a second one exits.
    pub(crate) ctrl_c_armed: bool,
    /// `@table` / `@table.column` references from the active profiles' cached schema.
    pub(crate) at_refs: Vec<String>,
    /// Text queued for the system clipboard, fulfilled by the run loop via the OS
    /// clipboard tool (pbcopy/wl-copy/xclip/clip) plus an OSC 52 escape for SSH.
    pub(crate) pending_clipboard: Option<String>,
    pub(crate) clipboard_copy: Option<ClipboardCopy>,
    pub(crate) session_save: Option<SessionSave>,
    /// In-flight direct-SQL command (/sql, /export, /chart, /explain) running
    /// off-thread; polled each loop tick so the UI never blocks on a query.
    /// The `Instant` is when the task was dispatched, so the status bar can
    /// show elapsed time alongside the spinner while the query runs.
    pub(crate) sql_task: Option<(
        std::sync::mpsc::Receiver<crate::render::TerminalEvent>,
        super::super::sql_task::SqlTask,
        std::time::Instant,
    )>,
    /// In-flight `/compact` running off-thread; polled each loop tick so the
    /// UI never blocks on the summariser. `None` until `/compact` runs.
    pub(crate) compact_task: Option<std::sync::mpsc::Receiver<CompactOutcome>>,
    pub(crate) pending_session_save: Option<RedactedSession>,
    pub(crate) last_query: Option<LastQuery>,
    /// Horizontal-scroll / column-selection state for wide result tables.
    /// Lives on the view, never on the transcript data.
    pub(crate) wide_table: WideTableView,
    /// The run panel: a run driven from the session, as a worker task
    /// (`run_panel.rs`). `None` until a run starts; its episode transcript
    /// and step list live here, never in the session conversation.
    pub(crate) run_panel: Option<super::super::run_panel::RunPanel>,
    pub(crate) runtime: Arc<RuntimeConfig>,
    pub(crate) state_db: SqliteStateStore,
    /// The session's composed tool universe — the executor and definitions
    /// every turn of this session dispatches through.
    pub(crate) session: Arc<crate::interactive::session_universe::SessionUniverse>,
    pub(crate) should_quit: bool,
    /// The startup trust modal's answer, stashed when the modal closes with
    /// a bound directory and drained exactly once by the event loop (see
    /// `take_trust_answer`), which recomposes the live runtime behind the
    /// snapshot above.
    pub(crate) pending_trust_answer: Option<std::path::PathBuf>,
}
