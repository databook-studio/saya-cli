use async_trait::async_trait;
use saya_agent::{
    AgentAttemptOutcome, AgentError, AgentEvent, AgentEventSink, AgentLimits, AgentMetric,
    AgentMetricOutcome, AgentMetricPhase, AgentMetricUseful, AgentRequest, AllowReadOnlyApproval,
    CancellationToken, ChatProvider, ChatRequest, ChatResponse, ProviderError, ProviderEvent,
    ProviderStream, TokenUsage, ToolCall, ToolConcurrency, ToolDefinition, ToolEffect, ToolError,
    ToolExecutor, run_agent_with_sink,
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::oneshot;

include!("agent_metrics/provider_timing.rs");
include!("agent_metrics/tool_outcomes.rs");
include!("agent_metrics/concurrent_tools.rs");
include!("agent_metrics/cancellation.rs");
