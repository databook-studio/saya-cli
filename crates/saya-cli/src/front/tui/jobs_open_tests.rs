//! `/start` and the job's consent on this surface: the card that asks it, the
//! answer that lets the job run, a deny that never does, the receipts the
//! operator reads, and the quit that waits for a job to settle.

use ratatui::crossterm::event::KeyCode;
use std::sync::mpsc::SyncSender;
use tokio::sync::{mpsc, oneshot};

use super::jobs::JobReport;
use super::jobs_fixture::{card, notices, press};
use super::model::Model;
use super::msg::{Cmd, Msg, Worker};
use super::run_front::Forward;
use super::update::update;
use super::view::approval;
use super::view::jobs::CARD_FIRST;
use crate::agent::tools::consent::Answer;
use crate::chat::Question;
use crate::cli::CodingCommand;
use crate::front::view::Entry;
use crate::interactive::tui::{CodingMessage, CodingPanel};

/// What today's panel puts to the operator for one `/start`: the job, its task,
/// the paths it may write, one row per database identity, how to answer it — and
/// the exact proposal last. It is the shape
/// `interactive::tui::coding::consent` states, read here as the card must show
/// it.
const CONSENT: &str = "Approve job pty-job with this exact scope:\ntask: controlled task with helpers\nwrite: a.txt, b.txt\n\"analytics\" p-0000000000000000000000000000000000000000000000000000000000000001\n\"warehouse\" p-0000000000000000000000000000000000000000000000000000000000000002\nUse `/job approve pty-job` or `/job deny pty-job`.\n{\"job_id\":\"pty-job\",\"operation\":\"start\"}";

/// A front over a queue a test reads.
fn front() -> (Forward, mpsc::UnboundedReceiver<Msg>) {
    let (msgs, queue) = mpsc::unbounded_channel();
    (Forward::new(msgs), queue)
}

/// A job's two report queues and its one result, as a test holds them.
type Channels = (
    SyncSender<CodingMessage>,
    SyncSender<CodingMessage>,
    SyncSender<Result<String, String>>,
);

/// A panel reporting on test channels, and the answer its job waits on.
fn panel() -> (CodingPanel, Channels, oneshot::Receiver<bool>) {
    let (panel, activity, receipts, done) = CodingPanel::reporting("pty-job");
    let (respond, answer) = oneshot::channel();
    activity
        .send(CodingMessage::Approval {
            text: String::from(CONSENT),
            respond,
        })
        .expect("the job's worker puts its consent on the queue");
    (panel, (activity, receipts, done), answer)
}

/// The one message a worker's look handed the loop.
fn look(front: &Forward, queue: &mut mpsc::UnboundedReceiver<Msg>) -> Msg {
    assert!(front.pump(), "the look found the queue");
    queue.try_recv().expect("the panel said something")
}

#[test]
fn the_consent_card_lists_the_identity_facts_verbatim() {
    let (front, mut queue) = front();
    let (panel, (_activity, _receipts, _done), _answer) = panel();
    front.attach_job_panel(panel);
    let mut model = Model::default();
    update(&mut model, look(&front, &mut queue));

    let card = model.card().expect("the consent is the card");
    assert_eq!(card.caller, "pty-job", "the job's own id names the asker");
    assert_eq!(
        card.facts, CONSENT,
        "the facts are the panel's own words, line for line"
    );
    let pane = approval(card, None, 30);
    let rows: Vec<String> = pane.target().to_vec();
    assert_eq!(
        rows,
        CONSENT.split('\n').map(str::to_owned).collect::<Vec<_>>(),
        "the card's body is one row per line of the consent"
    );
    let shown = pane.shown_rows(model.card_full(), 100);
    assert!(
        shown.contains(&String::from(
            "\"analytics\" p-0000000000000000000000000000000000000000000000000000000000000001"
        )),
        "the first identity is on the screen whole: {shown:?}"
    );
    assert!(
        shown.contains(&String::from(
            "\"warehouse\" p-0000000000000000000000000000000000000000000000000000000000000002"
        )),
        "and so is the second: {shown:?}"
    );
    assert!(
        shown.iter().any(|row| row.starts_with("write: ")),
        "the paths it may write are among the facts: {shown:?}"
    );
    assert_eq!(
        pane.withheld(100),
        1,
        "only the exact proposal is below the cap"
    );
}

