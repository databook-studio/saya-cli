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
    FOLDER, SCREEN_ROWS, helper_question, plan_question, shell_question, ungranted_question,
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
    let card = approval(&question, FOLDER, SCREEN_ROWS);
    let labels: String = card
        .choices()
        .iter()
        .map(|choice| choice.label.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        labels.contains("shell:python3"),
        "the card states the token `s` would record: {labels}"
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
    let card = approval(&question, FOLDER, SCREEN_ROWS);

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
        "no token pre-answers a plan, so `s` grants nothing: {:?}",
        card.choices()
    );
    assert_eq!(
        card_intent(&question, typed('v')),
        Some(CardIntent::View),
        "a long plan is opened by `v`"
    );
}

#[test]
fn v_opens_the_body_and_the_arrows_focus_a_choice() {
    let question = shell_question();

    assert_eq!(card_intent(&question, typed('v')), Some(CardIntent::View));
    assert_eq!(
        card_intent(&question, key(KeyCode::Down)),
        Some(CardIntent::Focus(1)),
        "an arrow walks the answers the panel offers"
    );
    assert_eq!(
        card_intent(&question, key(KeyCode::Up)),
        Some(CardIntent::Focus(-1))
    );
    assert_eq!(
        card_intent(&question, key(KeyCode::PageUp)),
        Some(CardIntent::Scroll(-super::cards_keys::CARD_PAGE_ROWS)),
        "the page keys still move the body"
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
fn v_opens_the_whole_body_in_the_model_and_the_page_keys_move_it() {
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

    update(&mut model, Msg::Key(key(KeyCode::PageDown)));
    assert_eq!(
        model.card_scroll(),
        super::cards_keys::CARD_PAGE_ROWS as usize,
        "a page key moves a page"
    );
    press(&mut model, 'v');
    assert!(!model.card_full(), "`v` folds the body back");
}

#[test]
fn enter_over_a_card_never_answers_it_and_still_sends_the_composer() {
    let question = helper_question();
    let mut model = model_with(question);
    update(&mut model, Msg::Paste("count the orders".to_owned()));

    let cmds = update(&mut model, Msg::Key(key(KeyCode::Enter)));

    // Enter is the composer's key, never the card's: a call can land while the
    // operator is typing the next line, and that line is what Enter sends.
    assert_eq!(
        cmds,
        vec![Cmd::Send(String::from("count the orders"))],
        "Enter sends the typed line and never answers the card"
    );
    assert_eq!(model.composer(), "", "the line was sent, not swallowed");
    assert!(
        model.card().is_some(),
        "the card is still waiting for an answer that was not given"
    );
}

#[test]
fn typing_over_a_card_reaches_the_composer_and_never_answers_it() {
    // A job's consent is given by name as well as by key (`02` §8): the
    // operator types `/job approve <id>` while the card asks.
    let mut model = model_with(helper_question());
    update(&mut model, Msg::Key(key(KeyCode::Char('/'))));
    update(&mut model, Msg::Key(key(KeyCode::Char('j'))));
    update(&mut model, Msg::Key(key(KeyCode::Char('o'))));
    update(&mut model, Msg::Key(key(KeyCode::Char('b'))));
    assert_eq!(
        model.composer(),
        "/job",
        "the composer is where typing goes"
    );
    assert!(model.card().is_some(), "and the card is untouched");
}

#[test]
fn enter_answers_the_choice_the_arrows_focused() {
    let question = shell_question();
    let mut model = model_with(question.clone());
    assert!(
        model.card_choice().is_none(),
        "a call opens with no answer focused"
    );

    update(&mut model, Msg::Key(key(KeyCode::Down)));
    assert_eq!(model.card_choice(), Some(0), "the first answer is focused");
    update(&mut model, Msg::Key(key(KeyCode::Down)));
    assert_eq!(model.card_choice(), Some(1), "and an arrow walks on");

    let cmds = update(&mut model, Msg::Key(key(KeyCode::Enter)));

    assert_eq!(
        cmds,
        vec![Cmd::Answer(question.id, Answer::Session)],
        "Enter answers with the answer the operator was looking at"
    );
    assert!(model.card().is_none(), "the card is answered");
}

#[test]
fn typing_clears_the_focus_so_enter_cannot_consent_behind_a_draft() {
    let question = helper_question();
    let mut model = model_with(question);

    update(&mut model, Msg::Key(key(KeyCode::Down)));
    assert_eq!(model.card_choice(), Some(0));

    update(&mut model, Msg::Key(key(KeyCode::Char('n'))));
    assert!(
        model.card_choice().is_none(),
        "a typed character is the composer's, and drops the focus"
    );
    let cmds = update(&mut model, Msg::Key(key(KeyCode::Enter)));
    assert!(
        matches!(cmds.as_slice(), [Cmd::Send(_)]),
        "Enter then sends the draft, as it always has: {cmds:?}"
    );
    assert!(model.card().is_some(), "and the call is still waiting");
}

#[test]
fn the_focus_walks_the_answers_that_are_offered_and_no_others() {
    let mut model = model_with(ungranted_question());
    update(&mut model, Msg::Key(key(KeyCode::Up)));
    assert_eq!(
        model.card_choice(),
        Some(0),
        "an arrow up from nothing lands on the first answer"
    );

    for _ in 0..9 {
        update(&mut model, Msg::Key(key(KeyCode::Down)));
    }
    assert_eq!(
        model.card_choice(),
        Some(2),
        "three answers: y, n and Esc — the walk stops at the last"
    );
}
