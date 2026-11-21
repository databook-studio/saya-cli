//! The session's compaction: summarise at 95% of a known window, then roll over.
//!
//! A chat session's working memory is what a run replays: the pairs of the turns
//! that settled. When a turn's own report says the run is at or past
//! [`CONTEXT_COMPACT_PERCENT`] of a window saya knows, the older turns are folded
//! into one summary by the engine's compaction — the same plan, summariser call
//! and validation `/compact` runs ([`engine`]), never a second compactor — the
//! summary is saved in the session's record, and the session rolls over with
//! `seed = summary + the newest tail pairs`: the run that follows starts from the
//! narrative and the turns the compaction kept verbatim, and the turns the
//! summary replaced are not replayed over it.
//!
//! Three rules bound the trigger:
//!
//! - **Absence is not zero.** With no answering report or an unknown window
//!   there is no decision, never a trigger.
//! - **One compaction at a time, and no retry loop.** A compaction that fails
//!   latches: the trigger stays silent until `/clear` or an explicit `/compact`
//!   re-arms it, so a session that cannot summarise does not burn the budget it
//!   was trying to save. `/compact` states the operator's own ask: one
//!   compaction now, whether or not a turn crossed the threshold.
//! - **The mode decides.** `auto` (the default) fires; `manual` compacts only on
//!   `/compact`; `off` silences the automatic trigger entirely.
//!
//! The engine's in-turn compaction at 90% stays as the last resort inside a turn:
//! this is the session's own boundary, after a turn settled, between the runs a
//! session composes.

use std::sync::atomic::{AtomicBool, Ordering};

use saya_agent::{CONTEXT_COMPACT_PERCENT, context_utilisation_percent};
use saya_config::CompactionMode;

use super::transcript::Transcript;
use crate::agent::turn_config::PromptOverrides;
use crate::commands::coding::NativeCodingJob;
use crate::interactive::session::continuation::compaction::automatic::compaction_gates;
use crate::interactive::session::continuation::compaction::engine::{self, Pass};
use crate::interactive::session::continuation::compaction::headless::last_answering_input;

/// What one settled turn's compaction did — and therefore what the surface does
/// next: nothing, a rollover asked for, or the failure to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Compacted {
    /// Nothing happened: the turn is below the threshold, the window is unknown,
    /// the mode does not compact automatically, or a failure latched the trigger.
    Idle,
    /// The summary the compaction saved in the record. The session rolls over
    /// with `RolloverCause::Compact`, seeded with this summary and the tail.
    Summarised(String),
    /// The compaction failed, in words a person reads. The trigger latched; the
    /// conversation is unchanged.
    Failed(String),
}

/// The session's compaction, and the latch a failure leaves behind.
///
/// One per session, not per run: a session opens a new run on every rollover,
/// and a compaction that failed on the run before must stay latched on the run
/// after it.
#[derive(Default)]
pub(crate) struct Compaction {
    /// Set when an automatic compaction failed: the trigger stays silent until
    /// `/clear` or an explicit `/compact` re-arms it.
    latched: AtomicBool,
}

impl Compaction {
    /// The compaction of one session's conversation.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Whether the trigger is latched: a failed compaction is never retried on
    /// its own.
    pub(crate) fn latched(&self) -> bool {
        self.latched.load(Ordering::Acquire)
    }

    /// After a turn settled: one compaction when the turn's report is at or past
    /// the threshold of a known window, and nothing otherwise.
    ///
    /// The report is the turn's own — the last answering call's `input_tokens` —
    /// so a stale numerator from an earlier turn can never fire, and a turn that
    /// was continued or budget-truncated never compacts (the gates are the
    /// automatic trigger's, read from the same typed output).
    pub(crate) async fn after_turn(
        &self,
        job: &NativeCodingJob,
        output: &saya_agent::AgentOutput,
        transcript: &mut Transcript,
    ) -> Compacted {
        if self.latched() {
            return Compacted::Idle;
        }
        if !due(job, output, transcript) {
            return Compacted::Idle;
        }
        self.compact(job, transcript).await
    }

    /// `/compact`: one compaction now, whether or not a turn crossed the
    /// threshold. The operator's ask clears the latch first, and a failure
    /// latches again — `/compact` is one try, not a retry loop.
    pub(crate) async fn force(
        &self,
        job: &NativeCodingJob,
        transcript: &mut Transcript,
    ) -> Compacted {
        self.latched.store(false, Ordering::Release);
        self.compact(job, transcript).await
    }

    /// `/clear` re-arms the trigger: the conversation that failed to summarise
    /// is gone, and so is the latch it left.
    pub(crate) fn clear(&self) {
        self.latched.store(false, Ordering::Release);
    }

    /// Runs the one compaction: the engine's plan and summariser call over the
    /// record's turns, then the summary saved in the record on success.
    async fn compact(&self, job: &NativeCodingJob, transcript: &mut Transcript) -> Compacted {
        // The turns are the conversation as the record holds it, and the history
        // is the working memory a run replays for them — the summary ahead of
        // the pairs the last compaction kept — exactly the shape the run's own
        // turns carry. Taken by value so the record can be written after the
        // call.
        let turns = transcript.record().turns.clone();
        let history = transcript.replayed();
        let result = engine::summarise(
            job.runtime_config(),
            &PromptOverrides::default(),
            &turns,
            &history,
            None,
        )
        .await;
        match result {
            Pass::Summarised { summary, .. } => {
                transcript.save_summary(&summary);
                // A summary that survived validation re-arms the trigger: the
                // working memory it leaves behind is smaller than the one it
                // replaced.
                self.latched.store(false, Ordering::Release);
                Compacted::Summarised(summary)
            }
            Pass::Failed { reason, .. } => {
                self.latched.store(true, Ordering::Release);
                Compacted::Failed(reason)
            }
            // Nothing to compact is not a failure: the conversation is short
            // enough already, and the trigger stays armed for a longer one.
            Pass::Nothing => Compacted::Idle,
        }
    }
}

/// Whether this settled turn may compact: `auto` mode, a known window, an
/// answering report at or past the threshold, and neither an admitted
/// continuation nor a budget salvage.
fn due(job: &NativeCodingJob, output: &saya_agent::AgentOutput, transcript: &Transcript) -> bool {
    let runtime = job.runtime_config();
    if runtime.resolved.ai.compaction != CompactionMode::Auto {
        return false;
    }
    let (continued, terminal_stop) = compaction_gates(Some(output));
    if continued || terminal_stop {
        return false;
    }
    let window = window(job, transcript);
    let percent = context_utilisation_percent(last_answering_input(output), window);
    percent.is_some_and(|percent| percent >= CONTEXT_COMPACT_PERCENT)
}

/// The window the session's model is known to have: the configured one when the
/// session declared it, and the table's entry for the record's model otherwise.
/// `None` is an unknown window — no decision, never a trigger.
fn window(job: &NativeCodingJob, transcript: &Transcript) -> Option<u64> {
    job.runtime_config()
        .resolved
        .ai
        .context_window_tokens
        .or_else(|| saya_config::context_window_tokens(&transcript.record().model))
}
