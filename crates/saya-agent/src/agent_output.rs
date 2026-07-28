use crate::{AgentEvent, ProviderError};
use thiserror::Error;

#[derive(Debug, Clone, Copy)]
pub struct AgentLimits {
    pub max_turns: usize,
    pub max_tool_calls: usize,
}
impl Default for AgentLimits {
    fn default() -> Self {
        Self {
            max_turns: 12,
            max_tool_calls: 24,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentOutput {
    pub answer: String,
    pub events: Vec<AgentEvent>,
}
#[derive(Debug, Error)]
pub enum AgentError {
    #[error("{0}")]
    Provider(#[from] ProviderError),
    #[error("agent limit reached: {0}")]
    Limit(&'static str),
    #[error("provider returned an unsupported tool call")]
    InvalidToolCall,
    #[error("request cancelled")]
    Cancelled,
}
