//! Loop invariant: a call that already failed in this run — a SQL statement
//! or a non-SQL tool invocation — is not re-executed, and a run that
//! exhausts its budget surfaces its best available answer. These drive
//! `run_agent_with_sink` end-to-end with a canned provider and a recording
//! executor, asserting the observable contract: execution counts, the
//! refusal message the model receives, and the nominated SQL on salvage.
//!
//! The cap on remembered failures is pinned by an inline unit test in
//! `loop_runner/failed_statements.rs` (it references the private
//! `MAX_REMEMBERED_FAILURES` constant); this file covers the end-to-end
//! behaviours.

use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentEventSink, AgentLimits, AgentRequest, AllowReadOnlyApproval, ChatMessage,
    ChatProvider, ChatRequest, ChatResponse, LocalStateEffect, ProviderError, ToolCall,
    ToolDefinition, ToolEffect, ToolError, ToolExecutor, run_agent_with_sink,
};
use std::sync::{Arc, Mutex};

include!("repeat_failure/failure_support.rs");
include!("repeat_failure/sql_retries.rs");
include!("repeat_failure/budget_nomination.rs");
include!("repeat_failure/other_tool_retries.rs");