#[test]
fn deny_never_starts_the_job() {
    let (front, mut queue) = front();
    let (panel, (_activity, _receipts, _done), answer) = panel();
    front.attach_job_panel(panel);
    let mut model = Model::default();
    update(&mut model, look(&front, &mut queue));
    let call = model.card().expect("the consent is the card").id;

    // The controlling spelling, exactly as the operator types it.
    model.insert("/deny pty-job");
    let cmds = update(&mut model, press(KeyCode::Enter));
    assert_eq!(cmds, vec![Cmd::Answer(call, Answer::Deny)]);
    assert!(
        !cmds.iter().any(|cmd| matches!(cmd, Cmd::Job(_))),
        "a deny starts nothing and sends no verb: {cmds:?}"
    );
    assert!(model.card().is_none(), "the card is answered and gone");

    assert!(front.answer(call, Answer::Deny), "the job waits on it");
    assert_eq!(
        answer.blocking_recv(),
        Ok(false),
        "the job is told no, in its own words"
    );
}

#[test]
fn a_consent_queued_behind_an_open_card_waits_its_turn() {
    let mut model = Model::default();
    update(&mut model, card(1));
    // The job's consent arrives underneath the session's open card.
    update(
        &mut model,
        Msg::Worker(Worker {
            job: Some(JobReport::Ask {
                job: String::from("pty-job"),
                question: Question {
                    id: 7,
                    caller: String::from("pty-job"),
                    facts: String::from(CONSENT),
                    grant: None,
                },
            }),
            ..Worker::default()
        }),
    );
    assert_eq!(
        model.card().map(|card| card.id),
        Some(1),
        "the open card is never replaced by a job's ask"
    );

    // `/approve` names the job, and never jumps the card that is open.
    model.insert("/approve pty-job");
    let cmds = update(&mut model, press(KeyCode::Enter));
    assert!(cmds.is_empty(), "no consent is given blind: {cmds:?}");
    assert_eq!(model.toast(), Some(CARD_FIRST));

    // Answering the open card hands the slot on, in arrival order.
    let cmds = update(&mut model, press(KeyCode::Char('y')));
    assert_eq!(cmds, vec![Cmd::Answer(1, Answer::Once)]);
    assert_eq!(
        model.card().map(|card| card.id),
        Some(7),
        "then the consent is what the operator is looking at"
    );

    model.insert("/approve pty-job");
    let cmds = update(&mut model, press(KeyCode::Enter));
    assert_eq!(
        cmds,
        vec![Cmd::Answer(7, Answer::Once)],
        "and `/approve` answers that consent, once"
    );
    assert!(model.card().is_none());
}

#[test]
fn a_start_line_is_the_jobs_command_and_never_a_turn() {
    let mut model = Model::default();
    model.insert("/start --id pty-job 'controlled task with helpers' --write a.txt");
    let cmds = update(&mut model, press(KeyCode::Enter));
    let exec = &cmds[0];
    let Cmd::Job(request) = exec else {
        panic!("a start is the job's own command, never a turn: {cmds:?}");
    };
    match &request.command {
        CodingCommand::Start { id, task, .. } => {
            assert_eq!(id, "pty-job");
            assert_eq!(task, "controlled task with helpers");
        }
        other => panic!("a start is a start: {other:?}"),
    }
    assert_eq!(cmds.len(), 1, "one command, and nothing else: {cmds:?}");
}

#[test]
fn a_receipt_is_a_transcript_line_in_the_workers_own_words() {
    let (front, mut queue) = front();
    let (panel, _activity, receipts, _done) = CodingPanel::reporting("pty-job");
    front.attach_job_panel(panel);
    let ack = "{\"pause_requested\":1,\"pause_acknowledged\":true,\"target\":\"child-a\"}";
    receipts
        .send(CodingMessage::Activity(String::from(ack)))
        .expect("the worker's receipt reaches its queue");
    let mut model = Model::default();
    update(&mut model, look(&front, &mut queue));
    assert_eq!(
        notices(&model),
        vec![String::from(ack)],
        "the receipt is the transcript's line, in the words the worker wrote it"
    );
    assert!(matches!(model.turns()[0].entries()[0], Entry::Notice(_)));
}

#[test]
fn a_quit_waits_for_the_job_to_settle() {
    let (front, mut queue) = front();
    let (panel, _activity, _receipts, done) = CodingPanel::reporting("pty-job");
    front.attach_job_panel(panel);

    front.quit();
    let Some(Msg::Worker(worker)) = queue.try_recv().ok() else {
        panic!("the surface says why it cannot leave");
    };
    assert_eq!(worker.message, crate::interactive::tui::JOB_RUNNING);
    assert!(
        queue.try_recv().is_err(),
        "and nothing asks the loop to leave"
    );

    done.send(Ok(String::from("job finished"))).unwrap();
    let mut model = Model::default();
    update(&mut model, look(&front, &mut queue));
    assert_eq!(notices(&model), vec![String::from("job finished")]);

    front.quit();
    assert!(
        matches!(queue.try_recv(), Ok(Msg::Quit)),
        "once the job has settled the quit is the loop's"
    );
}
