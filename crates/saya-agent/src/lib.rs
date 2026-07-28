//! Agent contracts for SAYA CLI.

mod approval;
mod contracts;
mod event_sink;
mod loop_runner;
mod providers;
mod streaming;

pub use approval::{ApprovalPolicy, ApprovalPolicyParseError};
pub use contracts::{
    AgentEvent, AgentRequest, AllowReadOnlyApproval, ApprovalDecider, ChatMessage, ChatRequest,
    ChatResponse, ProviderError, ToolCall, ToolDefinition, ToolExecutor,
};
pub use event_sink::{AgentEventSink, NoopEventSink};
pub use loop_runner::{AgentError, AgentLimits, AgentOutput, run_agent, run_agent_with_sink};
pub use providers::{OllamaProvider, OpenAiCompatibleProvider, ProviderSettings};
pub use streaming::{CancellationToken, ChatProvider, ProviderEvent, ProviderStream};
