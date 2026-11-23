//! The bell (UI3): a call that takes the card rings the terminal once, and
//! nothing else does — a call queued behind an open card rings when its turn
//! comes, not when it arrives.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::jobs::JobReport;
use super::model::Model;
use super::msg::{Cmd, Msg, Worker};
use super::update::update;
use super::view::cards_fixture::shell_question;
use crate::chat::{ChatEvent, Question};

/// One call, as the session's front delivers it.
fn asked(question: Question) -> Msg {
    Msg::Chat(ChatEvent::Question(question))
}

/// A press with nothing held.
fn key(code: KeyCode) -> Msg {
    Msg::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

#[test]
fn a_call_that_takes_the_card_rings_the_terminal_once() {
    let mut model = Model::default();

    let cmds = update(&mut model, asked(shell_question()));

    assert_eq!(
        cmds,
        vec![Cmd::Bell],
        "the operator is told the surface is waiting for them"
    );
    assert!(model.card().is_some(), "and the call is on the card");
}

#[test]
fn a_call_that_waits_its_turn_rings_only_when_it_takes_the_card() {
    let mut model = Model::default();
    assert_eq!(
        update(&mut model, asked(shell_question())),
        vec![Cmd::Bell],
        "the first call opens the card"
    );

    let queued = update(&mut model, asked(crate::chat::Question {
        id: 99,
        ..shell_question()
    }));
    assert!(
        queued.is_empty(),
        "a call queued behind an open card is not news yet: {queued:?}"
    );

    let cmds = update(&mut model, key(KeyCode::Char('y')));
    assert_eq!(
        cmds,
        vec![Cmd::Answer(8, crate::agent::tools::consent::Answer::Once), Cmd::Bell],
        "answering hands the card on, and the call that takes it rings"
    );
    assert!(
        model.card().map(|card| card.id) == Some(99),
        "the queued call is the one now waiting"
    );
}

#[test]
fn a_jobs_call_rings_like_the_sessions() {
    let mut model = Model::default();
    let report = JobReport::Ask {
        job: String::from("pty-job"),
        question: shell_question(),
    };

    let cmds = update(
        &mut model,
        Msg::Worker(Worker {
            job: Some(report),
            ..Worker::default()
        }),
    );

    assert_eq!(cmds, vec![Cmd::Bell], "a job's call is a call");
    assert!(model.card().is_some(), "and it takes the same card");
}
