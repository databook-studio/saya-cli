//! Round-trip reconstruction: catch SQL that answers a *different* question
//! than the one asked.
//!
//! The dangerous failure is SQL that runs cleanly and computes the wrong
//! thing — no error, a plausible table, nothing to notice. Re-running the
//! agent does not catch it: independent attempts agree on the wrong answer
//! about as readily as the right one.
//!
//! Round-tripping catches what agreement cannot. Show a fresh context ONLY the
//! SQL (and the shape of its result) and ask what question it answers. Then
//! judge whether that restatement answers the same question that was asked.
//! If it does not, the SQL computes something else.
//!
//! This module is the mechanism and its tests only — it is not wired into the
//! agent loop, the `ask` path, config, or the CLI. A later change decides when
//! it runs, after its predictive power has been measured.

use async_trait::async_trait;
use saya_agent::CancellationToken;

mod prompts;

/// Shape of a result, with no values: a row count and column names. Carrying
/// values to the restater would let it read the answer off the rows and echo
/// the question; the shape carries only what is needed to interpret the SQL.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ResultShape {
    pub row_count: usize,
    pub columns: Vec<String>,
}

/// A judge's verdict on whether a restatement answers the asked question.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Verdict {
    /// True when the restatement answers the question that was asked.
    pub agrees: bool,
    /// One short sentence on what differs. Present whether or not it agrees,
    /// so a reader can weigh the judgement rather than take a bare boolean.
    pub reason: String,
}

/// The outcome of one round-trip.
pub(crate) struct RoundTrip {
    /// What the model restated the SQL as, when it restated at all.
    pub restatement: Option<String>,
    /// The judge's verdict, when one was reached.
    pub verdict: Option<Verdict>,
    /// True only when a verdict was reached AND it disagreed. Absent evidence
    /// is never a disagreement — a verifier that reports trouble when it
    /// simply failed is worse than one that stays quiet.
    pub diverged: bool,
}

/// Asks a model to restate what a statement computes and to judge whether that
/// restatement answers the asked question. Exists so the logic is testable
/// against a fake, with no provider and no network — mirroring
/// [`crate::agent::decide::CandidateExecutor`].
#[async_trait]
pub(crate) trait Restater: Sync {
    /// Restates what a statement computes, having been shown ONLY the
    /// statement and the shape of its result. The question is deliberately not
    /// a parameter: a restater that can see the question will echo it and the
    /// signal disappears.
    async fn restate(&self, sql: &str, shape: &ResultShape) -> Option<String>;
    /// Judges whether a restatement answers the same question that was asked.
    async fn judge(&self, question: &str, restatement: &str) -> Option<Verdict>;
}

/// Run one round-trip: restate what `sql` computes (blind to `question`), then
/// judge whether that restatement answers `question`. See the module docs for
/// why the two steps are split.
///
/// `diverged` is set only by an explicit disagreeing verdict — absent
/// evidence (no restatement, no verdict, or cancellation) is never a
/// disagreement.
pub(crate) async fn round_trip(
    question: &str,
    sql: &str,
    shape: &ResultShape,
    restater: &dyn Restater,
    cancellation: &CancellationToken,
) -> RoundTrip {
    // A cancelled round-trip is never a considered one. Check before each model
    // call so cancellation is honored as soon as it is observed.
    if cancellation.is_cancelled() {
        return cancelled();
    }
    // Step 1 — restate, blind to the question. No restatement means the model
    // declined or could not restate; there is nothing to judge, so judge is
    // not called and the round-trip reports no divergence.
    let Some(restatement) = restater.restate(sql, shape).await else {
        return RoundTrip {
            restatement: None,
            verdict: None,
            diverged: false,
        };
    };
    if cancellation.is_cancelled() {
        return cancelled();
    }
    // Step 2 — judge. A missing verdict is absent evidence: never a
    // disagreement. The restatement is preserved so a reader can still see
    // what the model thought the SQL computed.
    let Some(verdict) = restater.judge(question, &restatement).await else {
        return RoundTrip {
            restatement: Some(restatement),
            verdict: None,
            diverged: false,
        };
    };
    let diverged = !verdict.agrees;
    RoundTrip {
        restatement: Some(restatement),
        verdict: Some(verdict),
        diverged,
    }
}

/// A cancelled round-trip carries no restatement and no verdict — it is not a
/// considered result, mirroring `decide::cancelled`.
fn cancelled() -> RoundTrip {
    RoundTrip {
        restatement: None,
        verdict: None,
        diverged: false,
    }
}

#[cfg(test)]
#[path = "../roundtrip_tests.rs"]
mod tests;
