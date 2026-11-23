//! The one table of keys the help sheet draws (`02` §11).
//!
//! A row names the presses it covers, the words the sheet shows beside them and
//! the state it is valid in. The presses themselves are read by `keys.rs` and
//! `cards_keys.rs`, never here: this table only says them, and the guard test in
//! `help_tests.rs` presses every row through that keymap, so a row the keymap
//! has stopped handling fails the suite instead of misleading an operator.
//!
//! `02` §11 asks for "only the keys valid in the current state (e.g. approval
//! keys only while a card is open)": that condition is [`When`], and the sheet
//! filters this table by it rather than keeping a list per state.

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

/// The state a row is valid in (`02` §11: "only the keys valid in the current
/// state").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum When {
    /// Only while a card is open: the answers the card offers.
    Card,
    /// Only while the open card offered a token a session grant could record.
    CardGrant,
    /// Only while no card is open: the composer's own keys.
    Composer,
    /// Only while a line is held behind a running turn.
    Queued,
    /// Only while a turn runs.
    Busy,
}

/// Which of `02` §11's three columns a row is filed under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Group {
    /// Sending, answering, editing.
    Conversation,
    /// Moving: the mode cycle, the card's body, the held line.
    Navigation,
    /// Looking at what the card withholds.
    CopyView,
}

impl Group {
    /// The columns, in the order `02` §11 names them.
    pub(crate) const ALL: [Self; 3] = [Self::Conversation, Self::Navigation, Self::CopyView];

    /// The column's heading, as `02` §11 words it.
    pub(crate) const fn heading(self) -> &'static str {
        match self {
            Self::Conversation => "Conversation",
            Self::Navigation => "Navigation",
            Self::CopyView => "Copy & view",
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
    /// The column the row is filed under.
    pub(crate) group: Group,
    /// The state the row is valid in.
    pub(crate) when: When,
}

/// The presses the table names, in the one spelling the keymap matches on.
const ENTER: Chord = bare(KeyCode::Enter);
const ALT_ENTER: Chord = alt(KeyCode::Enter);
const ESCAPE: Chord = bare(KeyCode::Esc);
const BACKSPACE: Chord = bare(KeyCode::Backspace);
const SHIFT_TAB: Chord = shift(KeyCode::BackTab);
const CTRL_G: Chord = ctrl('g');
/// The palette's chord, and the sheet's own opener: both read by the composer.
const CTRL_K: Chord = ctrl('k');
/// The detail key: the transcript's steps in full, or one compact line each.
const CTRL_O: Chord = ctrl('o');
const QUESTION: Chord = bare(KeyCode::Char('?'));
const YES: Chord = bare(KeyCode::Char('y'));
const ALSO_YES: Chord = bare(KeyCode::Char('a'));
const SESSION: Chord = bare(KeyCode::Char('s'));
const NO: Chord = bare(KeyCode::Char('n'));
const ALSO_NO: Chord = bare(KeyCode::Char('d'));
const VIEW: Chord = bare(KeyCode::Char('v'));
const UP: Chord = bare(KeyCode::Up);
const DOWN: Chord = bare(KeyCode::Down);
const PAGE_UP: Chord = bare(KeyCode::PageUp);
const PAGE_DOWN: Chord = bare(KeyCode::PageDown);

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

/// The sheet's rows, in the order `02` §11's columns are read.
///
/// Every row here is a press `keys.rs` or `cards_keys.rs` handles, in the state
/// that row is valid in — `every_key_the_sheet_shows_is_handled_by_the_keymap`
/// presses them all. The copy is the sheet's own; the keys are the keymap's.
pub(crate) const SHORTCUTS: &[Shortcut] = &[
    Shortcut {
        label: "Enter",
        chords: &[ENTER],
        action: "send the message, or queue it behind the running turn",
        group: Group::Conversation,
        when: When::Composer,
    },
    Shortcut {
        label: "Alt+Enter",
        chords: &[ALT_ENTER],
        action: "break the line inside the message",
        group: Group::Conversation,
        when: When::Composer,
    },
    Shortcut {
        label: "Backspace",
        chords: &[BACKSPACE],
        action: "delete the last character",
        group: Group::Conversation,
        when: When::Composer,
    },
    Shortcut {
        label: "Esc",
        chords: &[ESCAPE],
        action: "stop the running turn",
        group: Group::Conversation,
        when: When::Busy,
    },
    Shortcut {
        label: "y, a",
        chords: &[YES, ALSO_YES],
        action: "allow this call once",
        group: Group::Conversation,
        when: When::Card,
    },
    Shortcut {
        label: "s",
        chords: &[SESSION],
        action: "allow calls like it for this session",
        group: Group::Conversation,
        when: When::CardGrant,
    },
    Shortcut {
        label: "n, d",
        chords: &[NO, ALSO_NO],
        action: "deny this call",
        group: Group::Conversation,
        when: When::Card,
    },
    Shortcut {
        label: "Esc",
        chords: &[ESCAPE],
        action: "deny this call",
        group: Group::Conversation,
        when: When::Card,
    },
    Shortcut {
        label: "Shift+Tab",
        chords: &[SHIFT_TAB],
        action: "switch mode — ask, accept-edits, plan, read-only",
        group: Group::Navigation,
        when: When::Composer,
    },
    Shortcut {
        label: "Ctrl+G",
        chords: &[CTRL_G],
        action: "drop the line held behind the running turn",
        group: Group::Navigation,
        when: When::Queued,
    },
    Shortcut {
        label: "Ctrl+K",
        chords: &[CTRL_K],
        action: "open the command palette",
        group: Group::Navigation,
        when: When::Composer,
    },
    Shortcut {
        label: "Ctrl+O",
        chords: &[CTRL_O],
        action: "show every step in full, or fold each back to one line",
        group: Group::Navigation,
        when: When::Composer,
    },
    Shortcut {
        label: "?",
        chords: &[QUESTION],
        action: "open this sheet",
        group: Group::Navigation,
        when: When::Composer,
    },
    Shortcut {
        label: "↑, ↓",
        chords: &[UP, DOWN],
        action: "point at an answer, then Enter gives it",
        group: Group::Navigation,
        when: When::Card,
    },
    Shortcut {
        label: "PgUp, PgDn",
        chords: &[PAGE_UP, PAGE_DOWN],
        action: "page the body, a screen at a time",
        group: Group::Navigation,
        when: When::Card,
    },
    Shortcut {
        label: "v",
        chords: &[VIEW],
        action: "show the whole body, or fold it back",
        group: Group::CopyView,
        when: When::Card,
    },
];
