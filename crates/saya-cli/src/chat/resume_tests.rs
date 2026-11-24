//! The resume decision: one test per row of the table, plus the job this
//! process neither holds open nor sees finished.
//!
//! The decision is pure — the record's link and the facts this process holds
//! about the job — so every row is decided here without a store, a job or a run.

use saya_store::{RedactedSession, RedactedTurn, RunLink};
use saya_types::JobId;

use super::{Resume, RunFacts, decide};
use crate::chat::{SettledTurn, Transcript};

fn job(id: &str) -> JobId {
    JobId::parse(id).expect("the fixture's job id parses")
}

fn saved_turn(prompt: &str, answer: &str) -> RedactedTurn {
    RedactedTurn {
        user: prompt.to_owned(),
        assistant: answer.to_owned(),
        database_derived: false,
        tools: Vec::new(),
    }
}

fn pair(prompt: &str, answer: &str) -> SettledTurn {
    SettledTurn {
        prompt: prompt.to_owned(),
        answer: answer.to_owned(),
    }
}

/// A record linked to `job-051` at epoch 3, with two settled turns behind it.
fn linked() -> RedactedSession {
    RedactedSession {
        id: "chat-051".to_owned(),
        model: "current-model".to_owned(),
        approval_mode: "ask".to_owned(),
        turns: vec![
            saved_turn("how many rows?", "four"),
            saved_turn("and now?", "two"),
        ],
        run: Some(RunLink {
            job: job("job-051"),
            epoch: 3,
        }),
        ..RedactedSession::default()
    }
}

fn transcript(record: RedactedSession) -> Transcript {
    Transcript::from_record(record)
}

fn reason_of(decision: Resume) -> String {
    match decision {
        Resume::ReadOnly { reason } => reason,
        other => panic!("the session must open read-only, not {other:?}"),
    }
}

#[test]
fn a_run_open_in_this_process_is_attached_at_the_same_epoch() {
    let decision = decide(
        &transcript(linked()),
        RunFacts {
            open_in_this_process: true,
            ..RunFacts::default()
        },
    );
    assert_eq!(
        decision,
        Resume::Attach {
            job: job("job-051"),
            epoch: 3
        },
        "a run this process already holds is attached, at the epoch the link records"
    );
}

#[test]
fn a_finished_run_prepares_a_new_epoch_seeded_from_the_transcript() {
    let seeded = transcript(linked());
    let decision = decide(
        &seeded,
        RunFacts {
            finished: true,
            ..RunFacts::default()
        },
    );
    assert_eq!(
        decision,
        Resume::PrepareNewEpoch {
            seed: vec![pair("how many rows?", "four"), pair("and now?", "two")]
        },
        "a finished run prepares a new epoch seeded with the transcript, oldest first"
    );
}

#[test]
fn a_missing_job_dir_opens_read_only_with_its_reason() {
    let reason = reason_of(decide(
        &transcript(linked()),
        RunFacts {
            job_dir_missing: true,
            ..RunFacts::default()
        },
    ));
    assert!(
        reason.contains("folder") && reason.contains("read-only"),
        "the reason names the missing folder and says the session is read-only: {reason}"
    );
}

#[test]
fn a_record_without_a_link_opens_read_only() {
    let mut record = linked();
    record.run = None;
    let restored = transcript(record);
    let reason = reason_of(decide(&restored, RunFacts::default()));
    assert!(
        reason.contains("no run link"),
        "the reason says the record carries no run link: {reason}"
    );
    assert_eq!(
        restored.turns().len(),
        2,
        "a record without a link still restores its transcript"
    );
}

#[test]
fn a_managed_checkout_job_is_refused_not_resumable() {
    let decision = decide(
        &transcript(linked()),
        RunFacts {
            managed_checkout: true,
            ..RunFacts::default()
        },
    );
    let Resume::Refused {
        job: refused,
        detail,
    } = decision
    else {
        panic!("a managed-checkout job cannot be resumed: {decision:?}");
    };
    assert_eq!(refused, job("job-051"), "the refusal names the job");
    assert!(
        detail.contains("managed checkout"),
        "the refusal keeps the core's words: {detail}"
    );
}

#[test]
fn a_run_still_running_elsewhere_opens_read_only() {
    let reason = reason_of(decide(&transcript(linked()), RunFacts::default()));
    assert!(
        reason.contains("another process"),
        "a run this process does not hold and has not finished is not attached or replaced: {reason}"
    );
}
