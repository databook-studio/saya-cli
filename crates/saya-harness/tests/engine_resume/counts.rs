use super::*;
use saya_agent::{AgentEvent, AgentEventSink};
use saya_types::{Budgets, StepSpec};

struct ScriptedAnswerProvider {
    script: Mutex<VecDeque<ResumeTurn>>,
    requests: AtomicUsize,
}

enum ResumeTurn {
    Answer(&'static str),
    Tool(ToolCall),
}

impl ScriptedAnswerProvider {
    fn new(script: Vec<ResumeTurn>) -> Self {
        Self {
            script: Mutex::new(script.into()),
            requests: AtomicUsize::new(0),
        }
    }

    fn count(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ChatProvider for ScriptedAnswerProvider {
    fn name(&self) -> &str {
        "count-resume"
    }

    async fn complete(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        let turn = self.script.lock().unwrap().pop_front().unwrap();
        let message = match turn {
            ResumeTurn::Answer(text) => ChatMessage::text("assistant", text),
            ResumeTurn::Tool(call) => ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![call],
                tool_call_id: None,
            },
        };
        Ok(ChatResponse::new(message))
    }
}

struct HoldComplete(Mutex<Option<oneshot::Sender<()>>>);

#[async_trait]
impl AgentEventSink for HoldComplete {
    async fn emit(&self, event: AgentEvent) {
        if matches!(event, AgentEvent::Complete) {
            let started = self.0.lock().unwrap().take();
            if let Some(started) = started {
                let _ = started.send(());
                std::future::pending::<()>().await;
            }
        }
    }
}

#[derive(Default)]
struct CountedTool(AtomicUsize);

#[async_trait]
impl ToolExecutor for CountedTool {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(serde_json::json!({"ok": true}))
    }
}

fn bounded_step(goal: &str, turns: u64, tool_calls: Option<u64>) -> StepSpec {
    let mut budget = Budgets::default();
    budget.turns = Some(turns);
    budget.tool_calls = tool_calls;
    StepSpec::new(
        goal,
        Capabilities::default(),
        Some(budget),
        Vec::new(),
        None,
    )
    .unwrap()
}

fn inputs<'a>(
    run: &'a CrashedRun,
    provider: &'a ScriptedAnswerProvider,
    agent_stream: Option<Arc<dyn AgentEventSink>>,
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
            toolsets: &run.toolsets,
            cancellation: CancellationToken::default(),
        },
        request: request(),
        bounds: bounds(),
        wall_clock: None,
        token_ceiling: None,
        download_budget: None,
        journal_wire: None,
        agent_stream,
    }
}

