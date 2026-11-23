//! The approval panel's body (UI5): the call's own rows, the change it makes to
//! a file as a bounded diff, and the exact bytes `v` opens — one block, wrapped
//! to the panel's room, and painted row by row.
//!
//! The diff belongs *inside* the body: the operator reads the path and the
//! change it makes under it, then answers. It is bounded here rather than in the
//! pane above, so what `v` opens is the same rows the panel was handed — never a
//! second diff, never a second bound.
//!
//! A row carries the meaning it is painted with, never a colour: `+` is
//! [`Ink::Added`] and `-` is [`Ink::Removed`] whether the scheme is dark, light
//! or gone — and because the sign is part of the row's own text, a reader with
//! no colour loses nothing.

use super::cards::Card;
use super::cards_body;

/// What a body row is painted with, by meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ink {
    /// The call's own words: the title's block, a diff's context.
    Plain,
    /// A line the change adds.
    Added,
    /// A line the change takes away.
    Removed,
    /// A `@@` header or a note: where in the file the hunk sits.
    Context,
    /// The row that says how many the panel is not showing.
    Withheld,
}

/// One body row and its meaning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Row {
    pub(crate) text: String,
    pub(crate) ink: Ink,
}

impl Row {
    /// One row of a body that is the call's own words.
    fn plain(text: String) -> Self {
        Self {
            text,
            ink: Ink::Plain,
        }
    }

    /// The row's own text, as a test or a copy reads it.
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    /// The row that says how many the panel is not showing.
    fn withheld(count: usize) -> Self {
        Self {
            text: format!("+{count} more lines"),
            ink: Ink::Withheld,
        }
    }

    /// The row the change paints: a hunk header, a line added, a line taken
    /// away, or context.
    fn of_change(row: String) -> Self {
        let ink = match row.chars().next() {
            Some('+') => Ink::Added,
            Some('-') => Ink::Removed,
            Some('@') | Some('\\') => Ink::Context,
            _ => Ink::Plain,
        };
        Self { text: row, ink }
    }
}

impl Card {
    /// The rows the block shows at `room` cells: the call's own rows, the
    /// change it makes, and — when `v` has opened the rest — the exact bytes the
    /// card was holding back.
    ///
    /// Folded, the body stops at the card's cap and closes with the row that
    /// says how many it is not showing, so the cap is never a silent cut. `v`
    /// opens the whole body: the call's bytes, the change, and the bytes it was
    /// holding back.
    pub(crate) fn shown_rows(&self, full: bool, room: usize) -> Vec<Row> {
        let mut rows = self.body_rows(room);
        if full {
            rows.extend(plain(&cards_body::wrap(&self.details, room)));
            return rows;
        }
        capped(rows, self.cap)
    }

    /// How many rows the card holds back, so `v` knows whether it has anything
    /// to open: what the cap leaves out, and the exact bytes it always keeps
    /// behind `v` however short they are.
    pub(crate) fn withheld(&self, room: usize) -> usize {
        let body = self.body_rows(room).len();
        let cut = body.saturating_sub(self.cap);
        cut + cards_body::wrap(&self.details, room).len()
    }

    /// The body before the cap: the call's own rows and the change it makes.
    fn body_rows(&self, room: usize) -> Vec<Row> {
        let mut rows = plain(&cards_body::wrap(&self.target, room));
        rows.extend(change_rows(&self.change, room));
        rows
    }
}

/// The call's own rows, each carrying its words.
fn plain(rows: &[String]) -> Vec<Row> {
    rows.iter().cloned().map(Row::plain).collect()
}

/// The change's rows, wrapped to the panel's room and each painted by its sign;
/// a row that wraps keeps its meaning on every continuation.
fn change_rows(change: &[String], room: usize) -> Vec<Row> {
    change
        .iter()
        .flat_map(|row| {
            cards_body::wrap(std::slice::from_ref(row), room)
                .into_iter()
                .map(Row::of_change)
        })
        .collect()
}

/// `rows` cut to the card's `cap`, with one row after them saying how many the
/// cut left out.
///
/// The cap states whole rows an operator reads — the same rows the panel showed
/// before the count existed — and the count comes on top of them, so the cut is
/// never silent and never costs a row the operator was reading. At the tightest
/// frame the frame's own budget is what gives way, and the count is the first to
/// go: the answers, and the rows above them, never do.
fn capped(mut rows: Vec<Row>, cap: usize) -> Vec<Row> {
    if rows.len() <= cap {
        return rows;
    }
    let withheld = rows.len() - cap;
    rows.truncate(cap.max(1));
    rows.push(Row::withheld(withheld));
    rows
}
