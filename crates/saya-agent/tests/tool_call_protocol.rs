use async_trait::async_trait;
use saya_agent::{
    AgentError, AgentEvent, AgentEventSink, AgentLimits, AgentRequest, AllowReadOnlyApproval,
    CancellationToken, ChatMessage, ChatProvider, ChatRequest, ChatResponse, DESIGNATE_ANSWER_TOOL,
    LocalStateEffect, ProviderRecoveryPhase, ProviderRecoveryReason, REQUEST_CLARIFICATION_TOOL,
    TokenUsage, ToolCall, ToolConcurrency, ToolDefinition, ToolEffect, ToolError, ToolExecutor,
    run_agent_with_sink,
};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

include!("tool_call_protocol/protocol_support.rs");
include!("tool_call_protocol/invalid_batches.rs");
include!("tool_call_protocol/registered_shortcuts.rs");
include!("tool_call_protocol/message_scoped_ids.rs");
include!("tool_call_protocol/completed_effects.rs");
