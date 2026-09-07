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
    /// Extraction exceeded the post-turn timeout. The turn's answer is already
    /// in hand; learning is bounded so a long hang never gates the prompt.
    TimedOut,
    /// The provider, parse, or ingest step errored. Distinct from a timeout so a
    /// render can name the right thing without re-deriving the outcome.
    Failed,
}
