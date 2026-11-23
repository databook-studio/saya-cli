//! The sheet's first half: conversation, history and steps.

use ratatui::crossterm::event::KeyCode;

use super::help_keys::*;

/// The rows, in the order the sheet reads them.
pub(super) const ROWS: &[Shortcut] = &[
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
        label: "Ctrl+G",
        chords: &[CTRL_G],
        action: "drop the line held behind the running turn",
        group: Group::Conversation,
        when: When::Queued,
    },
    Shortcut {
        label: "↑, ↓",
        chords: &[UP, DOWN],
        action: "recall the previous or next line",
        group: Group::History,
        when: When::Composer,
    },
    Shortcut {
        label: "Ctrl+R",
        chords: &[CTRL_R],
        action: "search the input history",
        group: Group::History,
        when: When::Composer,
    },
    Shortcut {
        label: "type",
        chords: &[Chord {
            code: KeyCode::Char('a'),
            modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
        }],
        action: "narrow the search — while it is open",
        group: Group::History,
        when: When::Search,
    },
    Shortcut {
        label: "Ctrl+R",
        chords: &[CTRL_R],
        action: "step to an older match — while searching",
        group: Group::History,
        when: When::Search,
    },
    Shortcut {
        label: "Backspace",
        chords: &[BACKSPACE],
        action: "widen the search — while searching",
        group: Group::History,
        when: When::Search,
    },
    Shortcut {
        label: "Enter",
        chords: &[ENTER],
        action: "take the match onto the line — while searching",
        group: Group::History,
        when: When::Search,
    },
    Shortcut {
        label: "Esc",
        chords: &[ESCAPE],
        action: "close the search, draft untouched",
        group: Group::History,
        when: When::Search,
    },
    Shortcut {
        label: "Ctrl+O",
        chords: &[CTRL_O],
        action: "show every step in full, or fold each back to one line",
        group: Group::Steps,
        when: When::Composer,
    },
    Shortcut {
        label: "Ctrl+D",
        chords: &[CTRL_D],
        action: "open the diff of the files the last turn changed — once a turn has written",
        group: Group::Steps,
        when: When::Changed,
    },
    Shortcut {
        label: "Alt+↑, Alt+↓",
        chords: &[ALT_UP, ALT_DOWN],
        action: "walk the block cursor through the transcript",
        group: Group::Steps,
        when: When::Composer,
    },
    Shortcut {
        label: "Enter, →",
        chords: &[ENTER, RIGHT],
        action: "expand the block the cursor holds",
        group: Group::Steps,
        when: When::Cursor,
    },
    Shortcut {
        label: "←",
        chords: &[LEFT],
        action: "fold the block the cursor holds",
        group: Group::Steps,
        when: When::Cursor,
    },
    Shortcut {
        label: "c",
        chords: &[COPY],
        action: "copy the focused block's source",
        group: Group::Steps,
        when: When::Cursor,
    },
    Shortcut {
        label: "i",
        chords: &[INSPECT],
        action: "show the focused prompt's provenance",
        group: Group::Steps,
        when: When::Cursor,
    },
    Shortcut {
        label: "Esc",
        chords: &[ESCAPE],
        action: "leave the block cursor",
        group: Group::Steps,
        when: When::Cursor,
    },
];
