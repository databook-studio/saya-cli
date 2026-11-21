//! The composed frame: the panes T4-T13 in `02` §1's grid, in the order an
//! operator reads them.
//!
//! The frame is painted through the real panes onto ratatui's test backend, so
//! what these tests read is the screen, not a description of one.

use ratatui::Terminal;
use ratatui::backend::TestBackend;

use super::model::Model;
use super::run_draw::{Facts, frame};
use crate::chat::Question;

/// The screen `model` paints at `width × height`, one row per line.
fn painted(model: &Model, facts: &Facts, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("the test backend");
    terminal
        .draw(|area| frame(model, facts, area))
        .expect("the frame is painted");
    let screen = format!("{}", terminal.backend());
    screen
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

/// A turn already on screen: the operator's question and the model's answer.
fn conversation() -> Model {
    let mut model = Model::default();
    model.begin_turn(String::from("how many orders did we get?"));
    model.insert("a follow-up");
    model
}

#[test]
fn the_frame_stacks_the_transcript_the_status_line_the_card_and_the_composer() {
    let mut model = conversation();
    model.show_card(Question {
        id: 1,
        caller: String::from("the shell tool"),
        facts: String::from("cargo test"),
        grant: None,
    });
    let rows = painted(&model, &Facts::default(), 100, 30);
    let question = row_of(&rows, "how many orders did we get?");
    let status = row_of(&rows, "help");
    let card = row_of(&rows, "Approve? · the shell tool");
    let body = row_of(&rows, "cargo test");
    let composer = row_of(&rows, "╭ Message");
    assert!(question < status, "the transcript is above the status line");
    assert!(status < card, "the status line is above the docked card");
    assert!(card < body, "the card's body sits under its title");
    assert!(body < composer, "the card is docked above the composer");
    assert_eq!(
        composer,
        rows.len() - 3,
        "the box's three rows close the screen"
    );
    assert!(
        rows.iter().any(|row| row.contains("y allow once")),
        "the card says how it is answered"
    );
}

#[test]
fn an_empty_transcript_paints_the_empty_state_for_the_launch_facts() {
    let rows = painted(
        &Model::default(),
        &Facts {
            workspace: Some(String::from("~/shop")),
            ..Facts::default()
        },
        100,
        30,
    );
    assert!(rows.iter().any(|row| row.contains("~/shop")), "{rows:#?}");
    assert!(
        rows.iter().any(|row| row.contains("no database")),
        "with nothing connected the empty state says so"
    );
}

#[test]
fn the_composer_shows_the_draft_that_enter_would_send() {
    let rows = painted(&conversation(), &Facts::default(), 100, 30);
    assert!(
        rows.iter().any(|row| row.contains("a follow-up")),
        "the draft is what the box shows: {rows:#?}"
    );
    assert!(
        rows.iter().any(|row| row.contains("Enter send")),
        "and the box names the keys"
    );
}

#[test]
fn a_short_screen_still_keeps_the_composer_and_the_status_line() {
    let rows = painted(&conversation(), &Facts::default(), 80, 12);
    assert_eq!(rows.len(), 12);
    assert!(
        rows.iter().any(|row| row.contains("╭ Message")),
        "{rows:#?}"
    );
    assert!(rows.iter().any(|row| row.contains("help")));
}

#[test]
fn no_row_is_painted_past_the_terminal_edge() {
    let rows = painted(&conversation(), &Facts::default(), 60, 20);
    assert!(
        rows.iter().all(|row| row.chars().count() <= 60),
        "{rows:#?}"
    );
}
