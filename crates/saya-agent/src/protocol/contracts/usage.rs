//! Which provider call a [`crate::AgentEvent::Usage`] event reports.

use serde::{Deserialize, Serialize};

/// The kind of provider call a usage report describes. A turn may make several
/// provider calls — the answering rounds and, when learning is on, a separate
/// post-turn extraction call — and they bill differently, so the event names
/// which one it is. A consumer summing tokens for a cache hit rate wants the
/// answering calls only; folding the extraction call into that denominator
/// would quietly lower the rate.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum UsageCall {
    /// One round of the answering conversation, salvage's final call included —
    /// it produces the answer the reader sees.
    Answer,
    /// The post-turn extraction call, accounted separately from the answer
    /// exactly as the run output keeps it separate.
    Extraction,
}
