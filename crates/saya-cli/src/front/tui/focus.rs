//! The block cursor (`02` §6): the seat over the transcript's entries, and the
//! model's half of it.
//!
//! `Alt+↑/↓` moves one seat over the entries of every turn, oldest first, and
//! clamps at both ends rather than wrapping: a cursor that jumped from the
//! oldest entry to the newest would lose the operator's place. A first press
//! from no seat lands on the newest entry — the one being read.
//!
//! A seat is an [`EntryPath`]: a turn's place in the conversation and an
//! entry's place in that turn. That is what lets a fold opened here reach a
//! group in *any* turn, and not only the newest one — `02` §6 replaces "only
//! the newest group can be expanded". A failed group never folds, the rule T5
//! put in the painter and T1 in the reducer.
//!
//! What `c` copies is [`focus_text`]'s business; this module holds the seat,
//! the walk and the fold the seat acts on.

use super::model::Model;
use crate::front::view::{Entry, TurnView};

/// One entry of the conversation: its turn's place, and its own place in that
/// turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EntryPath {
    /// The turn's place in the conversation, oldest first.
    pub(crate) turn: usize,
    /// The entry's place in that turn, in reading order.
    pub(crate) entry: usize,
}

impl EntryPath {
    /// The seat at `turn`'s `entry`.
    pub(crate) const fn new(turn: usize, entry: usize) -> Self {
        Self { turn, entry }
    }
}

/// Every entry of the conversation, oldest first: the seats the cursor walks.
pub(crate) fn positions(turns: &[TurnView]) -> Vec<EntryPath> {
    turns
        .iter()
        .enumerate()
        .flat_map(|(turn, view)| {
            (0..view.entries().len()).map(move |entry| EntryPath::new(turn, entry))
        })
        .collect()
}

/// The newest entry of the conversation: where a first press lands.
pub(crate) fn newest(turns: &[TurnView]) -> Option<EntryPath> {
    positions(turns).last().copied()
}

/// The seat one step from `from` — newer with `down`, older without — clamped:
/// at either end the cursor stays where it is. From no seat at all, the first
/// press lands on the newest entry, because there is nothing newer to walk to.
pub(crate) fn walk(turns: &[TurnView], from: Option<EntryPath>, down: bool) -> Option<EntryPath> {
    let seats = positions(turns);
    if seats.is_empty() {
        return None;
    }
    let Some(index) = from.and_then(|from| seats.iter().position(|seat| *seat == from)) else {
        // No seat yet — or the conversation was cleared under the cursor — so
        // the walk starts at the newest entry.
        return seats.last().copied();
    };
    let next = match down {
        true => (index + 1).min(seats.len() - 1),
        false => index.saturating_sub(1),
    };
    seats.get(next).copied()
}

impl Model {
    /// The entry the block cursor holds, if it holds one.
    pub(crate) const fn focus(&self) -> Option<EntryPath> {
        self.focus
    }

    /// Whether the block cursor holds an entry: while it does, the keyboard is
    /// the cursor's and the composer is behind it (`02` §6).
    pub(crate) const fn in_cursor_mode(&self) -> bool {
        self.focus.is_some()
    }

    /// Walks the cursor one entry newer (`down`) or older, clamped at both
    /// ends.
    pub(crate) fn move_focus(&mut self, down: bool) {
        self.focus = walk(self.turns(), self.focus, down);
    }

    /// Leaves cursor mode: Esc's whole effect (`02` §6).
    pub(crate) fn clear_focus(&mut self) {
        self.focus = None;
    }

    /// The entry the cursor holds, while it holds one.
    pub(crate) fn focused_entry(&self) -> Option<&Entry> {
        let at = self.focus?;
        self.turns().get(at.turn)?.entries().get(at.entry)
    }

    /// Expands (`Some(false)`), folds (`Some(true)`) or toggles (`None`) the
    /// group the cursor holds. Any turn's group answers: the fold is opened
    /// here, not in the newest turn (`02` §6).
    pub(crate) fn fold_focus(&mut self, fold: Option<bool>) {
        let Some(at) = self.focus else {
            return;
        };
        let Some(entry) = self
            .transcript
            .turn_mut(at.turn)
            .and_then(|turn| turn.entry_mut(at.entry))
        else {
            return;
        };
        let Entry::Activity(activity) = entry else {
            return;
        };
        // A failed group never folds: what went wrong is the part the operator
        // has to read (T5's rule, kept here so the cursor cannot hide it).
        if activity.failed {
            return;
        }
        activity.folded = fold.unwrap_or(!activity.folded);
    }

    /// Whether the focused entry shows its provenance line (`02` §6's `i`).
    pub(crate) const fn inspector(&self) -> bool {
        self.inspector
    }

    /// Shows the inspector line, or takes it away.
    pub(crate) fn toggle_inspector(&mut self) {
        self.inspector = !self.inspector;
    }
}
