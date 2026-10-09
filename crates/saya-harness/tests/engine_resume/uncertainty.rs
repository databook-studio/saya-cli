use super::*;
use saya_agent::{AgentEvent, AgentEventSink, ToolConcurrency};
use saya_harness::engine::{EngineEventSink, SinkBudgets};
use saya_types::RunEvent;
use std::sync::atomic::{AtomicBool, AtomicUsize};

struct EffectExecutor(Arc<AtomicUsize>);

#[async_trait]
impl ToolExecutor for EffectExecutor {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(serde_json::json!({"effect": "applied"}))
    }
}

struct EffectProvider(AtomicUsize);

#[async_trait]
impl ChatProvider for EffectProvider {
    fn name(&self) -> &str {
        "effect-provider"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        if self.0.fetch_add(1, Ordering::SeqCst).is_multiple_of(2) {
            Ok(ChatResponse::new(ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: "effect-1".into(),
                    name: "apply_effect".into(),
                    arguments: serde_json::json!({}),
                }],
                tool_call_id: None,
            }))
        } else {
            Ok(ChatResponse::new(ChatMessage::text("assistant", "done")))
        }
    }
}

fn effect_toolset(counter: Arc<AtomicUsize>) -> StepToolset {
    StepToolset {
        executor: Arc::new(EffectExecutor(counter)),
        definitions: vec![ToolDefinition {
            name: "apply_effect".into(),
            description: "applies a counted test effect".into(),
            read_only: false,
            parameters: serde_json::json!({"type": "object"}),
            effect: ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: false,
                local_state: LocalStateEffect::WriteWorkspace,
            },
            concurrency: ToolConcurrency::Serial,
            completion: None,
        }],
    }
}

fn effect_plan() -> RunPlan {
    let mut capabilities = Capabilities::default();
    capabilities.workspace_write = true;
    RunPlan::new(vec![
        step("completed prefix"),
        StepSpec::new("apply the effect", capabilities, None, Vec::new(), None).unwrap(),
    ])
    .unwrap()
}

async fn effect_run(label: &str) -> CrashedRun {
    crashed_run(
        label,
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
            RunEvent::StepCompleted { step: 0 },
        ],
        RunStatus::Executing,
        effect_plan(),
    )
    .await
}

fn effect_toolsets(counter: Arc<AtomicUsize>) -> Vec<StepToolset> {
    vec![
        StepToolset {
            executor: Arc::new(NoTools::default()),
            definitions: Vec::new(),
        },
        effect_toolset(counter),
    ]
}

async fn drive_effect(
    run: &CrashedRun,
    step: usize,
    store: Arc<dyn RunStore>,
    counter: Arc<AtomicUsize>,
    provider: &EffectProvider,
    stream: Option<Arc<dyn AgentEventSink>>,
) -> Result<(), saya_harness::engine::EpisodeError> {
    let journal = Journal::open(&run.run_dir);
    let sink = EngineEventSink::new(
        run.run_id.clone(),
        RunState::Executing,
        journal.clone(),
        store.clone(),
        SinkBudgets {
            wall_clock: None,
            token_ceiling: None,
            download_budget: None,
            carried_usage: UsageTotals::default(),
        },
        std::time::Instant::now,
    );
    let sink = match stream {
        Some(stream) => sink.with_agent_stream(stream),
        None => sink,
    };
    let toolsets = effect_toolsets(counter);
    let approval = AllowApproval;
    EpisodeDriver::new(
        EpisodeCollaborators {
            provider,
            approval: &approval,
            toolsets: &toolsets,
            cancellation: CancellationToken::default(),
        },
        EpisodeRun {
            run_id: run.run_id.clone(),
            store,
            journal,
        },
        request(),
        bounds(),
    )
    .run_step(&sink, &run.plan, step, &workspace(&run.root))
    .await
}

struct InterruptAfterDelivery {
    run_dir: PathBuf,
    backup: Arc<Mutex<Option<Vec<u8>>>>,
    observed_durable_start: Arc<AtomicBool>,
}

#[async_trait]
impl AgentEventSink for InterruptAfterDelivery {
    async fn emit(&self, event: AgentEvent) {
        if !matches!(event, AgentEvent::ToolCompleted { .. }) {
            return;
        }
        let path = self.run_dir.join("events.ndjson");
        let bytes = fs::read(&path).unwrap();
        let events = Journal::open(&self.run_dir).read().unwrap();
        assert!(bytes.ends_with(b"\n"));
        assert!(events.contains(&RunEvent::StepStarted { step: 1 }));
        assert!(events.contains(&RunEvent::StepCompleted { step: 0 }));
        assert!(!events.contains(&RunEvent::StepCompleted { step: 1 }));
        *self.backup.lock().unwrap() = Some(bytes);
        self.observed_durable_start.store(true, Ordering::SeqCst);
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
    }
}

fn restore_journal(run: &CrashedRun, backup: &Mutex<Option<Vec<u8>>>) {
    let path = run.run_dir.join("events.ndjson");
    if path.is_dir() {
        fs::remove_dir(&path).unwrap();
        fs::write(path, backup.lock().unwrap().take().unwrap()).unwrap();
    }
}

#[path = "uncertainty/durable_effects.rs"]
mod durable_effects;

#[path = "uncertainty/uncertain_steps.rs"]
mod uncertain_steps;

#[path = "uncertainty/retry_policy.rs"]
mod retry_policy;

#[path = "uncertainty/start_failures.rs"]
mod start_failures;

#[path = "uncertainty/completed_prefix.rs"]
mod completed_prefix;
