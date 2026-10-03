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
    MAX_CLARIFICATION_OPTION_CHARS, MAX_CLARIFICATION_OPTIONS, MAX_CLARIFICATION_QUESTION_CHARS,
    MAX_TOOL_MESSAGE_BYTES, REQUEST_CLARIFICATION_TOOL, ShapedToolResult, budgets_from_env,
    run_agent_with_sink, shape_tool_result, tool_message_cap,
};
pub use protocol::approval::{ApprovalPolicy, ApprovalPolicyParseError};
pub use protocol::contracts::{
    AgentEvent, AgentMode, AgentModeParseError, AgentRequest, AllowReadOnlyApproval,
    ApprovalChoice, ApprovalDecider, ApprovalDecision, CONTEXT_COMPACT_PERCENT,
    CONTEXT_WARN_PERCENT, ChatMessage, ChatRequest, ChatResponse, ContextBlock, KnowledgeOutcome,
    LearningSkipReason, LocalStateEffect, OverrideFindingDto, ProposedClaimDto, ProviderError,
    ReasoningEffort, ResponseFormat, SessionGrants, SessionPolicy, SuppliedClaimDto,
    SuppliedContractDto, ToolCall, ToolConcurrency, ToolDefinition, ToolEffect, ToolError,
    ToolExecutor, ToolMetadata, ToolResultShape, UsageCall, context_utilisation_percent,
    read_only_permits,
};
pub use protocol::event_sink::{AgentEventSink, NoopEventSink};
pub use protocol::streaming::{
    CancellationToken, ChatProvider, MAX_STREAM_BYTES, ProviderEvent, ProviderStream, TokenUsage,
};
pub use providers::{
    AnthropicProvider, GeminiProvider, OllamaProvider, OpenAiCompatibleProvider, ProviderSettings,
};
