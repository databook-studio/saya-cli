//! Why post-turn learning was skipped after the gate admitted it.

use serde::{Deserialize, Serialize};

/// Why post-turn extraction was skipped after the gate admitted it
/// (`AgentEvent::KnowledgeLearningSkipped`). Two unexpected outcomes — a
/// timeout and an error — each surface; a gate decline is silent and has no
/// variant here. `#[non_exhaustive]` so a future cause (e.g. a bounded-cancel)
/// can be added without breaking serialization.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum LearningSkipReason {
    /// Extraction exceeded the post-turn timeout. Kept for the serialized
    /// contract (a session resumed from before this change may carry it in
    /// its history) and for a future caller that reintroduces a timeout; the
    /// `saya-cli` runtime no longer produces it — post-turn extraction has no
    /// wall-clock ceiling (an explicit owner decision), and a transport
    /// stall or an output-limit truncation now counts as a miss toward the
    /// per-session circuit breaker instead.
    TimedOut,
    /// The provider, parse, or ingest step errored. Distinct from a timeout so a
    /// render can name the right thing without re-deriving the outcome.
    Failed,
}
