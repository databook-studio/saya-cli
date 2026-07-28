//! Agent contracts for SAYA CLI.

mod agent_entry;
mod approval;
mod contracts;
mod event_sink;
mod history;
mod loop_runner;
mod providers;
mod streaming;

pub use agent_entry::run_agent;
pub use approval::{ApprovalPolicy, ApprovalPolicyParseError};
pub use contracts::{
    AgentEvent, AgentRequest, AllowReadOnlyApproval, ApprovalDecider, ChatMessage, ChatRequest,
    ChatResponse, ProviderError, ToolCall, ToolDefinition, ToolExecutor, ToolMetadata,
};
pub use event_sink::{AgentEventSink, NoopEventSink};
pub use loop_runner::{AgentError, AgentLimits, AgentOutput, run_agent_with_sink};
pub use providers::{OllamaProvider, OpenAiCompatibleProvider, ProviderSettings};
pub use streaming::{CancellationToken, ChatProvider, ProviderEvent, ProviderStream};
