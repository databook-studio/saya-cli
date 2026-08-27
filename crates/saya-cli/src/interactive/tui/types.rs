//! Shared TUI state types used by the event loop, application logic, and renderer.

use super::agent::Stream;
use super::complete::Candidate;
use super::history::History;
use super::input::InputBuffer;
use super::transcript::Transcript;
use crate::config::runtime::RuntimeConfig;
use saya_store::{RedactedSession, SqliteStateStore};
use std::cell::Cell;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use tokio::sync::oneshot;

/// Largest number of text rows the input box grows to before it stops expanding.
pub(crate) const MAX_INPUT_ROWS: usize = 6;

/// Live slash-command popup state.
pub(crate) struct Menu {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) candidates: Vec<Candidate>,
    pub(crate) selected: usize,
}

/// A pending tool-approval request awaiting the user's y/n answer.
pub(crate) struct PendingApproval {
    pub(crate) tool: String,
    /// Human-readable detail (e.g. the SQL) shown in the approval dialog.
    pub(crate) detail: Option<String>,
    pub(crate) respond: oneshot::Sender<bool>,
}

/// A selectable list of saved sessions to resume, filterable as you type.
pub(crate) struct Picker {
    pub(crate) entries: Vec<PickerEntry>,
    pub(crate) selected: usize,
    /// Case-insensitive substring filter over id + label.
    pub(crate) query: String,
}

/// One row in the session picker.
#[derive(Clone)]
pub(crate) struct PickerEntry {
    pub(crate) id: String,
    pub(crate) label: String,
}

/// State tied to an active agent request.
#[derive(Default)]
pub(crate) struct RequestState {
    pub(crate) stream: Option<Stream>,
    pub(crate) started: Option<std::time::Instant>,
    pub(crate) activity: Option<String>,
    pub(crate) pending_approval: Option<PendingApproval>,
}

/// UI overlays and modal interaction state.
/// A Ctrl+R (input history) or Ctrl+F (transcript) search overlay.
pub(crate) struct SearchOverlay {
    pub(crate) kind: SearchKind,
    pub(crate) query: String,
    /// Selected index into the filtered candidate list (history mode).
    pub(crate) selected: usize,
    /// The wrapped-line index the last Enter jumped to (transcript mode), so
    /// the next Enter walks to the *following* match instead of re-landing on
    /// the same one. `None` until the first jump, and reset whenever the query
    /// changes (so an edit restarts the search from the viewport top).
    pub(crate) last_match: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SearchKind {
    History,
    Transcript,
}

#[derive(Default)]
pub(crate) struct OverlayState {
    pub(crate) menu: Option<Menu>,
    pub(crate) picker_loading: Option<Receiver<Result<Vec<PickerEntry>, String>>>,
    pub(crate) picker: Option<Picker>,
    pub(crate) pending_resume: Option<String>,
    pub(crate) show_help: bool,
    pub(crate) selection_mode: bool,
    pub(crate) search: Option<SearchOverlay>,
}

/// A native clipboard helper running in the background alongside an OSC 52 write.
pub(crate) struct ClipboardCopy {
    pub(crate) native_result: Receiver<bool>,
    pub(crate) osc_error: Option<String>,
}

/// A redacted session save running outside the UI event loop.
pub(crate) struct SessionSave {
    pub(crate) result: Receiver<Result<(), String>>,
}

/// The most recent query the app ran (via /sql or an agent tool), so /export
/// (and later /chart) can re-run it. Re-running a SELECT stays read-only.
#[derive(Clone)]
pub(crate) struct LastQuery {
    pub(crate) sql: String,
    pub(crate) connection: Option<String>,
}

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
        super::sql_task::SqlTask,
        std::time::Instant,
    )>,
    pub(crate) pending_session_save: Option<RedactedSession>,
    pub(crate) last_query: Option<LastQuery>,
    pub(crate) runtime: Arc<RuntimeConfig>,
    pub(crate) state_db: SqliteStateStore,
    pub(crate) should_quit: bool,
}
