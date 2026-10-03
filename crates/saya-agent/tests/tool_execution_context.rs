//! Public executor-context delivery contracts.

use async_trait::async_trait;
use saya_agent::{
    AgentError, AgentLimits, AgentRequest, AllowReadOnlyApproval, CancellationToken, ChatMessage,
    ChatProvider, ChatRequest, ChatResponse, LocalStateEffect, NoopEventSink, ProviderError,
    ToolCall, ToolConcurrency, ToolDefinition, ToolEffect, ToolError, ToolExecutionContext,
    ToolExecutor, run_agent_with_sink,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

struct ScriptedProvider {
    responses: Mutex<Vec<ChatResponse>>,
}

#[async_trait]
impl ChatProvider for ScriptedProvider {
    fn name(&self) -> &str {
        "executor-context"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        Ok(self.responses.lock().unwrap().remove(0))
    }
}

struct LegacyCapExecutor {
    caps: Arc<Mutex<Vec<usize>>>,
}

#[async_trait]
impl ToolExecutor for LegacyCapExecutor {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        Err(ToolError::QueryFailedDetail(
            "legacy cap override was skipped".into(),
        ))
    }

    async fn execute_with_result_cap(
        &self,
        _: &str,
        _: serde_json::Value,
        result_cap: usize,
    ) -> Result<serde_json::Value, ToolError> {
        self.caps.lock().unwrap().push(result_cap);
        Ok(serde_json::json!({"ok": true}))
    }
}

struct ContextExecutor {
    started: AtomicUsize,
    started_notify: tokio::sync::Notify,
    caps: Arc<Mutex<Vec<usize>>>,
}

#[async_trait]
impl ToolExecutor for ContextExecutor {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        Err(ToolError::QueryFailedDetail(
            "context entrypoint was skipped".into(),
        ))
    }

    async fn execute_with_context(
        &self,
        _: &str,
        _: serde_json::Value,
        context: ToolExecutionContext,
    ) -> Result<serde_json::Value, ToolError> {
        self.started.fetch_add(1, Ordering::SeqCst);
        self.started_notify.notify_waiters();
        context.cancellation.cancelled().await;
        self.caps.lock().unwrap().push(context.result_cap);
        Ok(serde_json::json!({"ok": true}))
    }
}

fn request() -> AgentRequest {
    AgentRequest {
        prompt: "test executor context".into(),
        profile_names: Vec::new(),
        model: "mock-model".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

fn tool(name: &str, concurrency: ToolConcurrency) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: "executor context probe".into(),
        read_only: true,
        parameters: serde_json::json!({"type": "object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::None,
        },
        concurrency,
        completion: None,
    }
}

fn provider(calls: Vec<ToolCall>) -> ScriptedProvider {
    ScriptedProvider {
        responses: Mutex::new(vec![
            ChatResponse::new(ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: calls,
                tool_call_id: None,
            }),
            ChatResponse::new(ChatMessage::text("assistant", "done")),
        ]),
    }
}

fn calls(definitions: &[ToolDefinition]) -> Vec<ToolCall> {
    definitions
        .iter()
        .enumerate()
        .map(|(index, definition)| ToolCall {
            id: format!("call-{index}"),
            name: definition.name.clone(),
            arguments: serde_json::json!({}),
        })
        .collect()
}

fn limits() -> AgentLimits {
    AgentLimits {
        context_byte_budget: 5,
        ..AgentLimits::default()
    }
}

async fn wait_for_starts(executor: &ContextExecutor, expected: usize) {
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            let notified = executor.started_notify.notified();
            if executor.started.load(Ordering::SeqCst) >= expected {
                return;
            }
            notified.await;
        }
    })
    .await
    .expect("each context-aware executor call must receive the run token");
}

#[tokio::test]
async fn serial_delivery_uses_legacy_result_cap_override_through_context_default() {
    let caps = Arc::new(Mutex::new(Vec::new()));
    let executor = LegacyCapExecutor { caps: caps.clone() };
    let definitions = vec![
        tool("first", ToolConcurrency::Serial),
        tool("second", ToolConcurrency::Serial),
    ];
    let sink = NoopEventSink;

    let output = run_agent_with_sink(
        &provider(calls(&definitions)),
        &executor,
        request(),
        definitions,
        limits(),
        &AllowReadOnlyApproval,
        &sink,
        CancellationToken::new(),
    )
    .await
    .expect("the legacy cap-aware executor remains compatible");

    assert_eq!(output.answer, "done");
    assert_eq!(&*caps.lock().unwrap(), &[3, 2]);
}

#[tokio::test]
async fn concurrent_delivery_observes_the_original_cancellation_and_exact_caps() {
    let caps = Arc::new(Mutex::new(Vec::new()));
    let executor = ContextExecutor {
        started: AtomicUsize::new(0),
        started_notify: tokio::sync::Notify::new(),
        caps: caps.clone(),
    };
    let definitions = vec![
        tool("first", ToolConcurrency::Concurrent),
        tool("second", ToolConcurrency::Concurrent),
    ];
    let cancellation = CancellationToken::new();
    let sink = NoopEventSink;
    let provider = provider(calls(&definitions));
    let run = run_agent_with_sink(
        &provider,
        &executor,
        request(),
        definitions,
        limits(),
        &AllowReadOnlyApproval,
        &sink,
        cancellation.clone(),
    );
    tokio::pin!(run);

    tokio::select! {
        result = &mut run => panic!("run ended before context delivery: {result:?}"),
        _ = wait_for_starts(&executor, 2) => {}
    }
    cancellation.cancel();
    assert!(matches!(run.await, Err(AgentError::Cancelled)));

    let mut observed = caps.lock().unwrap().clone();
    observed.sort_unstable();
    assert_eq!(observed, vec![2, 3]);
}
