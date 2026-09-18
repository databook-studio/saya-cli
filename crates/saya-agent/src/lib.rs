//! Agent contracts for SAYA CLI.

mod agent_entry;
mod consensus;
mod history;
mod history_context;
mod loop_runner;
mod protocol;
mod providers;

pub use consensus::{Candidate, Consensus, fingerprint, tally};

pub use agent_entry::run_agent;
pub use history::{build_messages, turn_bytes};
pub use history_context::render_untrusted_block;
pub use loop_runner::{
    AgentError, AgentLimits, AgentOutput, DESIGNATE_ANSWER_TOOL, EnvBudgets,
    MAX_TOOL_MESSAGE_BYTES, budgets_from_env, run_agent_with_sink, tool_message_cap,
};
pub use protocol::approval::{ApprovalPolicy, ApprovalPolicyParseError};
pub use protocol::contracts::{
    AgentEvent, AgentRequest, AllowReadOnlyApproval, ApprovalChoice, ApprovalDecider,
    ApprovalDecision, ChatMessage, ChatRequest, ChatResponse, ContextBlock, KnowledgeOutcome,
    LearningSkipReason, LocalStateEffect, OverrideFindingDto, ProposedClaimDto, ProviderError,
    ReasoningEffort, ResponseFormat, SessionGrants, SessionPolicy, SuppliedClaimDto,
    SuppliedContractDto, ToolCall, ToolDefinition, ToolEffect, ToolError, ToolExecutor,
    ToolMetadata, ToolResultShape, UsageCall, read_only_permits,
};
pub use protocol::event_sink::{AgentEventSink, NoopEventSink};
pub use protocol::streaming::{
    CancellationToken, ChatProvider, MAX_STREAM_BYTES, ProviderEvent, ProviderStream, TokenUsage,
};
pub use providers::{
    AnthropicProvider, GeminiProvider, OllamaProvider, OpenAiCompatibleProvider, ProviderSettings,
};
