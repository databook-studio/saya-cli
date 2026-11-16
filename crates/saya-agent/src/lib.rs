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
pub use history_context::{ContextPreamble, render_untrusted_block};
pub use loop_runner::{
    AgentError, AgentLimits, AgentMailbox, AgentOutput, AgentRunControl, AgentSteering,
    AnsweringLimits, AnsweringScope, AuthorityRefusalContext, DEFAULT_COMPACTION_TRIGGER_PERCENT,
    DESIGNATE_ANSWER_TOOL, EnvBudgets, MAX_CLARIFICATION_OPTION_CHARS, MAX_CLARIFICATION_OPTIONS,
    MAX_CLARIFICATION_QUESTION_CHARS, MAX_STEERING_MESSAGE_BYTES, MAX_STEERING_MESSAGES,
    MAX_TOOL_MESSAGE_BYTES, ProgressCheck, ProgressHandle, ProgressLimits, ProgressObserver,
    ProgressReport, ProgressSnapshot, ProgressStop, ProgressStopKind, REQUEST_CLARIFICATION_TOOL,
    RequestPreflightConfig, SILENT_POLL_LIMIT, ShapedToolResult, SteeringError, agent_mailbox,
    budgets_from_env, run_agent_with_control, run_agent_with_sink, shape_tool_result,
    tool_message_cap,
};
pub use protocol::approval::{ApprovalPolicy, ApprovalPolicyParseError};
pub use protocol::contracts::{
    AgentEvent, AgentMode, AgentModeParseError, AgentRequest, AllowReadOnlyApproval,
    AnsweringObservation, ApprovalChoice, ApprovalDecider, ApprovalDecision, BackgroundExit,
    BudgetDimension, CONTEXT_COMPACT_PERCENT, CONTEXT_WARN_PERCENT, ChatMessage, ChatRequest,
    ChatResponse, ContextBlock, EffectAdmission, EffectAdmissionError, EffectCallKind,
    EffectOutcome, EffectOwnerFailure, EffectPermit, EffectSettlement, IMAGE_RESULT_FIELD,
    ImageBlock, ImmutableAuthorityRefusal, KnowledgeOutcome, LearningSkipReason, LocalStateEffect,
    MAX_REPLAYED_REASONING_BYTES, OverrideFindingDto, PermissionMode, PhysicalAttempt,
    PlanGateCell, ProposedClaimDto, ProviderContinuationPhase, ProviderError,
    ProviderRecoveryPhase, ProviderRecoveryReason, ReasoningEffort, ResponseFormat, SessionGrants,
    SessionPolicy, SuppliedClaimDto, SuppliedContractDto, ToolCall, ToolConcurrency,
    ToolDefinition, ToolEffect, ToolError, ToolExecutionContext, ToolExecutor, ToolMetadata,
    ToolResultShape, UsageCall, WORKSPACE_WRITE_TOKEN, context_utilisation_percent,
    read_only_permits,
};
pub use protocol::event_sink::{
    AgentEventSink, MAX_TOOL_DIAGNOSTIC_BYTES, MAX_TOOL_REASON_BYTES, NoopEventSink,
    TOOL_REASON_TRUNCATION_MARKER, ToolDiagnostic, ToolDiagnosticDetail, ToolDiagnosticStatus,
    bound_tool_reason,
};
pub use protocol::metrics::{
    AgentAttemptOutcome, AgentMetric, AgentMetricCount, AgentMetricOutcome, AgentMetricPhase,
    AgentMetricUseful,
};
pub use protocol::stream_item_limit::MAX_STREAM_EVENTS;
pub use protocol::streaming::{
    CancellationReason, CancellationToken, ChatProvider, ControlledProviderError, MAX_STREAM_BYTES,
    MAX_TOOL_CALLS_PER_RESPONSE, PhysicalAttemptControl, ProviderEvent, ProviderStream, TokenUsage,
};
pub use protocol::tool_transcript::{
    MAX_TOOL_CALL_ID_BYTES, MAX_TOOL_TRANSCRIPT_BYTES, MAX_TOOL_TRANSCRIPT_FIELD_BYTES,
    TOOL_TRANSCRIPT_ELISION_MARKER, ToolCallTranscript, bound_tool_transcript,
};
pub use providers::{
    AnthropicProvider, GeminiProvider, OllamaProvider, OpenAiCompatibleProvider, ProviderSettings,
};

pub use protocol::streaming::{PhysicalAttemptObservation, client_clock_stamp_ns};

pub use protocol::event_sink::ContextCompaction;

pub use protocol::streaming::PhysicalRequestMetadata;
pub use protocol::streaming::{ActorCorrelationDigest, CorrelationId, PhysicalRequestCorrelation};
