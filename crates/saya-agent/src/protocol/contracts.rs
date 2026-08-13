use async_trait::async_trait;
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
