use async_trait::async_trait;
use saya_agent::{
    AgentAttemptOutcome, AgentError, AgentEvent, AgentEventSink, AgentLimits, AgentMetric,
    AgentMetricCount, AgentMetricOutcome, AgentMetricPhase, AgentRequest, AllowReadOnlyApproval,
    CancellationToken, ChatProvider, ChatRequest, ChatResponse, ProviderError, ProviderEvent,
    ProviderRecoveryPhase, ProviderRecoveryReason, ProviderStream, TokenUsage, ToolError,
    ToolExecutor, run_agent_with_sink,
};
use std::sync::{Arc, Mutex};
include!("recovery_usage/no_tools.rs");
include!("recovery_usage/recording_sink.rs");
include!("recovery_usage/capture_sink.rs");
include!("recovery_usage/request.rs");
include!("recovery_usage/establishment_failure.rs");
include!("recovery_usage/max_then_success.rs");
include!("recovery_usage/latest_usage_then_success.rs");
include!("recovery_usage/optional_usage_retry.rs");
include!("recovery_usage/metric_sink.rs");
include!("recovery_usage/dropping_provider.rs");
include!("recovery_usage/cancel_on_retry.rs");
include!("recovery_usage/byte_limit_then_success.rs");
include!("recovery_usage/optional_usage_then_success.rs");
include!("recovery_usage/stream_establishment_failure_is_not_retried.rs");
include!("recovery_usage/retry_uses_latest_failed_snapshot_and_unambiguous_progress.rs");
include!("recovery_usage/retry_metrics_keep_absent_and_reported_zero_usage_distinct.rs");
include!("recovery_usage/cancellation_during_backoff_makes_no_next_provider_call.rs");
include!("recovery_usage/byte_limit_has_a_typed_retry_reason.rs");
include!("recovery_usage/unknown_failed_usage_is_absent_but_reported_zero_is_a_receipt.rs");
include!("recovery_usage/failed_and_successful_usage_saturates_at_u64_max.rs");
