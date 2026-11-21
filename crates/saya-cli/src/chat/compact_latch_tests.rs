//! A compaction that fails latches: no retry loop, one attempt per settled turn
//! at most, until `/clear` or an explicit `/compact` re-arms the trigger.
//!
//! The fixture provider answers the summarising call with an empty summary, which
//! the engine's validation refuses — a real failure of the real operation against
//! the real job engine, not a stubbed verdict.

#![cfg(unix)]

use super::compact::{Compacted, Compaction};
use super::compact_fixture::{open, record, settled};
use super::compact_provider::{AT_96, KNOWN_MODEL, Reporting};
use super::operator_front_fixture;

/// The turn whose text the summary must preserve: a line the engine pins, so a
/// summary that drops it is refused — and a dropped pin is the model's answer,
/// not the wire's, so it is never retried.
const PINNED: &str = "select count(*) from users";

/// Test 3: the failing compaction is tried once, the next settled turn at the
/// same 96% does not try again, and `/compact` clears the latch and tries once —
/// which then succeeds and re-arms the trigger.
#[tokio::test]
async fn a_failed_compaction_latches_until_slash_compact_clears_it() {
    // The first summary drops the pinned statement, which the engine's
    // validation refuses: the summariser spoke, and a second ask is a second hope.
    let provider = Reporting::start(AT_96, "a summary that forgot the query");
    let front = operator_front_fixture::non_interactive();
    let session = open("compact-latch", &provider, &front, "compact-latch").await;
    let mut transcript = record("compact-latch", KNOWN_MODEL);
    let compaction = Compaction::new();

    let mut output = None;
    for prompt in [PINNED, "second message", "third message", "fourth message"] {
        output = Some(settled(session.run.job(), &mut transcript, prompt).await);
    }
    let output = output.expect("four turns settled");

    let compacted = compaction
        .after_turn(session.run.job(), &output, &mut transcript)
        .await;
    assert!(
        matches!(compacted, Compacted::Failed(_)),
        "a summary that cannot be applied fails the compaction: {compacted:?}"
    );
    assert_eq!(
        provider.summarised(),
        1,
        "one attempt: a failed summarising call is not retried on its own"
    );
    assert!(
        compaction.latched(),
        "the failure latches the trigger until /clear or /compact"
    );
    assert!(
        transcript.record().compaction_summary.is_none(),
        "a failed compaction leaves the record exactly as it was"
    );

    // The next settled turn is at 96% again, and the trigger stays silent.
    let output = settled(session.run.job(), &mut transcript, "fifth message").await;
    assert_eq!(
        compaction
            .after_turn(session.run.job(), &output, &mut transcript)
            .await,
        Compacted::Idle,
        "a latched trigger does not try again on the next settled turn"
    );
    assert_eq!(
        provider.summarised(),
        1,
        "the latched trigger runs no second summarising call"
    );

    // `/compact` is the operator's own ask: it clears the latch and tries once.
    // This summary keeps the pinned statement, so the second try is applied.
    let good = format!("the first two turns ran {PINNED} and were folded after all");
    provider.set_summary(&good);
    assert_eq!(
        compaction.force(session.run.job(), &mut transcript).await,
        Compacted::Summarised(good.clone()),
        "/compact compacts now, whether or not a turn crossed the threshold"
    );
    assert_eq!(
        provider.summarised(),
        2,
        "/compact is one more attempt, not a retry loop"
    );
    assert!(
        !compaction.latched(),
        "a compaction that succeeded re-arms the trigger"
    );
    assert_eq!(
        transcript.record().compaction_summary.as_deref(),
        Some(good.as_str()),
        "the summary of the successful compaction is saved in the record"
    );

    // And `/clear` re-arms a latched trigger too: the conversation it failed to
    // summarise is gone.
    compaction.clear();
    assert!(
        !compaction.latched(),
        "/clear re-arms the automatic-compaction trigger"
    );
}
