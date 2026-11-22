//! Bare `/continue`: the turn a pause asks for.
//!
//! A progress guard stopping a turn is a **pause** — `TurnStop::Paused`, read
//! by chat as `ChatOutcome::Paused` — and the run stays open: the operator's
//! next message continues from where it stopped. `/continue` is that message.
//! It sends the literal `Continue.` through the same path a typed line uses
//! (`ChatSession::send`), so there is one admission rule, one queue and one
//! front event, and never a second way into the run (approved decision 11).
//!
//! Two rules, each with its own one sentence:
//!
//! * the last turn did not pause — there is nothing to continue;
//! * a turn is running — refused, not queued (A9's rule: a session op never
//!   runs while a turn runs, and `/continue` is one).

use super::{ChatOutcome, session::SessionState};

/// The literal message a pause is continued with: the words the operator would
/// have typed, so the next turn is an ordinary turn of the same run.
pub(crate) const CONTINUE_MESSAGE: &str = "Continue.";

/// What a session whose last turn did not pause answers. One sentence, pinned
/// by the tests.
pub(crate) const NOTHING_TO_CONTINUE: &str = "nothing to continue — the last turn did not pause";

/// What a session with a turn in flight answers. It is never queued behind the
/// turn: a session op waits for the turn to end, it does not ride along.
pub(crate) const TURN_RUNNING: &str =
    "a turn is running: /continue continues a paused turn, so it waits until this one ends";

/// Why bare `/continue` sent nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// The last turn answered, failed, was cancelled, or never ran.
    NothingToContinue,
    /// A turn, or the run's prepare, is in flight.
    TurnRunning,
}

impl Refusal {
    /// The sentence the operator reads. One wording per refusal, so the two
    /// surfaces cannot describe the same state differently.
    pub(crate) const fn sentence(self) -> &'static str {
        match self {
            Self::NothingToContinue => NOTHING_TO_CONTINUE,
            Self::TurnRunning => TURN_RUNNING,
        }
    }
}

/// Whether `last` is a turn that paused — the only turn `/continue` continues.
/// A budget stop is chat's partial answer and a backstop is a failure, so
/// neither is a pause.
pub(crate) fn paused(last: Option<&ChatOutcome>) -> bool {
    matches!(last, Some(ChatOutcome::Paused { .. }))
}

/// The refusal `/continue` answers, or `None` when the continuation message is
/// to be sent. `last` is how this session's last turn ended; `phase` is where
/// the session is.
pub(crate) fn refusal(last: Option<&ChatOutcome>, phase: SessionState) -> Option<Refusal> {
    match phase {
        // A prepare in flight is the run opening; a message sent now would be
        // held, and `/continue` is not a message the operator wants held.
        SessionState::Preparing | SessionState::Busy => Some(Refusal::TurnRunning),
        _ if paused(last) => None,
        _ => Some(Refusal::NothingToContinue),
    }
}
