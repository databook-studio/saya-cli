struct ScriptedProvider {
    requests: Arc<Mutex<Vec<ChatRequest>>>,
    responses: Mutex<VecDeque<ChatResponse>>,
}

#[async_trait]
impl ChatProvider for ScriptedProvider {
    fn name(&self) -> &str {
        "scripted"
    }

    async fn complete(
        &self,
        request: ChatRequest,
    ) -> Result<ChatResponse, saya_agent::ProviderError> {
        self.requests.lock().unwrap().push(request);
        Ok(self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("the test supplies every provider response"))
    }
}

struct RecordingTools {
    calls: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl ToolExecutor for RecordingTools {
    async fn execute(
        &self,
        name: &str,
        _: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        self.calls.lock().unwrap().push(name.into());
        Ok(serde_json::json!({"ok": true}))
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

fn request() -> AgentRequest {
    AgentRequest {
        prompt: "inspect the schema".into(),
        profile_names: vec!["analytics".into()],
        model: "mock-model".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

fn definitions() -> Vec<ToolDefinition> {
    vec![definition("schema_discovery")]
}

fn definitions_with_shortcuts() -> Vec<ToolDefinition> {
    vec![
        definition("schema_discovery"),
        definition(REQUEST_CLARIFICATION_TOOL),
        definition(DESIGNATE_ANSWER_TOOL),
    ]
}

fn definition(name: &str) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: "inspect schema".into(),
        read_only: true,
        parameters: serde_json::json!({"type": "object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::None,
        },
        concurrency: ToolConcurrency::Serial,
        completion: None,
    }
}

fn call(id: &str, name: &str, arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: name.into(),
        arguments,
    }
}

fn assistant(calls: Vec<ToolCall>) -> ChatMessage {
    ChatMessage {
        role: "assistant".into(),
        content: String::new(),
        tool_calls: calls,
        tool_call_id: None,
    }
}

fn provider(responses: Vec<ChatResponse>) -> (ScriptedProvider, Arc<Mutex<Vec<ChatRequest>>>) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    (
        ScriptedProvider {
            requests: requests.clone(),
            responses: Mutex::new(responses.into()),
        },
        requests,
    )
}
