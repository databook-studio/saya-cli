//! The turns a run has completed, and the history its next turn replays.
//!
//! Chat builds a turn's history from the (user prompt, assistant answer)
//! pairs of the turns before it — tool arguments and results are never
//! replayed, only the prompt and the final answer text
//! (`.../interactive/session/state/recording.rs`, `provider_request_history`)
//! — and leaves the bounding to the one shared assembly every run goes
//! through: `saya_agent::build_messages`, called by the agent loop against
//! `AgentLimits::context_byte_budget`. This module keeps the same list for an
//! open job run, so a further turn carries the same shape of history as the
//! next chat message and is bounded by the same function, against the same
//! budget: the newest pairs kept, the oldest dropped from the front.

use saya_agent::ChatMessage;
use std::sync::{Mutex, MutexGuard, PoisonError};

/// One completed turn of the run: the message the owner sent and the final
/// answer the turn settled on.
struct CompletedTurn {
    prompt: String,
    answer: String,
}

/// The run's completed turns, oldest first.
///
/// Append-only, so a later turn's history begins with exactly the history the
/// turn before it carried whenever the bound dropped nothing — the provider's
/// prompt cache then keeps what it already has.
pub(in crate::commands::coding::parent) struct TurnHistory {
    turns: Mutex<Vec<CompletedTurn>>,
}

impl TurnHistory {
    pub(in crate::commands::coding::parent) fn new() -> Self {
        Self {
            turns: Mutex::new(Vec::new()),
        }
    }

    /// Records a turn that completed, with the answer it settled on — a
    /// budget-partial answer included. A turn that failed or was cancelled is
    /// not recorded at all: chat writes no turn for one either, recording only
    /// the `Ok` arm of its settle (`.../application/streaming/settle.rs`), so
    /// the same case replays nothing.
    pub(in crate::commands::coding::parent) fn record(&self, prompt: &str, answer: &str) {
        self.lock().push(CompletedTurn {
            prompt: prompt.to_owned(),
            answer: answer.to_owned(),
        });
    }

    /// The history the turn about to run replays: one (user, assistant) pair
    /// per completed turn, oldest first, in chat's own shape. Empty for a
    /// run's first turn, so that turn's request is unchanged.
    ///
    /// Unbounded here on purpose — the shared assembly trims it to the run's
    /// context byte budget exactly as it trims chat's, so a job can never
    /// bound history by a rule of its own.
    pub(in crate::commands::coding::parent) fn replay(&self) -> Vec<ChatMessage> {
        self.lock()
            .iter()
            .flat_map(|turn: &CompletedTurn| {
                [
                    ChatMessage::text("user", turn.prompt.clone()),
                    ChatMessage::text("assistant", turn.answer.clone()),
                ]
            })
            .collect()
    }

    fn lock(&self) -> MutexGuard<'_, Vec<CompletedTurn>> {
        self.turns.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
