//! What one chat session says to its front, and how a turn ended.
//!
//! A session talks to exactly one thing: its front. Every value below is a
//! contract a front projects — nothing here decides how it is shown.
//!
//! The turn's own reading of its stop lives with the engine
//! ([`TurnStop`]); [`ChatOutcome::from_stop`] is the chat side of it, so the
//! wording a stop carries survives unchanged and only the class is re-framed.

use std::path::PathBuf;

use saya_agent::{AgentEvent, BudgetDimension};

use crate::agent::tools::CaptureEvent;
use crate::commands::coding::TurnStop;

/// One session event, as the surface receives it.
#[derive(Debug)]
pub(crate) enum ChatEvent {
    /// The run is prepared and the session takes messages: `db` names the
    /// connected profile when there is one, `workspace` the run's root when
    /// the session opened a workspace.
    Ready {
        db: Option<String>,
        workspace: Option<PathBuf>,
    },
    /// A call with an effect is put to the operator and waits for its answer.
    Question(Question),
    /// One turn ended, and how it ended.
    TurnEnded(ChatOutcome),
    /// What a successful query handed the surface — the typed result or the
    /// refusal that kept it — for the preview, `/export`, `/chart` and
    /// `/investigation save` to read.
    QueryCaptured(CaptureEvent),
    /// One event of the engine's own loop, passed through unchanged: text,
    /// tool steps, metrics and recovery all reach the front on this one
    /// stream.
    Agent(AgentEvent),
}

/// One call put to the user: the id it is answered under, who asks, what the
/// call does, and the grant a session-wide yes records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Question {
    /// The session's own number for this question. One is pending at a time;
    /// a later asker waits its turn rather than being refused.
    pub(crate) id: u64,
    /// The caller's line in the asking tool's own words, so one front can put
    /// every caller's call to the user the same way.
    pub(crate) caller: String,
    /// What the call does, as the facts body the user reads before answering.
    pub(crate) facts: String,
    /// The grammar token a session-wide yes records, when the call suggests
    /// one.
    pub(crate) grant: Option<String>,
}

/// How one turn ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ChatOutcome {
    /// The turn finished and the model answered, with or without tool calls.
    Answered,
    /// The turn's own question budget stopped it before it answered in full.
    /// The run keeps taking turns, and the next one starts unbudgeted.
    Stopped(Budget),
    /// A progress guard stopped the turn. It is a pause: the operator's next
    /// message continues from where it stopped.
    Paused {
        /// The guard's own sentence, naming the key to raise.
        message: String,
    },
    /// Something else stopped the turn — a backstop included. A backstop is a
    /// runaway bound the operator raised, never an invitation to continue.
    Failed {
        /// What stopped it, in the words of whoever stopped it.
        message: String,
    },
    /// The operator cancelled the turn. Nothing is recorded and the run stays
    /// open, so the next message is a fresh turn.
    Cancelled,
}

/// Which bound of the turn's own question budget stopped it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Budget {
    /// The dimension the budget ran out on.
    pub(crate) dimension: BudgetDimension,
}

impl ChatOutcome {
    /// The reading of one turn: its latched stop, and whether the operator
    /// cancelled it.
    ///
    /// A cancel outranks the stop, because a cancelled turn's stop is the
    /// cancellation working — nothing is recorded from it and the run stays
    /// open. With no stop and no cancel the turn answered.
    pub(crate) fn from_stop(stop: Option<TurnStop>, cancelled: bool) -> Self {
        if cancelled {
            return Self::Cancelled;
        }
        match stop {
            None => Self::Answered,
            Some(TurnStop::Budget { dimension, .. }) => Self::Stopped(Budget { dimension }),
            Some(TurnStop::Paused { message }) => Self::Paused { message },
            Some(TurnStop::Failed { message }) => Self::Failed { message },
        }
    }
}

#[cfg(test)]
#[path = "event_tests.rs"]
mod tests;
