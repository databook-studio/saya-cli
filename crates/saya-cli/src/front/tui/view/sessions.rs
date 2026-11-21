//! The sessions picker: the saved sessions an operator chooses from, the filter
//! typed over them, and the two commands the list offers.
//!
//! The pane that paints those rows is [`super::sessions_rows`]. This module owns
//! only what the picker *is*, so moving its selection, filtering it and asking
//! before a delete are decisions a test can make without a key press. The key
//! that opens the picker, and the keymap that walks it, belong to the runtime.
//!
//! `d` deletes nothing. It puts the picker in a confirm state that owns the
//! keyboard and names the row it would delete; only a `y` answering it yields a
//! [`Command::Delete`], and `n` or Esc withdraws the ask.

use super::sessions_row::SessionRow;
use crate::front::tui::theme::Charset;

/// The pane's title, on its top border.
pub(crate) const TITLE: &str = "Sessions";

/// What the top border says a press does while nothing is typed.
pub(crate) const FILTER_HINT: &str = "type to filter";

/// What the pane says when the filter matches no saved session.
pub(crate) const NO_MATCH: &str = "no session matches";

/// What the pane says when there is nothing saved at all.
pub(crate) const NO_SESSIONS: &str = "no saved sessions";

/// The verb the confirm asks with: `delete <title>? y/n`.
pub(crate) const CONFIRM_ASK: &str = "delete";

/// What a press asks the runtime to do. A press that only moves the picker,
/// filters it or closes it asks for nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Command {
    /// Resume the row this id names: the decision on the row says what that
    /// does — attach the open run, prepare a new epoch, or open read-only.
    Resume {
        /// The saved session's id.
        id: String,
    },
    /// Delete the row this id names. Only a `y` answering the confirm yields
    /// this: `d` asks first, and nothing is deleted until the operator says so.
    Delete {
        /// The saved session's id.
        id: String,
    },
}

/// The picker: the rows it lists, the filter typed into it, the row selected in
/// what the filter leaves, and whether `d` is waiting for an answer.
#[derive(Debug, Clone)]
pub(crate) struct Picker {
    /// Every saved session, newest first, as the store listed them.
    pub(crate) rows: Vec<SessionRow>,
    /// The filter text, as typing left it.
    pub(crate) query: String,
    /// The selected row's index into [`Picker::listed`], clamped to it
    /// whenever the filter leaves fewer rows.
    pub(crate) selected: usize,
    /// Whether `d` asked and the operator has not answered: while it is set
    /// the confirm owns the keyboard, and only `y`, `n` and Esc are read.
    pub(crate) confirming: bool,
    /// Whether the picker is still up. Esc on the list closes it.
    pub(crate) open: bool,
    /// Which glyph set the pane draws with.
    pub(crate) charset: Charset,
}

impl Default for Picker {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            query: String::new(),
            selected: 0,
            confirming: false,
            open: true,
            charset: Charset::Utf8,
        }
    }
}

impl Picker {
    /// The picker over `rows`, nothing typed and the first row selected.
    pub(crate) fn new(rows: Vec<SessionRow>) -> Self {
        Self {
            rows,
            ..Self::default()
        }
    }

    /// The filter, as typing left it.
    pub(crate) fn query(&self) -> &str {
        &self.query
    }

    /// The rows the filter leaves, in the store's order.
    pub(crate) fn listed(&self) -> Vec<&SessionRow> {
        self.rows.iter().filter(|row| self.matches(row)).collect()
    }

    /// Whether the filter keeps `row`: its title or the database it was on. The
    /// match is case-blind, because an operator typing a database's name does
    /// not stop to spell it.
    fn matches(&self, row: &SessionRow) -> bool {
        if self.query.is_empty() {
            return true;
        }
        let query = self.query.to_lowercase();
        row.title.to_lowercase().contains(&query)
            || row
                .database
                .as_deref()
                .is_some_and(|database| database.to_lowercase().contains(&query))
    }

    /// The selected row's index, clamped to the rows the filter leaves.
    pub(crate) fn selected(&self) -> usize {
        self.selected.min(self.listed().len().saturating_sub(1))
    }

    /// The selected row, when the picker is up and the filter leaves one.
    pub(crate) fn selected_row(&self) -> Option<&SessionRow> {
        if !self.open {
            return None;
        }
        self.listed().into_iter().nth(self.selected())
    }

    /// Whether the picker is still up.
    pub(crate) const fn is_open(&self) -> bool {
        self.open
    }

    /// Whether `d` is waiting for an answer.
    pub(crate) const fn is_confirming(&self) -> bool {
        self.confirming
    }

    /// What the confirm asks, when it is asking.
    pub(crate) fn ask(&self) -> Option<String> {
        if !self.confirming {
            return None;
        }
        let row = self.selected_row()?;
        Some(format!("{CONFIRM_ASK} {}? y/n", row.title))
    }

    /// Puts one typed character at the filter's end. Every keystroke narrows
    /// the list, so the selection starts again at the first row the filter
    /// leaves. The confirm owns the keyboard while it asks: typing is ignored.
    pub(crate) fn type_char(&mut self, character: char) {
        if self.confirming {
            return;
        }
        self.query.push(character);
        self.selected = 0;
    }

    /// Removes the filter's last character, if it has one.
    pub(crate) fn backspace(&mut self) -> bool {
        self.selected = 0;
        self.query.pop().is_some()
    }

    /// Moves the selection one row down, and no further than the last row.
    pub(crate) fn down(&mut self) {
        if self.confirming || !self.open {
            return;
        }
        let len = self.listed().len();
        if len == 0 {
            return;
        }
        self.selected = (self.selected() + 1).min(len - 1);
    }

    /// Moves the selection one row up, and no further than the first row.
    pub(crate) fn up(&mut self) {
        if self.confirming || !self.open {
            return;
        }
        self.selected = self.selected().saturating_sub(1);
    }

    /// Resumes the selected row: the pane closes and the runtime acts on the
    /// row's own decision.
    pub(crate) fn resume(&mut self) -> Option<Command> {
        if self.confirming {
            return None;
        }
        let id = self.selected_row()?.id.clone();
        self.open = false;
        Some(Command::Resume { id })
    }

    /// Asks before deleting the selected row. This press deletes nothing: it
    /// puts the picker in the confirm state, which is the only way to a
    /// [`Command::Delete`].
    pub(crate) fn ask_delete(&mut self) -> Option<Command> {
        if self.confirming || self.selected_row().is_none() {
            return None;
        }
        self.confirming = true;
        None
    }

    /// Answers the confirm: `y` deletes the row it named, `n` withdraws the ask.
    /// The picker stays up either way — the runtime takes the deleted row out
    /// of it once the store has deleted the record.
    pub(crate) fn answer(&mut self, yes: bool) -> Option<Command> {
        if !self.confirming {
            return None;
        }
        self.confirming = false;
        if !yes {
            return None;
        }
        let id = self.selected_row()?.id.clone();
        Some(Command::Delete { id })
    }

    /// Esc: withdraws the ask when the confirm is open, and closes the picker
    /// when it is not. Neither asks the runtime for anything.
    pub(crate) fn escape(&mut self) -> Option<Command> {
        if self.confirming {
            self.confirming = false;
        } else {
            self.open = false;
        }
        None
    }
}
