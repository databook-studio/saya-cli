//! A settled turn at or past the threshold of a known window compacts once and
//! rolls the session over; a turn below it — or a session with no known window —
//! does nothing at all.
//!
//! Each test drives the real job engine and the reporting fixture provider, so
//! what is asserted is how many summarising calls ran, what the record saved, and
//! what the run after the compaction actually sent.

#![cfg(unix)]

use std::sync::Arc;

use super::compact::{Compacted, Compaction};
use super::compact_fixture::{open, pairs, record, settled};
use super::compact_provider::{AT_94, AT_96, KNOWN_MODEL, Reporting};
use super::operator_front_fixture;
use super::rollover::{Ending, Rollover, RolloverCause};
use super::rollover_fixture::{messages, next_run};

/// The four prompts a compaction needs: the plan folds the older turns and keeps
/// the newest two verbatim, so a shorter conversation has nothing to compact.
const FOUR: [&str; 4] = [
    "first message",
    "second message",
    "third message",
    "fourth message",
];

/// Test 1: a settled turn at 96% of a known window runs exactly one compaction,
/// saves its summary in the record, and rolls over seeded with that summary and
/// the newest tail pairs.
#[tokio::test]
async fn a_turn_at_96_percent_compacts_once_and_rolls_over_from_the_summary() {
    let summary = "the first two turns were about the users table";
    let provider = Reporting::start(AT_96, summary);
    let front = operator_front_fixture::non_interactive();
    let session = open("compact-96", &provider, &front, "compact-96").await;
    let mut transcript = record("compact-96", KNOWN_MODEL);
    let compaction = Compaction::new();

    let mut output = None;
    for prompt in FOUR {
        output = Some(settled(session.run.job(), &mut transcript, prompt).await);
    }
    let output = output.expect("four turns settled");

    let compacted = compaction
        .after_turn(session.run.job(), &output, &mut transcript)
        .await;
    assert_eq!(
        compacted,
        Compacted::Summarised(summary.to_owned()),
        "96% of a known window compacts"
    );
    assert_eq!(
        provider.summarised(),
        1,
        "exactly one compaction runs for the turn that crossed the threshold"
    );
    assert_eq!(
        transcript.record().compaction_summary.as_deref(),
        Some(summary),
        "the summary is saved in the session's record"
    );
    assert_eq!(
        pairs(&transcript).len(),
        4,
        "the transcript keeps every turn it settled: what was said does not change"
    );

    // The rollover the compaction asks for: a new run, seeded from the summary
    // and the two pairs the compaction kept verbatim.
    let rollover = Rollover::new(Arc::clone(&session.application));
    let rolled = rollover
        .roll(
            Ending::new(session.run),
            RolloverCause::Compact,
            next_run(front.front(), "compact-96-2"),
            &mut transcript,
        )
        .await
        .expect("the compaction rollover prepares the next run");

    let answered = settled(rolled.job(), &mut transcript, "fifth message").await;
    assert_eq!(answered.answer, "turn 4 answered");
    let sent = messages(&provider.last_body());
    assert_eq!(
        sent.len(),
        8,
        "the next run's first request is its system message, the summary pair, the \
         two tail pairs and its prompt: {sent:?}"
    );
    assert_eq!(
        sent[1],
        ("user".to_owned(), summary.to_owned()),
        "the summary stands at the front of the run that follows the compaction: {sent:?}"
    );
    assert!(
        sent[2].0 == "assistant" && !sent[2].1.is_empty(),
        "the summary rides as one pair, its answer the assistant's acknowledgement: {sent:?}"
    );
    assert_eq!(
        &sent[3..7],
        [
            ("user".to_owned(), "third message".to_owned()),
            ("assistant".to_owned(), "turn 2 answered".to_owned()),
            ("user".to_owned(), "fourth message".to_owned()),
            ("assistant".to_owned(), "turn 3 answered".to_owned()),
        ],
        "the pairs the compaction kept verbatim follow the summary: {sent:?}"
    );
    assert!(
        sent[7].0 == "user" && sent[7].1.contains("fifth message"),
        "the new prompt follows them: {sent:?}"
    );
}

/// Test 2: 94% of a known window is below the threshold, and a session whose
/// model no window is known for has no decision at all — neither compacts.
#[tokio::test]
async fn below_the_threshold_and_an_unknown_window_compact_nothing() {
    let front = operator_front_fixture::non_interactive();

    let below = Reporting::start(AT_94, "a summary nobody asked for");
    let session = open("compact-94", &below, &front, "compact-94").await;
    let mut transcript = record("compact-94", KNOWN_MODEL);
    let compaction = Compaction::new();
    let mut output = None;
    for prompt in FOUR {
        output = Some(settled(session.run.job(), &mut transcript, prompt).await);
    }
    let output = output.expect("four turns settled");
    assert_eq!(
        compaction
            .after_turn(session.run.job(), &output, &mut transcript)
            .await,
        Compacted::Idle,
        "94% of a known window is below the compaction threshold"
    );
    assert_eq!(
        below.summarised(),
        0,
        "no summarising call runs below the threshold"
    );
    assert!(
        transcript.record().compaction_summary.is_none(),
        "no summary is saved below the threshold"
    );

    let unknown = Reporting::start(AT_96, "a summary nobody asked for");
    let session = open("compact-unknown", &unknown, &front, "compact-unknown").await;
    let mut transcript = record("compact-unknown", "a-model-no-table-knows");
    let compaction = Compaction::new();
    let mut output = None;
    for prompt in FOUR {
        output = Some(settled(session.run.job(), &mut transcript, prompt).await);
    }
    let output = output.expect("four turns settled");
    assert_eq!(
        compaction
            .after_turn(session.run.job(), &output, &mut transcript)
            .await,
        Compacted::Idle,
        "an unknown window is no decision, never a trigger"
    );
    assert_eq!(
        unknown.summarised(),
        0,
        "a session with no known window never compacts"
    );
    assert_eq!(
        unknown.input_tokens(),
        AT_96,
        "the report is absent-and-known the other way round: the window is what \
         is unknown, not the numerator"
    );
    assert!(
        !compaction.latched(),
        "nothing failed: an unknown window is not a latch"
    );
}
