use async_trait::async_trait;
use saya_types::{ClaimId, ClaimStatus};
use serde::{Deserialize, Serialize};

use crate::protocol::streaming::TokenUsage;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRequest {
    pub prompt: String,
    pub profile_names: Vec<String>,
    pub model: String,
    /// Optional extra system context appended to the base SAYA system prompt (e.g. available database connections).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub history: Vec<ChatMessage>,
    /// Untrusted, labelled database context rendered into the user turn — never the
    /// system message. `#[serde(default)]` keeps old serialized requests deserializable;
    /// `skip_serializing_if` keeps an empty vector off the wire.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context_blocks: Vec<ContextBlock>,
}

/// A labelled, untrusted chunk of database context (a contract, a schema note, a
/// comment) that reaches the model as quoted data inside the user turn, never as
/// policy in the system message. Nothing populates this in Phase 2a; Phase 2b wires
/// recall in.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextBlock {
    /// Short machine-ish label for the block's source, e.g. "database-contracts".
    pub label: String,
    /// The block's content. Untrusted.
    pub body: String,
    /// True when the source had more to give than the caller's budget allowed.
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn text(role: &str, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolMetadata {
    pub name: String,
    pub status: String,
}

/// The shape the caller wants the response in. Provider-neutral intent —
/// **not** an OpenAI wire spelling — so `saya-agent` stays provider-agnostic.
/// Each provider translates the variant it honours (`JsonObject` →
/// `response_format: {"type":"json_object"}` on OpenAI, `format: "json"` on
/// Ollama) or drops it; a provider with no equivalent degrades to today's
/// behaviour (the prompt already asks for JSON, `strip_markdown_fences` already
/// handles fences). `Text` is the default so the main agent loop — which never
/// sets this — keeps answering in prose (invariant 1: JSON mode is for the
/// extraction call only).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ResponseFormat {
    /// Free-form prose. The default; the main loop's request.
    #[default]
    Text,
    /// The response must be a single JSON object. The extraction call sets this
    /// so a reasoning model does not spend tokens on chain-of-thought we discard.
    JsonObject,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolDefinition>,
    /// How the caller wants the response shaped. Defaults to [`ResponseFormat::Text`];
    /// the extraction call sets [`ResponseFormat::JsonObject`]. The main agent
    /// loop never sets it (left to `Default`), so a prose answer stays prose.
    #[serde(default)]
    pub response_format: ResponseFormat,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChatResponse {
    pub message: ChatMessage,
    /// The chain-of-thought the model produced for this call, or `None` when
    /// the provider reported no reasoning. Lives **here, on `ChatResponse`** —
    /// transport for one call — and never on `ChatMessage`, which is what gets
    /// replayed to the provider as history and what session persistence is
    /// shaped around. That placement is the point of S23's Q2: with no
    /// reasoning field on `ChatMessage`, a session writer or history builder
    /// has nowhere to copy it, so "reasoning is never persisted" and "reasoning
    /// is never replayed as history" (S23 invariants 1 and 2) are structural,
    /// not remembered. `None` is the absent case — a provider that omits
    /// chain-of-thought — distinct from `Some(String::new())`, a model that
    /// reasoned and produced nothing; both survive `#[serde(default)]`, so a
    /// response serialized before this field existed (no `reasoning` key)
    /// deserializes to `None`. Capture is unconditional (invariant 4): this is
    /// populated whether or not the user has asked to see thinking — the
    /// `show_thinking` toggle that gates *display* is S24.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    /// Token counts the provider reported for this response, or `None` when the
    /// provider reported nothing. `None` is the absent case — distinct from
    /// `Some(TokenUsage::default())`, which would read as "this call cost
    /// nothing" — so a caller cannot mistake a silent provider for a free one
    /// (invariant 1: absent is not zero, mirroring the streaming path, which
    /// models absence by simply not emitting a `Usage` event). The streaming
    /// path's `collect()` threads the last `Usage` event it sees here; Gemini's
    /// non-streaming `complete()` threads the usage it parses directly.
    /// `#[serde(default)]` keeps a serialized response written before this field
    /// existed (no `usage` key) deserializing to `None`.
    #[serde(default)]
    pub usage: Option<TokenUsage>,
}

/// The three distinguishable states of a turn's recall, as
/// [`AgentEvent::KnowledgeSupplied`] carries them (spec P1b §3). `Off` (recall
/// disabled by config), `Skipped` (the privacy gate closed — SAYA was not
/// allowed to look), and `Ran` (recall ran against the store) are three facts a
/// user reads differently; collapsing them into a single "no event" would hide
/// the distinction between "SAYA was not allowed to look" and "SAYA looked and
/// had nothing". `Ran { store_unavailable: true }` records a store failure that
/// degraded recall to an empty result — the turn still completes (recall is
/// fail-soft, spec §3).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum KnowledgeOutcome {
    /// Recall is off by config; SAYA did not look.
    Off,
    /// The privacy gate closed; SAYA was not allowed to look. No store query.
    Skipped,
    /// Recall ran against the store. `store_unavailable` is true when a store
    /// failure degraded recall to an empty result.
    Ran { store_unavailable: bool },
}

