/// A decider that counts how many times it was asked and answers `answer`.
/// The stand-in for a session's approval surface — the terminal prompt, the
/// TUI's modal — so a test can pin who is asked and how often.
struct RecordingOperator {
    calls: std::sync::atomic::AtomicUsize,
    answer: bool,
}

impl RecordingOperator {
    fn new(answer: bool) -> Arc<Self> {
        Arc::new(Self {
            calls: std::sync::atomic::AtomicUsize::new(0),
            answer,
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl saya_agent::ApprovalDecider for RecordingOperator {
    async fn approve(
        &self,
        _tool: &saya_agent::ToolDefinition,
        _arguments: &serde_json::Value,
    ) -> bool {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.answer
    }
}

/// A provider that issues one `shell` call and then answers: the scripted
/// turn the end-to-end wiring test drives.
struct ScriptedShell {
    issued: std::sync::atomic::AtomicBool,
    arguments: serde_json::Value,
}

impl ScriptedShell {
    fn new(arguments: serde_json::Value) -> Self {
        Self {
            issued: std::sync::atomic::AtomicBool::new(false),
            arguments,
        }
    }
}

#[async_trait::async_trait]
impl saya_agent::ChatProvider for ScriptedShell {
    fn name(&self) -> &str {
        "scripted-shell"
    }

    async fn complete(
        &self,
        _request: saya_agent::ChatRequest,
    ) -> Result<saya_agent::ChatResponse, saya_agent::ProviderError> {
        if !self.issued.swap(true, std::sync::atomic::Ordering::SeqCst) {
            let mut message = saya_agent::ChatMessage::text("assistant", "");
            message.tool_calls = vec![saya_agent::ToolCall {
                id: "call-1".into(),
                name: "shell".into(),
                arguments: self.arguments.clone(),
            }];
            Ok(saya_agent::ChatResponse::new(message))
        } else {
            Ok(saya_agent::ChatResponse::new(
                saya_agent::ChatMessage::text("assistant", "done"),
            ))
        }
    }
}

/// The turn's events, kept for the assertions.
struct ShellTurnSink(std::sync::Mutex<Vec<saya_agent::AgentEvent>>);

#[async_trait::async_trait]
impl saya_agent::AgentEventSink for ShellTurnSink {
    async fn emit(&self, event: saya_agent::AgentEvent) {
        self.0.lock().unwrap().push(event);
    }
}
