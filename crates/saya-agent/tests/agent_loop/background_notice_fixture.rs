// The background-notice battery's fixtures: a provider that records every
// request, a tool surface whose live runs and exits the test drives, and the
// controls each test runs under.

use std::collections::VecDeque;
use std::time::Instant;

use saya_agent::{BackgroundExit, ProgressLimits, ProviderError};

/// Records every request and answers from a script. A request the script has
/// no answer for is a test failure, not an empty reply.
struct ScriptedProvider {
    answers: Mutex<VecDeque<ChatResponse>>,
    requests: Arc<Mutex<Vec<(Instant, ChatRequest)>>>,
}

#[async_trait::async_trait]
impl saya_agent::ChatProvider for ScriptedProvider {
    fn name(&self) -> &str {
        "scripted-background"
    }

    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        self.requests
            .lock()
            .unwrap()
            .push((Instant::now(), request));
        Ok(self
            .answers
            .lock()
            .unwrap()
            .pop_front()
            .expect("the script has an answer for every request"))
    }
}

fn text(answer: &str) -> ChatResponse {
    ChatResponse::new(ChatMessage::text("assistant", answer))
}

fn tool_call(name: &str) -> ChatResponse {
    ChatResponse::new(ChatMessage {
        role: "assistant".into(),
        content: String::new(),
        tool_calls: vec![ToolCall {
            id: "call-1".into(),
            name: name.into(),
            arguments: serde_json::json!({}),
        }],
        tool_call_id: None,
        images: Vec::new(),
        reasoning: None,
        reasoning_signature: None,
    })
}

fn bg_definitions() -> Vec<ToolDefinition> {
    vec![ToolDefinition {
        name: "start_bg".into(),
        description: "start a background run".into(),
        read_only: true,
        parameters: serde_json::json!({"type": "object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: saya_agent::LocalStateEffect::None,
        },
        concurrency: saya_agent::ToolConcurrency::Concurrent,
        completion: None,
    }]
}

#[derive(Default)]
struct FakeState {
    live: bool,
    /// Exits the tool call produces, so a test can make them happen while the
    /// turn is running rather than before it started.
    starts: Vec<BackgroundExit>,
    pending: Vec<BackgroundExit>,
    /// The next wait produces this exit after the named delay, unless the
    /// deadline arrives first.
    exit_on_wait: Option<(Duration, BackgroundExit)>,
}

/// A surface whose background state the test drives: the tool call makes a
/// run live, and the test decides when that run ends.
#[derive(Clone, Default)]
struct FakeBackground {
    state: Arc<Mutex<FakeState>>,
}

impl FakeBackground {
    fn exit(&self, exit: BackgroundExit) {
        let mut state = self.state.lock().unwrap();
        state.live = false;
        state.pending.push(exit);
    }

    fn pending(&self) -> bool {
        !self.state.lock().unwrap().pending.is_empty()
    }
}

#[async_trait::async_trait]
impl ToolExecutor for FakeBackground {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let mut state = self.state.lock().unwrap();
        let starts = std::mem::take(&mut state.starts);
        // Exits the test injected with the call mean the runs already ended;
        // otherwise the start leaves one live.
        state.live = starts.is_empty();
        state.pending.extend(starts);
        Ok(serde_json::json!({"id": 1, "status": "running"}))
    }

    fn take_background_exits(&self) -> Vec<BackgroundExit> {
        std::mem::take(&mut self.state.lock().unwrap().pending)
    }

    fn background_live(&self) -> bool {
        self.state.lock().unwrap().live
    }

    fn background_notice_pending(&self) -> bool {
        self.pending()
    }

    async fn wait_background_exit(
        &self,
        deadline: Instant,
        _cancellation: &CancellationToken,
    ) -> bool {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let (sleep, exit) = {
            let state = self.state.lock().unwrap();
            match &state.exit_on_wait {
                Some((after, exit)) => ((*after).min(remaining), Some(exit.clone())),
                None => (remaining, None),
            }
        };
        tokio::time::sleep(sleep).await;
        let mut state = self.state.lock().unwrap();
        if let Some(exit) = exit {
            state.exit_on_wait = None;
            state.live = false;
            state.pending.push(exit);
        }
        !state.pending.is_empty()
    }
}

fn exit(id: u64, code: Option<i32>, killed: bool, secs: u64, lines: u64) -> BackgroundExit {
    BackgroundExit {
        id,
        exit_code: code,
        killed,
        elapsed: Duration::from_secs(secs),
        output_lines: lines,
    }
}

/// Records the notices the loop shows to a display surface.
#[derive(Default)]
struct NoticeSink {
    background: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl saya_agent::AgentEventSink for NoticeSink {
    async fn emit(&self, _: saya_agent::AgentEvent) {}

    async fn background_exit(&self, text: &str) {
        self.background.lock().unwrap().push(text.to_owned());
    }
}

fn control<'a>(
    sink: &'a dyn saya_agent::AgentEventSink,
    cancellation: CancellationToken,
    wall_clock: Option<Duration>,
    background_wait: Option<Duration>,
) -> saya_agent::AgentRunControl<'a> {
    let limits = AgentLimits {
        progress: Some(ProgressLimits {
            wall_clock,
            ..ProgressLimits::default()
        }),
        ..AgentLimits::default()
    };
    let control = saya_agent::AgentRunControl::new(
        limits,
        saya_agent::AnsweringLimits::default(),
        &AllowReadOnlyApproval,
        sink,
        cancellation,
    );
    match background_wait {
        Some(slice) => control.with_background_wait(slice),
        None => control,
    }
}

fn notice_messages(request: &ChatRequest) -> Vec<&str> {
    request
        .messages
        .iter()
        .filter(|message| message.role == "user" && message.content.contains("background shell"))
        .map(|message| message.content.as_str())
        .collect()
}

fn bg_request() -> AgentRequest {
    AgentRequest {
        prompt: "start it".into(),
        profile_names: vec![],
        model: "mock-model".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}