/// One claim as **supplied** to a turn's context block, in the DTO shape that
/// crosses the crate boundary into [`AgentEvent::KnowledgeSupplied`]. Carries
/// the claim id (so a later phase can name exactly which saved claims shaped
/// an answer), its kind, the short rendered value the prompt block shows, a
/// column when the claim is column-scoped, and its persisted status — so a
/// `Candidate` reads as `candidate`, distinct from `confirmed` (spec P1b §4.5).
///
/// No raw payload, evidence, or SQL. `value` is the same short rendered form
/// the prompt block already shows (a column name, an alias), not the stored
/// payload — and it is named **supplied**, never *used*: a confirmed claim
/// being supplied does not mean the generated SQL honoured it (we have
/// measured that it frequently does not).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SuppliedClaimDto {
    pub claim_id: ClaimId,
    /// The claim kind token (`table_alias`, `default_time_column`, …).
    pub kind: String,
    /// The short rendered value the prompt block shows, not the stored payload.
    pub value: String,
    /// A column name when the claim is column-scoped; `None` for table-level
    /// claims. `skip_serializing_if` keeps it off the wire when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<String>,
    pub status: ClaimStatus,
}

/// One object's claims, as supplied to the turn, in the DTO shape that crosses
/// the crate boundary into [`AgentEvent::KnowledgeSupplied`]. `profile` is the
/// human-facing profile **name**, never the opaque [`saya_types::ProfileIdentity`]
/// — the identity has no field here, by construction (spec P1b §3). `schema_state`
/// is the contract's aggregated state token (`current` / `needs_review` /
/// `live_schema_unavailable`); `stale` never appears (a contract aggregating to
/// `Stale` is dropped by the model-path policy before supply).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SuppliedContractDto {
    /// The human-facing profile name. Never the opaque identity.
    pub profile: String,
    /// The object's qualified name (`catalog.schema.object`).
    pub object: String,
    /// The aggregated schema state token; `stale` never appears here.
    pub schema_state: String,
    pub claims: Vec<SuppliedClaimDto>,
}

/// One candidate claim **proposed** (persisted) this turn, in the DTO shape that
/// crosses the crate boundary into [`AgentEvent::KnowledgeProposed`] (spec P2d).
/// Mirrors [`SuppliedClaimDto`]'s vocabulary — same `claim_id` / `kind` / `value`
/// / `column` / `status` — and adds `profile` and `object`, because a proposal is
/// a single flat claim, not a claim nested under a contract stanza. `profile` is
/// the human-facing profile **name**, never the opaque
/// [`saya_types::ProfileIdentity`] (no identity field, by construction).
///
/// `value` is the same short rendered form a later recall would show (a column
/// name, an alias), reusing the recall render path's `claim_value` so a proposal
/// can never name a value recall would not — not the stored payload. `status` is
/// the status the claim *landed with*: `contract_propose` stores only a
/// `Candidate`, so a `KnowledgeProposed` event never reads as established (a
/// candidate is inert until a human confirms it). No raw SQL, evidence, or cells.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProposedClaimDto {
    pub claim_id: ClaimId,
    /// The human-facing profile name. Never the opaque identity.
    pub profile: String,
    /// The object's qualified name (`catalog.schema.object`).
    pub object: String,
    /// The claim kind token (`table_alias`, `default_time_column`, …).
    pub kind: String,
    /// The short rendered value, not the stored payload.
    pub value: String,
    /// A column name when the claim is column-scoped; `None` for table-level
    /// claims. `skip_serializing_if` keeps it off the wire when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<String>,
    /// The status the claim landed with — `Candidate` for a proposal.
    pub status: ClaimStatus,
}

