//! The surface's end of today's job plumbing: what a job's worker queues — its
//! activity and the calls it asks — read as the drawer's rows and the card's
//! question, and the same card answer resuming the job's own waiting end.

use saya_agent::ApprovalChoice;
use tokio::sync::{mpsc, oneshot};

use super::jobs::JobReport;
use super::jobs_desk::NO_SUCH_JOB;
use super::msg::Msg;
use super::run_front::Forward;
use crate::agent::tools::consent::Answer;
use crate::chat::Question;
use crate::interactive::tui::CodingMessage;

/// A front over a queue a test reads.
fn front() -> (Forward, mpsc::UnboundedReceiver<Msg>) {
    let (msgs, queue) = mpsc::unbounded_channel();
    (Forward::new(msgs), queue)
}

/// The question the card is asked, as the surface states it.
fn question(id: u64) -> Question {
    Question {
        id,
        caller: String::from("fix-flaky: the shell tool"),
        facts: String::from("$ cargo test"),
        grant: None,
    }
}

#[test]
fn an_activity_line_becomes_a_row_of_the_run() {
    let (front, mut queue) = front();
    front.deliver_job(
        "fix-flaky",
        CodingMessage::Activity(String::from("step 2 · query demo")),
    );
    let Some(Msg::Worker(worker)) = queue.try_recv().ok() else {
        panic!("the row lands on the queue's worker slot");
    };
    let Some(JobReport::Listed(rows)) = worker.job else {
        panic!("an activity line is a list of rows");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, "fix-flaky");
    assert_eq!(rows[0].summary, "step 2 · query demo");
}

#[test]
fn a_jobs_call_becomes_one_question_on_the_same_card_slot() {
    let (front, mut queue) = front();
    let (respond, answer) = oneshot::channel();
    front.ask_job(String::from("fix-flaky"), question(1), respond);

    let Some(Msg::Worker(worker)) = queue.try_recv().ok() else {
        panic!("a job's call lands on the queue's worker slot");
    };
    let Some(JobReport::Ask { job, question }) = worker.job else {
        panic!("a call is asked, not listed");
    };
    assert_eq!(job, "fix-flaky");
    assert_eq!(question.id, 1, "the plumbing's own id is the card's id");

    assert!(
        front.answer(1, Answer::Once),
        "the card's answer reaches the job's waiting end"
    );
    assert_eq!(
        answer.blocking_recv(),
        Ok(ApprovalChoice::AllowOnce),
        "and it is reported in the plumbing's own words"
    );
    assert!(!front.answer(1, Answer::Once), "nothing is answered twice");
}

#[test]
fn a_session_wide_yes_is_reported_as_one() {
    let (front, _queue) = front();
    let (respond, answer) = oneshot::channel();
    front.ask_job(String::from("docs-pass"), question(2), respond);
    assert!(front.answer(2, Answer::Session));
    assert!(
        matches!(
            answer.blocking_recv(),
            Ok(ApprovalChoice::AllowSession { .. })
        ),
        "a session-wide yes is a session-wide yes"
    );
}

#[test]
fn the_plumbings_own_call_question_is_read_as_a_card_too() {
    let (front, mut queue) = front();
    let (respond, answer) = oneshot::channel();
    front.deliver_job(
        "fix-flaky",
        CodingMessage::Question {
            subject: String::from("the shell tool"),
            target: String::from("$ cargo test"),
            respond,
        },
    );
    let Some(Msg::Worker(worker)) = queue.try_recv().ok() else {
        panic!("a call lands on the queue");
    };
    let Some(JobReport::Ask { question, .. }) = worker.job else {
        panic!("a call is asked");
    };
    assert!(question.caller.contains("fix-flaky"));
    assert_eq!(question.facts, "$ cargo test");

    assert!(front.answer(question.id, Answer::Deny));
    assert_eq!(answer.blocking_recv(), Ok(ApprovalChoice::Deny));
}

#[test]
fn a_scope_question_is_answered_the_same_way() {
    let (front, mut queue) = front();
    let (respond, answer) = oneshot::channel();
    front.deliver_job(
        "fix-flaky",
        CodingMessage::Approval {
            text: String::from("Approve job fix-flaky with this exact scope"),
            respond,
        },
    );
    let Some(Msg::Worker(worker)) = queue.try_recv().ok() else {
        panic!("the question lands on the queue");
    };
    let Some(JobReport::Ask { question, .. }) = worker.job else {
        panic!("the scope question is asked");
    };
    assert!(front.answer(question.id, Answer::Once));
    assert_eq!(answer.blocking_recv(), Ok(true), "a yes is a yes");
}

#[test]
fn cancelling_with_no_job_says_so_instead_of_doing_nothing() {
    let (front, mut queue) = front();
    front.cancel_job("fix-flaky");
    let Some(Msg::Worker(worker)) = queue.try_recv().ok() else {
        panic!("the surface says what it could not do");
    };
    assert_eq!(worker.message, NO_SUCH_JOB);
}

// ---------------------------------------------------------------------------
// what a job's text may show: the old app's `safe_text` pins, moved here
// ---------------------------------------------------------------------------
//
// The deleted App pushed a job's activity into its transcript through one
// redacting, bounded filter, and three tests pinned it. The drawer and the
// consent card keep the same guarantee on this surface, so the pins follow
// them: a secret never reaches a row, a control character never reaches the
// terminal, and neither a row nor a consent's facts grow without a bound.

mod what_a_jobs_text_may_show {
    use super::super::jobs_desk::{facts, summary};

    /// A terminal escape, a bell, a carriage return, and a backspace are
    /// stripped; a row keeps no control character at all — a row is one row.
    #[test]
    fn control_characters_never_reach_a_row() {
        let hostile = "ok\u{1b}[31mred\u{1b}[0m\u{7}bell\rreturn\u{8}back\nline\ttab";
        let shown = summary(hostile);
        assert!(
            !shown.chars().any(char::is_control),
            "a row carries no control character at all: {shown:?}"
        );
        assert!(!shown.contains('\u{1b}') && !shown.contains('\u{7}'));
        assert!(shown.starts_with("ok"), "{shown:?}");
    }

    /// A consent is read row by row, so its own newlines survive — every other
    /// control character does not.
    #[test]
    fn a_consent_keeps_its_rows_and_drops_every_other_control_character() {
        let shown = facts("run:\u{1b}[2J\u{7} rm -rf build\nand then: make");
        assert_eq!(shown, "run:[2J rm -rf build\nand then: make");
    }

    /// A secret-shaped value is replaced before anything is shown, in both
    /// kinds of text a job hands the surface.
    #[test]
    fn secrets_are_redacted_before_display() {
        let row = summary("connecting with password=hunter2-sentinel done");
        assert!(!row.contains("hunter2-sentinel"), "{row}");
        assert!(row.contains("connecting"), "{row}");
        let consent = facts("export TOKEN=ghp_hunter2-sentinel\nrun");
        assert!(!consent.contains("hunter2-sentinel"), "{consent}");
    }

    /// A row is one line of at most 120 characters; a consent's facts stop at
    /// sixteen kibibytes of characters, with the exactly-at-the-bound text
    /// untouched.
    #[test]
    fn both_texts_are_bounded() {
        let long = "x".repeat(40 * 1024);
        assert_eq!(summary(&long).chars().count(), 120);
        assert_eq!(facts(&long).chars().count(), 16 * 1024);
        let exact = "y".repeat(16 * 1024);
        assert_eq!(
            facts(&exact),
            exact,
            "positive control: at the bound nothing is cut"
        );
    }
}
