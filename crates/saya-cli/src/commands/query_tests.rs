//! Tests for the ask path's clarification outcome (B3c): a scripted provider
//! that stops to ask must surface on both surfaces — the headless exit code
//! (6, the paused class) and the TUI transcript's Question block.

use super::ask_exit_code;
use super::settle_ask;
use crate::agent::runtime::AgentRuntimeError;
use crate::agent::tools::DatabaseTools;
use crate::commands::{capture_output_start, capture_output_take};
use crate::render::{RenderFormat, clarification_text, render_event};
use crate::stream_render::terminal_event;
use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentOutput, AgentRequest, AllowReadOnlyApproval, BudgetDimension,
    CancellationReason, CancellationToken, ChatMessage, ChatProvider, ChatRequest, ChatResponse,
    ProviderError, ProviderStream, TokenUsage, ToolCall, run_agent_with_sink,
};
use std::sync::Mutex;

/// The scripted provider: one turn — the model asks which metric "active
/// users" means instead of assuming one. A second provider call would pop
/// from an empty vec and panic, so "the turn ended" is proven by the run
/// returning at all.
struct ScriptedProvider {
    turns: Mutex<Vec<ChatMessage>>,
}

#[async_trait]
impl ChatProvider for ScriptedProvider {
    fn name(&self) -> &str {
        "scripted"
    }
    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        panic!("the loop must drive providers through stream()")
    }
    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        let message = self.turns.lock().unwrap().remove(0);
        let events = if message.tool_calls.is_empty() {
            vec![
                Ok(saya_agent::ProviderEvent::TextDelta(message.content)),
                Ok(saya_agent::ProviderEvent::Done),
            ]
        } else {
            vec![
                Ok(saya_agent::ProviderEvent::ToolCalls(message.tool_calls)),
                Ok(saya_agent::ProviderEvent::Done),
            ]
        };
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}

/// The question the scripted model asks — deliberately a metric ambiguity,
/// the material definition the guidance names.
const QUESTION: &str = "Which definition of \"active users\" should I use?";
const OPTIONS: [&str; 2] = [
    "sessions in the last 30 days",
    "purchases in the last 90 days",
];

fn clarification_turn() -> ChatMessage {
    ChatMessage {
        role: "assistant".into(),
        content: String::new(),
        tool_calls: vec![ToolCall {
            id: "call-1".into(),
            name: saya_agent::REQUEST_CLARIFICATION_TOOL.into(),
            arguments: serde_json::json!({
                "question": QUESTION,
                "options": OPTIONS,
            }),
        }],
        tool_call_id: None,
    }
}

fn agent_request() -> AgentRequest {
    AgentRequest {
        prompt: "how many active users do we have".into(),
        profile_names: vec!["analytics".into()],
        model: "mock-model".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

include!("query_tests/clarification_surface.rs");

// ===========================================================================
// The one classification for a settled `ask`. After the work has settled —
// on the work-ready branch or the setup-error branch, and independent of
// which select branch won — the question token's first stop reason decides:
// User 130, Budget 6 with truthful partial wording, OwnerFailure 5, and only
// a run that stopped for no typed reason falls through to the old mapping
// (clarification 6, natural 0, setup error 5).
// ===========================================================================

/// A natural completion: the model answered, nothing stopped.
fn natural_output() -> AgentOutput {
    AgentOutput {
        answer: "42 active users".into(),
        events: Vec::new(),
        used_bounded_sql_query: true,
        tool_metadata: Vec::new(),
        usage: TokenUsage::default(),
        learning_usage: None,
        truncated: false,
        answer_sql: None,
    }
}

/// The same completion whose run paused for input instead of answering.
fn clarification_output() -> AgentOutput {
    let mut out = natural_output();
    out.events.push(AgentEvent::clarification_needed(
        QUESTION,
        OPTIONS.iter().map(|option| option.to_string()).collect(),
    ));
    out
}

include!("query_tests/settled_exit_codes.rs");
