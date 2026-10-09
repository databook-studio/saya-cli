use super::*;

/// A per-test scratch root: the state database and the run directory both
/// live under it, and one cleanup covers both.
pub(super) fn temp_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "saya-engine-approval-{label}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

/// A run standing at `planned` — bound plan persisted, approval not yet
/// decided. The state a fresh run is in when the approval surface decides.
pub(super) struct PlannedRun {
    pub(super) root: PathBuf,
    pub(super) run_dir: PathBuf,
    pub(super) store: Arc<SqliteStateStore>,
    pub(super) run_id: RunId,
}

impl PlannedRun {
    pub(super) fn workspace(&self) -> Workspace {
        let dir = self.root.join("workspace");
        fs::create_dir_all(&dir).unwrap();
        Workspace::open(&dir).unwrap()
    }
}

pub(super) async fn planned_run(label: &str) -> PlannedRun {
    let root = temp_root(label);
    let store = Arc::new(SqliteStateStore::new(root.join("state.sqlite3")));
    let run_id = RunId::parse(&format!("run-{label}")).unwrap();
    store
        .create_run(NewRun {
            id: run_id.clone(),
            capabilities: RunCapabilityFlags::default(),
            budgets: RunBudgets::default(),
        })
        .await
        .unwrap();
    let run_dir = root.join("run");
    fs::create_dir_all(&run_dir).unwrap();
    PlannedRun {
        root,
        run_dir,
        store,
        run_id,
    }
}

/// A scripted planner provider: serves its answers in order and records
/// every request it received (the `engine_plan.rs` rig).
pub(super) struct ScriptedPlanner {
    script: Mutex<VecDeque<String>>,
    pub(super) requests: Mutex<Vec<ChatRequest>>,
}

impl ScriptedPlanner {
    pub(super) fn new(script: Vec<String>) -> Self {
        Self {
            script: Mutex::new(script.into()),
            requests: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl ChatProvider for ScriptedPlanner {
    fn name(&self) -> &str {
        "scripted-planner"
    }

    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        self.requests.lock().unwrap().push(request);
        match self.script.lock().unwrap().pop_front() {
            Some(text) => Ok(ChatResponse::new(ChatMessage::text("assistant", text))),
            None => panic!("planner script exhausted"),
        }
    }
}

/// A plan proposal carrying `steps` as one JSON line.
pub(super) fn plan_json(steps: &str) -> String {
    format!(r#"{{"steps": [{steps}]}}"#)
}

pub(super) fn step_json(goal: &str, capabilities: &str) -> String {
    format!(
        r#"{{"goal": "{goal}", "capabilities": {capabilities}, "budget": null, "expects": [], "endpoint": null}}"#
    )
}

pub(super) fn planner_request(run_goal: &str) -> PlanRequest {
    PlanRequest {
        model: "mock-model".into(),
        run_goal: run_goal.into(),
    }
}

pub(super) fn workspace_write_scopes() -> Capabilities {
    let mut scopes = Capabilities::default();
    scopes.workspace_write = true;
    scopes
}

/// One tool definition the plan approval covers: it requires approval (a
/// read-shaped SQL probe — `database_data`, no external side effect, a
/// contained read) so the per-call decider is consulted, and it is visible
/// in every step because a contained read is not capability-scoped.
pub(super) fn approved_read_tool() -> saya_agent::ToolDefinition {
    saya_agent::ToolDefinition {
        name: "sql_probe".into(),
        description: "sql_probe".into(),
        read_only: true,
        parameters: serde_json::json!({"type": "object"}),
        effect: ToolEffect {
            database_data: true,
            external_side_effect: false,
            requires_approval: true,
            local_state: LocalStateEffect::Read,
        },
        concurrency: saya_agent::ToolConcurrency::Serial,
        completion: None,
    }
}

/// One scripted episode turn: a tool call or a terminal prose answer.
pub(super) enum Turn {
    Tools(Vec<ToolCall>),
    Answer(&'static str),
}

/// A scripted episode provider, the `engine_episode.rs` pattern.
pub(super) struct ScriptedEpisode {
    script: Mutex<VecDeque<Turn>>,
}

impl ScriptedEpisode {
    pub(super) fn new(script: Vec<Turn>) -> Self {
        Self {
            script: Mutex::new(script.into()),
        }
    }
}

#[async_trait]
impl ChatProvider for ScriptedEpisode {
    fn name(&self) -> &str {
        "scripted-episode"
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
            None => panic!("episode script exhausted"),
        }
    }
}

/// A tool executor that records every call it received.
#[derive(Default)]
pub(super) struct RecordingTools {
    pub(super) calls: Mutex<Vec<String>>,
}

#[async_trait]
impl ToolExecutor for RecordingTools {
    async fn execute(
        &self,
        name: &str,
        _: serde_json::Value,
    ) -> Result<serde_json::Value, saya_agent::ToolError> {
        self.calls.lock().unwrap().push(name.into());
        Ok(serde_json::json!({"rows": 1}))
    }
}

pub(super) fn call(name: &str) -> ToolCall {
    ToolCall {
        id: format!("c-{name}"),
        name: name.into(),
        arguments: serde_json::json!({}),
    }
}

/// The approval surface the composition root drives, mirrored as a counting
/// channel: every interactive ask is sent over the channel and counted, the
/// way `tui/agent.rs` collects approvals — never a stdin prompt. Policy-
/// shaped tools are auto-decided without an ask.
pub(super) struct CountingApproval {
    pub(super) prompts: AtomicUsize,
}

impl CountingApproval {
    pub(super) fn prompted(&self) -> usize {
        self.prompts.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ApprovalDecider for CountingApproval {
    async fn approve(&self, tool: &saya_agent::ToolDefinition, _: &serde_json::Value) -> bool {
        if read_only_permits(&tool.effect) {
            return true;
        }
        // The ask would go to the UI's modal over a channel; the count is
        // the property users feel: per-call prompts after a plan approval.
        self.prompts.fetch_add(1, Ordering::SeqCst);
        let (respond, answer) = oneshot::channel();
        drop(respond);
        answer.await.unwrap_or(false)
    }
}
