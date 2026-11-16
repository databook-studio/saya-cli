use async_trait::async_trait;
use saya_agent::*;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Mutex, Notify};

struct Provider {
    calls: AtomicUsize,
    messages: Mutex<Vec<ChatMessage>>,
}
#[async_trait]
impl ChatProvider for Provider {
    fn name(&self) -> &str {
        "steering-fixture"
    }
    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!()
    }
    async fn stream(
        &self,
        request: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        *self.messages.lock().await = request.messages;
        let events = if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            vec![
                ProviderEvent::ToolCalls(vec![ToolCall {
                    id: "read-1".into(),
                    name: "read".into(),
                    arguments: serde_json::json!({}),
                }]),
                ProviderEvent::Done,
            ]
        } else {
            vec![
                ProviderEvent::TextDelta("finished".into()),
                ProviderEvent::Done,
            ]
        };
        Ok(Box::pin(futures_util::stream::iter(
            events.into_iter().map(Ok),
        )))
    }
}
struct Tools {
    retained: AtomicBool,
    settling: Notify,
    settled: Notify,
    entered: Notify,
    release: Notify,
}
#[async_trait]
impl ToolExecutor for Tools {
    async fn settle_invocation(&self) -> Result<(), ToolError> {
        if self.retained.load(Ordering::SeqCst) {
            self.settling.notify_one();
            self.settled.notified().await;
            self.retained.store(false, Ordering::SeqCst);
        }
        Ok(())
    }

    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        self.retained.store(true, Ordering::SeqCst);
        self.entered.notify_one();
        self.release.notified().await;
        Ok(serde_json::json!({"value": 7}))
    }
}
fn start(
    provider: Arc<Provider>,
    tools: Arc<Tools>,
    mailbox: AgentMailbox,
    token: CancellationToken,
) -> tokio::task::JoinHandle<Result<AgentOutput, AgentError>> {
    tokio::spawn(async move {
        let request = AgentRequest {
            prompt: "inspect".into(),
            profile_names: vec![],
            model: "fixture".into(),
            system_prompt: None,
            history: vec![],
            context_blocks: vec![],
        };
        let definitions = vec![ToolDefinition {
            name: "read".into(),
            description: "read fixture".into(),
            read_only: true,
            parameters: serde_json::json!({"type":"object"}),
            effect: ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: false,
                local_state: LocalStateEffect::Read,
            },
            concurrency: ToolConcurrency::Serial,
            completion: None,
        }];
        let control = AgentRunControl::new(
            AgentLimits {
                max_turns: Some(3),
                ..AgentLimits::default()
            },
            AnsweringLimits {
                max_requests: Some(3),
                max_tool_calls: Some(2),
                max_reported_tokens: None,
            },
            &AllowReadOnlyApproval,
            &NoopEventSink,
            token,
        )
        .with_mailbox(mailbox);
        run_agent_with_control(
            provider.as_ref(),
            tools.as_ref(),
            request,
            definitions,
            control,
        )
        .await
    })
}
fn fixture() -> (Arc<Provider>, Arc<Tools>) {
    (
        Arc::new(Provider {
            calls: AtomicUsize::new(0),
            messages: Mutex::new(vec![]),
        }),
        Arc::new(Tools {
            retained: AtomicBool::new(false),
            settling: Notify::new(),
            settled: Notify::new(),
            entered: Notify::new(),
            release: Notify::new(),
        }),
    )
}
#[tokio::test]
async fn pause_waits_for_tool_join_then_delivers_bounded_steering() {
    let (provider, tools) = fixture();
    let (steering, mailbox) = agent_mailbox();
    let task = start(
        provider.clone(),
        tools.clone(),
        mailbox,
        CancellationToken::new(),
    );
    tokio::time::timeout(Duration::from_secs(2), tools.entered.notified())
        .await
        .unwrap();
    let ticket = steering.pause().unwrap();
    assert!(!steering.is_paused(ticket));
    steering.send("focus on the schema").unwrap();
    tools.release.notify_one();
    tokio::time::timeout(Duration::from_secs(2), tools.settling.notified())
        .await
        .unwrap();
    assert!(!steering.is_paused(ticket));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    tools.settled.notify_one();
    assert!(
        tokio::time::timeout(Duration::from_secs(2), steering.wait_paused(ticket))
            .await
            .unwrap()
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    steering.resume().unwrap();
    let output = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(output.answer, "finished");
    let messages = provider.messages.lock().await;
    assert!(
        messages
            .iter()
            .any(|message| message.role == "user" && message.content == "focus on the schema")
    );
}
#[tokio::test]
async fn full_mailbox_cannot_block_cancellation_at_paused_boundary() {
    let (provider, tools) = fixture();
    let (steering, mailbox) = agent_mailbox();
    let ticket = steering.pause().unwrap();
    for _ in 0..MAX_STEERING_MESSAGES {
        steering.send("queued").unwrap();
    }
    assert_eq!(steering.send("overflow"), Err(SteeringError::Full));
    assert_eq!(
        steering.send(&"x".repeat(MAX_STEERING_MESSAGE_BYTES + 1)),
        Err(SteeringError::TooLarge)
    );
    let token = CancellationToken::new();
    let task = start(provider.clone(), tools, mailbox, token.clone());
    assert!(
        tokio::time::timeout(Duration::from_secs(2), steering.wait_paused(ticket))
            .await
            .unwrap()
    );
    token.cancel();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap(),
        Err(AgentError::Cancelled)
    ));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    assert_eq!(steering.send("late"), Err(SteeringError::Closed));
}
