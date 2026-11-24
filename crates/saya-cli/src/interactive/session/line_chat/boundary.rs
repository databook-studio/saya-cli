//! The line front's turn boundary: what a settled turn does to the session's
//! working memory.
//!
//! Every settled line turn crosses the boundary the chat layer states. The turn
//! joins the session's transcript, and the session's [`Compaction`] reads **that
//! turn's own** typed output — the last answering call's `input_tokens`, with the
//! continuation and budget-salvage gates A8 defined — so a stale numerator from
//! an earlier turn can never fire. When it summarises, the run is rolled through
//! [`Rollover`] with `RolloverCause::Compact`: the run that follows is a new job,
//! seeded with the summary and the tail the compaction kept verbatim.
//!
//! `/compact` and `/clear` are the same boundary's other two triggers, never a
//! second compactor. Nothing here prints: a [`Said`] is the line the surface
//! says, on the channel its own wording already uses.

use std::future::Future;
use std::sync::Arc;

use saya_agent::AgentOutput;

use super::run::{Run, hand_over};
use crate::chat::{ChatOutcome, Compacted, Compaction, Rollover, SessionRun, Transcript};
use crate::commands::coding::NativeCodingJob;
use crate::interactive::session::commands::SessionAction;
use crate::interactive::session::continuation::compaction::automatic::{
    auto_failure_message, auto_success_message,
};
use crate::interactive::session::continuation::compaction::manual::{
    self, COMPACT_KEEP_TAIL_TURNS,
};

/// The line a `/compact` with no conversation to fold says: the engine's own
/// no-work sentence, so the operator reads the same words on every surface.
pub(crate) fn nothing_to_compact() -> Said {
    Said::Message(NOTHING_TO_COMPACT.to_owned())
}

/// What a `/compact` with nothing to fold says, as the operator's own ask.
const NOTHING_TO_COMPACT: &str = "Nothing to compact: the conversation is short enough already.";

/// One settled line turn, as the boundary reads it.
pub(super) struct Turn<'a> {
    pub(super) outcome: &'a ChatOutcome,
    pub(super) prompt: &'a str,
    pub(super) answer: &'a str,
    pub(super) output: Option<AgentOutput>,
}

/// The line a boundary event states, on the channel it belongs on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Said {
    Nothing,
    /// A line for the result channel.
    Message(String),
    /// A line for the error channel.
    Error(String),
}

impl Said {
    /// The action the surface emits, when there is one to emit.
    pub(crate) fn action(&self) -> Option<SessionAction> {
        match self {
            Self::Nothing => None,
            Self::Message(message) => Some(SessionAction::Message(message.clone())),
            Self::Error(message) => Some(SessionAction::Error(message.clone())),
        }
    }
}

/// What one turn's boundary did.
pub(super) enum Bounded {
    /// Nothing compacted — or the compaction failed and latched: the session
    /// stays on the run it was on and says this.
    Kept(Said),
    /// The compaction summarised and the run was rolled.
    Rolled {
        successor: Box<crate::chat::RunComposition>,
        said: Said,
    },
    /// The compaction summarised but the run it ended could not be replaced:
    /// that run is closed, so the session has none left.
    Ended(Said),
}

/// One settled turn's whole boundary: the transcript takes the turn, the
/// session's compaction reads its own typed output, and a compaction rolls the
/// run over — exactly one `after_turn`, at most one rollover.
///
/// `run` is the session's run as the boundary finds it and gives it up: the job
/// is borrowed only for the decision, and the run — the object that owns the
/// folder's lease — is handed over only when a compaction actually summarised.
/// `next` states the successor and is called only then.
pub(super) async fn after_turn<F, Fut>(
    compaction: &Compaction,
    rollover: &Rollover,
    run: &mut Option<Run>,
    transcript: &mut Transcript,
    turn: &Turn<'_>,
    next: F,
) -> Bounded
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<SessionRun, String>>,
{
    let compacted = match run.as_ref().map(Run::job) {
        Some(job) => settle(compaction, job, transcript, turn).await,
        None => return Bounded::Kept(Said::Nothing),
    };
    let turns = transcript.turns().len();
    if !matches!(compacted, Compacted::Summarised(_)) {
        return Bounded::Kept(word(&compacted, turns, true));
    }
    rolled(compacted, turns, true, rollover, run, transcript, next).await
}

