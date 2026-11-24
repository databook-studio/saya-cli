//! The two surfaces UI5 adds, on a real frame: the `Changed:` line a turn closes
//! with, and the scrollable pane behind it — at both review sizes, with colour
//! off and with `--ascii`.

use ratatui::Terminal;
use ratatui::backend::TestBackend;

use super::changes_fixture::{accept_edits_model, edited, model_with_edits, read_step};
use super::model::Model;
use super::run_draw::{Facts, frame};
use super::theme::{Charset, Scheme};

/// The screen `model` paints at `width × height`, one row per line.
fn painted(model: &Model, facts: &Facts, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("the test backend");
    terminal
        .draw(|area| frame(model, facts, area))
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

/// The facts of a launch with the default scheme and glyph set.
fn facts() -> Facts {
    Facts::default()
}

#[test]
fn a_turn_that_changed_two_files_states_the_changed_line() {
    let model = model_with_edits();
    let rows = painted(&model, &facts(), 100, 30);

    let changed = row_of(&rows, "Changed:");
    assert_eq!(
        rows[changed].trim(),
        "Changed: src/calc.py (+1 −1), notes/todo.md (+3 −0) · Ctrl+D diff",
        "one line names every file, both counts and the key"
    );
    assert!(
        rows[changed].chars().count() <= 100,
        "and it fits the screen it is painted on"
    );
}

#[test]
fn the_changed_line_closes_the_turn_it_belongs_to() {
    let model = model_with_edits();
    let rows = painted(&model, &facts(), 100, 30);

    let message = row_of(&rows, "tighten the pricing tests");
    let step = row_of(&rows, "src/calc.py");
    let changed = row_of(&rows, "Changed:");
    let composer = row_of(&rows, "╭ Message");

    assert!(
        message < step && step < changed,
        "the line closes the turn it belongs to: {rows:#?}"
    );
    assert!(
        changed < composer,
        "and sits above the composer, not inside it"
    );
}

#[test]
fn a_turn_that_changed_no_file_states_no_changed_line() {
    let mut model = Model::default();
    model.begin_turn(String::from("how many orders did we get?"));
    let turn = model.transcript.current();
    turn.push(crate::front::Entry::Activity(
        crate::front::view::Activity {
            steps: vec![read_step()],
            ended: true,
            ..Default::default()
        },
    ));

    let rows = painted(&model, &facts(), 100, 30);
    assert!(
        !rows.iter().any(|row| row.contains("Changed:")),
        "a turn that only read files claims no change: {rows:#?}"
    );
}

#[test]
fn accept_edits_gets_the_changed_line_with_no_card() {
    let model = accept_edits_model();
    let rows = painted(&model, &facts(), 100, 30);

    assert!(
        model.card().is_none(),
        "accept-edits puts no call to the operator"
    );
    assert!(
        rows.iter().any(|row| row.contains("Changed: src/calc.py")),
        "and the turn still says what it changed: {rows:#?}"
    );
}

#[test]
fn the_changes_pane_shows_both_files_diffs() {
    let mut model = model_with_edits();
    model.open_changes();
    let rows = painted(&model, &facts(), 100, 30);

    let title = row_of(&rows, "╭ Changed");
    let first = row_of(&rows, "src/calc.py");
    let minus = row_of(&rows, "-return total - discount - discount");
    let plus = row_of(&rows, "+return total - discount");
    let second = row_of(&rows, "notes/todo.md");
    let keys = row_of(&rows, "Esc close");

    assert!(title < first, "the pane opens above its rows");
    assert!(
        first < minus && minus < plus,
        "the first file reads top down"
    );
    assert!(plus < second, "then the second file, under the first");
    assert!(second < keys, "and the key row closes the pane");
    assert!(
        rows.iter().any(|row| row.contains("[+3 −0]")),
        "each file states its counts: {rows:#?}"
    );
}

#[test]
fn the_changes_pane_scrolls_down_the_whole_diff() {
    let mut model = model_with_edits();
    model.open_changes();
    let top = painted(&model, &facts(), 100, 30);

    let mut scrolled = model_with_edits();
    scrolled.open_changes();
    scrolled.scroll_changes(8);
    let rows = painted(&scrolled, &facts(), 100, 30);

    assert_ne!(
        top, rows,
        "a scrolled pane paints different rows: {rows:#?}"
    );
    assert!(
        rows.iter().any(|row| row.contains("+one")),
        "the last rows of the second file come into view: {rows:#?}"
    );
}

#[test]
fn the_changes_pane_at_sixty_columns_keeps_its_words() {
    let mut model = model_with_edits();
    model.open_changes();
    let rows = painted(&model, &facts(), 60, 24);

    assert!(
        rows.iter().all(|row| row.chars().count() <= 60),
        "no row is painted past the edge: {rows:#?}"
    );
    assert!(
        rows.iter().any(|row| row.contains("Changed")),
        "the pane is still named: {rows:#?}"
    );
}

#[test]
fn colour_off_paints_the_same_words() {
    for open in [false, true] {
        let mut model = model_with_edits();
        if open {
            model.open_changes();
        }
        let dark = painted(&model, &facts(), 100, 30);
        let plain = painted(
            &model,
            &Facts {
                scheme: Scheme::NoColour,
                ..Facts::default()
            },
            100,
            30,
        );
        assert_eq!(dark, plain, "02 §12: the same words, colour or not");
    }
}

#[test]
fn ascii_paints_the_same_words() {
    let mut model = model_with_edits();
    model.open_changes();
    let rows = painted(
        &model,
        &Facts {
            charset: Charset::Ascii,
            ..Facts::default()
        },
        100,
        30,
    );

    assert!(
        rows.iter().all(|row| row.is_ascii()),
        "--ascii never paints a glyph a terminal cannot draw: {rows:#?}"
    );
    assert!(
        rows.iter().any(|row| row.contains("Changed")),
        "the pane is still named: {rows:#?}"
    );
}

/// One edited step whose change is long enough to be worth scrolling.
fn long_model() -> Model {
    let mut model = Model::default();
    model.begin_turn(String::from("tighten the pricing tests"));
    let turn = model.transcript.current();
    turn.push(crate::front::Entry::Activity(
        crate::front::view::Activity {
            steps: vec![edited("src/calc.py", "one", "ONE")],
            ended: true,
            ..Default::default()
        },
    ));
    model
}

#[test]
fn a_one_line_change_still_paints_a_changed_line() {
    let rows = painted(&long_model(), &facts(), 100, 30);
    assert!(
        rows.iter()
            .any(|row| row.contains("Changed: src/calc.py (+1 −1)")),
        "one file and both counts: {rows:#?}"
    );
}