/// One confirmed claim the turn's SQL contradicted, in the DTO shape that
/// crosses the crate boundary into [`AgentEvent::KnowledgeOverridden`] (spec
/// A1). Mirrors nothing about a claim being *used* — the finding says the claim
/// was contradicted and names the time-named columns the SQL **referenced**
/// instead, which is all the extractor can prove from names. `claimed_value`
/// is the value the claim specifies (the claimed time column), carried so a
/// render can say "where you specified Y". No opaque identity, no raw SQL.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OverrideFindingDto {
    pub claim_id: ClaimId,
    /// The claim kind token. Only `default_time_column` is ever produced.
    pub kind: String,
    /// The value the claim specifies — for `default_time_column`, the claimed
    /// time column. "Where you specified Y" in the render.
    pub claimed_value: String,
    /// Time-named columns the SQL referenced instead, as written, sorted for
    /// determinism. Observed references, not an asserted "used" column.
    pub observed_columns: Vec<String>,
}

// `arguments` carries a `serde_json::Value`, which is not `Eq`, so this enum is
// `PartialEq` only.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum AgentEvent {
    AssistantText {
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
    /// SQL before it runs, not after — spec P1b §1/§2). The payload says
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
    /// A candidate claim was **proposed** — persisted — this turn (spec P2d).
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
    /// A confirmed claim the turn's SQL **contradicted** — spec A1. Emitted at
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
    /// re-deriving it. No raw response, no payload (spec packet-54 decision 1/2).
    KnowledgeLearningSkipped {
        reason: LearningSkipReason,
    },
    Complete,
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
    /// means the caller emits nothing (spec A1: "if it returns nothing, say
    /// nothing").
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
}

#[async_trait]
pub trait ApprovalDecider: Send + Sync {
    /// Decides whether a tool call may run. `arguments` is the raw call payload
    /// so implementations can show the user what they are approving.
    async fn approve(&self, tool: &ToolDefinition, arguments: &serde_json::Value) -> bool;
}

pub struct AllowReadOnlyApproval;

