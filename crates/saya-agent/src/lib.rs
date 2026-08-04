//! Agent contracts for SAYA CLI.

mod agent_entry;
mod history;
mod loop_runner;
mod protocol;
mod providers;

pub use agent_entry::run_agent;
pub use loop_runner::{AgentError, AgentLimits, AgentOutput, run_agent_with_sink};
pub use protocol::approval::{ApprovalPolicy, ApprovalPolicyParseError};
pub use protocol::contracts::{
    AgentEvent, AgentRequest, AllowReadOnlyApproval, ApprovalDecider, ChatMessage, ChatRequest,
    ChatResponse, ProviderError, ToolCall, ToolDefinition, ToolExecutor, ToolMetadata,
};
pub use protocol::event_sink::{AgentEventSink, NoopEventSink};
pub use protocol::streaming::{CancellationToken, ChatProvider, ProviderEvent, ProviderStream};
pub use providers::{
    AnthropicProvider, GeminiProvider, OllamaProvider, OpenAiCompatibleProvider, ProviderSettings,
};
