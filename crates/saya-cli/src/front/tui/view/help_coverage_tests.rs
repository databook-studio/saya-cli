//! The key sheet is the whole keyboard: every key the surface answers is
//! listed, filed under one of the six groups, and the sheet scrolls when it is
//! taller than the screen.

use super::help::{self, Context};
use super::help_fixture::{idle, words};
use crate::interactive::tui::wrap::cell_width;

/// The sheet's inner rows, without its border cells.
fn inside(context: Context, width: usize, height: usize, scroll: usize) -> Vec<String> {
    help::rows_scrolled(context, width, height, scroll)
        .iter()
        .map(|row| {
            let text = row.text();
            let inner: String = text.chars().skip(2).collect();
            let kept = cell_width(&inner).saturating_sub(2);
            inner
                .chars()
                .take(kept)
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect()
}

/// Every group heading the sheet can print.
const HEADINGS: [&str; 6] = [
    "Conversation",
    "History",
    "Steps",
    "Approvals",
    "Jobs",
    "Navigation",
];

#[test]
fn the_sheet_lists_every_group_it_names() {
    let rows = inside(Context::default(), 100, 60, 0);

    for heading in HEADINGS {
        assert!(
            rows.iter().any(|row| row == heading),
            "the sheet files keys under {heading:?}: {rows:#?}"
        );
    }
}

#[test]
fn a_group_heading_is_never_left_without_a_row() {
    let rows = inside(Context::default(), 100, 60, 0);

    for (index, row) in rows.iter().enumerate() {
        if !HEADINGS.contains(&row.as_str()) {
            continue;
        }
        let next = rows.get(index + 1).map(String::as_str).unwrap_or_default();
        assert!(
            !next.is_empty(),
            "{row:?} has no key row under it: {rows:#?}"
        );
        assert!(
            !HEADINGS.contains(&next),
            "{row:?} is followed by another heading, not a key: {rows:#?}"
        );
    }
}

#[test]
fn every_key_the_surface_answers_is_listed() {
    let rows = inside(Context::default(), 120, 70, 0).join("\n");

    for key in [
        "Enter",
        "Alt+Enter",
        "Shift+Tab",
        "Ctrl+K",
        "Ctrl+O",
        "Ctrl+R",
        "Ctrl+J",
        "Ctrl+G",
        "?",
        "Esc",
        "y, a",
        "s",
        "n, d",
        "v",
        "PgUp, PgDn",
        "x",
    ] {
        assert!(rows.contains(key), "the sheet lists {key:?}: {rows}");
    }
}

#[test]
fn the_approvals_group_lists_every_key_the_panel_answers() {
    let sheet = words(idle(), 120, 70).join("\n");

    for key in ["Tab", "↑, ↓", "Enter", "y, a", "s", "n, d", "Esc", "v"] {
        assert!(
            sheet.contains(key),
            "the sheet lists the panel's {key:?}: {sheet}"
        );
    }
    for words in [
        "give the keyboard to the panel, or back",
        "choose an answer",
        "give the answer the arrows point at",
        "open the full details, or fold them back",
    ] {
        assert!(
            sheet.contains(words),
            "the approvals group says what the panel's keys do ({words:?}): {sheet}"
        );
    }
}

#[test]
fn a_sheet_taller_than_the_screen_scrolls() {
    let top = inside(Context::default(), 100, 12, 0);
    let scrolled = inside(Context::default(), 100, 12, 8);

    assert_eq!(top.len(), 12, "the sheet is the height it was given");
    assert_eq!(scrolled.len(), 12, "a scrolled sheet is the same height");
    assert!(
        !scrolled.contains(&"Conversation".to_owned()),
        "the first group has scrolled away: {scrolled:#?}"
    );
    assert!(
        scrolled
            .iter()
            .any(|row| !top.contains(row) && !row.is_empty()),
        "scrolling shows rows the top of the sheet did not: {scrolled:#?}"
    );
}
