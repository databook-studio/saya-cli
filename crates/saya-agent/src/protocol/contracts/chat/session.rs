use serde::{Deserialize, Serialize};

use super::ChatMessage;

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
