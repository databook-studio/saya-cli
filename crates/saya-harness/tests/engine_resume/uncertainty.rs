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

#[tokio::test]
async fn a_completed_effect_survives_step_mirror_failure_without_replay() {
    let run = effect_run("completed-effect-mirror").await;
    let counter = Arc::new(AtomicUsize::new(0));
    let provider = EffectProvider(AtomicUsize::new(0));
    let store: Arc<dyn RunStore> = Arc::new(RefuseStepMirrorStore {
        inner: (*run.store).clone(),
        refuse_status: RunStepStatus::Done,
    });

    assert!(
        drive_effect(&run, 1, store, Arc::clone(&counter), &provider, None)
            .await
            .is_err()
    );
    assert_eq!(counter.load(Ordering::SeqCst), 1);
    assert!(run.journal().contains(&RunEvent::StepCompleted { step: 1 }));

    let outcome = resume(&run.run_dir, run.inputs()).await.unwrap();

    assert!(matches!(outcome, ResumeOutcome::Settled { .. }));
    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn an_effect_without_durable_completion_is_uncertain_until_explicit_retry() {
    for torn in [false, true] {
        let label = if torn { "torn-effect" } else { "absent-effect" };
        let run = effect_run(label).await;
        let counter = Arc::new(AtomicUsize::new(0));
        let provider = EffectProvider(AtomicUsize::new(0));
        let backup = Arc::new(Mutex::new(None));
        let observed = Arc::new(AtomicBool::new(false));
        let observer: Arc<dyn AgentEventSink> = Arc::new(InterruptAfterDelivery {
            run_dir: run.run_dir.clone(),
            backup: Arc::clone(&backup),
            observed_durable_start: Arc::clone(&observed),
        });

        let interrupted = drive_effect(
            &run,
            1,
            run.store.clone(),
            Arc::clone(&counter),
            &provider,
            Some(observer),
        )
        .await;
        assert!(
            interrupted.is_err(),
            "expected append interruption after effect {}, got {interrupted:?}; journal {:?}",
            counter.load(Ordering::SeqCst),
            run.journal()
        );
        restore_journal(&run, &backup);
        assert!(observed.load(Ordering::SeqCst));
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        if torn {
            use std::io::Write;
            fs::OpenOptions::new()
                .append(true)
                .open(run.run_dir.join("events.ndjson"))
                .unwrap()
                .write_all(b"{\"type\":\"step_completed\"")
                .unwrap();
        }

        let retry_provider = EffectProvider(AtomicUsize::new(0));
        let retry_toolsets = effect_toolsets(Arc::clone(&counter));
        let mut inputs = run.inputs();
        inputs.collaborators.provider = &retry_provider;
        inputs.collaborators.toolsets = &retry_toolsets;
        let outcome = resume(&run.run_dir, inputs).await.unwrap();
        assert!(matches!(
            outcome,
            ResumeOutcome::IncompleteEffects { step: 1, .. }
        ));
        assert_eq!(counter.load(Ordering::SeqCst), 1);

        let mut inputs = run.inputs();
        inputs.collaborators.provider = &retry_provider;
        inputs.collaborators.toolsets = &retry_toolsets;
        inputs.incomplete_policy = saya_harness::engine::IncompletePolicy::Retry;
        let retried = resume(&run.run_dir, inputs).await.unwrap();
        assert!(matches!(
            retried,
            ResumeOutcome::Resumed { first_step: 1, .. }
        ));
        assert_eq!(counter.load(Ordering::SeqCst), 2);
        assert_eq!(retry_provider.0.load(Ordering::SeqCst), 2);
        let events = run.journal();
        assert_eq!(
            events
                .iter()
                .filter(|event| **event == RunEvent::StepStarted { step: 0 })
                .count(),
            1,
            "the durable prefix is never rerun"
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| **event == RunEvent::StepStarted { step: 1 })
                .count(),
            2,
            "only the incomplete effect is retried"
        );
    }
}

