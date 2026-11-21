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

/// The conversation a run carries, oldest first: the pairs it was seeded with
/// when it opened, then the turns it has completed.
///
/// Append-only in its completed turns, so a later turn's history begins with
/// exactly the history the turn before it carried whenever the bound dropped
/// nothing — the provider's prompt cache then keeps what it already has.
pub(in crate::commands::coding::parent) struct TurnHistory {
    /// The conversation the run was seeded with when it opened, in the order it
    /// arrived: a session replays the transcript of the run before it into the
    /// run it prepares. It stays at the front of every turn's history, so the
    /// first turn already carries the conversation it continues.
    seed: Vec<CompletedTurn>,
    turns: Mutex<Vec<CompletedTurn>>,
}

impl TurnHistory {
    /// The history a run opens with: the seeded pairs, oldest first, ahead of a
    /// list of completed turns that starts empty.
    ///
    /// An empty seed is the run that begins with no conversation, which is
    /// every job surface today — its first turn sends exactly the bytes it
    /// always did.
    pub(in crate::commands::coding::parent) fn seeded(pairs: Vec<(String, String)>) -> Self {
        Self {
            seed: pairs
                .into_iter()
                .map(|(prompt, answer)| CompletedTurn { prompt, answer })
                .collect(),
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
    /// per seeded and completed turn, oldest first, in chat's own shape. Empty
    /// for a run seeded with nothing, before its first turn, so that turn's
    /// request is unchanged.
    ///
    /// Unbounded here on purpose — the shared assembly trims it to the run's
    /// context byte budget exactly as it trims chat's, so a job can never
    /// bound history by a rule of its own.
    pub(in crate::commands::coding::parent) fn replay(&self) -> Vec<ChatMessage> {
        self.seed
            .iter()
            .chain(self.lock().iter())
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
