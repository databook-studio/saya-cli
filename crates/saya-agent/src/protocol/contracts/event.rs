//! The agent event stream (`AgentEvent`) the loop emits across a turn, and the
//! reasons post-turn learning was skipped.

use serde::{Deserialize, Serialize};

use super::{KnowledgeOutcome, OverrideFindingDto, ProposedClaimDto, SuppliedContractDto};

// `arguments` carries a `serde_json::Value`, which is not `Eq`, so this enum is
// `PartialEq` only.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum AgentEvent {
    AssistantText {
        text: String,
    },
    /// One delta of the model's chain-of-thought for this turn, streamed the way
    /// [`AgentEvent::AssistantText`] streams the answer. Capture is
    /// unconditional; the `show_thinking` toggle gates **display** only and is
    /// not a precondition for this event. The event carries reasoning across
    /// the crate boundary and no further: the headless renderer renders it to
    /// nothing (it
    /// is content the user has not asked for, not progress — see the note on granularity
    /// note on `terminal_event`), and the TUI accepts it without displaying it
    /// (display is a separate concern). Reasoning is **never** on [`ChatMessage`], so this
    /// event is the only way the turn's
    /// thinking leaves `saya-agent` — and it leaves to in-memory consumers only,
    /// never to a serialized session.
    ReasoningText {
        text: String,
    },
    /// A tool was requested. `arguments` is the raw call payload (e.g. the SQL),
    /// surfaced so the user can see exactly what will run before approving it.
    ToolRequested {
        name: String,
        #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
        arguments: serde_json::Value,
    },
    ToolCompleted {
        name: String,
        summary: String,
    },
    ToolDenied {
        name: String,
        reason: String,
    },
    /// What recall **supplied** to this turn's context block, emitted once per
    /// turn *before* any provider request (so a reader can see what shaped the
    /// SQL before it runs, not after/§2). The payload says
    /// **supplied**, never *used*: a confirmed claim being supplied does not
    /// mean the generated SQL honoured it. Carries at most what recall supplied
    /// (already capped: ≤5 objects, ≤12 claims/object); no raw SQL or evidence.
    KnowledgeSupplied {
        outcome: KnowledgeOutcome,
        contracts: Vec<SuppliedContractDto>,
        /// Claims the byte or count bounds dropped (not the schema policy). A
        /// non-zero count is the event's way of saying "the list above is a
        /// subset, not the whole"; zero means the supply path kept everything.
        dropped_by_bounds: usize,
    },
    /// A candidate claim was **proposed** — persisted — this turn.
    /// Emitted once per persisted proposal, at the moment the store accepts it
    /// (the `Stored` arm), so a refused, duplicate, or validation-failed proposal
    /// emits nothing: the event names what was *written*, never what was merely
    /// *asked for*. Carries the persisted claim's id, profile name, object, kind,
    /// rendered value, and the `Candidate` status it landed with — never the
    /// opaque identity, raw SQL, or evidence. Bounded by the tool's per-turn
    /// proposal cap (≤8); the event stream inherits that bound, so no unbounded
    /// field is needed.
    KnowledgeProposed {
        claim: ProposedClaimDto,
    },
    /// Post-turn extraction has started. The answer is already streamed and on
    /// screen at this point, but the turn is not over: extraction is a second
    /// provider call that the loop awaits, so an adapter stays busy until it
    /// resolves. Emitted so that wait can be labelled — an unexplained spinner
    /// after a finished answer reads as a hang, which is what forces the
    /// extraction budget to be tighter than the work needs.
    ///
    /// Carries nothing. It is a progress signal, not content: an adapter with
    /// no progress surface (the headless renderer) is right to ignore it.
    KnowledgeLearningStarted,
    /// A confirmed claim the turn's SQL **contradicted**. Emitted at
    /// most once per turn, after the loop, carrying every finding the detector
    /// raised across the turn's statements. Silent when there is nothing to say
    /// (the detector fails closed on unparseable SQL, partial column lists, joins,
    /// and ambiguous objects); no event is emitted for an empty finding set.
    ///
    /// The finding says the claim was contradicted and names the time-named
    /// columns the SQL **referenced** — observed references, not "the time column
    /// SAYA used": from names alone the role of a column (predicate vs projection)
    /// is unknowable, so the finding stops at "these were referenced where the
    /// claim named a different column." No opaque identity, no raw SQL.
    KnowledgeOverridden {
        findings: Vec<OverrideFindingDto>,
    },
    /// Post-turn extraction was **skipped after the turn already succeeded** —
    /// the turn's answer is unaffected, but no memory was recorded for it. Emitted
    /// at most once per turn, after the loop, only when extraction was *expected*
    /// to run (the gate admitted it) and then failed unexpectedly: it timed out
    /// or the provider/parse/ingest step errored. A gate that *declines* emits
    /// nothing — declining is the common case on ordinary turns and a line every
    /// turn would be noise; only an unexpected failure surfaces. Carries the
    /// reason so a render can distinguish "timed out" from "failed" without
    /// re-deriving it. No raw response, no payload.
    KnowledgeLearningSkipped {
        reason: LearningSkipReason,
    },
    Complete,
    /// The model designated the SQL that answers the question — emitted once,
    /// at the terminal turn, so a headless reader can pair the prose answer
    /// with the query that produced it instead of guessing from the last query
    /// that ran. Carries the SQL text only (already user-visible via tool-call
    /// detail); never result rows.
    AnswerDesignated {
        sql: String,
    },
}