#[tokio::test]
async fn later_incomplete_risky_steps_are_checked_before_any_suffix_work() {
    let mut capabilities = Capabilities::default();
    capabilities.workspace_write = true;
    let risky = StepSpec::new("later effect", capabilities, None, Vec::new(), None).unwrap();
    let plan = RunPlan::new(vec![step("earlier safe"), risky]).unwrap();
    let run = crashed_run(
        "later-risky-started",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 1 },
        ],
        RunStatus::Executing,
        plan,
    )
    .await;

    let outcome = resume(&run.run_dir, run.inputs()).await.unwrap();

    assert!(matches!(
        outcome,
        ResumeOutcome::IncompleteEffects { step: 1, .. }
    ));
    assert_eq!(run.stubs.provider.count(), 0);
    assert!(!run.journal().contains(&RunEvent::StepStarted { step: 0 }));
}

#[tokio::test]
async fn a_completed_nonprefix_step_is_never_replayed() {
    let plan = RunPlan::new(vec![
        step("not started"),
        step("completed out of order"),
        step("finish the plan"),
    ])
    .unwrap();
    let run = crashed_run(
        "nonprefix-completed",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
        ],
        RunStatus::Executing,
        plan,
    )
    .await;
    let journal = Journal::open(&run.run_dir);
    let sink = EngineEventSink::new(
        run.run_id.clone(),
        RunState::Executing,
        journal.clone(),
        run.store.clone(),
        SinkBudgets {
            wall_clock: None,
            token_ceiling: None,
            download_budget: None,
            carried_usage: UsageTotals::default(),
        },
        std::time::Instant::now,
    );
    let driver = EpisodeDriver::new(
        EpisodeCollaborators {
            provider: &run.stubs.provider,
            approval: &run.stubs.approval,
            toolsets: &run.toolsets,
            cancellation: CancellationToken::default(),
        },
        EpisodeRun {
            run_id: run.run_id.clone(),
            store: run.store.clone(),
            journal,
        },
        request(),
        bounds(),
    );
    driver
        .run_step(&sink, &run.plan, 1, &workspace(&run.root))
        .await
        .unwrap();
    assert_eq!(run.stubs.provider.count(), 1);

    let outcome = resume(&run.run_dir, run.inputs()).await.unwrap();

    assert!(matches!(
        outcome,
        ResumeOutcome::Resumed { first_step: 0, .. }
    ));
    assert_eq!(run.stubs.provider.count(), 3);
    assert_eq!(
        run.journal()
            .iter()
            .filter(|event| **event == RunEvent::StepStarted { step: 1 })
            .count(),
        1
    );
}

#[tokio::test]
async fn an_incomplete_workspace_step_requires_explicit_retry() {
    let mut capabilities = Capabilities::default();
    capabilities.workspace_write = true;
    let risky = StepSpec::new("write the report", capabilities, None, Vec::new(), None).unwrap();
    let run = crashed_run(
        "uncertain-workspace",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
        ],
        RunStatus::Executing,
        RunPlan::new(vec![risky]).unwrap(),
    )
    .await;
    let expected = ResumeOutcome::IncompleteEffects {
        step: 0,
        goal: "write the report".into(),
        effects: vec![saya_harness::engine::ResumeEffect::WorkspaceWrite],
    };

    let first = resume(&run.run_dir, run.inputs()).await.unwrap();

    assert_eq!(first, expected);
    assert_eq!(run.stubs.provider.count(), 0);
    assert_eq!(run.tool_calls.load(Ordering::Relaxed), 0);
    let first_journal = run.journal();
    assert_eq!(
        first_journal.last(),
        Some(&RunEvent::Paused {
            reason: PauseReason::ProcessDeath
        })
    );
    assert!(!first_journal.contains(&RunEvent::StepCompleted { step: 0 }));

    let repeated = resume(&run.run_dir, run.inputs()).await.unwrap();

    assert_eq!(repeated, expected);
    assert_eq!(run.stubs.provider.count(), 0);
    assert_eq!(run.tool_calls.load(Ordering::Relaxed), 0);
    assert_eq!(run.journal(), first_journal);
}

