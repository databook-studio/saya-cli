//! The agent event stream (`AgentEvent`) the loop emits across a turn. Its
//! constructors live in `builders`; the reasons post-turn learning was skipped
//! in `learning`.

use serde::{Deserialize, Serialize};

use crate::protocol::streaming::TokenUsage;

use super::{
    KnowledgeOutcome, LearningSkipReason, OverrideFindingDto, ProposedClaimDto,
    SuppliedContractDto, UsageCall,
};

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
    /// The provider stream for this turn failed mid-response (dropped, stalled,
    /// incomplete, or over the `MAX_STREAM_BYTES` bound) and the loop is
    /// retrying the turn with the conversation as it stood at the turn start.
    /// Emitted once before each retried attempt, so a sink that has been
    /// accumulating [`AgentEvent::AssistantText`] (and
    /// [`AgentEvent::ReasoningText`]) deltas must **replace** the text emitted
    /// so far for this turn, never append to it: the partial attempt's answer
    /// is discarded, and the retried stream re-emits the full answer as fresh
    /// deltas. Carries nothing — the replacement arrives as new deltas.
    TurnReset,
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
    /// The token counts one provider call reported — one event per call that
    /// reported any, named by `call` (every answering round is its own event;
    /// the extraction call is a separate one). Emitted **only** when the
    /// provider actually reported usage, so absence on the stream means
    /// "unknown", not "cost nothing", and `usage` is carried verbatim: a
    /// reported zero stays a number, an unreported one serializes `null`.
    Usage {
        call: UsageCall,
        usage: TokenUsage,
    },
    /// The model designated the SQL that answers the question — emitted once,
    /// at the terminal turn, so a headless reader can pair the prose answer
    /// with the query that produced it instead of guessing from the last query
    /// that ran. Carries the SQL text only (already user-visible via tool-call
    /// detail); never result rows.
    AnswerDesignated {
        sql: String,
    },
    /// The consensus decision over multiple candidate attempts — emitted once,
    /// after all attempts, whenever more than one attempt ran (including when
    /// there is no winner, because "the attempts disagreed" is the most
    /// interesting thing a reader can learn and hiding it would misrepresent a
    /// guess as a consensus). Carries the winning SQL (or `None` when the
    /// attempts did not agree and no evidence broke the tie) and the vote
    /// tallies. SQL text only, never result rows — mirroring `AnswerDesignated`.
    ConsensusDecided {
        /// The winning attempt's SQL, or `None` when no agreement/evidence
        /// picked a winner.
        sql: Option<String>,
        attempts: usize,
        /// How many produced a result that could vote.
        voted: usize,
        votes: usize,
        /// Leading votes minus runner-up; `0` when tied.
        margin: usize,
        tied: bool,
        /// True when a tie was resolved by fan-out evidence, not by votes.
        probe_broke_tie: bool,
    },
}
