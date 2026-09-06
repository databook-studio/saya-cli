//! Tool contracts: definitions, effects, the executor trait, and the
//! local-state effect a tool may have.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::ToolError;

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
