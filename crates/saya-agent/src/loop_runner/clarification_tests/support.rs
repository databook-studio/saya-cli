/// A provider that pops one scripted response per `stream` call and counts
/// the requests it received — the no-further-provider-call assertion reads
/// the count.
struct ScriptedProvider {
    turns: Mutex<Vec<ChatMessage>>,
    requests: Mutex<usize>,
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
        *self.requests.lock().unwrap() += 1;
        let message = self.turns.lock().unwrap().remove(0);
        let events = if message.tool_calls.is_empty() {
            vec![
                Ok(ProviderEvent::TextDelta(message.content)),
                Ok(ProviderEvent::Done),
            ]
        } else {
            vec![
                Ok(ProviderEvent::ToolCalls(message.tool_calls)),
                Ok(ProviderEvent::Done),
            ]
        };
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}

/// Records every executed tool name; answers anything with a fixed marker.
struct RecordingTools {
    calls: Arc<Mutex<Vec<String>>>,
    fail_names: Vec<String>,
}

#[async_trait]
impl ToolExecutor for RecordingTools {
    async fn execute(
        &self,
        name: &str,
        _: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        self.calls.lock().unwrap().push(name.into());
        if self.fail_names.iter().any(|failed| failed == name) {
            return Err(ToolError::UnsupportedTool);
        }
        Ok(serde_json::json!({"asked": true}))
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

fn clarification_definition() -> ToolDefinition {
    ToolDefinition {
        name: REQUEST_CLARIFICATION_TOOL.into(),
        description: "ask one focused question".into(),
        read_only: true,
        parameters: serde_json::json!({"type": "object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::None,
        },
        concurrency: crate::ToolConcurrency::Serial,
        completion: Some("question asked — the turn pauses for the user's answer".into()),
    }
}

fn definitions() -> Vec<ToolDefinition> {
    vec![
        clarification_definition(),
        ToolDefinition {
            name: "schema_discovery".into(),
            description: "schema discovery".into(),
            read_only: true,
            parameters: serde_json::json!({"type": "object"}),
            effect: ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: false,
                local_state: LocalStateEffect::None,
            },
            concurrency: crate::ToolConcurrency::Serial,
            completion: None,
        },
    ]
}

fn request() -> AgentRequest {
    AgentRequest {
        prompt: "how many active users".into(),
        profile_names: vec!["analytics".into()],
        model: "mock-model".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

fn clarification_call(
    question: impl Into<serde_json::Value>,
    options: impl Into<serde_json::Value>,
) -> ToolCall {
    ToolCall {
        id: "call-1".into(),
        name: REQUEST_CLARIFICATION_TOOL.into(),
        arguments: serde_json::json!({"question": question.into(), "options": options.into()}),
    }
}

struct CustomDenyApproval;

#[async_trait]
impl ApprovalDecider for CustomDenyApproval {
    async fn approve(&self, _: &ToolDefinition, _: &serde_json::Value) -> bool {
        false
    }

    fn refusal_detail(&self, _: &ToolDefinition, _: &serde_json::Value) -> Option<String> {
        Some("questions require an approved investigation plan".into())
    }
}
