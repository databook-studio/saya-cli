//! The card's keys, asserted as the answers they send: `y`/`a` once, `s` the
//! session grant the card showed, `n`/`d`/Esc deny, Enter nothing at all — and
//! the plan card's own two answers.
//!
//! The last tests drive the press through the model's update, because a key
//! that reads right and answers nothing would still be a broken card: what the
//! operator presses has to reach the runtime as one `Cmd::Answer`.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::cards_keys::{CardIntent, card_intent};
use super::model::Model;
use super::msg::{Cmd, Msg};
use super::update::update;
use super::view::cards::approval;
use super::view::cards_fixture::{
    ASK, SCREEN_ROWS, helper_question, plan_question, shell_question, ungranted_question,
};
use crate::agent::tools::consent::Answer;
use crate::chat::{ChatEvent, Question};

/// One press, as the input thread hands it over.
fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// The press as a character.
fn typed(character: char) -> KeyEvent {
    key(KeyCode::Char(character))
}

#[test]
fn y_and_a_allow_the_call_once() {
    let question = shell_question();

    for character in ['y', 'a', 'Y', 'A'] {
        assert_eq!(
            card_intent(&question, typed(character)),
            Some(CardIntent::Answer(Answer::Once)),
            "`{character}` allows this call once, as it does today"
        );
    }
}

#[test]
fn s_grants_the_token_the_card_showed_and_is_nothing_when_none_was_offered() {
    let question = shell_question();
    let card = approval(&question, ASK, SCREEN_ROWS);
    assert!(
        card.keys().contains("shell:python3"),
        "the card states the token `s` would record: {}",
        card.keys()
    );

    assert_eq!(
        card_intent(&question, typed('s')),
        Some(CardIntent::Answer(Answer::Session)),
        "`s` grants exactly the token the card offered"
    );

    let ungranted = ungranted_question();
    assert_eq!(
        card_intent(&ungranted, typed('s')),
        None,
        "a card that offered no token has none to take"
    );
}

#[test]
fn n_d_and_esc_deny_the_call() {
    let question = shell_question();

    for press in [typed('n'), typed('d'), key(KeyCode::Esc)] {
        assert_eq!(
            card_intent(&question, press),
            Some(CardIntent::Answer(Answer::Deny)),
            "`{}` denies, as it does today",
            press.code
        );
        assert_ne!(
            card_intent(&question, press),
            Some(CardIntent::Answer(Answer::Once)),
            "a denial is never a yes"
        );
    }
}

#[test]
fn enter_on_an_approval_card_is_nothing_at_all() {
    let question = helper_question();

    assert_eq!(
        card_intent(&question, key(KeyCode::Enter)),
        None,
        "Enter never consents: the card can appear while the operator is typing"
    );
    assert_eq!(
        card_intent(&question, key(KeyCode::Tab)),
        None,
        "and neither does a key the card has no answer for"
    );
}

#[test]
fn a_held_chord_is_never_an_answer() {
    let question = shell_question();

    for held in [KeyModifiers::CONTROL, KeyModifiers::ALT] {
        let press = KeyEvent::new(KeyCode::Char('a'), held);
        assert_eq!(
            card_intent(&question, press),
            None,
            "Ctrl/Alt+A mid-word must not allow a call"
        );
    }
}

#[test]
fn the_plan_card_answers_approve_or_keep_planning() {
    let question = plan_question();
    let card = approval(&question, ASK, SCREEN_ROWS);

    assert_eq!(
        card_intent(&question, typed('y')),
        Some(CardIntent::Answer(Answer::Once)),
        "`y` approves the plan"
    );
    assert_eq!(
        card_intent(&question, typed('n')),
        Some(CardIntent::Answer(Answer::Deny)),
        "`n` keeps planning"
    );
    assert_eq!(
        card_intent(&question, typed('s')),
        None,
        "no token pre-answers a plan, so `s` grants nothing: {}",
        card.keys()
    );
    assert_eq!(
        card_intent(&question, typed('v')),
        Some(CardIntent::View),
        "a long plan is opened by `v`"
    );
}

#[test]
fn v_opens_the_body_and_the_arrows_move_it() {
    let question = shell_question();

    assert_eq!(card_intent(&question, typed('v')), Some(CardIntent::View));
    assert_eq!(
        card_intent(&question, key(KeyCode::Down)),
        Some(CardIntent::Scroll(1))
    );
    assert_eq!(
        card_intent(&question, key(KeyCode::PageUp)),
        Some(CardIntent::Scroll(-super::cards_keys::CARD_PAGE_ROWS))
    );
}

/// A model holding `question` as its open card.
fn model_with(question: Question) -> Model {
    let mut model = Model::default();
    update(&mut model, Msg::Chat(ChatEvent::Question(question)));
    assert!(model.card().is_some(), "the card is open");
    model
}

/// One key press driven through the model's update.
fn press(model: &mut Model, character: char) -> Vec<Cmd> {
    update(model, Msg::Key(typed(character)))
}

#[test]
fn the_models_card_answers_with_once_and_with_the_session_grant() {
    let question = shell_question();
    let mut once = model_with(question.clone());
    assert_eq!(
        press(&mut once, 'y'),
        vec![Cmd::Answer(question.id, Answer::Once)],
        "`y` reaches the runtime as the answer for this call"
    );
    assert!(once.card().is_none(), "an answered card closes");

    let mut session = model_with(question.clone());
    assert_eq!(
        press(&mut session, 's'),
        vec![Cmd::Answer(question.id, Answer::Session)],
        "`s` answers with the session grant the operator was shown"
    );
}

#[test]
fn v_opens_the_whole_body_in_the_model_and_the_arrows_move_it() {
    let mut model = model_with(shell_question());
    assert!(
        !model.card_full() && model.card_scroll() == 0,
        "a call opens folded, at its first row"
    );

    assert!(
        press(&mut model, 'v').is_empty(),
        "looking at the body asks the runtime for nothing"
    );
    assert!(model.card_full(), "`v` opens the whole body");
    assert!(model.card().is_some(), "and the card is still the open one");

    update(&mut model, Msg::Key(key(KeyCode::Down)));
    assert_eq!(model.card_scroll(), 1, "an arrow moves the body one row");
    update(&mut model, Msg::Key(key(KeyCode::PageDown)));
    assert_eq!(
        model.card_scroll(),
        1 + super::cards_keys::CARD_PAGE_ROWS as usize,
        "a page key moves a page"
    );
    press(&mut model, 'v');
    assert!(!model.card_full(), "`v` folds the body back");
}

#[test]
fn enter_over_a_card_neither_answers_it_nor_sends_the_composer() {
    let question = helper_question();
    let mut model = model_with(question);
    update(&mut model, Msg::Paste("count the orders".to_owned()));

    let cmds = update(&mut model, Msg::Key(key(KeyCode::Enter)));

    assert!(
        cmds.is_empty(),
        "Enter over a card asks the runtime for nothing: {cmds:?}"
    );
    assert_eq!(
        model.composer(),
        "count the orders",
        "the typed line stays where it was: Enter sent nothing"
    );
    assert!(
        model.card().is_some(),
        "the card is still waiting for an answer that was not given"
    );
}
