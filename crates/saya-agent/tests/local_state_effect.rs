//! The loop's fail-closed guards for local-state-writing tools.
//!
//! A tool declaring `LocalStateEffect::WriteCandidate` must be *denied* (a
//! `ToolDenied` event with a reason, the turn continuing) when the runner was
//! not constructed with candidate writes permitted, and must run normally when
//! the permission is on. `LocalStateEffect::WriteWorkspace` behaves the same
//! way behind its own permission. Tools declaring `None` or `Read` are
//! unaffected either way.

use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentEventSink, AgentLimits, AgentRequest, AllowReadOnlyApproval, ApprovalDecider,
    ChatProvider, ChatRequest, ChatResponse, LocalStateEffect, ToolCall, ToolConcurrency,
    ToolDefinition, ToolEffect, ToolError, ToolExecutor, run_agent_with_sink,
};
use std::sync::{Arc, Mutex};
include!("local_state_effect/one_call_provider.rs");
include!("local_state_effect/policy_provider.rs");
include!("local_state_effect/recording_executor.rs");
include!("local_state_effect/recording_sink.rs");
include!("local_state_effect/candidate_tool.rs");
include!("local_state_effect/workspace_tool.rs");
include!("local_state_effect/session_tool.rs");
include!("local_state_effect/request.rs");
include!("local_state_effect/policy_tool.rs");
include!("local_state_effect/run_policy_turn.rs");
include!("local_state_effect/deny_approval.rs");
include!("local_state_effect/allow_approval.rs");
include!("local_state_effect/expected_policy.rs");
include!(
    "local_state_effect/write_candidate_tool_is_denied_by_default_and_does_not_end_the_turn.rs"
);
include!("local_state_effect/write_candidate_tool_runs_when_candidate_writes_are_permitted.rs");
include!("local_state_effect/read_local_state_tool_is_unaffected_by_the_candidate_permission.rs");
include!("local_state_effect/policy_matrix_matches_single_and_multi_call_execution.rs");
include!("local_state_effect/external_side_effect_without_approval_is_refused_not_auto_run.rs");
include!("local_state_effect/external_side_effect_with_approval_runs_when_approved.rs");
include!(
    "local_state_effect/external_side_effect_with_approval_is_denied_when_approval_refused.rs"
);
include!("local_state_effect/session_write_tool_runs_without_either_write_permit.rs");
include!(
    "local_state_effect/workspace_write_tool_is_denied_by_default_and_does_not_end_the_turn.rs"
);
include!("local_state_effect/workspace_write_tool_runs_when_workspace_writes_are_permitted.rs");
