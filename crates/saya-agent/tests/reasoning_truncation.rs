//! A reply the provider cut off at the output cap with no visible text and
//! no tool call spent everything on reasoning. Its continuation message must be
//! the act-now note, not the append-to-file note written for partial file
//! edits — the model never emitted a file, so append advice would mislead it.

use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentEventSink, AgentLimits, AgentRequest, AllowReadOnlyApproval,
    CancellationToken, ChatProvider, ChatRequest, ChatResponse, LocalStateEffect, ProviderError,
    ProviderEvent, ProviderStream, ToolConcurrency, ToolDefinition, ToolEffect, ToolError,
    ToolExecutor, run_agent_with_sink,
};
use std::sync::{Arc, Mutex};

/// The exact act-now wording a reasoning-only truncation must push.
const ACT_NOW_NOTE: &str = "Your previous response used its whole output budget on reasoning and produced no text or tool call; it was discarded. Do not re-plan from the start.";

/// The exact append-to-file wording that must stay for a text truncation.
const APPEND_NOTE: &str = "Your previous response was cut off at the provider's output-token limit. The incomplete part was discarded and is not in the conversation. If you were writing a file, resume it with the append variant of edit using the size and digest reported in your earlier tool result, do not restart the file. Emit less per response so the next one fits.";

/// Scripted stream provider (mirrors `continuation_script_provider`): turn 1
/// streams one reasoning delta then caps with `OutputTruncated` carrying
/// `partial_text` and no tool JSON; later turns answer naturally. Every
/// request is recorded so tests can inspect what the provider saw.
struct ReasoningScriptProvider {
    requests: Arc<Mutex<Vec<ChatRequest>>>,
    calls: Mutex<usize>,
    partial_text: String,
}

#[async_trait]
impl ChatProvider for ReasoningScriptProvider {
    fn name(&self) -> &str {
        "reasoning-truncation-script"
    }
    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!("the main loop drives the provider through stream")
    }
    async fn stream(
        &self,
        request: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        self.requests.lock().unwrap().push(request);
        let turn = {
            let mut calls = self.calls.lock().unwrap();
            *calls += 1;
            *calls
        };
        if turn > 1 {
            return Ok(Box::pin(futures_util::stream::iter(vec![
                Ok(ProviderEvent::TextDelta("done".into())),
                Ok(ProviderEvent::Done),
            ])));
        }
        Ok(Box::pin(futures_util::stream::iter(vec![
            Ok(ProviderEvent::ReasoningDelta("thinking hard".into())),
            Err(ProviderError::output_truncated(
                self.partial_text.clone(),
                Vec::new(),
            )),
        ])))
    }
}

struct NoTools;

#[async_trait]
impl ToolExecutor for NoTools {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        Ok(serde_json::json!({"rows": 1}))
    }
}

struct NoopSink;

#[async_trait]
impl AgentEventSink for NoopSink {
    async fn emit(&self, _: AgentEvent) {}
}

/// Runs one truncated turn (carrying `partial_text`) plus its continuation,
/// and returns every request the provider received.
async fn run_continuation(partial_text: &str) -> Vec<ChatRequest> {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = ReasoningScriptProvider {
        requests: requests.clone(),
        calls: Mutex::new(0),
        partial_text: partial_text.into(),
    };
    run_agent_with_sink(
        &provider,
        &NoTools,
        AgentRequest {
            prompt: "show data".into(),
            profile_names: vec!["analytics".into()],
            model: "mock-model".into(),
            system_prompt: None,
            history: Vec::new(),
            context_blocks: Vec::new(),
        },
        vec![ToolDefinition {
            name: "bounded_sql_query".into(),
            description: "read-only query".into(),
            read_only: true,
            parameters: serde_json::json!({"type":"object"}),
            effect: ToolEffect {
                database_data: true,
                external_side_effect: false,
                requires_approval: true,
                local_state: LocalStateEffect::None,
            },
            concurrency: ToolConcurrency::Serial,
            completion: None,
        }],
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &NoopSink,
        CancellationToken::new(),
    )
    .await
    .expect("the run recovers after one truncation");
    requests.lock().unwrap().clone()
}

/// The continuation note the provider saw next: the second request's last
/// message, asserted to be user-role.
fn continuation_note(requests: &[ChatRequest]) -> String {
    assert_eq!(requests.len(), 2, "truncated plus continuation");
    let last = requests[1]
        .messages
        .last()
        .expect("the continuation request carries messages");
    assert_eq!(last.role, "user", "the continuation note is user-role");
    last.content.clone()
}

/// A truncation that emitted only reasoning continues with the act-now note.
#[tokio::test]
async fn reasoning_only_truncation_continues_with_the_act_now_note() {
    let requests = run_continuation("").await;
    assert_eq!(continuation_note(&requests), ACT_NOW_NOTE, "act-now note");
}

/// A truncation that emitted visible text keeps the append-to-file note.
#[tokio::test]
async fn text_truncation_keeps_the_append_continuation_note() {
    let requests = run_continuation("x").await;
    assert_eq!(continuation_note(&requests), APPEND_NOTE, "append note");
}
