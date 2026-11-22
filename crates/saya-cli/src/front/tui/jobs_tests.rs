//! The jobs drawer through the pure update: the two keys that open it, the calls
//! a job puts on the same card the session uses, and the drawer's own keys —
//! `↵` open · `a` approve · `x` cancel · `Esc` close (`02` §8).

use ratatui::crossterm::event::KeyCode;

use super::jobs_fixture::{asked, card, ctrl_j, model, notices, press};
use super::msg::Cmd;
use super::update::update;
use super::view::jobs::{CARD_FIRST, DETAIL_UNIT};
use crate::agent::tools::consent::Answer;

#[test]
fn ctrl_j_opens_the_drawer_and_tells_the_session_nothing() {
    let mut model = model();
    let cmds = update(&mut model, ctrl_j());
    assert!(
        cmds.is_empty(),
        "opening the drawer is not a command: {cmds:?}"
    );
    assert!(model.jobs_open(), "Ctrl+J opens the jobs drawer");
    assert_eq!(model.jobs().rows.len(), 3, "it lists the run's jobs");
}

#[test]
fn slash_job_opens_the_drawer_and_sends_nothing() {
    let mut model = model();
    // Set whole: `/` on an empty composer is the palette's key (`02` §11), so a
    // slash line reaches the composer as text.
    model.insert("/job");
    let cmds = update(&mut model, press(KeyCode::Enter));
    assert!(cmds.is_empty(), "`/job` is not a turn: {cmds:?}");
    assert!(model.jobs_open(), "bare `/job` opens the drawer");
    assert_eq!(model.composer(), "", "and the line is not kept");
}

#[test]
fn slash_job_with_an_argument_is_the_jobs_command_not_a_turn() {
    let mut model = model();
    model.insert("/job show fix-flaky");
    let cmds = update(&mut model, press(KeyCode::Enter));
    assert_eq!(
        cmds,
        vec![Cmd::Job(crate::slash::JobRequest::job(
            crate::cli::CodingCommand::Show {
                id: String::from("fix-flaky")
            }
        ))],
        "the job's own verb is the job surface's, read by today's plumbing"
    );
}

#[test]
fn a_jobs_call_queues_behind_the_open_card_and_shows_once_it_is_answered() {
    let mut model = model();
    update(&mut model, card(1));
    assert_eq!(
        model.card().map(|card| card.id),
        Some(1),
        "the session asks first"
    );

    let cmds = update(&mut model, asked("fix-flaky", 7));
    assert!(
        cmds.is_empty(),
        "a job's ask asks for nothing yet: {cmds:?}"
    );
    assert_eq!(
        model.card().map(|card| card.id),
        Some(1),
        "the open card is never replaced by a job's ask"
    );

    let cmds = update(&mut model, press(KeyCode::Char('y')));
    assert_eq!(
        cmds,
        vec![Cmd::Answer(1, Answer::Once)],
        "the first card is answered first"
    );
    assert_eq!(
        model.card().map(|card| card.id),
        Some(7),
        "then the job's call takes the card, in the order the asks arrived"
    );
}

#[test]
fn two_jobs_asks_keep_the_order_they_arrived_in() {
    let mut model = model();
    update(&mut model, card(1));
    update(&mut model, asked("fix-flaky", 7));
    update(&mut model, asked("docs-pass", 8));
    update(&mut model, press(KeyCode::Char('y')));
    assert_eq!(model.card().map(|card| card.id), Some(7));
    update(&mut model, press(KeyCode::Char('n')));
    assert_eq!(
        model.card().map(|card| card.id),
        Some(8),
        "FIFO, ask by ask"
    );
}

#[test]
fn an_empty_card_slot_takes_the_jobs_call_at_once() {
    let mut model = model();
    update(&mut model, asked("fix-flaky", 7));
    assert_eq!(model.card().map(|card| card.id), Some(7));
    assert_eq!(
        model.card().map(|card| card.caller.as_str()),
        Some("fix-flaky: the shell tool")
    );
}

#[test]
fn a_opens_the_card_for_the_jobs_call_and_never_answers_it() {
    let mut model = model();
    update(&mut model, ctrl_j());
    // The job's call arrives while the drawer is open, and takes the card.
    update(&mut model, asked("fix-flaky", 7));
    assert_eq!(model.card().map(|card| card.id), Some(7));

    let cmds = update(&mut model, press(KeyCode::Char('a')));
    assert!(cmds.is_empty(), "`a` answers nothing by itself: {cmds:?}");
    assert_eq!(
        model.card().map(|card| card.id),
        Some(7),
        "the card for that job's call is the one showing"
    );
    assert!(!model.jobs_open(), "the drawer gets out of its way");
}

