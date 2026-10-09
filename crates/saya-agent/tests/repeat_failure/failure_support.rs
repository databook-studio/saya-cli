/// A provider that returns one canned assistant message per turn (in order)
/// and records every request it was sent, so a test can inspect the tool
/// messages the model received. The default `stream` delegates to `complete`,
/// which the loop drives once per turn.
struct CannedProvider {
    responses: Mutex<Vec<ChatMessage>>,
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}

#[async_trait]
impl ChatProvider for CannedProvider {
    fn name(&self) -> &str {
        "canned"
    }
    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        self.requests.lock().unwrap().push(request);
        let message = self.responses.lock().unwrap().remove(0);
        Ok(ChatResponse::new(message))
    }
}

/// An executor that records each call's name and either always fails (with a
/// recognisable error) or always succeeds. The recorded length is the
/// execution count — the core assertion for repeat refusal.
struct ScriptedExecutor {
    calls: Arc<Mutex<Vec<String>>>,
    fail: bool,
    error: String,
}

/// A failing executor whose two calls must rendezvous. It observes the agent
/// scheduler at the public boundary: serial execution reaches the timeout,
/// while the batch path releases both failures for repeat-memory recording.
struct BatchFailingExecutor {
    calls: Arc<Mutex<Vec<String>>>,
    barrier: Arc<tokio::sync::Barrier>,
}

#[async_trait]
impl ToolExecutor for BatchFailingExecutor {
    async fn execute(
        &self,
        name: &str,
        _: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        self.calls.lock().unwrap().push(name.into());
        self.barrier.wait().await;
        Err(ToolError::QueryFailedDetail("bad batch".into()))
    }
}

#[async_trait]
impl ToolExecutor for ScriptedExecutor {
    async fn execute(
        &self,
        name: &str,
        _: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        self.calls.lock().unwrap().push(name.into());
        if self.fail {
            Err(ToolError::QueryFailedDetail(self.error.clone()))
        } else {
            Ok(serde_json::json!({"rows": 1}))
        }
    }
}

struct RecordingSink {
    events: Arc<Mutex<Vec<AgentEvent>>>,
}

#[async_trait]
impl AgentEventSink for RecordingSink {
    async fn emit(&self, event: AgentEvent) {
        self.events.lock().unwrap().push(event);
    }
}

/// A read-only database query tool carrying a `sql` string argument. It needs
/// no approval so single-call turns run on the sequential path and multi-call
/// turns run on the batch path — both exercise the repeat-refusal and outcome
/// recording logic.
fn query_tool() -> ToolDefinition {
    ToolDefinition {
        name: "bounded_sql_query".into(),
        description: "read-only query".into(),
        read_only: true,
        parameters: serde_json::json!({"type":"object"}),
        effect: ToolEffect {
            database_data: true,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::None,
        },
        concurrency: saya_agent::ToolConcurrency::Concurrent,
        completion: None,
    }
}

fn call(id: &str, sql: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: "bounded_sql_query".into(),
        arguments: serde_json::json!({"sql": sql}),
    }
}

fn tool_turn(id: &str, sql: &str) -> ChatMessage {
    ChatMessage {
        role: "assistant".into(),
        content: String::new(),
        tool_calls: vec![call(id, sql)],
        tool_call_id: None,
    }
}

/// A non-SQL tool the executor can fail on demand. It needs no approval and
/// declares no effects, so like the query tool it auto-runs; before the
/// anti-repeat memory was generalized beyond `sql`, an identical failing
/// call to it was never a repeat.
fn program_tool() -> ToolDefinition {
    ToolDefinition {
        name: "run_program".into(),
        description: "run a program".into(),
        read_only: false,
        parameters: serde_json::json!({"type":"object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::None,
        },
        concurrency: saya_agent::ToolConcurrency::Serial,
        completion: None,
    }
}

fn program_call(id: &str, command: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: "run_program".into(),
        arguments: serde_json::json!({"command": command}),
    }
}

fn program_turn(id: &str, command: &str) -> ChatMessage {
    ChatMessage {
        role: "assistant".into(),
        content: String::new(),
        tool_calls: vec![program_call(id, command)],
        tool_call_id: None,
    }
}

fn request() -> AgentRequest {
    AgentRequest {
        prompt: "answer the question".into(),
        profile_names: vec!["analytics".into()],
        model: "mock-model".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

/// Runs the agent with a canned provider returning the given responses and a
/// scripted executor, returning `(output, calls, requests)`.
async fn run(
    responses: Vec<ChatMessage>,
    executor: ScriptedExecutor,
    limits: AgentLimits,
) -> (saya_agent::AgentOutput, Vec<String>, Vec<ChatRequest>) {
    run_with_tools(responses, executor, limits, vec![query_tool()]).await
}

/// Like [`run`], but with explicit tool definitions — the non-SQL repeat
/// tests register `run_program` instead of the query tool.
async fn run_with_tools(
    responses: Vec<ChatMessage>,
    executor: ScriptedExecutor,
    limits: AgentLimits,
    definitions: Vec<ToolDefinition>,
) -> (saya_agent::AgentOutput, Vec<String>, Vec<ChatRequest>) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = CannedProvider {
        responses: Mutex::new(responses),
        requests: requests.clone(),
    };
    let output = run_agent_with_sink(
        &provider,
        &executor,
        request(),
        definitions,
        limits,
        &AllowReadOnlyApproval,
        &RecordingSink {
            events: Arc::new(Mutex::new(Vec::new())),
        },
        saya_agent::CancellationToken::new(),
    )
    .await
    .expect("run completes");
    let calls = executor.calls.lock().unwrap().clone();
    let reqs = requests.lock().unwrap().clone();
    (output, calls, reqs)
}

/// The most recent `tool`-role message whose content contains `needle`, from
/// the messages sent on the last provider turn — i.e. what the model actually
/// received for the failing/repeated call.
fn last_tool_message_containing(requests: &[ChatRequest], needle: &str) -> Option<String> {
    requests.last().and_then(|req| {
        req.messages
            .iter()
            .rev()
            .find(|m| m.role == "tool" && m.content.contains(needle))
            .map(|m| m.content.clone())
    })
}
