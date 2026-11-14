//! A `designate_answer` call that arrives with no prose must not end the run
//! with an empty answer: the loop takes one bounded follow-up turn to recover
//! the prose, the designated SQL survives every way the run can end, and the
//! recovery is invisible when the designation already carries its sentence.

use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentLimits, AgentRequest, AllowReadOnlyApproval, CancellationToken, ChatMessage,
    ChatProvider, ChatRequest, ChatResponse, LocalStateEffect, NoopEventSink, ProviderError,
    ProviderEvent, ProviderStream, ToolCall, ToolDefinition, ToolEffect, ToolError, ToolExecutor,
    run_agent, run_agent_with_sink,
};
use std::sync::{Arc, Mutex};

include!("designation_recovery/prose_recovery.rs");
include!("designation_recovery/complete_only.rs");
include!("designation_recovery/repeat_suppression.rs");
include!("designation_recovery/turn_ceiling.rs");

#[derive(Default)]
struct DiagnosticSink(Mutex<Vec<saya_agent::ToolDiagnostic>>);
#[async_trait]
impl saya_agent::AgentEventSink for DiagnosticSink {
    async fn emit(&self, _: AgentEvent) {}
    async fn observe_tool(&self, diagnostic: saya_agent::ToolDiagnostic) {
        self.0.lock().unwrap().push(diagnostic);
    }
}
