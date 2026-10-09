//! Tests for the clarification arm (B3c): a `request_clarification` call ends
//! the turn with a `ClarificationNeeded` event and a short tool result — no
//! further provider call — while a malformed call feeds a tool error and the
//! turn continues.

use super::super::clarification_args::{REQUEST_CLARIFICATION_TOOL, parse};
use crate::{
    AgentEvent, AgentEventSink, AgentLimits, AgentRequest, AllowReadOnlyApproval, ApprovalDecider,
    CancellationToken, ChatMessage, ChatProvider, ChatRequest, ChatResponse, LocalStateEffect,
    ProviderError, ProviderEvent, ProviderStream, ToolCall, ToolDefinition, ToolEffect, ToolError,
    ToolExecutor, run_agent_with_sink,
};
use async_trait::async_trait;
use std::sync::{Arc, Mutex};

include!("clarification_tests/support.rs");
include!("clarification_tests/turn_results.rs");
include!("clarification_tests/refusal_paths.rs");
include!("clarification_tests/argument_contract.rs");
