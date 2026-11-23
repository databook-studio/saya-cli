//! The one table of keys the help sheet draws (`02` §11).
//!
//! A row names the presses it covers, the words the sheet shows beside them,
//! the group it is filed under and the state it answers in. The presses
//! themselves are read by `keys.rs`, `cards_keys.rs`, `history_keys.rs` and
//! `overlay_keys.rs`, never here: this table only says them, and the guard test
//! in `help_tests.rs` presses every row through that keymap, so a row the keymap
//! has stopped handling fails the suite instead of misleading an operator.
//!
//! The sheet is the whole keyboard, not one state's slice of it: every group is
//! always listed, and the action says which state the key acts in ("while a
//! card is open", "while the drawer is open"). That keeps the sheet stable to
//! read and impossible to leave a live key out of.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// One key press, as the keymap reads it: a code and the modifiers held with
/// it. The sheet names the presses the keymap matches on, so a row that says
/// `Ctrl+G` is the very chord `keys.rs` reads as the drop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Chord {
    pub(crate) code: KeyCode,
    pub(crate) modifiers: KeyModifiers,
}

impl Chord {
    /// The press, as the input thread hands it over.
    pub(crate) const fn event(self) -> KeyEvent {
        KeyEvent::new(self.code, self.modifiers)
    }
}

/// The state a row is valid in: the state the guard presses it in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum When {
    /// Only while a card is open: the answers the card offers.
    Card,
    /// Only while the open card offered a token a session grant could record.
    CardGrant,
    /// Only while no card is open and no turn runs: the composer's idle keys.
    Idle,
    /// Only while no card is open: the composer's own keys.
    Composer,
    /// Only while a line is held behind a running turn.
    Queued,
    /// Only while a turn runs.
    Busy,
    /// Only while the input history's search is open.
    Search,
    /// Only while the block cursor holds an entry.
    Cursor,
    /// Only while the jobs drawer is open.
    Jobs,
    /// Only while the drawer's cancel confirm is open.
    JobsConfirm,
    /// Only once a turn has changed a file: the changes pane has something to
    /// open (UI5).
    Changed,
}

/// Which of the sheet's groups a row is filed under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Group {
    /// Sending and editing a message.
    Conversation,
    /// The input history and its search.
    History,
    /// The transcript's steps, the block cursor and the detail key.
    Steps,
    /// Answering a call.
    Approvals,
    /// The background jobs drawer.
    Jobs,
    /// Moving between surfaces.
    Navigation,
}

impl Group {
    /// The groups, in the order the sheet reads them.
    pub(crate) const ALL: [Self; 6] = [
        Self::Conversation,
        Self::History,
        Self::Steps,
        Self::Approvals,
        Self::Jobs,
        Self::Navigation,
    ];

    /// The group's heading.
    pub(crate) const fn heading(self) -> &'static str {
        match self {
            Self::Conversation => "Conversation",
            Self::History => "History",
            Self::Steps => "Steps",
            Self::Approvals => "Approvals",
            Self::Jobs => "Jobs",
            Self::Navigation => "Navigation",
        }
    }
}

/// One row of the sheet: the presses it names, the words it shows and the state
/// it is valid in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Shortcut {
    /// The presses, as the sheet writes them (`y, a`, `Ctrl+G`).
    pub(crate) label: &'static str,
    /// The presses themselves, for the guard that presses them through the
    /// keymap. Every chord a label names is here.
    pub(crate) chords: &'static [Chord],
    /// What the presses do, in the surface's own words.
    pub(crate) action: &'static str,
    /// The group the row is filed under.
    pub(crate) group: Group,
    /// The state the row is valid in.
    pub(crate) when: When,
}

/// The presses the table names, in the one spelling the keymap matches on.
pub(super) const ENTER: Chord = bare(KeyCode::Enter);
pub(super) const ALT_ENTER: Chord = alt(KeyCode::Enter);
pub(super) const ESCAPE: Chord = bare(KeyCode::Esc);
pub(super) const BACKSPACE: Chord = bare(KeyCode::Backspace);
pub(super) const SHIFT_TAB: Chord = shift(KeyCode::BackTab);
pub(super) const TAB: Chord = bare(KeyCode::Tab);
pub(super) const CTRL_G: Chord = ctrl('g');
pub(super) const CTRL_J: Chord = ctrl('j');
pub(super) const CTRL_K: Chord = ctrl('k');
pub(super) const CTRL_O: Chord = ctrl('o');
pub(super) const CTRL_R: Chord = ctrl('r');
/// The changes pane's chord (UI5): the diff of the last turn that wrote. `d`
/// alone stays a letter, and a deny alias on a focused panel.
pub(super) const CTRL_D: Chord = ctrl('d');
pub(super) const QUESTION: Chord = bare(KeyCode::Char('?'));
pub(super) const SLASH: Chord = bare(KeyCode::Char('/'));
pub(super) const YES: Chord = bare(KeyCode::Char('y'));
pub(super) const ALSO_YES: Chord = bare(KeyCode::Char('a'));
pub(super) const SESSION: Chord = bare(KeyCode::Char('s'));
pub(super) const NO: Chord = bare(KeyCode::Char('n'));
pub(super) const ALSO_NO: Chord = bare(KeyCode::Char('d'));
pub(super) const X: Chord = bare(KeyCode::Char('x'));
pub(super) const VIEW: Chord = bare(KeyCode::Char('v'));
pub(super) const COPY: Chord = bare(KeyCode::Char('c'));
pub(super) const INSPECT: Chord = bare(KeyCode::Char('i'));
pub(super) const UP: Chord = bare(KeyCode::Up);
pub(super) const DOWN: Chord = bare(KeyCode::Down);
pub(super) const ALT_UP: Chord = alt(KeyCode::Up);
pub(super) const ALT_DOWN: Chord = alt(KeyCode::Down);
pub(super) const PAGE_UP: Chord = bare(KeyCode::PageUp);
pub(super) const PAGE_DOWN: Chord = bare(KeyCode::PageDown);
pub(super) const RIGHT: Chord = bare(KeyCode::Right);
pub(super) const LEFT: Chord = bare(KeyCode::Left);

/// A press with nothing else held.
const fn bare(code: KeyCode) -> Chord {
    shifted(code, KeyModifiers::NONE)
}

/// Ctrl and one character: the chords that drop a line, not the ones that type.
const fn ctrl(character: char) -> Chord {
    shifted(KeyCode::Char(character), KeyModifiers::CONTROL)
}

/// A press with Alt held.
const fn alt(code: KeyCode) -> Chord {
    shifted(code, KeyModifiers::ALT)
}

/// A press with Shift held.
const fn shift(code: KeyCode) -> Chord {
    shifted(code, KeyModifiers::SHIFT)
}

/// The one place a chord is built, so a table row cannot spell a press the
/// keymap would never see.
const fn shifted(code: KeyCode, modifiers: KeyModifiers) -> Chord {
    Chord { code, modifiers }
}

/// The sheet's rows, in [`Group::ALL`]'s order: the conversation, history and
/// steps in [`super::help_rows_a`], the approvals, jobs and navigation in
/// [`super::help_rows_b`].
pub(crate) fn shortcuts() -> impl Iterator<Item = &'static Shortcut> {
    super::help_rows_a::ROWS
        .iter()
        .chain(super::help_rows_b::ROWS.iter())
}
