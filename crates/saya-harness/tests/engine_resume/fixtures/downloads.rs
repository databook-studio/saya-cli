/// A fetch-capable executor's stand-in: every call claims `bytes` against
/// the shared wallet — the clone the step executors hold — and surfaces a
/// refused claim as the typed tool error a download reports, so the model
/// reads the real reason. The tool's name and shape are the stand-in's own;
/// what it must share with the resume is the wallet, the sharing a
/// fetch-capable run builds at assembly.
struct Claiming {
    wallet: DownloadBudget,
    bytes: u64,
}

#[async_trait]
impl ToolExecutor for Claiming {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        if self.wallet.claim(self.bytes) {
            Ok(serde_json::json!({ "bytes": self.bytes }))
        } else {
            Err(ToolError::Fetch(
                "the download budget is spent: the claim was refused".into(),
            ))
        }
    }
}

/// One scripted provider turn (the engine episode tests' shape): a turn of
/// tool calls, or a terminal prose answer.
enum Turn {
    Tools(Vec<ToolCall>),
    Answer(&'static str),
}

/// A scripted `ChatProvider`: serves its turns in order. Responses carry no
/// usage — the download budget is the budget under test, not the ceiling.
struct ScriptProvider {
    script: Mutex<VecDeque<Turn>>,
}

impl ScriptProvider {
    fn new(script: Vec<Turn>) -> Self {
        Self {
            script: Mutex::new(script.into()),
        }
    }
}

#[async_trait]
impl ChatProvider for ScriptProvider {
    fn name(&self) -> &str {
        "script"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        match self.script.lock().unwrap().pop_front() {
            Some(Turn::Answer(text)) => Ok(ChatResponse::new(ChatMessage::text("assistant", text))),
            Some(Turn::Tools(calls)) => Ok(ChatResponse::new(ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: calls,
                tool_call_id: None,
            })),
            None => panic!("provider script exhausted"),
        }
    }
}

/// The stand-in download tool's definition: auto-runnable (no approval, no
/// external side effect, no database data) so the loop's own gates are the
/// only thing that can refuse it — the wallet's claim is what the test
/// exercises, not the loop's.
fn download_definition() -> ToolDefinition {
    ToolDefinition {
        name: "http_download".into(),
        description: "the stand-in download tool".into(),
        read_only: false,
        parameters: serde_json::json!({"type": "object"}),
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

fn tool_call(name: &str) -> ToolCall {
    ToolCall {
        id: "c-download".into(),
        name: name.into(),
        arguments: serde_json::json!({}),
    }
}

/// The resume inputs for a run with an armed download wallet, shared by
/// clone between this input and the resumed step's executor — the sharing a
/// fetch-capable run builds, so the tool's claims and the sink's checks are
/// one wallet. Every other input is the crashed run's own.
fn download_inputs<'a>(
    run: &'a CrashedRun,
    provider: &'a ScriptProvider,
    toolsets: &'a [StepToolset],
    wallet: DownloadBudget,
) -> ResumeRun<'a> {
    ResumeRun {
        run_id: run.run_id.clone(),
        store: run.store.clone(),
        plan: run.plan.clone(),
        incomplete_policy: saya_harness::engine::IncompletePolicy::Conservative,
        workspace: workspace(&run.root),
        collaborators: EpisodeCollaborators {
            provider,
            approval: &run.stubs.approval,
            toolsets,
            cancellation: CancellationToken::default(),
        },
        request: request(),
        bounds: bounds(),
        wall_clock: None,
        token_ceiling: None,
        download_budget: Some(wallet),
        journal_wire: None,
        agent_stream: None,
    }
}
