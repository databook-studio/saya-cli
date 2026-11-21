//! The session's transcript: the turns that settled, and the saved record they
//! ride.
//!
//! A session outlives the run behind it, so what it remembers of the
//! conversation is what it saves: one (prompt, answer) pair per turn that
//! settled *answered*, oldest first, and every other field of the record the
//! last save wrote. A turn that stopped, paused, failed or was cancelled
//! records nothing — the same rule the run's own history follows, so a resumed
//! epoch is seeded with exactly the answers a person actually read.

use saya_store::{RedactedSession, RedactedTurn};

use super::ChatOutcome;

/// One settled turn: what the operator asked, and the answer the turn settled
/// on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SettledTurn {
    /// The operator's own words.
    pub(crate) prompt: String,
    /// The answer the turn settled on.
    pub(crate) answer: String,
}

/// A session's saved record, read as its transcript.
pub(crate) struct Transcript {
    record: RedactedSession,
}

impl Transcript {
    /// The transcript of a saved record: every pair it holds, and everything
    /// else the record carries — a field this session does not touch is a
    /// field the next save must write unchanged, so a record loaded with no
    /// run link saves back byte for byte.
    pub(crate) fn from_record(record: RedactedSession) -> Self {
        Self { record }
    }

    /// The record as it was loaded, and as the next save writes it.
    pub(crate) fn record(&self) -> &RedactedSession {
        &self.record
    }

    /// The record the next save writes.
    pub(crate) fn into_record(self) -> RedactedSession {
        self.record
    }

    /// The settled turns, oldest first: what a new epoch is seeded with.
    pub(crate) fn turns(&self) -> Vec<SettledTurn> {
        self.record
            .turns
            .iter()
            .map(|turn| SettledTurn {
                prompt: turn.user.clone(),
                answer: turn.assistant.clone(),
            })
            .collect()
    }

    /// Records one turn, and reports whether it was recorded: only a turn that
    /// settled answered is. A turn the operator cancelled, a guard paused, a
    /// budget stopped or a failure ended left no answer a person read, so the
    /// transcript — and the seed of the run that follows — carries nothing
    /// from it.
    pub(crate) fn settle(&mut self, prompt: &str, answer: &str, outcome: &ChatOutcome) -> bool {
        if !matches!(outcome, ChatOutcome::Answered) {
            return false;
        }
        self.record.turns.push(RedactedTurn {
            user: prompt.to_owned(),
            assistant: answer.to_owned(),
            database_derived: false,
            tools: Vec::new(),
        });
        true
    }
}

#[cfg(test)]
#[path = "transcript_tests.rs"]
mod tests;
