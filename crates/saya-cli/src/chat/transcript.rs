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
use crate::interactive::session::continuation::compaction::manual::COMPACT_KEEP_TAIL_TURNS;

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
    /// How many of the record's oldest turns the last compaction folded into
    /// the summary: the pairs a run no longer replays. Kept in memory, because
    /// the record states the summary and never the fold — a record loaded from
    /// disk is a conversation whose fold this session did not perform.
    folded: usize,
}

impl Transcript {
    /// The transcript of a saved record: every pair it holds, and everything
    /// else the record carries — a field this session does not touch is a
    /// field the next save must write unchanged, so a record loaded with no
    /// run link saves back byte for byte.
    pub(crate) fn from_record(record: RedactedSession) -> Self {
        Self { record, folded: 0 }
    }

    /// The record as it was loaded, and as the next save writes it.
    pub(crate) fn record(&self) -> &RedactedSession {
        &self.record
    }

    /// The record as the next save writes it.
    pub(crate) fn into_record(self) -> RedactedSession {
        self.record
    }

    /// The record, for the one field the session states itself after a run is
    /// prepared: the run link a rollover replaces.
    pub(crate) fn record_mut(&mut self) -> &mut RedactedSession {
        &mut self.record
    }

    /// Forgets the conversation: `/clear` starts the next run with nothing, so
    /// neither the seed nor the saved turns carry a word of it — and no summary
    /// of it either, since a summary of a cleared conversation is a memory the
    /// operator asked to forget.
    pub(crate) fn clear(&mut self) {
        self.record.turns.clear();
        self.record.compaction_summary = None;
        self.folded = 0;
    }

    /// Saves the narrative a compaction folded the older turns into: it is the
    /// working memory the run after the rollover replays, ahead of the pairs the
    /// compaction kept verbatim. The turns themselves stay — what was said does
    /// not change when working memory shrinks.
    pub(crate) fn save_summary(&mut self, summary: &str) {
        let summary = summary.trim();
        self.record.compaction_summary = (!summary.is_empty()).then(|| summary.to_owned());
        // The fold is the plan's own: every turn but the tail it keeps
        // verbatim, so the pairs before the tail are the ones a run stops
        // replaying.
        self.folded = self
            .record
            .turns
            .len()
            .saturating_sub(COMPACT_KEEP_TAIL_TURNS);
    }

    /// The working memory a run replays, as the compaction plans against it:
    /// the summary a compaction saved, ahead of the pairs it kept verbatim —
    /// the same shape [`super::Rollover`] seeds the run that follows with. A
    /// summariser that read the folded turns again would re-pin facts the
    /// session has already forgotten, which is exactly what a compaction must
    /// never do.
    pub(crate) fn replayed(&self) -> Vec<saya_agent::ChatMessage> {
        let mut history = Vec::new();
        if let Some(summary) = &self.record.compaction_summary {
            history.push(saya_agent::ChatMessage::text("user", summary.clone()));
            history.push(saya_agent::ChatMessage::text(
                "assistant",
                super::rollover::SUMMARY_ACK,
            ));
        }
        for turn in self.turns().iter().skip(self.folded) {
            history.push(saya_agent::ChatMessage::text("user", turn.prompt.clone()));
            history.push(saya_agent::ChatMessage::text(
                "assistant",
                turn.answer.clone(),
            ));
        }
        history
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
