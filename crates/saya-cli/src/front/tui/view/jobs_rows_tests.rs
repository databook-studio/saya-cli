//! The jobs drawer's pane: `02` §8's box, its rows, its key row, and where it
//! sits — the transcript's right-hand side from 100 columns up, a bottom sheet
//! below that. Every assertion is read back as words, so a reworded row fails.

use super::composer::Row;
use super::jobs::{Drawer, NO_JOBS};
use super::jobs_fixture::drawer;
use super::jobs_rows::{SHEET_ROWS, SIDE_WIDTH, placement, rows, side_width};
use crate::front::tui::theme::Charset;
use crate::interactive::tui::wrap::cell_width;

/// One painted row as the words it shows.
fn text(row: &Row) -> String {
    row.runs.iter().map(|run| run.text.as_str()).collect()
}

/// The pane over the drawer's three jobs, one row per line.
fn painted(drawer: &Drawer, width: usize, height: usize) -> Vec<String> {
    rows(drawer, width, height, Charset::Utf8)
        .iter()
        .map(text)
        .collect()
}

#[test]
fn the_pane_names_itself_and_lists_the_three_states_with_their_facts() {
    let lines = painted(&drawer(), SIDE_WIDTH, SHEET_ROWS);
    assert!(
        lines[0].contains("Background jobs"),
        "02 §8 titles the box: {lines:#?}"
    );
    let body = lines.join("\n");
    assert!(body.contains("● fix-flaky"), "a running job: {body}");
    assert!(
        body.contains("running 3m · 2 helpers"),
        "with its facts: {body}"
    );
    assert!(body.contains("⏸ docs-pass"), "a paused job: {body}");
    assert!(
        body.contains("paused: no progress"),
        "with its reason: {body}"
    );
    assert!(body.contains("✓ bump-deps"), "a finished job: {body}");
    assert!(
        body.contains("finished · 4 files changed"),
        "with its result: {body}"
    );
}

#[test]
fn the_key_row_names_the_four_keys_of_section_eight() {
    let lines = painted(&drawer(), SIDE_WIDTH, SHEET_ROWS);
    let keys = lines.last().expect("the pane closes with its key row");
    assert!(
        keys.contains("↵ open · a approve · x cancel · Esc close"),
        "02 §8's own key row: {lines:#?}"
    );
}

#[test]
fn the_ask_replaces_the_key_row_while_the_confirm_is_open() {
    let mut drawer = drawer();
    drawer.ask_cancel();
    let lines = painted(&drawer, SIDE_WIDTH, SHEET_ROWS);
    let last = lines.last().expect("the pane closes with the ask");
    assert!(last.contains("cancel fix-flaky? y/n"), "{lines:#?}");
    assert!(
        !last.contains("x cancel"),
        "the ask owns the border it replaces: {lines:#?}"
    );
}

#[test]
fn the_ascii_pane_says_the_same_words() {
    // Wider than `02` §8's box, because `[enter]` is seven cells where `↵` is
    // one: the panes say the same words, and the narrow one cuts its key row.
    let lines: Vec<String> = rows(&drawer(), 64, SHEET_ROWS, Charset::Ascii)
        .iter()
        .map(text)
        .collect();
    let body = lines.join("\n");
    assert!(body.contains("* fix-flaky"), "● is *: {body}");
    assert!(body.contains("[||] docs-pass"), "⏸ is [||]: {body}");
    assert!(body.contains("[ok] bump-deps"), "✓ is [ok]: {body}");
    assert!(
        body.contains("running 3m · 2 helpers"),
        "the words do not change: {body}"
    );
    assert!(
        body.contains("[enter] open - a approve - x cancel - Esc close"),
        "{body}"
    );
}

#[test]
fn the_drawer_is_a_side_pane_from_a_hundred_columns_and_a_sheet_below() {
    assert_eq!(
        placement(100),
        super::jobs_rows::Placement::Side,
        "at 100 columns the drawer takes the transcript's side"
    );
    assert_eq!(placement(140), super::jobs_rows::Placement::Side);
    assert_eq!(
        placement(80),
        super::jobs_rows::Placement::Bottom,
        "at 80 it is a bottom sheet"
    );
    assert_eq!(placement(99), super::jobs_rows::Placement::Bottom);
    assert_eq!(side_width(120), SIDE_WIDTH, "the side pane is 02 §8's box");
    assert!(
        side_width(80) < 80,
        "a side pane never takes the whole width"
    );
}

#[test]
fn the_sheet_and_the_side_pane_list_the_same_rows() {
    let side = painted(&drawer(), SIDE_WIDTH, SHEET_ROWS).join("\n");
    let sheet = painted(&drawer(), 80, SHEET_ROWS).join("\n");
    for word in ["● fix-flaky", "⏸ docs-pass", "✓ bump-deps"] {
        assert!(side.contains(word), "the side pane lists {word}: {side}");
        assert!(sheet.contains(word), "the sheet lists {word}: {sheet}");
    }
}

#[test]
fn no_row_is_wider_than_the_width_it_is_given() {
    for width in [40, SIDE_WIDTH, 60, 80] {
        for height in [3, SHEET_ROWS, 10] {
            for row in rows(&drawer(), width, height, Charset::Utf8) {
                let painted = text(&row);
                assert!(
                    cell_width(&painted) <= width,
                    "a {width}-column pane drew {} cells: {painted:?}",
                    cell_width(&painted)
                );
            }
        }
    }
}

#[test]
fn the_pane_is_exactly_its_height() {
    for height in [3, SHEET_ROWS, 10] {
        assert_eq!(
            rows(&drawer(), SIDE_WIDTH, height, Charset::Utf8).len(),
            height,
            "a {height}-row pane is {height} rows"
        );
    }
    assert!(rows(&drawer(), SIDE_WIDTH, 0, Charset::Utf8).is_empty());
}

#[test]
fn an_empty_drawer_says_there_are_no_jobs() {
    let lines = painted(&Drawer::default(), SIDE_WIDTH, SHEET_ROWS);
    assert!(
        lines.join("\n").contains(NO_JOBS),
        "an empty drawer says so: {lines:#?}"
    );
}

#[test]
fn a_list_longer_than_the_pane_scrolls_to_the_selected_row() {
    let mut drawer = drawer();
    drawer.rows.extend((0..8).map(|at| super::jobs::Job {
        id: format!("job-{at}"),
        ..super::jobs::Job::default()
    }));
    for _ in 0..9 {
        drawer.down();
    }
    let lines = painted(&drawer, SIDE_WIDTH, SHEET_ROWS);
    assert!(
        lines.iter().any(|line| line.contains("job-6")),
        "the selected row stays on screen: {lines:#?}"
    );
    assert!(
        !lines.iter().any(|line| line.contains("● fix-flaky")),
        "the rows far above it scroll away: {lines:#?}"
    );
}