#[tokio::test]
async fn an_incomplete_failed_scratch_step_is_also_uncertain() {
    let mut capabilities = Capabilities::default();
    capabilities.scratch = true;
    let plan = RunPlan::new(vec![
        StepSpec::new("stage the data", capabilities, None, Vec::new(), None).unwrap(),
    ])
    .unwrap();
    let run = crashed_run(
        "failed-scratch",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepFailed { step: 0 },
            RunEvent::Paused {
                reason: PauseReason::ProcessDeath,
            },
        ],
        RunStatus::Paused,
        plan,
    )
    .await;

    let outcome = resume(&run.run_dir, run.inputs()).await.unwrap();

    assert_eq!(
        outcome,
        ResumeOutcome::IncompleteEffects {
            step: 0,
            goal: "stage the data".into(),
            effects: vec![saya_harness::engine::ResumeEffect::Scratch],
        }
    );
    assert_eq!(run.stubs.provider.count(), 0);
    assert_eq!(
        run.journal().last(),
        Some(&RunEvent::Paused {
            reason: PauseReason::ProcessDeath
        })
    );
}

#[tokio::test]
async fn an_unavailable_step_toolset_fails_closed_as_unknown() {
    let mut run = crashed_run(
        "unknown-toolset",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
        ],
        RunStatus::Executing,
        RunPlan::new(vec![step("unknown capability")]).unwrap(),
    )
    .await;
    run.toolsets.clear();

    let outcome = resume(&run.run_dir, run.inputs()).await.unwrap();

    assert_eq!(
        outcome,
        ResumeOutcome::IncompleteEffects {
            step: 0,
            goal: "unknown capability".into(),
            effects: vec![saya_harness::engine::ResumeEffect::Unknown],
        }
    );
    assert_eq!(run.stubs.provider.count(), 0);
}

/// Explicit retry begins only the incomplete suffix: a completed prefix stays
/// authoritative even when the resumed operator opts into retrying uncertainty.
#[tokio::test]
async fn explicit_retry_runs_only_the_incomplete_step() {
    let mut capabilities = Capabilities::default();
    capabilities.workspace_write = true;
    let plan = RunPlan::new(vec![
        StepSpec::new(
            "completed write",
            capabilities.clone(),
            None,
            Vec::new(),
            None,
        )
        .unwrap(),
        StepSpec::new("incomplete write", capabilities, None, Vec::new(), None).unwrap(),
    ])
    .unwrap();
    let run = crashed_run(
        "retry-incomplete",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
            RunEvent::StepCompleted { step: 0 },
            RunEvent::StepStarted { step: 1 },
        ],
        RunStatus::Executing,
        plan,
    )
    .await;
    let mut inputs = run.inputs();
    inputs.incomplete_policy = saya_harness::engine::IncompletePolicy::Retry;

    let outcome = resume(&run.run_dir, inputs).await.unwrap();

    assert_eq!(
        outcome,
        ResumeOutcome::Resumed {
            first_step: 1,
            state: RunState::Completed
        }
    );
    assert_eq!(run.stubs.provider.count(), 1);
    let events = run.journal();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, RunEvent::StepStarted { step: 0 }))
            .count(),
        1,
        "the durable completed prefix must not repeat"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, RunEvent::StepStarted { step: 1 }))
            .count(),
        2,
        "the explicit retry is represented by a new StepStarted"
    );
}

