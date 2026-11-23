//! The two seats on a real frame (UI3): the panel that has the keyboard
//! answers where it stands, and the one that does not names the key that
//! gives it over.

use std::time::{Duration, Instant};

use ratatui::Terminal;
use ratatui::backend::TestBackend;

use super::model::Model;
use super::msg::Msg;
use super::run_draw::{Facts, frame};
use super::update::update;
use super::view::cards_fixture::shell_question;
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

/// One call, put to the operator as the session puts one.
fn asked(question: Question) -> Msg {
    Msg::Chat(ChatEvent::Question(question))
}

/// The frame `model` paints at `width × height`, one row per line.
fn painted(model: &Model, facts: &Facts, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("the test backend");
    terminal
        .draw(|area| frame(model, facts, area))
        .expect("the frame is painted");
    format!("{}", terminal.backend())
}

/// A model holding a call that took the keyboard, or one that did not.
fn waiting(focused: bool) -> Model {
    let at = start();
    let mut model = model_at(at);
    if !focused {
        update(&mut model, Msg::Paste(String::from("please ")));
    } else {
        model.at(at + Duration::from_secs(2));
    }
    update(&mut model, asked(shell_question()));
    assert_eq!(model.panel_focused(), focused, "the case's own seat");
    model
}

/// (f) The two seats say which one holds the keyboard, at both review sizes.
#[test]
fn the_unfocused_panel_says_tab_to_answer_and_the_focused_one_does_not() {
    let facts = Facts {
        workspace: Some(String::from("~/shop")),
        ..Facts::default()
    };
    for (width, height) in [(100_u16, 30_u16), (60, 24)] {
        let unfocused = painted(&waiting(false), &facts, width, height);
        assert!(
            unfocused.contains("Tab to answer"),
            "{width}x{height}: the unfocused panel names the key that answers: {unfocused}"
        );
        assert!(
            unfocused.contains("Tab answer the approval"),
            "{width}x{height}: and the composer it left enabled names it too: {unfocused}"
        );
        assert!(
            !unfocused.contains("Message (disabled"),
            "{width}x{height}: a composer that has the keyboard is not disabled: {unfocused}"
        );

        let focused = painted(&waiting(true), &facts, width, height);
        assert!(
            !focused.contains("Tab to answer"),
            "{width}x{height}: the focused panel answers where it stands: {focused}"
        );
        assert!(
            focused.contains("↑↓ choose"),
            "{width}x{height}: and names the arrows it reads: {focused}"
        );
        assert!(
            focused.contains("Message (disabled"),
            "{width}x{height}: the composer gives way: {focused}"
        );
    }
}

/// `v` stays documented in the panel's own bottom rule, in both seats.
#[test]
fn the_panels_bottom_rule_documents_v() {
    let facts = Facts {
        workspace: Some(String::from("~/shop")),
        ..Facts::default()
    };
    for focused in [true, false] {
        let screen = painted(&waiting(focused), &facts, 100, 30);
        assert!(
            screen.contains("v full details"),
            "the rule names `v` whether or not the panel has the keyboard: {screen}"
        );
    }
}
