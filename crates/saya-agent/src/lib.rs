//! Agent contracts for SAYA CLI.

mod approval;
mod contracts;
mod loop_runner;
mod providers;

pub use approval::{ApprovalPolicy, ApprovalPolicyParseError};
pub use contracts::{
    AgentEvent, AgentRequest, AllowReadOnlyApproval, ApprovalDecider, ChatMessage, ChatProvider,
    ChatRequest, ChatResponse, ProviderError, ToolCall, ToolDefinition, ToolExecutor,
};
pub use loop_runner::{AgentError, AgentLimits, AgentOutput, run_agent};
pub use providers::{OllamaProvider, OpenAiCompatibleProvider, ProviderSettings};
