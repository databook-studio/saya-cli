//! Which seat the keyboard is in while an approval waits (UI3): the panel
//! answers only when it has the focus, and typing is never an answer.
//!
//! The rule these cases pin: a call can open while the operator is typing, so
//! the panel takes the keyboard only when the composer is empty and no key was
//! typed in the last 1.5 s; otherwise the composer keeps it, `Tab` moves it, and
//! a letter the panel has no answer for never lands in the draft.
//!
//! The clock is the tests' own: every case sets the reading the surface would
//! have taken from its injected clock, so no case sleeps.

use std::time::{Duration, Instant};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::model::Model;
use super::msg::{Cmd, Msg};
use super::update::update;
use super::view::cards_fixture::shell_question;
use crate::agent::tools::consent::Answer;
use crate::chat::{ChatEvent, Question};

/// The reading a case starts from.
fn start() -> Instant {
    Instant::now()
}

/// A model whose injected clock read `at`.
fn model_at(at: Instant) -> Model {
    let mut model = Model::default();
    model.at(at);
    model
}

/// One character, as the input thread hands it over.
fn typed(character: char) -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE))
}

/// One key that types nothing.
fn key(code: KeyCode) -> Msg {
    Msg::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

/// One call, put to the operator as the session puts one.
fn asked(question: Question) -> Msg {
    Msg::Chat(ChatEvent::Question(question))
}

/// Types `text` one press at a time, and answers every command it earned.
fn type_text(model: &mut Model, text: &str) -> Vec<Cmd> {
    text.chars()
        .flat_map(|character| update(model, typed(character)))
        .collect()
}

/// The answers the typing reached the runtime as.
fn answered(cmds: &[Cmd]) -> Vec<&Cmd> {
    cmds.iter()
        .filter(|cmd| matches!(cmd, Cmd::Answer(..)))
        .collect()
}

/// (a) A card that opens over a draft leaves the keyboard where the operator's
/// hands are: every letter typed after it is the composer's.
#[test]
fn a_card_that_opens_over_a_draft_never_answers_the_typing() {
    let at = start();
    let mut model = model_at(at);
    update(&mut model, Msg::Paste(String::from("please ")));
    let opener = update(&mut model, asked(shell_question()));
    assert_eq!(opener, vec![Cmd::Bell], "the call takes the card, once");
    assert!(model.card().is_some(), "the card waits for an answer");
    assert!(
        !model.panel_focused(),
        "a draft is on the line, so the composer keeps the keyboard"
    );

    let cmds = type_text(&mut model, "yes add sand");

    assert!(
        answered(&cmds).is_empty(),
        "no letter answers the call: {cmds:?}"
    );
    assert_eq!(
        model.card().map(|card| card.id),
        Some(8),
        "the call is still waiting"
    );
    assert_eq!(
        model.composer(),
        "please yes add sand",
        "every letter reached the composer, `y` and `a` included"
    );
}

/// (b) An empty composer is not enough: a key pressed half a second ago means
/// the operator's hands are on the line, and the panel waits to be focused.
#[test]
fn a_card_that_opens_just_after_a_key_leaves_the_keyboard_in_the_composer() {
    let at = start();
    let mut model = model_at(at);
    // A press that types nothing, so the composer stays empty and only the
    // clock can tell the two seats apart.
    update(&mut model, key(KeyCode::Backspace));
    model.at(at + Duration::from_millis(500));
    update(&mut model, asked(shell_question()));
    assert!(
        !model.panel_focused(),
        "a key 0.5 s ago is typing, so the panel does not take the keyboard"
    );

    let cmds = type_text(&mut model, "dn");

    assert!(
        answered(&cmds).is_empty(),
        "`d` and `n` are letters here, not a denial: {cmds:?}"
    );
    assert_eq!(model.card().map(|card| card.id), Some(8), "still waiting");
    assert_eq!(model.composer(), "dn", "the letters are the draft's");
}

/// (c) An empty composer and two seconds of quiet: the panel takes the
/// keyboard, and `y` means what it always meant.
#[test]
fn an_idle_empty_composer_gives_the_panel_the_keyboard() {
    let at = start();
    let mut model = model_at(at);
    update(&mut model, key(KeyCode::Backspace));
    model.at(at + Duration::from_secs(2));
    update(&mut model, asked(shell_question()));
    assert!(model.panel_focused(), "the panel takes the keyboard");

    let cmds = update(&mut model, typed('y'));

    assert_eq!(
        cmds,
        vec![Cmd::Answer(8, Answer::Once)],
        "`y` allows the call once, exactly as today"
    );
    assert!(model.card().is_none(), "the answer closed the card");
    assert_eq!(model.composer(), "", "and nothing leaked into the draft");
}

/// (d) `Tab` moves the keyboard: to the panel, where `n` denies, and back to
/// the composer, where `n` is a letter.
#[test]
fn tab_moves_the_keyboard_between_the_panel_and_the_composer() {
    let at = start();
    let mut model = model_at(at);
    model.at(at + Duration::from_secs(2));
    update(&mut model, asked(shell_question()));
    assert!(
        model.panel_focused(),
        "a quiet, empty line gives the panel it"
    );

    update(&mut model, key(KeyCode::Tab));
    assert!(!model.panel_focused(), "`Tab` puts the keyboard back");
    let cmds = update(&mut model, typed('n'));
    assert_eq!(
        model.composer(),
        "n",
        "`n` is a letter in the draft: {cmds:?}"
    );
    assert!(model.card().is_some(), "and the call still waits");

    update(&mut model, key(KeyCode::Tab));
    assert!(model.panel_focused(), "`Tab` again takes it back");
    let cmds = update(&mut model, typed('n'));
    assert_eq!(
        cmds,
        vec![Cmd::Answer(8, Answer::Deny)],
        "here `n` denies the call"
    );
    assert!(model.card().is_none(), "and the card is answered");
}

/// (e) A key the panel has no answer for does nothing at all: no letter
/// reaches the draft the panel has disabled.
#[test]
fn a_key_the_panel_cannot_answer_never_lands_in_the_draft() {
    let at = start();
    let mut model = model_at(at);
    model.at(at + Duration::from_secs(2));
    update(&mut model, asked(shell_question()));
    assert!(model.panel_focused(), "the panel holds the keyboard");

    let cmds = type_text(&mut model, "xq");

    assert!(cmds.is_empty(), "nothing is asked of the runtime: {cmds:?}");
    assert_eq!(model.composer(), "", "no letter leaks into the draft");
    assert_eq!(
        model.card().map(|card| card.id),
        Some(8),
        "and the call waits for an answer that was not given"
    );
}
