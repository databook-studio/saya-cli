//! One pair per answered turn and nothing for a turn that ended any other way;
//! a record saved before the run link existed comes back byte for byte.

use saya_agent::BudgetDimension;
use saya_store::{RedactedSession, RedactedTurn};

use super::{SettledTurn, Transcript};
use crate::chat::{Budget, ChatOutcome};

/// A saved record from before the session kept a run link: identity, model,
/// mode, and one settled turn. It has no `run` key and never had one.
fn saved() -> RedactedSession {
    RedactedSession {
        id: "chat-051".to_owned(),
        profile: Some("analytics".to_owned()),
        provider: "ollama".to_owned(),
        model: "current-model".to_owned(),
        approval_mode: "ask".to_owned(),
        agent_mode: "build".to_owned(),
        turns: vec![saved_turn("how many rows?", "four")],
        ..RedactedSession::default()
    }
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

fn load(text: &str) -> RedactedSession {
    serde_json::from_str(text).expect("the saved record reads back")
}

#[test]
fn an_answered_turn_records_one_pair() {
    let mut transcript = Transcript::from_record(saved());
    let recorded = transcript.settle("and the second?", "two", &ChatOutcome::Answered);
    assert!(recorded, "a turn that answered is recorded");
    assert_eq!(
        transcript.turns(),
        vec![
            pair("how many rows?", "four"),
            pair("and the second?", "two")
        ],
        "an answered turn is appended as exactly one prompt/answer pair"
    );
    assert_eq!(
        transcript.record().turns.len(),
        2,
        "the record the session saves carries the answered turn too"
    );
}

#[test]
fn a_cancelled_turn_records_nothing() {
    let mut transcript = Transcript::from_record(saved());
    let recorded = transcript.settle("stop that", "half an answer", &ChatOutcome::Cancelled);
    assert!(!recorded, "a cancelled turn is not recorded");
    assert_eq!(
        transcript.turns(),
        vec![pair("how many rows?", "four")],
        "a cancelled turn leaves the transcript as it was"
    );
    assert_eq!(
        transcript.record().turns.len(),
        1,
        "a cancelled turn leaves the saved record as it was"
    );
}

#[test]
fn an_unanswered_turn_records_nothing() {
    let outcomes = [
        ChatOutcome::Stopped(Budget {
            dimension: BudgetDimension::LogicalRequests,
        }),
        ChatOutcome::Paused {
            message: "stopped: no progress for 8 of 8 turns".to_owned(),
        },
        ChatOutcome::Failed {
            message: "stopped: the logical_requests backstop was reached".to_owned(),
        },
    ];
    for outcome in outcomes {
        let mut transcript = Transcript::from_record(saved());
        let recorded = transcript.settle("carry on", "an unfinished answer", &outcome);
        assert!(!recorded, "a turn that ended {outcome:?} is not recorded");
        assert_eq!(
            transcript.turns(),
            vec![pair("how many rows?", "four")],
            "a turn that ended {outcome:?} leaves the transcript as it was"
        );
    }
}

#[test]
fn a_record_without_a_link_round_trips_byte_identical() {
    let text = serde_json::to_string(&saved()).expect("the saved record serializes");
    assert!(
        !text.contains("\"run\""),
        "the fixture predates the run link: {text}"
    );
    let rewritten = serde_json::to_string(&Transcript::from_record(load(&text)).into_record())
        .expect("the reloaded record serializes");
    assert_eq!(
        rewritten, text,
        "a record without a run link must come back byte identical"
    );
}

#[test]
fn the_saved_turns_read_back_as_pairs() {
    let mut record = saved();
    record.turns = vec![saved_turn("first", "one"), saved_turn("second", "two")];
    let transcript = Transcript::from_record(record);
    assert_eq!(
        transcript.turns(),
        vec![pair("first", "one"), pair("second", "two")],
        "the saved turns read back as pairs, oldest first"
    );
}
