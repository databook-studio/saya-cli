//! The line front's turn boundary: one settled turn over the threshold runs
//! exactly one `after_turn` and one rollover; a turn below it runs neither.
//!
//! The test drives the chat layer's own [`Compaction`] and [`Rollover`] over a
//! real job and a real provider — the pieces the line front wires — and counts
//! how many summarising requests arrived and how often the run was replaced.
//! The turns are the engine's own output shape: an answering call that reports
//! the context it measured, which is the number the trigger reads.

#![cfg(unix)]

use std::cell::Cell;
use std::sync::Arc;

use saya_agent::{AgentEvent, AgentOutput, TokenUsage, UsageCall};
use saya_store::RedactedSession;

use super::boundary::{self, Bounded, Said, Turn};
use super::boundary_fixture::{Fixture, next_run, session};
use super::run::Run;
use crate::chat::{ChatFront, ChatOutcome, Compaction, Rollover, Transcript};

/// The phrase every summarising request carries: the engine's own prompt.
const SUMMARISE: &str = "Summarise the following earlier conversation turns";

/// The four prompts a compaction needs: the plan folds the older turns and
/// keeps the newest two verbatim.
const FOUR: [&str; 4] = [
    "first message",
    "second message",
    "third message",
    "fourth message",
];

/// The window the fixture's run declares: a known one, so the trigger has a
/// numerator to read and a denominator to read it against.
const WINDOW: u64 = 128_000;

/// An answering call that measured 96% of it: past the threshold.
const AT_96: u64 = 122_880;

/// An answering call that measured 31% of it: nowhere near it.
const AT_31: u64 = 40_000;

#[tokio::test]
async fn a_turn_over_the_threshold_compacts_once_and_rolls_over_and_below_it_does_neither() {
    // Over the threshold: four settled turns, the fourth crossing it.
    let (rolled, said, summarised, saved) = cross_boundary(
        session("line-boundary-over", WINDOW).await,
        "over-next",
        AT_96,
    )
    .await;
    assert_eq!(rolled, 1, "exactly one rollover for the one compaction");
    assert_eq!(
        summarised, 1,
        "exactly one after_turn summarised: one provider request"
    );
    assert!(saved, "the summary is saved in the session's record");
    assert!(
        matches!(&said, Said::Message(message)
            if message.starts_with("Automatic compaction: Compacted 2 turns")),
        "the automatic success line names the folded turns: {said:?}"
    );

    // Below it: the same conversation against a numerator none of it fills.
    let (rolled, said, summarised, saved) = cross_boundary(
        session("line-boundary-below", WINDOW).await,
        "below-next",
        AT_31,
    )
    .await;
    assert_eq!(rolled, 0, "no rollover below the threshold");
    assert_eq!(summarised, 0, "and no summarising call");
    assert!(!saved, "and nothing saved");
    assert_eq!(said, Said::Nothing, "and nothing to say");
}

/// Every settled turn crosses the boundary as the line front states it, with
/// the run the session is on handed over only when one compacts. Returns how
/// many times the run was replaced, what the boundary said last, how many
/// summarising requests arrived, and whether a summary was saved.
async fn cross_boundary(fixture: Fixture, next: &str, tokens: u64) -> (usize, Said, usize, bool) {
    let compaction = Compaction::new();
    let rollover = Rollover::new(Arc::clone(&fixture.application));
    let mut transcript = Transcript::from_record(RedactedSession {
        id: "line-boundary".to_owned(),
        ..RedactedSession::default()
    });
    // The run the session is on: handed over by the boundary, replaced by the
    // successor a compaction rolls into. No other handle to the job is kept —
    // the run object owns the folder's lease.
    let front = Arc::clone(&fixture.front);
    let chat_front = Arc::clone(&front) as Arc<dyn ChatFront>;
    let mut run = Some(Run::opened(fixture.run, Arc::clone(&chat_front), None).await);
    let composes = Cell::new(0);
    let (mut rolled, mut said) = (0, Said::Nothing);
    for prompt in FOUR {
        let answer = format!("the answer to {prompt}");
        let answered = ChatOutcome::Answered;
        let turn = Turn {
            outcome: &answered,
            prompt,
            answer: &answer,
            output: Some(output(tokens)),
        };
        let name = next.to_owned();
        let counter = &composes;
        let compose = move || {
            counter.set(counter.get() + 1);
            std::future::ready(Ok(next_run(&name)))
        };
        let bounded = boundary::after_turn(
            &compaction,
            &rollover,
            &mut run,
            &mut transcript,
            &turn,
            compose,
        )
        .await;
        match bounded {
            Bounded::Kept(line) => said = line,
            Bounded::Rolled {
                successor,
                said: line,
            } => {
                rolled += 1;
                said = line;
                run = Some(Run::opened(*successor, Arc::clone(&chat_front), None).await);
            }
            Bounded::Ended(line) => {
                panic!("the run after the compaction could not open: {line:?}")
            }
        }
    }
    assert_eq!(
        composes.get(),
        rolled,
        "a successor is composed once per compaction, and never without one"
    );
    assert!(run.is_some(), "the session still holds a run");
    let summarised = fixture
        .provider
        .bodies()
        .iter()
        .filter(|body| body.contains(SUMMARISE))
        .count();
    assert_eq!(
        fixture.provider.bodies().len(),
        summarised,
        "the only request the provider saw is the summarising call"
    );
    if let Some(body) = fixture
        .provider
        .bodies()
        .iter()
        .find(|b| b.contains(SUMMARISE))
    {
        assert!(
            body.contains("first message"),
            "the plan reads the session's own turns: {body}"
        );
    }
    assert_eq!(
        transcript.turns().len(),
        FOUR.len(),
        "the transcript keeps every turn it settled"
    );
    (
        rolled,
        said,
        summarised,
        transcript.record().compaction_summary.is_some(),
    )
}

/// One turn's typed output, as the engine states it: the answering call's
/// report of the context it measured — the number the trigger reads.
fn output(input_tokens: u64) -> AgentOutput {
    // A non-exhaustive struct from another crate: stated field by field.
    let mut usage = TokenUsage::default();
    usage.input_tokens = input_tokens;
    AgentOutput {
        answer: "the turn answered".to_owned(),
        events: vec![AgentEvent::Usage {
            call: UsageCall::Answer,
            usage,
        }],
        used_bounded_sql_query: false,
        tool_metadata: Vec::new(),
        usage: TokenUsage::default(),
        learning_usage: None,
        truncated: false,
        answer_sql: None,
    }
}
