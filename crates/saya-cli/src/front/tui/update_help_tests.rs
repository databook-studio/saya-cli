//! `02` §11's key sheet through the pure update: what opens it (`?` on an empty
//! composer, and bare `/help` instead of the catalogue printed into the
//! transcript), what a draft does with the same character, and what closes it.

use ratatui::crossterm::event::KeyCode;

use super::model::Model;
use super::model_overlay::Overlay;
use super::msg::Cmd;
use super::update::update;
use super::update_fixture::{enter, escape, press};
use crate::slash::SlashCommand;

#[test]
fn a_question_mark_on_an_empty_composer_opens_the_help_sheet() {
    let mut model = Model::default();
    let cmds = update(&mut model, press(KeyCode::Char('?')));

    assert!(cmds.is_empty(), "the sheet asks for nothing: {cmds:?}");
    assert_eq!(
        model.overlay(),
        Some(&Overlay::Help),
        "`?` opens the key sheet"
    );
    assert_eq!(model.composer(), "", "and types nothing");
}

#[test]
fn a_question_mark_on_a_draft_is_typed_into_it() {
    let mut model = Model::default();
    model.insert("how many rows");

    let cmds = update(&mut model, press(KeyCode::Char('?')));

    assert!(cmds.is_empty(), "typing sends nothing: {cmds:?}");
    assert!(
        !model.overlay_open(),
        "a draft is not a place to open a pane"
    );
    assert_eq!(model.composer(), "how many rows?");
}

#[test]
fn esc_closes_the_help_sheet() {
    let mut model = Model::default();
    update(&mut model, press(KeyCode::Char('?')));

    let cmds = update(&mut model, escape());

    assert!(cmds.is_empty(), "Esc asks for nothing: {cmds:?}");
    assert!(!model.overlay_open(), "the sheet is closed");
}

#[test]
fn slash_help_opens_the_sheet_instead_of_a_transcript_notice() {
    let mut model = Model::default();
    model.insert("/help");

    let cmds = update(&mut model, enter());

    assert!(cmds.is_empty(), "`/help` is the surface's: {cmds:?}");
    assert_eq!(model.overlay(), Some(&Overlay::Help), "the sheet opens");
    assert!(
        model.turns().is_empty(),
        "and the catalogue is not printed into the transcript"
    );
    assert_eq!(model.composer(), "", "the line is taken");
}

#[test]
fn help_with_a_topic_is_still_the_sessions_to_answer() {
    let mut model = Model::default();
    model.insert("/help status");

    let cmds = update(&mut model, enter());

    assert!(
        !model.overlay_open(),
        "only bare `/help` opens the sheet: a topic is the session's"
    );
    assert_eq!(
        cmds,
        vec![Cmd::Op(SlashCommand::Help(Some(String::from("status"))))]
    );
}