/// Why post-turn extraction was skipped after the gate admitted it
/// (`AgentEvent::KnowledgeLearningSkipped`, spec packet-54 decision 1). Two
/// unexpected outcomes — a timeout and an error — each surface; a gate decline
/// is silent and has no variant here. `#[non_exhaustive]` so a future cause
/// (e.g. a bounded-cancel) can be added without breaking serialization.
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

impl AgentEvent {
    pub fn assistant_text(text: impl Into<String>) -> Self {
        Self::AssistantText { text: text.into() }
    }

    /// Builds one chain-of-thought delta event, mirroring [`AgentEvent::assistant_text`].
    /// The caller is `receive`, forwarding a `ProviderEvent::ReasoningDelta` so the
    /// turn's thinking crosses the crate boundary the same way the answer does.
    /// Display is gated elsewhere; this event carries the text, it does not
    /// decide whether to show it.
    pub fn reasoning_text(text: impl Into<String>) -> Self {
        Self::ReasoningText { text: text.into() }
    }

    pub fn tool_requested(name: impl Into<String>, arguments: serde_json::Value) -> Self {
        Self::ToolRequested {
            name: name.into(),
            arguments,
        }
    }

    /// Builds the per-turn `KnowledgeSupplied` event from recall's outcome, the
    /// supplied contracts, and the count the bounds dropped.
    pub fn knowledge_supplied(
        outcome: KnowledgeOutcome,
        contracts: Vec<SuppliedContractDto>,
        dropped_by_bounds: usize,
    ) -> Self {
        Self::KnowledgeSupplied {
            outcome,
            contracts,
            dropped_by_bounds,
        }
    }

    /// Builds the per-proposal `KnowledgeProposed` event for one persisted
    /// candidate claim. The caller is the propose tool, at the `Stored` arm.
    pub fn knowledge_proposed(claim: ProposedClaimDto) -> Self {
        Self::KnowledgeProposed { claim }
    }

    /// Builds the per-turn `KnowledgeOverridden` event carrying every finding
    /// the detector raised across the turn's statements. The caller is the
    /// runtime, after the loop drains the override log; an empty `findings`
    /// means the caller emits nothing.
    pub fn knowledge_overridden(findings: Vec<OverrideFindingDto>) -> Self {
        Self::KnowledgeOverridden { findings }
    }

    /// Builds the per-turn `KnowledgeLearningSkipped` event the runtime emits
    /// when the gate admitted extraction but it then timed out or errored (spec
    /// packet-54). The caller is the runtime, after the loop; a gate decline
    /// never calls this — declining is silent, and only an unexpected failure
    /// surfaces.
    pub fn knowledge_learning_skipped(reason: LearningSkipReason) -> Self {
        Self::KnowledgeLearningSkipped { reason }
    }

    pub fn complete() -> Self {
        Self::Complete
    }

    /// Builds the terminal `AnswerDesignated` event carrying the SQL the model
    /// flagged as the answering query. Emitted once, at the terminal turn.
    pub fn answer_designated(sql: impl Into<String>) -> Self {
        Self::AnswerDesignated { sql: sql.into() }
    }
}
