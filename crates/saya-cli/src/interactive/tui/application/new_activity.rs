//! Phase 8 packet 2: the App-facing "new activity" view counter.
//!
//! The count itself lives on `Transcript` beside `scroll_up` (the append hook
//! that raises it is there); this module is the app's seam onto it — an
//! accessor for the chrome and the explicit return to the live edge. No part
//! of it is persisted, replayed, or read by the model's context.

use super::super::types::App;

impl App {
    /// Rows that landed below a scrolled-up reader since they last returned to
    /// the live edge. Zero while following the tail: a count there would name
    /// rows already on screen, which is a lie.
    pub(crate) fn unseen_new_rows(&self) -> usize {
        self.transcript.unseen_new_rows()
    }

    /// Returns the viewport to the live edge and clears the unseen count.
    /// Explicit only — nothing calls this on the user's behalf, so new
    /// activity never steals their viewport. Bound to `Shift+End` in the key
    /// dispatch: that arm sits after the approval and Esc blocks and after the
    /// shared Ctrl+C disarm, so it can neither answer a modal nor leave the
    /// "press again to exit" prompt armed across a return.
    pub(crate) fn return_to_live_edge(&mut self) {
        self.transcript.scroll_to_bottom();
    }
}

#[cfg(test)]
#[path = "new_activity_tests.rs"]
mod new_activity_tests;
