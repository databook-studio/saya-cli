use async_trait::async_trait;
use saya_types::{ClaimId, ClaimStatus};
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    pub message: ChatMessage,
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
    Complete,
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
    use super::LocalStateEffect;

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
}
