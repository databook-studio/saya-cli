//! Application state: construction, busy state, and input-box sizing.

use super::super::history::History;
use super::super::input::InputBuffer;
use super::super::transcript::{BlockKind, Transcript};
use super::super::types::{App, MAX_INPUT_ROWS, OverlayState, RequestState};
use crate::config::runtime::RuntimeConfig;
use saya_store::SqliteStateStore;
use std::cell::Cell;
use std::sync::Arc;

impl App {
    pub(crate) fn new(
        profiles: Vec<String>,
        runtime: Arc<RuntimeConfig>,
        state_db: SqliteStateStore,
        session: Arc<crate::interactive::session_universe::SessionUniverse>,
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
            compact_task: None,
            pending_session_save: None,
            last_query: None,
            wide_table: Default::default(),
            run_panel: None,
            runtime,
            state_db,
            session,
            should_quit: false,
            pending_trust_answer: None,
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
        self.request.stream.is_some() || self.sql_task.is_some() || self.compact_task.is_some()
    }

    /// Number of visible text rows the input box should show when wrapped to
    /// `width`, clamped to [`MAX_INPUT_ROWS`]. Counts **visual** rows — a long
    /// single line wraps and grows the box — so the box tracks what the user
    /// actually sees rather than the logical line count.
    pub(crate) fn input_rows(&self, width: usize) -> usize {
        let inner = width.saturating_sub(2).max(1);
        self.input.visual_row_count(inner).clamp(1, MAX_INPUT_ROWS)
    }
}
