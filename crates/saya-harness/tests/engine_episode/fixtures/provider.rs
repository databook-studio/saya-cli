/// One scripted provider turn.
enum Turn {
    /// An assistant turn carrying tool calls.
    Tools(Vec<ToolCall>),
    /// A terminal prose answer (no tool calls).
    Answer(&'static str),
    /// A provider failure.
    Fail,
}

/// A scripted `ChatProvider`: serves its turns in order and records every
/// request it received, so tests assert on the definitions and the brief
/// the loop actually sent.
struct ScriptProvider {
    script: Mutex<VecDeque<Turn>>,
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}

impl ScriptProvider {
    fn new(script: Vec<Turn>) -> Self {
        Self {
            script: Mutex::new(script.into()),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn requests(&self) -> Vec<ChatRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl ChatProvider for ScriptProvider {
    fn name(&self) -> &str {
        "script"
    }

    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        self.requests.lock().unwrap().push(request);
        match self.script.lock().unwrap().pop_front() {
            Some(Turn::Answer(text)) => Ok(ChatResponse::new(ChatMessage::text("assistant", text))),
            Some(Turn::Tools(calls)) => Ok(ChatResponse::new(ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: calls,
                tool_call_id: None,
            })),
            Some(Turn::Fail) => Err(ProviderError::Request("scripted failure".into())),
            None => panic!("provider script exhausted"),
        }
    }
}
