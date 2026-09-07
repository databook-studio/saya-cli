//! The constructors for [`AgentEvent`]. The enum lives in `event.rs`; these
//! builders are its named way in, so a caller never assembles a variant
//! literal (several payloads cross crates and are `#[non_exhaustive]`).

use super::{
    KnowledgeOutcome, LearningSkipReason, OverrideFindingDto, ProposedClaimDto,
    SuppliedContractDto, UsageCall,
};
use crate::AgentEvent;
use crate::protocol::streaming::TokenUsage;

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
    /// when the gate admitted extraction but it then timed out or errored. The
    /// caller is the runtime, after the loop; a gate decline never calls this —
    /// declining is silent, and only an unexpected failure surfaces.
    pub fn knowledge_learning_skipped(reason: LearningSkipReason) -> Self {
        Self::KnowledgeLearningSkipped { reason }
    }

    pub fn complete() -> Self {
        Self::Complete
    }

    /// Builds the per-call [`AgentEvent::Usage`] event for one provider call's
    /// reported token counts. The caller is `receive` (answering rounds) or the
    /// CLI runtime (the extraction call), and each passes the call kind so a
    /// consumer can tell the answer's cost from the extraction's.
    pub fn usage(call: UsageCall, usage: TokenUsage) -> Self {
        Self::Usage { call, usage }
    }

    /// Builds the terminal `AnswerDesignated` event carrying the SQL the model
    /// flagged as the answering query. Emitted once, at the terminal turn.
    pub fn answer_designated(sql: impl Into<String>) -> Self {
        Self::AnswerDesignated { sql: sql.into() }
    }

    /// Builds the `ConsensusDecided` event carrying the winning SQL (or `None`
    /// when no winner emerged) and the vote tallies. Emitted once, after all
    /// attempts, whenever more than one attempt ran.
    pub fn consensus_decided(
        sql: Option<String>,
        attempts: usize,
        voted: usize,
        votes: usize,
        margin: usize,
        tied: bool,
        probe_broke_tie: bool,
    ) -> Self {
        Self::ConsensusDecided {
            sql,
            attempts,
            voted,
            votes,
            margin,
            tied,
            probe_broke_tie,
        }
    }
}
