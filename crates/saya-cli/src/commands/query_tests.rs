//! Tests for the ask path's clarification outcome (B3c): a scripted provider
//! that stops to ask must surface on both surfaces — the headless exit code
//! (6, the paused class) and the TUI transcript's Question block.

use super::ask_exit_code;
use crate::agent::tools::DatabaseTools;
use crate::render::{RenderFormat, clarification_text, render_event};
use crate::stream_render::terminal_event;
use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentRequest, AllowReadOnlyApproval, CancellationToken, ChatMessage, ChatProvider,
    ChatRequest, ChatResponse, ProviderError, ProviderStream, ToolCall, run_agent_with_sink,
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

/// One scripted run, both surfaces observed: the headless mapping exits 6,
/// the TUI block carries the same question, and the structured event rides
/// the NDJSON stream.
#[tokio::test]
async fn ambiguous_metric_requires_input_across_surfaces() {
    let provider = ScriptedProvider {
        turns: Mutex::new(vec![clarification_turn()]),
    };
    // The real definitions (the tool advertised) and a no-connector executor:
    // the ask needs no database.
    let definitions = DatabaseTools::definitions(false, false, false, false, false);
    let executor = DatabaseTools::new(None, 10, false);
    let output = run_agent_with_sink(
        &provider,
        &executor,
        agent_request(),
        definitions,
        saya_agent::AgentLimits::default(),
        &AllowReadOnlyApproval,
        &saya_agent::NoopEventSink,
        CancellationToken::new(),
    )
    .await
    .expect("the turn ends on the clarification, never in an error");

    // Headless surface: the run ended on a clarification, so `saya ask`
    // returns the paused exit code, not 0 and not a failure.
    assert_eq!(
        ask_exit_code(&output),
        6,
        "the paused-for-input class is exit 6: {:?}",
        output.events
    );

    // The TUI surface: the transcript's Question block carries the same
    // question. The tui subtree's `apply_event` is private to it, so the
    // block is observed through the shared shaper both adapters render from —
    // the stream_events tests pin that `apply_event` pushes exactly this text.
    let options: Vec<String> = OPTIONS.iter().map(|option| option.to_string()).collect();
    let block_text = clarification_text(QUESTION, &options);
    assert!(
        block_text.contains(QUESTION),
        "the TUI block carries the question: {block_text:?}"
    );
    assert!(
        block_text.contains("1. sessions in the last 30 days")
            && block_text.contains("2. purchases in the last 90 days"),
        "the TUI block numbers the options: {block_text:?}"
    );

    // And the headless NDJSON stream carries the structured event.
    let event = output
        .events
        .iter()
        .find(|event| matches!(event, AgentEvent::ClarificationNeeded { .. }))
        .expect("the clarification event is on the run");
    let terminal = terminal_event(event.clone()).expect("the event renders headlessly");
    let rendered = render_event(&terminal, RenderFormat::Ndjson);
    assert!(
        rendered
            .stdout
            .contains(r#""event":"clarification_needed""#),
        "the ndjson stream tags it: {:?}",
        rendered.stdout
    );
}

/// A run that never asked exits 0 — the mapping must not leak the paused
/// class into ordinary completions.
#[tokio::test]
async fn an_ordinary_completion_exits_zero() {
    let provider = ScriptedProvider {
        turns: Mutex::new(vec![ChatMessage::text("assistant", "42 active users")]),
    };
    let definitions = DatabaseTools::definitions(false, false, false, false, false);
    let executor = DatabaseTools::new(None, 10, false);
    let output = run_agent_with_sink(
        &provider,
        &executor,
        agent_request(),
        definitions,
        saya_agent::AgentLimits::default(),
        &AllowReadOnlyApproval,
        &saya_agent::NoopEventSink,
        CancellationToken::new(),
    )
    .await
    .expect("an ordinary turn completes");
    assert_eq!(ask_exit_code(&output), 0);
    assert_eq!(output.answer, "42 active users");
}
