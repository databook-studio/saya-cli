use super::cell::Cell;
use super::helper::PROGRAM;
use super::*;

// -- the scripted provider, the approval stub, and the engine drive -------

/// A scripted provider turn.
pub(super) enum Turn {
    /// An assistant turn carrying tool calls.
    Tools(Vec<ToolCall>),
    /// A terminal prose answer (no tool calls).
    Answer(&'static str),
    /// A provider failure.
    Fail,
}

/// A scripted `ChatProvider`: serves its turns in order and records every
/// request it received — the model lane is read from what it received.
pub(super) struct ScriptProvider {
    script: Mutex<VecDeque<Turn>>,
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}

impl ScriptProvider {
    pub(super) fn new(script: Vec<Turn>) -> Self {
        Self {
            script: Mutex::new(script.into()),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(super) fn requests(&self) -> Vec<ChatRequest> {
        self.requests.lock().expect("provider request lock").clone()
    }
}

#[async_trait]
impl ChatProvider for ScriptProvider {
    fn name(&self) -> &str {
        "script"
    }

    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        self.requests
            .lock()
            .expect("provider request lock")
            .push(request);
        match self
            .script
            .lock()
            .expect("provider script lock")
            .pop_front()
        {
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

pub(super) struct AllowApproval;

#[async_trait]
impl ApprovalDecider for AllowApproval {
    async fn approve(&self, _: &ToolDefinition, _: &serde_json::Value) -> bool {
        true
    }
}

pub(super) fn bounds() -> ManifestBounds {
    ManifestBounds {
        max_files: 512,
        max_file_bytes: 256 * 1024,
    }
}

/// A step carrying the runner capability: the driver narrows the run's
/// tool universe to it, and the loop's workspace-write gate is satisfied
/// by the capability the step declares.
pub(super) fn runner_step(goal: &str) -> StepSpec {
    let mut capabilities = Capabilities::default();
    capabilities.workspace_write = true;
    capabilities.runner = Some(RunnerScope::new(vec![PROGRAM.to_owned()]).expect("shaped"));
    StepSpec::new(goal, capabilities, None, Vec::new(), None).expect("well-shaped")
}

pub(super) fn sink(cell: &Cell) -> EngineEventSink {
    EngineEventSink::new(
        cell.run_id.clone(),
        RunState::Approved,
        Journal::open(cell.run.root()),
        cell.store.clone(),
        SinkBudgets {
            wall_clock: None,
            token_ceiling: None,
            download_budget: None,
            carried_usage: UsageTotals::default(),
        },
        std::time::Instant::now,
    )
}

/// Drives one plan step end to end: brief, narrowed tools, the loop, the
/// tool result back to the provider, the sink's journal and store
/// mirrors. The driver is per-step, exactly like the composition root's.
pub(super) async fn drive_step(
    cell: &Cell,
    sink: &EngineEventSink,
    tool: RunProgram,
    provider: &ScriptProvider,
    plan: &RunPlan,
    step: usize,
) -> Result<(), EpisodeError> {
    let approval = AllowApproval;
    let definition = tool.definition();
    let executor: Arc<dyn ToolExecutor> = Arc::new(tool);
    // One toolset per plan step over the same executor, the way the
    // composition root builds them; only the driven step's episode runs.
    let toolsets: Vec<StepToolset> = (0..plan.steps.len())
        .map(|_| StepToolset {
            executor: Arc::clone(&executor),
            definitions: vec![definition.clone()],
        })
        .collect();
    let driver = EpisodeDriver::new(
        EpisodeCollaborators {
            provider,
            approval: &approval,
            toolsets: &toolsets,
            cancellation: CancellationToken::default(),
        },
        EpisodeRun {
            run_id: cell.run_id.clone(),
            store: cell.store.clone(),
            journal: Journal::open(cell.run.root()),
        },
        EpisodeRequest {
            model: "mock-model".into(),
            profile_names: Vec::new(),
            memory_allows_candidate_writes: false,
        },
        bounds(),
    );
    let workspace = Workspace::open(cell.run.workspace()).expect("the run workspace opens");
    driver.run_step(sink, plan, step, &workspace).await
}

pub(super) fn run_program_call(args: &[&str]) -> ToolCall {
    ToolCall {
        id: "c-run".to_owned(),
        name: RUN_PROGRAM_TOOL.to_owned(),
        arguments: serde_json::json!({
            "program": PROGRAM,
            "args": args,
            "timeout_seconds": 30
        }),
    }
}
