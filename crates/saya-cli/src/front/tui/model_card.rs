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

use std::time::{Duration, Instant};

use super::model::Model;
use super::view::cards::choices_of;
use crate::chat::Question;

/// How long after a key press the operator still counts as typing: a call that
/// opens inside this window leaves the keyboard in the composer.
pub(crate) const QUIET_OF_TYPING: Duration = Duration::from_millis(1500);

/// Which of the two seats holds the keyboard while a call waits (UI3).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Seat {
    /// The composer: every printable key is the draft's.
    #[default]
    Composer,
    /// The approval panel: its keys answer the call, and a key it has no
    /// answer for does nothing at all.
    Panel,
}

impl Model {
    /// Puts a call to the operator: one card at a time, opening folded at its
    /// first row, with the keyboard where the operator's hands are
    /// ([`Model::opening_seat`]).
    pub(crate) fn show_card(&mut self, card: Question) {
        self.card = Some(card);
        // A card owns the surface: the `@` list is the composer's, and `Tab`
        // answers the card now, so a list that was open goes away rather than
        // taking the one key the operator needs (UI3).
        self.mention = None;
        self.card_full = false;
        self.card_scroll = 0;
        self.card_choice = None;
        self.seat = self.opening_seat();
    }

    /// Takes the open card, if one is waiting for an answer.
    pub(crate) fn take_card(&mut self) -> Option<Question> {
        self.card_full = false;
        self.card_scroll = 0;
        self.card_choice = None;
        self.seat = Seat::Composer;
        self.card.take()
    }

    /// Whether the open card's whole body is shown, or only the rows the
    /// screen has for it.
    pub(crate) const fn card_full(&self) -> bool {
        self.card_full
    }

    /// How far the open card's body is scrolled, in rows.
    pub(crate) const fn card_scroll(&self) -> usize {
        self.card_scroll
    }

    /// Shows the open card's whole body, or folds it back to its rows.
    pub(crate) fn toggle_card_full(&mut self) {
        self.card_full = !self.card_full;
    }

    /// Moves the open card's body by `rows`, a negative count moving it up. It
    /// never moves above its first row; the pane clamps the far end against
    /// the body it paints.
    pub(crate) fn scroll_card(&mut self, rows: isize) {
        let moved = self.card_scroll as isize + rows;
        self.card_scroll = moved.max(0) as usize;
    }

    /// Takes the reading of the injected clock the message being folded was
    /// stamped with. The model never reads a clock of its own.
    pub(crate) fn at(&mut self, now: Instant) {
        self.now = Some(now);
    }

    /// Records that a key was pressed at the reading just taken: a call that
    /// opens inside [`QUIET_OF_TYPING`] of it leaves the keyboard in the
    /// composer.
    pub(crate) fn typed_now(&mut self) {
        if self.now.is_some() {
            self.typed_at = self.now;
        }
    }

    /// Whether the open card's panel holds the keyboard, so its keys answer.
    pub(crate) fn panel_focused(&self) -> bool {
        self.card.is_some() && self.seat == Seat::Panel
    }

    /// The seat a call that opens now must take: the panel only on an empty
    /// composer that has been quiet for [`QUIET_OF_TYPING`], because a call can
    /// land in the middle of a sentence.
    pub(crate) fn opening_seat(&self) -> Seat {
        if !self.composer.is_empty() {
            return Seat::Composer;
        }
        match (self.typed_at, self.now) {
            (Some(typed), Some(now)) if now.saturating_duration_since(typed) < QUIET_OF_TYPING => {
                Seat::Composer
            }
            _ => Seat::Panel,
        }
    }

    /// `Tab`: the keyboard moves to the other seat, or back. Only a waiting
    /// card has two seats; with the slot free this changes nothing an operator
    /// can see.
    pub(crate) fn toggle_seat(&mut self) {
        self.seat = match self.seat {
            Seat::Panel => Seat::Composer,
            Seat::Composer => Seat::Panel,
        };
    }

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
