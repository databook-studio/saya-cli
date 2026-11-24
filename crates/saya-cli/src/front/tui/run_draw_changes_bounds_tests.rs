//! The changes pane's bounds (UI5), on a real frame: a change too large to diff
//! and one that is not text are each one size row, the line above the composer
//! still counts what the turn wrote, and neither costs the pane more rows than
//! the frame gives it.

use ratatui::Terminal;
use ratatui::backend::TestBackend;

use super::changes_fixture::{edited, written};
use super::model::Model;
use super::run_draw::{Facts, frame};
use crate::front::view::{Activity, Entry, Step};

/// The screen `model` paints at `width × height`, one row per line.
fn painted(model: &Model, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("the test backend");
    terminal
        .draw(|area| frame(model, &Facts::default(), area))
        .expect("the frame is painted");
    format!("{}", terminal.backend())
        .lines()
        .map(|row| row.trim_matches('"').to_owned())
        .collect()
}

/// The first row whose text contains `needle`.
fn row_of(rows: &[String], needle: &str) -> usize {
    rows.iter()
        .position(|row| row.contains(needle))
        .unwrap_or_else(|| panic!("no row says {needle:?} in {rows:#?}"))
}

/// A model whose only turn ran `steps`.
fn model_of(steps: Vec<Step>) -> Model {
    let mut model = Model::default();
    model.begin_turn(String::from("rework the export"));
    let turn = model.transcript.current();
    turn.push(Entry::Activity(Activity {
        steps,
        ended: true,
        ..Activity::default()
    }));
    model
}

#[test]
fn a_change_too_large_to_diff_is_one_size_row_with_its_counts() {
    let huge = "x".repeat(100_000);
    let mut model = model_of(vec![written("data/dump.csv", &huge)]);
    let rows = painted(&model, 100, 30);
    let changed = row_of(&rows, "Changed: data/dump.csv");
    let composer = row_of(&rows, "╭ Message");
    assert!(
        changed < composer,
        "the line above the composer counts the whole write: {rows:#?}"
    );

    model.open_changes();
    let pane = painted(&model, 100, 30);
    let size = row_of(&pane, "too large to show a diff");
    assert!(
        pane[size].contains("change: 100000 bytes"),
        "and the pane states the size it withheld: {:#?}",
        pane[size]
    );
    assert!(
        !pane.iter().any(|row| row.contains("xxx")),
        "with not one byte of the file painted: {pane:#?}"
    );
}

#[test]
fn a_binary_change_is_one_size_row_and_no_diff_rows() {
    let mut model = model_of(vec![edited("assets/logo.png", "a\u{0}b", "c\u{0}d")]);
    let rows = painted(&model, 100, 30);
    model.open_changes();
    let pane = painted(&model, 100, 30);

    assert!(
        rows.iter()
            .any(|row| row.contains("Changed: assets/logo.png")),
        "the turn still says it changed the file: {rows:#?}"
    );
    let size = row_of(&pane, "no diff shown");
    assert!(
        pane[size].contains("change: binary"),
        "the pane states it as binary rather than painting it: {:#?}",
        pane[size]
    );
    assert!(
        !pane.iter().any(|row| row.contains("\u{0}")),
        "and not one byte of it reaches a cell: {pane:#?}"
    );
}

#[test]
fn the_pane_keeps_to_the_rows_the_frame_gives_it() {
    let before = (0..30)
        .map(|line| format!("old {line}\n"))
        .collect::<String>();
    let after = (0..30)
        .map(|line| format!("new {line}\n"))
        .collect::<String>();
    let mut model = model_of(vec![edited("src/big.py", &before, &after)]);
    model.open_changes();

    let rows = painted(&model, 100, 30);
    assert_eq!(rows.len(), 30, "the pane is exactly its frame: {rows:#?}");

    model.scroll_changes(200);
    let end = painted(&model, 100, 30);
    assert_eq!(end.len(), 30, "and it stays its frame scrolled to the end");
    assert!(
        end.iter().any(|row| row.contains("+21 more lines")),
        "what the pane cannot show is counted in the pane's own row: {end:#?}"
    );
}
