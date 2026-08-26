use crate::{AgentEvent, ProviderError, ToolMetadata};
use thiserror::Error;

#[derive(Debug, Clone, Copy)]
pub struct AgentLimits {
    pub max_turns: usize,
    pub max_tool_calls: usize,
    /// Whether the loop may execute tools that declare
    /// [`LocalStateEffect::WriteCandidate`](crate::LocalStateEffect::WriteCandidate).
    /// Defaults to **not permitted**: a tool that can write a candidate claim
    /// must not start writing merely because it was registered. Phase 4 turns
    /// this on under an explicit config setting; until then nothing can enable
    /// it, which is correct.
    pub permit_candidate_writes: bool,
}
impl Default for AgentLimits {
    fn default() -> Self {
        Self {
            max_turns: 12,
            max_tool_calls: 24,
            permit_candidate_writes: false,
        }
    }
}
// `events` holds `AgentEvent`, which carries a `serde_json::Value` and is
// therefore `PartialEq` but not `Eq`.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentOutput {
    pub answer: String,
    pub events: Vec<AgentEvent>,
    pub used_bounded_sql_query: bool,
    pub tool_metadata: Vec<ToolMetadata>,
    /// Token counts summed over every provider turn of this run (zero when
    /// the provider does not report usage).
    pub usage: crate::TokenUsage,
}
#[derive(Debug, Error)]
pub enum AgentError {
    #[error("{0}")]
    Provider(#[from] ProviderError),
    #[error("agent limit reached: {0}")]
    Limit(&'static str),
    #[error("provider returned an unsupported tool call")]
    InvalidToolCall,
    #[error("conversation history is invalid")]
    InvalidHistory,
    #[error("conversation context exceeds the safe limit")]
    ContextLimit,
    #[error("request cancelled")]
    Cancelled,
}
