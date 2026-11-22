//! The command palette's state (`02` §11): the query typed into it, the row
//! selected in the list it yields, and the words the pane says around them.
//!
//! The rows are the catalogue's — read through [`super::palette_match`], never
//! copied — and the pane that paints them is [`super::palette_rows`]. This
//! module owns only what the palette *is*, so moving its selection is a
//! decision a test can make without a key press. The key that opens the pane,
//! and the keymap that walks it, belong to the runtime.

use super::palette_match::{self, Entry};
use crate::front::tui::theme::Charset;

/// The pane's title, on its top border.
pub(crate) const TITLE: &str = "Commands";

/// The label the usage row starts with.
pub(crate) const USAGE: &str = "usage:";

/// What the pane says when the query matches nothing: the empty row.
pub(crate) const NO_MATCH: &str = "no command matches";

/// The rows the pane asks the frame for: the chrome's three and nine command
/// rows. The catalogue is longer than any terminal, so the list scrolls rather
/// than the pane growing.
pub(crate) const ROWS: usize = 12;

/// The palette: the query typed into it, the row selected in the list it
/// yields, and the glyph set it draws with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Palette {
    /// The query text, as typing left it.
    pub(crate) query: String,
    /// The selected row's index into [`Palette::entries`], clamped to the list
    /// whenever the list is shorter than it.
    pub(crate) selected: usize,
    /// Which glyph set the pane draws with.
    pub(crate) charset: Charset,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            query: String::new(),
            selected: 0,
            charset: Charset::Utf8,
        }
    }
}

impl Palette {
    /// The query, as typing left it.
    pub(crate) fn query(&self) -> &str {
        &self.query
    }

    /// The rows the query matches, best first.
    pub(crate) fn entries(&self) -> Vec<Entry> {
        palette_match::entries(&self.query)
    }

    /// The selected row's index, clamped to the list.
    pub(crate) fn selected(&self) -> usize {
        let len = self.entries().len();
        self.selected.min(len.saturating_sub(1))
    }

    /// The selected row, when the query matches anything.
    pub(crate) fn selected_entry(&self) -> Option<Entry> {
        self.entries().into_iter().nth(self.selected())
    }

    /// Puts one typed character at the query's end. Every keystroke narrows the
    /// list, so the selection starts again at the best row.
    pub(crate) fn push(&mut self, character: char) {
        self.query.push(character);
        self.selected = 0;
    }

    /// Removes the query's last character, if it has one.
    pub(crate) fn backspace(&mut self) -> bool {
        self.selected = 0;
        self.query.pop().is_some()
    }

    /// Moves the selection one row down, and no further than the last row.
    pub(crate) fn down(&mut self) {
        let len = self.entries().len();
        if len == 0 {
            return;
        }
        self.selected = (self.selected + 1).min(len - 1);
    }

    /// Moves the selection one row up, and no further than the first row.
    pub(crate) fn up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }
}
