//! The approval panel on a real frame (UI3): a bordered, accent-titled box
//! directly above the composer, at both review sizes, with colour off and with
//! `--ascii`.
//!
//! The panes are strings; this file proves the frame paints them where an
//! operator looks — and that the composer below says it is disabled.

use ratatui::Terminal;
use ratatui::backend::TestBackend;

use super::model::Model;
use super::run_draw::{Facts, frame};
use super::theme::{Charset, Scheme};
use super::view::cards_fixture::{FOLDER, shell_question, sql_question};
use crate::chat::Question;

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

/// A launch bound to a folder, with a call waiting for an answer.
fn waiting(question: Question) -> (Model, Facts) {
    let mut model = Model::default();
    model.begin_turn(String::from("tighten the pricing tests"));
    model.ask(question);
    (
        model,
        Facts {
            workspace: FOLDER.map(str::to_owned),
            ..Facts::default()
        },
    )
}

/// The first row whose text contains `needle`.
fn row_of(rows: &[String], needle: &str) -> usize {
    rows.iter()
        .position(|row| row.contains(needle))
        .unwrap_or_else(|| panic!("no row says {needle:?} in {rows:#?}"))
}

#[test]
fn the_panel_is_a_full_width_box_docked_above_the_composer() {
    let (model, facts) = waiting(shell_question());
    let rows = painted(&model, &facts, 100, 30);

    let title = row_of(&rows, "Allow this command?");
    let top = &rows[title];
    assert!(
        top.trim_start().starts_with('╭') && top.trim_end().ends_with('╮'),
        "the panel opens with a rule: {top:?}"
    );
    assert_eq!(
        top.chars().count(),
        98,
        "the panel spans the content column: {top:?}"
    );

    let command = row_of(&rows, "$ python3 -m unittest");
    let folder = row_of(&rows, "on your machine, not sandboxed");
    let allow = row_of(&rows, "[y] Allow once");
    let session = row_of(&rows, "[s] Allow for this session");
    let deny = row_of(&rows, "[n] Deny");
    let escape = row_of(&rows, "[Esc] Deny");
    let composer = row_of(&rows, "╭ Message");
    assert!(title < folder, "the title opens the panel");
    assert!(folder < command, "where it runs sits above the command");
    assert!(command < allow, "the answers sit under the command");
    assert!(
        allow < session && session < deny && deny < escape,
        "the choices are on their own lines, in the order they are offered"
    );
    assert!(escape < composer, "the panel is docked above the composer");
    assert!(
        rows[escape + 1].contains("╰") && rows[escape].contains("│"),
        "the panel closes before the composer: {rows:#?}"
    );
}

#[test]
fn twenty_four_rows_keep_every_choice_and_the_whole_command() {
    let (model, facts) = waiting(sql_question());
    let rows = painted(&model, &facts, 60, 24);
    let screen = rows.join("\n");

    for word in [
        "Allow this query?",
        "SELECT COUNT(*) AS orders_since_december",
        "FROM orders",
        "on connection demo",
        "[y] Allow once",
        "[s] Allow for this session · sql:demo",
        "[n] Deny",
        "[Esc] Deny",
    ] {
        assert!(screen.contains(word), "60x24 shows {word:?}: {screen}");
    }
    assert!(
        rows.iter().all(|row| row.chars().count() <= 60),
        "no row passes the edge: {rows:#?}"
    );
    let top = row_of(&rows, "Allow this query?");
    assert_eq!(
        rows[top].chars().count(),
        58,
        "the panel is the content column's width: {:?}",
        rows[top]
    );
}

#[test]
fn the_composer_says_it_is_disabled_under_the_panel() {
    let (model, facts) = waiting(shell_question());
    let rows = painted(&model, &facts, 100, 30);
    let screen = rows.join("\n");

    assert!(
        screen.contains("Message (disabled"),
        "the composer names its state: {screen}"
    );
    assert!(
        screen.contains("Answer the approval above"),
        "and says what to do about it: {screen}"
    );
}

#[test]
fn the_panel_says_the_same_words_with_colour_off() {
    let (model, facts) = waiting(shell_question());
    let dark = painted(&model, &facts, 100, 30);
    let plain = painted(
        &model,
        &Facts {
            scheme: Scheme::NoColour,
            ..facts.clone()
        },
        100,
        30,
    );
    assert_eq!(dark, plain, "02 §12: colour off is the same words");
}

#[test]
fn an_ascii_panel_draws_only_seven_bit_bytes() {
    let (model, facts) = waiting(shell_question());
    let rows = painted(
        &model,
        &Facts {
            charset: Charset::Ascii,
            ..facts
        },
        60,
        24,
    );
    let screen = rows.join("\n");
    assert!(
        screen.bytes().all(|byte| byte.is_ascii()),
        "the ascii panel is seven-bit: {screen}"
    );
    for word in ["Allow this command?", "[y] Allow once", "[Esc] Deny"] {
        assert!(screen.contains(word), "the ascii panel says {word:?}");
    }
}

#[test]
fn a_tall_screen_centres_the_panel_away_from_the_composer() {
    let (model, facts) = waiting(shell_question());
    let rows = painted(&model, &facts, 100, 44);
    let composer = row_of(&rows, "╭ Message");
    let close = rows
        .iter()
        .rposition(|row| row.contains('╰') && row.contains("choose"))
        .unwrap_or_else(|| panic!("the panel's closing rule: {rows:#?}"));

    assert!(
        composer > close + 1,
        "a tall screen leaves room between the panel and the composer: {rows:#?}"
    );
}

#[test]
fn the_focused_choice_is_marked() {
    let (mut model, facts) = waiting(shell_question());
    let rows = painted(&model, &facts, 100, 30);
    assert!(
        !rows.iter().any(|row| row.contains('▸')),
        "no choice is focused until an arrow is pressed: {rows:#?}"
    );

    model.move_choice(1);
    let rows = painted(&model, &facts, 100, 30);
    let marked = row_of(&rows, "▸ [s] Allow for this session");
    assert!(
        marked > row_of(&rows, "[y] Allow once"),
        "the second choice is the focused one: {rows:#?}"
    );
}
