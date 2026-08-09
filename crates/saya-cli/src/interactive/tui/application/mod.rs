//! Application state transitions: input, menus, streaming, and clipboard.

mod input_actions;
mod picker;
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
            pending_session_save: None,
            runtime,
            state_db,
            should_quit: false,
        }
    }

    /// Whether an agent request is currently streaming.
    pub(crate) fn is_busy(&self) -> bool {
        self.request.stream.is_some()
    }

    /// Number of visible text rows the input box should show (clamped).
    pub(crate) fn input_rows(&self) -> usize {
        self.input.lines().len().clamp(1, MAX_INPUT_ROWS)
    }
}
