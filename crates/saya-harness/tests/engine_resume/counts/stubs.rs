//! The battery's doubles: a scripted answer provider that counts admitted
//! requests and serves answer-or-tool-call turns, an event sink that
//! holds the completion event open once it is signalled, and a tool
//! executor that counts executions.

use std::collections::VecDeque;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentEventSink, ChatMessage, ChatProvider, ChatRequest, ChatResponse,
    ProviderError, ToolCall, ToolError, ToolExecutor,
};
use tokio::sync::oneshot;

pub(super) struct ScriptedAnswerProvider {
    script: Mutex<VecDeque<ResumeTurn>>,
    requests: AtomicUsize,
}

pub(super) enum ResumeTurn {
    Answer(&'static str),
    Tool(ToolCall),
}

impl ScriptedAnswerProvider {
    pub(super) fn new(script: Vec<ResumeTurn>) -> Self {
        Self {
            script: Mutex::new(script.into()),
            requests: AtomicUsize::new(0),
        }
    }

    pub(super) fn count(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ChatProvider for ScriptedAnswerProvider {
    fn name(&self) -> &str {
        "count-resume"
    }

    async fn complete(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        let turn = self.script.lock().unwrap().pop_front().unwrap();
        let message = match turn {
            ResumeTurn::Answer(text) => ChatMessage::text("assistant", text),
            ResumeTurn::Tool(call) => ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![call],
                tool_call_id: None,
            },
        };
        Ok(ChatResponse::new(message))
    }
}

pub(super) struct HoldComplete(pub(super) Mutex<Option<oneshot::Sender<()>>>);

#[async_trait]
impl AgentEventSink for HoldComplete {
    async fn emit(&self, event: AgentEvent) {
        if matches!(event, AgentEvent::Complete) {
            let started = self.0.lock().unwrap().take();
            if let Some(started) = started {
                let _ = started.send(());
                std::future::pending::<()>().await;
            }
        }
    }
}

#[derive(Default)]
pub(super) struct CountedTool(pub(super) AtomicUsize);

#[async_trait]
impl ToolExecutor for CountedTool {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(serde_json::json!({"ok": true}))
    }
}
