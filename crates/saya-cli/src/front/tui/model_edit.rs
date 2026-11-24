//! Editing the last message: the one message a turn was opened with, the Esc
//! that arms the edit, and the flag a resend carries so the turn it replaces is
//! rewound before the edited one runs.
//!
//! The text is kept raw — the same line the turn received — so an edited
//! `--param` value is still redacted on the way out, never before it is sent.

use super::model::Model;

/// What the composer remembers of the last turn it sent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Edit {
    /// The message the last turn was opened with, as the run received it.
    last: Option<String>,
    /// The first Esc on an empty composer: the next Esc restores `last`.
    armed: bool,
    /// The restored line is an edit of `last`, so sending it replaces that
    /// turn: its file changes are rewound before the edited message runs.
    replacing: bool,
}

impl Model {
    /// Records the message a turn was opened with: what Esc-Esc restores.
    pub(crate) fn remember_turn(&mut self, message: String) {
        self.edit = Edit {
            last: Some(message),
            ..Edit::default()
        };
    }

    /// Forgets the editable turn: a line that is not a turn leaves nothing for
    /// Esc-Esc to restore.
    pub(crate) fn forget_turn(&mut self) {
        self.edit = Edit::default();
    }

    /// The first Esc: only an empty composer under a settled turn, which is
    /// where the keymap already sends [`Intent::EditLast`](super::keys::Intent).
    pub(crate) fn arm_edit(&mut self) {
        if self.edit.last.is_some() && !self.is_busy() {
            self.edit.armed = true;
        }
    }

    /// Takes the arm the first Esc set.
    pub(crate) fn take_armed(&mut self) -> bool {
        std::mem::take(&mut self.edit.armed)
    }

    /// Drops the arm: a key that is not the second Esc is no longer editing.
    pub(crate) fn disarm_edit(&mut self) {
        self.edit.armed = false;
    }

    /// The second Esc: puts the last turn's message back on the composer.
    pub(crate) fn restore_last(&mut self) -> bool {
        if self.is_busy() {
            return false;
        }
        let Some(text) = self.edit.last.clone() else {
            return false;
        };
        self.composer = text;
        self.edit.armed = false;
        self.edit.replacing = true;
        true
    }

    /// Takes the resend flag: the line being sent replaces the last turn.
    pub(crate) fn take_replacing(&mut self) -> bool {
        std::mem::take(&mut self.edit.replacing)
    }

    /// Drops the last turn from the visible conversation, so the edited one
    /// takes its place rather than following it.
    pub(crate) fn drop_last_turn(&mut self) {
        self.transcript.drop_last_turn();
    }
}