/// A database-data-only step is allowed to make a new observation: it may see
/// newer rows or cost more, so resume makes no claim to replay the same result.
#[tokio::test]
async fn a_database_observation_can_restart_as_a_fresh_observation() {
    let plan = RunPlan::new(vec![step("inspect the data")]).unwrap();
    let mut run = crashed_run(
        "fresh-observation",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
        ],
        RunStatus::Executing,
        plan,
    )
    .await;
    run.toolsets[0].definitions.push(ToolDefinition {
        name: "bounded_sql_query".into(),
        description: String::new(),
        read_only: false,
        parameters: serde_json::json!({"type": "object"}),
        effect: ToolEffect {
            database_data: true,
            external_side_effect: false,
            requires_approval: true,
            local_state: LocalStateEffect::None,
        },
        concurrency: saya_agent::ToolConcurrency::Serial,
        completion: None,
    });

    let outcome = resume(&run.run_dir, run.inputs()).await.unwrap();

    assert_eq!(
        outcome,
        ResumeOutcome::Resumed {
            first_step: 0,
            state: RunState::Completed
        }
    );
    assert_eq!(run.stubs.provider.count(), 1);
}

#[tokio::test]
async fn read_only_and_approval_flags_do_not_hide_declared_external_effects() {
    let mut run = crashed_run(
        "read-only-external-effect",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
        ],
        RunStatus::Executing,
        RunPlan::new(vec![step("inspect and publish")]).unwrap(),
    )
    .await;
    run.toolsets[0].definitions.push(ToolDefinition {
        name: "publish_report".into(),
        description: String::new(),
        read_only: true,
        parameters: serde_json::json!({"type": "object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: true,
            requires_approval: false,
            local_state: LocalStateEffect::None,
        },
        concurrency: saya_agent::ToolConcurrency::Serial,
        completion: None,
    });

    let outcome = resume(&run.run_dir, run.inputs()).await.unwrap();

    assert_eq!(
        outcome,
        ResumeOutcome::IncompleteEffects {
            step: 0,
            goal: "inspect and publish".into(),
            effects: vec![saya_harness::engine::ResumeEffect::ExternalSideEffect],
        }
    );
    assert_eq!(run.stubs.provider.count(), 0);
}

