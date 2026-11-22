//! The turn in flight, and the one line held behind it.
//!
//! `02` §7's queue is one deep, so these two facts are one concern and live
//! together: a message sent opens a turn and marks it running, a message sent
//! while a turn runs is held instead, and the held line is taken — or dropped —
//! when that turn ends. Nothing here decides what a press means; the keymap
//! reads the press and the update carries the decision out.

use super::model::Model;

impl Model {
    /// The message a running turn holds behind it, if any.
    pub(crate) fn queued(&self) -> Option<&str> {
        self.queued.as_deref()
    }

    /// Holds `text` behind the running turn, and answers the line it
    /// displaced: the queue is one deep, so the operator's last word wins.
    pub(crate) fn queue(&mut self, text: String) -> Option<String> {
        self.queued.replace(text)
    }

    /// Takes the held line, if there is one.
    pub(crate) fn take_queued(&mut self) -> Option<String> {
        self.queued.take()
    }

    /// Drops the line held behind the running turn, if there is one: Ctrl+G's
    /// whole effect (`02` §7). Nothing is sent, and the turn keeps running.
    pub(crate) fn drop_queued(&mut self) {
        self.queued = None;
    }

    /// Opens a turn with the operator's own words.
    pub(crate) fn begin_turn(&mut self, message: String) {
        self.transcript.begin_turn(message);
    }

    /// Marks the running turn as started.
    pub(crate) fn start_turn(&mut self) {
        self.busy = true;
    }

    /// Marks the running turn as ended.
    pub(crate) fn end_turn(&mut self) {
        self.busy = false;
    }
}