#[async_trait]
impl ApprovalDecider for AllowReadOnlyApproval {
    async fn approve(&self, _: &ToolDefinition, _: &serde_json::Value) -> bool {
        true
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProviderError {
    #[error("provider request failed: {0}")]
    Request(String),
    #[error("provider returned an invalid response")]
    InvalidResponse,
    #[error("provider is not configured: {0}")]
    Configuration(String),
    #[error("provider stream was cancelled")]
    Cancelled,
}

impl ProviderError {
    pub fn configuration(message: impl Into<String>) -> Self {
        Self::Configuration(message.into())
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ToolError {
    #[error("data sharing is disabled for this cloud provider")]
    DataSharingDisabled,
    #[error("invalid query arguments")]
    InvalidQueryArguments,
    #[error("unsupported read-only tool")]
    UnsupportedTool,
    #[error("invalid tool arguments: expected an object")]
    ArgumentsNotObject,
    #[error("invalid tool arguments: unsupported property")]
    UnsupportedProperty,
    #[error("invalid tool arguments: connection must be a string")]
    ConnectionNotString,
    #[error("invalid tool arguments: sql must be a string")]
    SqlNotString,
    #[error("no database profile is selected")]
    NoConnectionSelected,
    #[error("unknown connection \"{target}\"; available connections: {available}")]
    UnknownConnection { target: String, available: String },
    #[error("read-only query failed")]
    QueryFailed,
    #[error("read-only query failed: {0}")]
    QueryFailedDetail(String),
    #[error("read-only query timed out")]
    QueryTimedOut,
    #[error("query result unavailable")]
    QueryResultUnavailable,
    #[error("schema discovery failed: {0}")]
    SchemaDiscoveryFailed(String),
    #[error("{0}")]
    Chart(String),
}

/// What local state a tool may touch — contracts, the schema cache, anything
/// persisted on the user's machine. Declared per tool so "may this tool write
/// local state?" is a property the loop reads rather than something inferred
/// from a tool's name. Phase 3a introduces the type; Phase 3c adds the first
/// tool that declares `WriteCandidate`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum LocalStateEffect {
    /// Touches no local state.
    #[default]
    None,
    /// Reads local state (contracts, cache) and writes nothing.
    Read,
    /// May persist a *candidate* claim. Never a confirmed one — confirmation is a
    /// human action and has no tool.
    WriteCandidate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolEffect {
    pub database_data: bool,
    pub external_side_effect: bool,
    pub requires_approval: bool,
    /// What local state this tool may touch. `#[serde(default)]` keeps the
    /// pre-3a serialized form (no key) deserializing to `None`.
    #[serde(default)]
    pub local_state: LocalStateEffect,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub read_only: bool,
    pub parameters: serde_json::Value,
    pub effect: ToolEffect,
}

#[async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn execute(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError>;
}

#[cfg(test)]
mod tests {
    use super::{ChatMessage, ChatRequest, ChatResponse, LocalStateEffect, ResponseFormat};

    /// S23 deliverable 4 (the structural guarantee): `ChatMessage` — what gets
    /// replayed to the provider as history and what session persistence is
    /// shaped around — has **no** reasoning field. A `ChatMessage` carrying
    /// reasoning-shaped content serializes to the same wire form today had
    /// before this slice, because there is nowhere on the type to put the
    /// reasoning. If a field is ever added here, this test fails and the
    /// reviewer is forced to justify breaking S23 invariants 1 and 2.
    #[test]
    fn chat_message_has_no_reasoning_field_and_wire_is_unchanged() {
        let message = ChatMessage::text("assistant", "the answer is 42");
        let json = serde_json::to_string(&message).expect("serializes");
        // The reasoning this turn *would have* carried. It must not appear in
        // the message's wire form — there is no field for it.
        let reasoning = "I reasoned about the row values and the time column";
        assert!(
            !json.contains(reasoning),
            "reasoning leaked onto ChatMessage wire form: {json}"
        );
        assert!(
            !json.contains("reasoning"),
            "a `reasoning` key appeared on ChatMessage: {json}"
        );
        // The wire form is exactly role + content + tool_calls + tool_call_id,
        // the pre-S23 shape.
        assert_eq!(
            json, r#"{"role":"assistant","content":"the answer is 42"}"#,
            "ChatMessage wire form changed: {json}"
        );
    }

    /// S23 deliverable 4 (the field lives on `ChatResponse`, the transport):
    /// a response carrying reasoning serializes the reasoning under a
    /// `reasoning` key, and one with `None` omits it (`skip_serializing_if`),
    /// so a response written before this slice (no `reasoning` key)
    /// deserializes to `None` — old serialized responses stay valid.
    #[test]
    fn chat_response_carries_reasoning_and_round_trips() {
        let with = ChatResponse {
            message: ChatMessage::text("assistant", "ok"),
            reasoning: Some("because the column is nullable".into()),
            ..Default::default()
        };
        let json = serde_json::to_string(&with).expect("serializes");
        assert!(
            json.contains(r#""reasoning":"because the column is nullable""#),
            "reasoning must appear on ChatResponse: {json}"
        );
        let back: ChatResponse = serde_json::from_str(&json).expect("deserializes back");
        assert_eq!(back.reasoning, with.reasoning);

        // `None` is omitted from the wire (a present-but-null would read as
        // "the model reasoned and produced nothing", which is a different
        // fact than "the provider reported no reasoning").
        let without = ChatResponse {
            message: ChatMessage::text("assistant", "ok"),
            ..Default::default()
        };
        let json = serde_json::to_string(&without).expect("serializes");
        assert!(
            !json.contains("reasoning"),
            "None reasoning must be off the wire: {json}"
        );
        // A pre-S23 response (no `reasoning` key) deserializes to `None`.
        let old = r#"{"message":{"role":"assistant","content":"ok"}}"#;
        let old_response: ChatResponse = serde_json::from_str(old).expect("old form deserializes");
        assert_eq!(old_response.reasoning, None);
    }

    /// `ResponseFormat::Text` is the default — the whole point of leaving the
    /// field unset on the main loop's request. If this regresses, invariant 1
    /// (JSON mode is for the extraction call only) breaks silently.
    #[test]
    fn response_format_defaults_to_text() {
        assert_eq!(ResponseFormat::default(), ResponseFormat::Text);
        // A request built with struct-update (`..Default::default()`) — the
        // shape the main loop and the call sites use — defaults to `Text`.
        let request = ChatRequest {
            model: "m".into(),
            messages: Vec::new(),
            tools: Vec::new(),
            ..Default::default()
        };
        assert_eq!(request.response_format, ResponseFormat::Text);
    }

    /// `ResponseFormat` is provider-neutral intent, not an OpenAI wire spelling:
    /// it serializes as `text` / `json_object` (snake_case) and round-trips, so a
    /// serialized `ChatRequest` stays readable and stable.
    #[test]
    fn response_format_round_trips_through_snake_case() {
        for (variant, expected) in [
            (ResponseFormat::Text, "text"),
            (ResponseFormat::JsonObject, "json_object"),
        ] {
            let text = serde_json::to_string(&variant).expect("serializes");
            assert_eq!(text, format!("\"{expected}\""), "{variant:?}");
            let back: ResponseFormat = serde_json::from_str(&text).expect("deserializes back");
            assert_eq!(back, variant, "{variant:?}");
        }
    }

    /// The back-compat guarantee: a `ChatRequest` serialized before this slice
    /// (no `response_format` key) deserializes to the default `Text`, so old
    /// serialized requests stay valid.
    #[test]
    fn chat_request_without_response_format_key_defaults_to_text() {
        let json = r#"{"model":"m","messages":[],"tools":[]}"#;
        let request: ChatRequest = serde_json::from_str(json).expect("old form deserializes");
        assert_eq!(request.response_format, ResponseFormat::Text);
    }

    /// The back-compat guarantee: a `ToolEffect` serialized before this slice
    /// (no `local_state` key) deserializes to the default `None`.
    #[test]
    fn tool_effect_without_local_state_key_defaults_to_none() {
        let json = r#"{
            "database_data": false,
            "external_side_effect": false,
            "requires_approval": false
        }"#;
        let effect: super::ToolEffect = serde_json::from_str(json).expect("old form deserializes");
        assert_eq!(effect.local_state, LocalStateEffect::None);
    }

    /// Each variant round-trips through snake_case.
    #[test]
    fn local_state_effect_round_trips_through_snake_case() {
        for (variant, expected) in [
            (LocalStateEffect::None, "none"),
            (LocalStateEffect::Read, "read"),
            (LocalStateEffect::WriteCandidate, "write_candidate"),
        ] {
            let text = serde_json::to_string(&variant).expect("serializes");
            assert_eq!(text, format!("\"{expected}\""), "{variant:?}");
            let back: LocalStateEffect = serde_json::from_str(&text).expect("deserializes back");
            assert_eq!(back, variant, "{variant:?}");
        }
    }

    /// `KnowledgeOverridden` serializes under its `knowledge_overridden` type tag
    /// (spec A1) and carries the finding's fields, with no opaque identity — the
    /// DTO has no such field, by construction.
    #[test]
    fn knowledge_overridden_serializes_with_type_tag_and_findings() {
        use super::{AgentEvent, OverrideFindingDto};
        use saya_types::ClaimId;
        let event = AgentEvent::knowledge_overridden(vec![OverrideFindingDto {
            claim_id: ClaimId::parse("c-rental-time").unwrap(),
            kind: "default_time_column".into(),
            claimed_value: "return_date".into(),
            observed_columns: vec!["rental_date".into()],
        }]);
        let json = serde_json::to_string(&event).expect("serializes");
        assert!(json.contains(r#""type":"knowledge_overridden""#), "{json}");
        assert!(
            json.contains("return_date"),
            "carries the claimed value: {json}"
        );
        assert!(
            json.contains("rental_date"),
            "carries the observed column: {json}"
        );
        // No opaque identity field exists on the DTO; a fabricated one must not
        // appear in the serialized event.
        let fake_identity =
            "sha256:9f2a8c7b1e4d0a6f3c5b8e2d7a9f1c4b6e8a0d2f4c6b8e0a2d4f6c8b0e2d4f6";
        assert!(!json.contains(fake_identity), "identity leaked: {json}");
    }

    /// `KnowledgeLearningSkipped` serializes under its `knowledge_learning_skipped`
    /// type tag and carries the reason; both reasons round-trip (spec packet-54
    /// decision 1 — `#[non_exhaustive]` enum with the same derive set as siblings).
    #[test]
    fn knowledge_learning_skipped_serializes_with_type_tag_and_reason() {
        use super::{AgentEvent, LearningSkipReason};
        for (reason, token) in [
            (LearningSkipReason::TimedOut, "timed_out"),
            (LearningSkipReason::Failed, "failed"),
        ] {
            let event = AgentEvent::knowledge_learning_skipped(reason);
            let json = serde_json::to_string(&event).expect("serializes");
            assert!(
                json.contains(r#""type":"knowledge_learning_skipped""#),
                "type tag for {reason:?}: {json}"
            );
            assert!(
                json.contains(&format!(r#""reason":"{token}""#)),
                "reason token for {reason:?}: {json}"
            );
            let back: AgentEvent = serde_json::from_str(&json).expect("deserializes back");
            assert_eq!(back, event, "round-trips for {reason:?}");
        }
    }
}