/// EpisodeDriver cannot enter provider/tool work when StepStarted cannot be
/// appended. Use a real journal filesystem fault rather than a test seam.
#[tokio::test]
async fn a_step_start_append_failure_prevents_episode_work() {
    let run = crashed_run(
        "start-append-failure",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
        ],
        RunStatus::Approved,
        RunPlan::new(vec![step("the only step")]).unwrap(),
    )
    .await;
    let journal_path = run.run_dir.join("events.ndjson");
    fs::remove_file(&journal_path).unwrap();
    fs::create_dir(&journal_path).unwrap();
    let journal = Journal::open(&run.run_dir);
    let sink = EngineEventSink::new(
        run.run_id.clone(),
        RunState::Approved,
        journal.clone(),
        run.store.clone(),
        SinkBudgets {
            wall_clock: None,
            token_ceiling: None,
            download_budget: None,
            carried_usage: UsageTotals::default(),
        },
        std::time::Instant::now,
    );
    let driver = EpisodeDriver::new(
        EpisodeCollaborators {
            provider: &run.stubs.provider,
            approval: &run.stubs.approval,
            toolsets: &run.toolsets,
            cancellation: CancellationToken::default(),
        },
        EpisodeRun {
            run_id: run.run_id.clone(),
            store: run.store.clone(),
            journal,
        },
        request(),
        bounds(),
    );

    assert!(
        driver
            .run_step(&sink, &run.plan, 0, &workspace(&run.root))
            .await
            .is_err()
    );
    assert_eq!(run.stubs.provider.count(), 0);
    assert!(
        RunStore::list_steps(&*run.store, &run.run_id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_step_start_mirror_refusal_prevents_episode_work() {
    let run = crashed_run(
        "start-mirror-failure",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
        ],
        RunStatus::Approved,
        RunPlan::new(vec![step("the only step")]).unwrap(),
    )
    .await;
    let store: Arc<dyn RunStore> = Arc::new(RefuseStepMirrorStore {
        inner: (*run.store).clone(),
        refuse_status: RunStepStatus::Running,
    });
    let journal = Journal::open(&run.run_dir);
    let sink = EngineEventSink::new(
        run.run_id.clone(),
        RunState::Approved,
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
    let driver = EpisodeDriver::new(
        EpisodeCollaborators {
            provider: &run.stubs.provider,
            approval: &run.stubs.approval,
            toolsets: &run.toolsets,
            cancellation: CancellationToken::default(),
        },
        EpisodeRun {
            run_id: run.run_id.clone(),
            store,
            journal,
        },
        request(),
        bounds(),
    );

    assert!(
        driver
            .run_step(&sink, &run.plan, 0, &workspace(&run.root))
            .await
            .is_err()
    );
    assert_eq!(run.stubs.provider.count(), 0);
    assert_eq!(run.tool_calls.load(Ordering::Relaxed), 0);
    assert!(run.journal().contains(&RunEvent::StepStarted { step: 0 }));
    assert!(
        RunStore::list_steps(&*run.store, &run.run_id)
            .await
            .unwrap()
            .is_empty()
    );
}

/// A synced StepCompleted followed by a wire interruption precedes the step
/// mirror. Resume must trust that durable prefix, continue the next step, and
/// never call the provider for the completed step again.
#[tokio::test]
async fn a_durable_completed_prefix_survives_mirror_interruption_on_resume() {
    use futures_util::FutureExt;

    let run = crashed_run(
        "completion-wire-interruption",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
        ],
        RunStatus::Executing,
        RunPlan::new(vec![step("completed step"), step("next step")]).unwrap(),
    )
    .await;
    let journal = Journal::open(&run.run_dir).with_wire(Arc::new(|event| {
        if matches!(event, RunEvent::StepCompleted { step: 0 }) {
            panic!("interrupt after the durable completion append");
        }
    }));
    let sink = EngineEventSink::new(
        run.run_id.clone(),
        RunState::Approved,
        Journal::open(&run.run_dir),
        run.store.clone(),
        SinkBudgets {
            wall_clock: None,
            token_ceiling: None,
            download_budget: None,
            carried_usage: UsageTotals::default(),
        },
        std::time::Instant::now,
    );
    let driver = EpisodeDriver::new(
        EpisodeCollaborators {
            provider: &run.stubs.provider,
            approval: &run.stubs.approval,
            toolsets: &run.toolsets,
            cancellation: CancellationToken::default(),
        },
        EpisodeRun {
            run_id: run.run_id.clone(),
            store: run.store.clone(),
            journal,
        },
        request(),
        bounds(),
    );
    let interrupted =
        std::panic::AssertUnwindSafe(driver.run_step(&sink, &run.plan, 0, &workspace(&run.root)))
            .catch_unwind()
            .await;

    assert!(interrupted.is_err());
    assert!(run.journal().contains(&RunEvent::StepCompleted { step: 0 }));
    assert_eq!(
        RunStore::list_steps(&*run.store, &run.run_id)
            .await
            .unwrap()[0]
            .status,
        RunStepStatus::Running,
        "the injected interruption happened before the step mirror"
    );
    assert_eq!(run.stubs.provider.count(), 1);

    let outcome = resume(&run.run_dir, run.inputs()).await.unwrap();

    assert_eq!(
        outcome,
        ResumeOutcome::Resumed {
            first_step: 1,
            state: RunState::Completed
        }
    );
    assert_eq!(run.stubs.provider.count(), 2);
    let events = run.journal();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, RunEvent::StepStarted { step: 0 }))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, RunEvent::StepCompleted { step: 0 }))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, RunEvent::StepStarted { step: 1 }))
            .count(),
        1
    );
}
