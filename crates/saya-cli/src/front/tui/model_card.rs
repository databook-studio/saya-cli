//! The card's own model state: how many calls are waiting, and which answer
//! the operator's arrows are pointing at.
//!
//! Kept apart from [`super::model`] so the surface's state struct stays one
//! short declaration per concern, and apart from [`super::model_turn`] because
//! a waiting call is not a turn.
//!
//! The focus is deliberately not sticky across a typed character: a call can
//! land while the operator is typing the next line, and Enter must never turn a
//! draft into a consent. Only an arrow press points at an answer, and typing
//! puts the pointer down again.

use super::model::Model;
use super::view::cards::choices_of;

impl Model {
    /// How many calls are waiting for an answer: the one on the card and the
    /// ones queued behind it, in the order they arrived. Zero with the slot
    /// free.
    pub(crate) fn card_waiting(&self) -> usize {
        if self.card().is_some() {
            1 + self.held.len()
        } else {
            0
        }
    }

    /// The answer the operator's arrows point at, if they have pointed at one.
    pub(crate) fn card_choice(&self) -> Option<usize> {
        self.card_choice
    }

    /// Walks the focused answer by `step`, a negative count moving up: the
    /// first arrow press lands on the first answer, and the walk stops at the
    /// ends of the list rather than wrapping around it.
    pub(crate) fn move_choice(&mut self, step: isize) {
        let Some(question) = self.card() else {
            return;
        };
        let answers = choices_of(question, super::view::cards::family_of(question));
        if answers.is_empty() {
            return;
        }
        let last = answers.len() - 1;
        let landed = match self.card_choice {
            None => 0,
            Some(at) => at.saturating_add_signed(step).min(last),
        };
        self.card_choice = Some(landed);
    }

    /// Drops the focus: a typed character is the composer's, so the next Enter
    /// is the composer's too.
    pub(crate) fn clear_choice(&mut self) {
        self.card_choice = None;
    }
}
