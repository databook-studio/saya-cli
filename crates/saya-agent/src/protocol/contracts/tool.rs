//! Tool contracts: definitions, effects, the executor trait, and the
//! local-state effect a tool may have.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::ToolError;

/// What local state a tool may touch — contracts, the schema cache, run
/// workspaces, anything persisted on the user's machine. Declared per tool so
/// "may this tool write local state?" is a property the loop reads rather than
/// something inferred from a tool's name.
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
    /// May write files inside the run workspace (contained, atomic, never
    /// executable). Gated like [`LocalStateEffect::WriteCandidate`]: the loop
    /// refuses it unless the runner was constructed with workspace writes
    /// permitted, so a registered write tool cannot write merely by being
    /// registered.
    WriteWorkspace,
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
    /// One-line summary the loop reports when this tool succeeds, overriding
    /// the generic read-only/write wording — a tool whose action is neither
    /// "read-only database" nor "local-state write" (e.g. one that writes a
    /// file and opens a browser) states what it actually did. The failure
    /// summary is derived from the same text and always keeps the substring
    /// "failed", which the status derivation and the statement-outcome memory
    /// key on. `#[serde(default)]` accepts serialized forms without the key;
    /// skipping `None` keeps serialized definitions byte-stable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion: Option<String>,
}

#[async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn execute(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError>;
}