#[test]
fn a_on_a_job_that_owes_no_call_changes_nothing() {
    let mut model = model();
    update(&mut model, ctrl_j());
    let cmds = update(&mut model, press(KeyCode::Char('a')));
    assert!(cmds.is_empty(), "there is no card to open: {cmds:?}");
    assert!(model.card().is_none());
    assert_eq!(model.toast(), None, "and nothing is said about it");
}

#[test]
fn a_card_open_keeps_the_keyboard_so_the_drawer_does_not_open_over_it() {
    let mut model = model();
    update(&mut model, asked("fix-flaky", 7));
    let cmds = update(&mut model, ctrl_j());
    assert!(cmds.is_empty());
    assert!(
        !model.jobs_open(),
        "the card is what the operator is looking at"
    );
    let cmds = update(&mut model, press(KeyCode::Char('a')));
    assert_eq!(
        cmds,
        vec![Cmd::Answer(7, Answer::Once)],
        "`a` here is the card's own once-allow, not the drawer's"
    );
}

#[test]
fn a_says_the_card_comes_first_when_one_is_queued_ahead() {
    let mut model = model();
    update(&mut model, ctrl_j());
    // A card arrives while the drawer is open, and the job's ask queues behind
    // it: the drawer cannot jump the queue its own key would approve (`02` §5).
    update(&mut model, card(1));
    update(&mut model, asked("fix-flaky", 7));

    let cmds = update(&mut model, press(KeyCode::Char('a')));
    assert!(cmds.is_empty(), "a never consents blind: {cmds:?}");
    assert_eq!(model.toast(), Some(CARD_FIRST));
    assert_eq!(model.card().map(|card| card.id), Some(1));
    assert!(model.jobs_open(), "the drawer stays up behind the card");
}

#[test]
fn x_asks_before_it_cancels() {
    let mut model = model();
    update(&mut model, ctrl_j());
    let cmds = update(&mut model, press(KeyCode::Char('x')));
    assert!(cmds.is_empty(), "x cancels nothing yet: {cmds:?}");
    assert!(model.jobs().is_confirming(), "x asks first");

    let cmds = update(&mut model, press(KeyCode::Char('y')));
    assert_eq!(
        cmds,
        vec![Cmd::CancelJob(String::from("fix-flaky"))],
        "y cancels the row it named, once"
    );
    assert!(!model.jobs().is_confirming());
}

#[test]
fn n_withdraws_the_cancel_and_asks_for_nothing() {
    let mut model = model();
    update(&mut model, ctrl_j());
    update(&mut model, press(KeyCode::Char('x')));
    let cmds = update(&mut model, press(KeyCode::Char('n')));
    assert!(cmds.is_empty(), "n cancels nothing: {cmds:?}");
    assert!(!model.jobs().is_confirming());
    assert!(model.jobs_open(), "the drawer stays where it was");
}

#[test]
fn esc_closes_the_drawer_with_no_command() {
    let mut model = model();
    update(&mut model, ctrl_j());
    let cmds = update(&mut model, press(KeyCode::Esc));
    assert!(cmds.is_empty(), "Esc asks for nothing: {cmds:?}");
    assert!(!model.jobs_open(), "and the drawer is gone");
    assert!(model.toast().is_none());
}

#[test]
fn esc_over_the_confirm_withdraws_the_ask_and_keeps_the_drawer() {
    let mut model = model();
    update(&mut model, ctrl_j());
    update(&mut model, press(KeyCode::Char('x')));
    let cmds = update(&mut model, press(KeyCode::Esc));
    assert!(cmds.is_empty());
    assert!(!model.jobs().is_confirming());
    assert!(model.jobs_open());
}

#[test]
fn the_drawer_walks_its_rows() {
    let mut model = model();
    update(&mut model, ctrl_j());
    update(&mut model, press(KeyCode::Down));
    update(&mut model, press(KeyCode::Down));
    assert_eq!(model.jobs().selected(), 2);
    update(&mut model, press(KeyCode::Up));
    assert_eq!(model.jobs().selected(), 1);
}

#[test]
fn enter_shows_the_jobs_summary_line_and_says_which_unit_is_missing() {
    let mut model = model();
    update(&mut model, ctrl_j());
    let cmds = update(&mut model, press(KeyCode::Enter));
    assert!(cmds.is_empty(), "opening a detail runs nothing: {cmds:?}");
    assert!(
        !model.jobs_open(),
        "the drawer makes way for the transcript"
    );
    let said = notices(&model);
    assert_eq!(said.len(), 2, "the summary line and the sentence: {said:?}");
    assert!(said[0].contains("fix-flaky") && said[0].contains("running 3m"));
    assert_eq!(said[1], DETAIL_UNIT);
}
