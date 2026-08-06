//! Shared TUI state types used by the event loop, application logic, and renderer.

use super::agent::Stream;
use super::complete::Candidate;
use super::history::History;
use super::input::InputBuffer;
use super::transcript::Transcript;
use crate::config::runtime::RuntimeConfig;
use saya_store::SqliteStateStore;
use std::cell::Cell;
use std::sync::Arc;
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

/// A selectable list of saved sessions to resume.
pub(crate) struct Picker {
    pub(crate) entries: Vec<PickerEntry>,
    pub(crate) selected: usize,
}

/// One row in the session picker.
pub(crate) struct PickerEntry {
    pub(crate) id: String,
    pub(crate) label: String,
}

/// Interactive application state.
pub(crate) struct App {
    pub(crate) input: InputBuffer,
    pub(crate) transcript: Transcript,
    pub(crate) profiles: Vec<String>,
    pub(crate) menu: Option<Menu>,
    pub(crate) pending: Option<String>,
    pub(crate) stream: Option<Stream>,
    /// When the active request started streaming, for the elapsed timer.
    pub(crate) stream_started: Option<std::time::Instant>,
    /// The tool the agent is currently running, shown in the status bar.
    pub(crate) activity: Option<String>,
    pub(crate) spinner: usize,
    pub(crate) history: History,
    /// Transcript viewport (width, height) captured during the last render,
    /// so key-driven scrolling can clamp to the real size.
    pub(crate) viewport: Cell<(u16, u16)>,
    /// Set after a first Ctrl+C on an empty line; a second one exits.
    pub(crate) ctrl_c_armed: bool,
    /// `@table` / `@table.column` references from the active profiles' cached schema.
    pub(crate) at_refs: Vec<String>,
    /// A tool-approval request from the agent awaiting the user's answer.
    pub(crate) pending_approval: Option<PendingApproval>,
    /// Open session picker, if any.
    pub(crate) picker: Option<Picker>,
    /// A session id the user chose to resume, handled by the run loop.
    pub(crate) pending_resume: Option<String>,
    /// Whether the help overlay is shown.
    pub(crate) show_help: bool,
    /// Selection mode: when on, mouse capture is released so the terminal's own
    /// drag-select + copy works (at the cost of wheel scrolling). Toggled with Ctrl+O.
    pub(crate) selection_mode: bool,
    /// Text queued for the system clipboard, fulfilled by the run loop via the OS
    /// clipboard tool (pbcopy/wl-copy/xclip/clip) plus an OSC 52 escape for SSH.
    pub(crate) pending_clipboard: Option<String>,
    pub(crate) runtime: Arc<RuntimeConfig>,
    pub(crate) state_db: SqliteStateStore,
    pub(crate) should_quit: bool,
}