fn count_rows_for_step(events: &[RunEvent], step: usize) -> Vec<(Option<u64>, Option<u64>)> {
    let Some(start) = events
        .iter()
        .position(|event| *event == RunEvent::StepStarted { step })
    else {
        return Vec::new();
    };
    events[start + 1..]
        .iter()
        .take_while(|event| !matches!(event, RunEvent::StepStarted { step: next } if *next != step))
        .filter_map(|event| match event {
            RunEvent::Usage {
                tokens: None,
                turns,
                tool_calls,
                cached_input_tokens: None,
                cache_creation_input_tokens: None,
                ..
            } if turns.is_some() || tool_calls.is_some() => Some((*turns, *tool_calls)),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn source_counts_carry_through_public_resume_of_same_incomplete_step() {
    let plan = RunPlan::new(vec![
        step("completed prefix"),
        bounded_step("bounded step", 2, Some(1)),
    ])
    .unwrap();
    let mut run = crashed_run(
        "count-resume",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
            RunEvent::StepCompleted { step: 0 },
        ],
        RunStatus::Executing,
        plan,
    )
    .await;
    let tool = Arc::new(CountedTool::default());
    let executor: Arc<dyn ToolExecutor> = tool.clone();
    run.toolsets[1] = StepToolset {
        executor,
        definitions: vec![ToolDefinition {
            name: "probe".into(),
            description: "probe".into(),
            read_only: true,
            parameters: serde_json::json!({"type": "object"}),
            effect: saya_agent::ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: false,
                local_state: LocalStateEffect::None,
            },
            concurrency: saya_agent::ToolConcurrency::Serial,
            completion: None,
        }],
    };
    let provider = ScriptedAnswerProvider::new(vec![
        ResumeTurn::Answer("first admitted response"),
        ResumeTurn::Tool(ToolCall {
            id: "probe-1".into(),
            name: "probe".into(),
            arguments: serde_json::json!({}),
        }),
        ResumeTurn::Answer("must not be admitted"),
    ]);
    let (started, reached) = oneshot::channel();
    let stream: Arc<dyn AgentEventSink> = Arc::new(HoldComplete(Mutex::new(Some(started))));

    {
        let active = resume(&run.run_dir, inputs(&run, &provider, Some(stream)));
        tokio::pin!(active);
        tokio::select! {
            result = &mut active => panic!("the first response should be held before step completion: {result:?}"),
            result = reached => result.expect("the actual episode reaches its completion event"),
        }
    }

    assert_eq!(
        count_rows_for_step(&run.journal(), 1),
        vec![(Some(0), Some(0)), (Some(1), None), (None, Some(0))],
        "a completed zero-call batch is known zero in source-owned journal rows"
    );

    let error = resume(&run.run_dir, inputs(&run, &provider, None))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ResumeError::Episode {
            source: saya_harness::engine::EpisodeError::BudgetStopped { .. }
        }
    ));
    assert_eq!(
        provider.count(),
        2,
        "resume admits only the remaining request"
    );
    assert_eq!(
        tool.0.load(Ordering::SeqCst),
        1,
        "the already-admitted tool runs once"
    );
    assert_eq!(
        count_rows_for_step(&run.journal(), 1),
        vec![
            (Some(0), Some(0)),
            (Some(1), None),
            (None, Some(0)),
            (Some(1), None),
            (None, Some(1)),
        ],
        "the repeated initialization is a no-op and the resumed delta carries forward"
    );
    assert!(matches!(
        run.journal().last(),
        Some(RunEvent::Paused {
            reason: saya_types::PauseReason::BudgetExhausted
        })
    ));
}

#[tokio::test]
async fn legacy_started_finite_count_budget_refuses_unknown_before_provider() {
    let plan = RunPlan::new(vec![bounded_step("legacy incomplete step", 2, None)]).unwrap();
    let run = crashed_run(
        "legacy-counts",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
        ],
        RunStatus::Executing,
        plan,
    )
    .await;
    let provider = ScriptedAnswerProvider::new(vec![ResumeTurn::Answer("must not run")]);

    let error = resume(&run.run_dir, inputs(&run, &provider, None))
        .await
        .unwrap_err();

    assert!(
        error.to_string().contains("count evidence"),
        "unexpected refusal: {error}"
    );
    assert_eq!(provider.count(), 0);
    assert!(count_rows_for_step(&run.journal(), 0).is_empty());

    let mut capabilities = Capabilities::default();
    capabilities.workspace_write = true;
    let mut budget = Budgets::default();
    budget.turns = Some(2);
    let effectful_plan = RunPlan::new(vec![
        StepSpec::new(
            "uncertain effect outranks missing count evidence",
            capabilities,
            Some(budget),
            Vec::new(),
            None,
        )
        .unwrap(),
    ])
    .unwrap();
    let effectful = crashed_run(
        "legacy-counts-uncertain",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
        ],
        RunStatus::Executing,
        effectful_plan,
    )
    .await;
    let provider = ScriptedAnswerProvider::new(vec![ResumeTurn::Answer("must not run")]);
    assert!(matches!(
        resume(&effectful.run_dir, inputs(&effectful, &provider, None)).await,
        Ok(ResumeOutcome::IncompleteEffects { step: 0, .. })
    ));
    assert_eq!(
        provider.count(),
        0,
        "H1 effect uncertainty retains precedence"
    );
}
