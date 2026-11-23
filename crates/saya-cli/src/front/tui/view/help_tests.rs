//! The help sheet (`02` §11), read as the keys it lists and the words it paints.
//!
//! The sheet is the whole keyboard: every group is listed in every state, and
//! the last test is the guard that keeps the sheet and the keymap one keyboard —
//! every press the sheet lists is pressed through `keys.rs` in the state the row
//! names, so a row nothing handles fails.

use ratatui::crossterm::event::KeyCode;

use super::help::{self, Context};
use super::help_fixture::{idle, model_in, plain, words};
use super::help_keys::{Group, When, shortcuts};
use crate::front::tui::keys::{Intent, intent};
use crate::interactive::tui::wrap::cell_width;

#[test]
fn the_sheet_lists_every_group_in_every_state() {
    let sheet = words(idle(), 100, 60).join("\n");
    for heading in [
        "Conversation",
        "History",
        "Steps",
        "Approvals",
        "Jobs",
        "Navigation",
    ] {
        assert!(
            sheet.contains(heading),
            "the sheet lists {heading:?} whatever the state: {sheet}"
        );
    }
}

#[test]
fn the_sheet_says_which_state_a_key_acts_in() {
    let sheet = words(idle(), 120, 60).join("\n");
    for words in [
        "while a card is open",
        "while it is open",
        "while searching",
        "while the drawer asks",
    ] {
        assert!(
            sheet.contains(words),
            "a key that only acts in one state says so ({words:?}): {sheet}"
        );
    }
}

#[test]
fn no_row_is_wider_than_the_sheet() {
    for (width, height) in [(100, 30), (80, 24), (60, 12), (24, 8), (24, 2), (24, 1)] {
        let rows = help::rows(Context::default(), width, height);
        assert_eq!(
            rows.len(),
            height,
            "at {width}x{height} the sheet is exactly its region"
        );
        for row in &rows {
            assert!(
                cell_width(&row.text()) <= width.max(12),
                "{:?} is wider than {width}: {}",
                row.text(),
                cell_width(&row.text())
            );
        }
    }
}

#[test]
fn the_body_scrolls_rather_than_spilling() {
    let top = words(Context::default(), 100, 12);
    let down = help::rows_scrolled(Context::default(), 100, 12, 6);
    let down: Vec<String> = down
        .iter()
        .map(|row| {
            let text = row.text();
            let inner: String = text.chars().skip(2).collect();
            let kept = inner.chars().count().saturating_sub(2);
            let mut trimmed: String = inner.chars().take(kept).collect();
            trimmed.truncate(trimmed.trim_end().len());
            trimmed
        })
        .collect();

    assert_eq!(down.len(), 12, "a scrolled sheet keeps its height");
    assert!(
        !down.contains(&String::from("Conversation")),
        "the first group has scrolled off: {down:?}"
    );
    assert!(
        down.iter().any(|row| !top.contains(row) && !row.is_empty()),
        "scrolling shows rows the top did not: {down:?}"
    );
}

#[test]
fn every_key_the_sheet_shows_is_handled_by_the_keymap() {
    let mut pressed = 0;
    for shortcut in shortcuts() {
        assert!(
            !shortcut.chords.is_empty(),
            "`{}` names no press, so nothing can check it",
            shortcut.label
        );
        let model = model_in(shortcut.when);
        for chord in shortcut.chords {
            match intent(&model, chord.event()) {
                Some(Intent::Refused(reason)) => panic!(
                    "the sheet lists `{}` ({:?}), and the keymap refuses it in {:?}: {reason}",
                    shortcut.label, chord.code, shortcut.when
                ),
                Some(_) => pressed += 1,
                None => panic!(
                    "the sheet lists `{}` ({:?}), and the keymap handles no such press in {:?}",
                    shortcut.label, chord.code, shortcut.when
                ),
            }
        }
    }
    assert!(
        pressed >= shortcuts().count(),
        "every shortcut was pressed through the keymap: {pressed} presses"
    );
    for when in [
        When::Card,
        When::Search,
        When::Cursor,
        When::Jobs,
        When::JobsConfirm,
    ] {
        assert!(
            shortcuts().any(|shortcut| shortcut.when == when),
            "the table carries the {when:?} state's keys"
        );
    }
    assert!(
        shortcuts().any(|shortcut| shortcut.chords.contains(&plain(KeyCode::Char('?')))),
        "the sheet lists the key that opens it"
    );
}

#[test]
fn the_approvals_rows_are_answered_by_the_panel_seat() {
    let mut pressed = 0;
    for shortcut in shortcuts().filter(|shortcut| shortcut.group == Group::Approvals) {
        assert!(
            matches!(shortcut.when, When::Card | When::CardGrant),
            "`{}` is filed under approvals, so a card is the state it acts in",
            shortcut.label
        );
        let model = model_in(shortcut.when);
        for chord in shortcut.chords {
            match intent(&model, chord.event()) {
                Some(
                    Intent::ToggleSeat
                    | Intent::AnswerFocused
                    | Intent::AnswerCard(_)
                    | Intent::ViewCard
                    | Intent::FocusChoice(_)
                    | Intent::ScrollCard(_),
                ) => pressed += 1,
                other => panic!(
                    "the panel answers `{}` ({:?}) as its own: got {other:?}",
                    shortcut.label, chord.code
                ),
            }
        }
    }
    assert!(
        pressed >= 10,
        "the panel's own keys were pressed through the keymap: {pressed}"
    );
}