/// `/compact`: one compaction now, whether or not a turn crossed the threshold,
/// and the same rollover the automatic trigger asks for.
pub(super) async fn forced<F, Fut>(
    compaction: &Compaction,
    rollover: &Rollover,
    run: &mut Option<Run>,
    transcript: &mut Transcript,
    next: F,
) -> Bounded
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<SessionRun, String>>,
{
    let compacted = match run.as_ref().map(Run::job) {
        Some(job) => compaction.force(job, transcript).await,
        None => return Bounded::Kept(Said::Nothing),
    };
    let turns = transcript.turns().len();
    if !matches!(compacted, Compacted::Summarised(_)) {
        return Bounded::Kept(word(&compacted, turns, false));
    }
    rolled(compacted, turns, false, rollover, run, transcript, next).await
}

/// `/clear`: the latch is re-armed, the conversation is forgotten and the run is
/// rolled with nothing. The only line is a refusal: the state layer already says
/// the conversation was cleared.
pub(super) async fn cleared<F, Fut>(
    rollover: &Rollover,
    run: &mut Option<Run>,
    transcript: &mut Transcript,
    next: F,
) -> Bounded
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<SessionRun, String>>,
{
    let next = match next().await {
        Ok(next) => next,
        Err(reason) => return Bounded::Kept(Said::Error(not_replaced(&reason))),
    };
    match rollover
        .roll(
            hand_over(run.take()),
            crate::chat::RolloverCause::Clear,
            next,
            transcript,
        )
        .await
    {
        Ok(successor) => Bounded::Rolled {
            successor: Box::new(successor),
            said: Said::Nothing,
        },
        Err(error) => Bounded::Ended(Said::Error(not_replaced(&error.to_string()))),
    }
}

/// The transcript takes the turn and the session's compaction reads that turn's
/// own typed output: exactly one `after_turn` per settled turn.
pub(super) async fn settle(
    compaction: &Compaction,
    job: &Arc<NativeCodingJob>,
    transcript: &mut Transcript,
    turn: &Turn<'_>,
) -> Compacted {
    // A turn that did not answer leaves no words a person read, so there is
    // nothing to shrink — the rule the run's own history follows.
    if !transcript.settle(turn.prompt, turn.answer, turn.outcome) {
        return Compacted::Idle;
    }
    let Some(output) = turn.output.as_ref() else {
        return Compacted::Idle;
    };
    compaction.after_turn(job, output, transcript).await
}

/// One compaction that summarised, and the rollover it asks for.
async fn rolled<F, Fut>(
    compacted: Compacted,
    turns: usize,
    automatic: bool,
    rollover: &Rollover,
    run: &mut Option<Run>,
    transcript: &mut Transcript,
    next: F,
) -> Bounded
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<SessionRun, String>>,
{
    // The count the success line names is the plan's own.
    let compacted_turns = turns.saturating_sub(COMPACT_KEEP_TAIL_TURNS);
    let said = word(&compacted, compacted_turns, automatic);
    let next = match next().await {
        Ok(next) => next,
        Err(reason) => return Bounded::Kept(Said::Error(not_replaced(&reason))),
    };
    match rollover
        .roll(
            hand_over(run.take()),
            crate::chat::RolloverCause::Compact,
            next,
            transcript,
        )
        .await
    {
        Ok(successor) => Bounded::Rolled {
            successor: Box::new(successor),
            said,
        },
        Err(error) => Bounded::Ended(Said::Error(not_replaced(&error.to_string()))),
    }
}

/// The line a compaction says, in its trigger's own voice.
fn word(compacted: &Compacted, compacted_turns: usize, automatic: bool) -> Said {
    match compacted {
        Compacted::Summarised(summary) => Said::Message(if automatic {
            auto_success_message(compacted_turns, summary)
        } else {
            manual::success_message(compacted_turns, summary)
        }),
        Compacted::Failed(reason) => Said::Error(if automatic {
            auto_failure_message(reason)
        } else {
            manual::failure_message(reason)
        }),
        Compacted::Idle if automatic => Said::Nothing,
        Compacted::Idle => Said::Message(NOTHING_TO_COMPACT.to_owned()),
    }
}

/// Why the run after a compaction never opened, in words a person reads.
fn not_replaced(reason: &str) -> String {
    format!("the run after the compaction could not be prepared: {reason}")
}
