//! The card's own model state: how many calls are waiting, and which answer
//! the operator's arrows are pointing at.
//!
//! Kept apart from [`super::model`] so the surface's state struct stays one
//! short declaration per concern, and apart from [`super::model_turn`] because
//! a waiting call is not a turn.

use super::model::Model;

impl Model {
    /// How many calls are waiting for an answer: the one on the card and the
    /// ones queued behind it, in the order they arrived. Zero with the slot
    /// free.
    pub(crate) fn card_waiting(&self) -> usize {
        1
    }

    /// The answer the operator's arrows point at, if they have pointed at one.
    pub(crate) fn card_choice(&self) -> Option<usize> {
        None
    }

    /// Walks the focused answer by `step`, a negative count moving up: the
    /// first arrow press lands on the first answer, and the walk stops at the
    /// ends of the list rather than wrapping around it.
    pub(crate) fn move_choice(&mut self, _step: isize) {}

    /// Drops the focus: a typed character is the composer's, so the next Enter
    /// is the composer's too.
    pub(crate) fn clear_choice(&mut self) {}
}
