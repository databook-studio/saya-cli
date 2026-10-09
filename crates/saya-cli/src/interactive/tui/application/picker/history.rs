//! Replaying a resumed session's history and scrolling the transcript.

use super::super::super::replay::history_blocks;
use super::super::super::types::App;
use crate::interactive::session::state::SessionState;

impl App {
    /// Replays a resumed session's saved turns into the transcript so the user
    /// sees the prior conversation instead of an empty panel. Does nothing for a
    /// session with no completed turns, leaving the welcome message in place.
    pub(crate) fn show_history(&mut self, state: &SessionState) {
        if state.turns.is_empty() {
            return;
        }
        self.transcript.clear();
        for (kind, text) in history_blocks(state) {
            self.transcript.push(kind, text);
        }
        self.transcript.scroll_to_bottom();
    }

    /// Scrolls the transcript by `delta` pages (negative = up), using the last
    /// rendered viewport height.
    pub(crate) fn scroll_pages(&mut self, up: bool) {
        let (_, height) = self.viewport.get();
        self.scroll_lines(up, (height as usize).saturating_sub(1).max(1));
    }

    /// Scrolls the transcript by `n` lines (for the mouse wheel).
    pub(crate) fn scroll_lines(&mut self, up: bool, n: usize) {
        let (width, height) = self.viewport.get();
        if up {
            self.transcript
                .scroll_up(n, width as usize, (height as usize).max(1));
        } else {
            self.transcript.scroll_down(n);
        }
    }
}
