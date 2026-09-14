//! Resume — contract tests (M1-5, resume slice).
//!
//! Seven guarantees, one per test:
//! 1. A crash at **every** journal position: for each prefix of a driven
//!    two-step run's journal, resume continues at the first incomplete
//!    step and lands the run in the right final state — completed steps
//!    are not re-run and their events are not re-journalled.
//! 2. A completed run resumes to "nothing to do" rather than re-running
//!    the last step.
//! 3. A step executing at the crash restarts from its beginning, and the
//!    restart is visible in the journal.
//! 4. A second engine on the same run directory is refused while the
//!    first holds the lock.
//! 5. A journal with a torn final line — the crash that matters most —
//!    replays to the last whole event rather than erroring the run.
//! 6. The token ceiling binds the **run**, not each invocation: a resumed
//!    run seeded from the journal's usage record pauses at its cumulative
//!    spend, never at a re-armed ceiling.
//! 7. Seeding rides the repaired record: a torn usage line never counts
//!    toward the carried spend.
//! 8. The download budget binds the **run** the same way: a resumed run's
//!    wallet is seeded from the download spend the journal records, so a
//!    resumed download is refused what the record already holds the run
//!    having spent, and the resumed claims record the level they reached.

use std::{
    collections::VecDeque,
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use saya_agent::{
    ApprovalDecider, CancellationToken, ChatMessage, ChatProvider, ChatRequest, ChatResponse,
    LocalStateEffect, ProviderError, TokenUsage, ToolCall, ToolDefinition, ToolEffect, ToolError,
    ToolExecutor,
};
use saya_harness::HarnessError;
use saya_harness::engine::{
    EngineEventSink, EpisodeCollaborators, EpisodeDriver, EpisodeRequest, EpisodeRun,
    ManifestBounds, ResumeError, ResumeOutcome, ResumeRun, RunState, SinkBudgets, StepToolset,
    TransitionEvent, UsageTotals, resume,
};
use saya_harness::fetch::DownloadBudget;
use saya_harness::journal::Journal;
use saya_harness::lock::RunLock;
use saya_harness::workspace::Workspace;
use saya_store::{
    NewRun, RunBudgets, RunCapabilityFlags, RunStatus, RunStepStatus, RunStore, SqliteStateStore,
};
use saya_types::{Capabilities, PauseReason, RunEvent, RunId, RunPlan, StepSpec};

/// A per-test scratch root: the state database and the run directory both
/// live under it, and one cleanup covers both.
fn temp_root(label: &str) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("saya-engine-resume-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn step(goal: &str) -> StepSpec {
    StepSpec::new(goal, Capabilities::default(), None, Vec::new(), None).unwrap()
}

/// The two-step plan the prefix fixture is driven with.
fn two_step_plan() -> RunPlan {
    RunPlan::new(vec![step("read the schema"), step("write the summary")]).unwrap()
}

/// A provider that answers every request — one request per step driven —
/// and records each one, so tests count exactly the work resume did.
#[derive(Default)]
struct AnswerProvider {
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}

impl AnswerProvider {
    fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

#[async_trait]
impl ChatProvider for AnswerProvider {
    fn name(&self) -> &str {
        "answer"
    }

    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        self.requests.lock().unwrap().push(request);
        Ok(ChatResponse::new(ChatMessage::text("assistant", "done")))
    }
}

/// A provider whose every answer reports usage — the spend a resumed
/// episode's turn makes — ten tokens per call, recorded like its sibling
/// above so tests can count the work.
#[derive(Default)]
struct UsageProvider {
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}

impl UsageProvider {
    fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

#[async_trait]
impl ChatProvider for UsageProvider {
    fn name(&self) -> &str {
        "usage-answer"
    }

    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        self.requests.lock().unwrap().push(request);
        let mut response = ChatResponse::new(ChatMessage::text("assistant", "done"));
        response.usage = Some(TokenUsage::new(10, 0));
        Ok(response)
    }
}

/// No scripted turn ever calls a tool; executing one is a test failure.
#[derive(Default)]
struct NoTools;

#[async_trait]
impl ToolExecutor for NoTools {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        panic!("no resume test may execute a tool call")
    }
}

/// Auto-approves everything; the stub tools declare no approval need.
#[derive(Default)]
struct AllowApproval;

#[async_trait]
impl ApprovalDecider for AllowApproval {
    async fn approve(&self, _: &ToolDefinition, _: &serde_json::Value) -> bool {
        true
    }
}

/// The stub collaborators every resume under test carries. The executors
/// live in the per-step toolsets (one `NoTools` behind a shared `Arc`), so
/// no executor field is needed here.
#[derive(Default)]
struct Stubs {
    provider: AnswerProvider,
    approval: AllowApproval,
}

fn bounds() -> ManifestBounds {
    ManifestBounds {
        max_files: 8,
        max_file_bytes: 64 * 1024,
    }
}

fn workspace(root: &std::path::Path) -> Workspace {
    let dir = root.join("workspace");
    fs::create_dir_all(&dir).unwrap();
    Workspace::open(&dir).unwrap()
}

fn request() -> EpisodeRequest {
    EpisodeRequest {
        model: "mock-model".into(),
        profile_names: Vec::new(),
        memory_allows_candidate_writes: false,
    }
}

/// A run whose journal holds `events` and whose store row stands at
/// `status` — the posture a real crash leaves: the journal is the
/// authority, the store at or behind it, nothing ahead of it.
struct CrashedRun {
    root: PathBuf,
    run_dir: PathBuf,
    store: Arc<SqliteStateStore>,
    run_id: RunId,
    stubs: Stubs,
    plan: RunPlan,
    /// One toolset per plan step, over one shared `NoTools` executor — no
    /// resume test under this rig executes a tool call.
    toolsets: Vec<StepToolset>,
}

impl CrashedRun {
    fn journal(&self) -> Vec<RunEvent> {
        Journal::open(&self.run_dir).read().unwrap()
    }

    fn inputs(&self) -> ResumeRun<'_> {
        ResumeRun {
            run_id: self.run_id.clone(),
            store: self.store.clone(),
            plan: self.plan.clone(),
            workspace: workspace(&self.root),
            collaborators: EpisodeCollaborators {
                provider: &self.stubs.provider,
                approval: &self.stubs.approval,
                toolsets: &self.toolsets,
                cancellation: CancellationToken::default(),
            },
            request: request(),
            bounds: bounds(),
            wall_clock: None,
            token_ceiling: None,
            download_budget: None,
            journal_wire: None,
            agent_stream: None,
        }
    }
}

impl Drop for CrashedRun {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Walks the store's status machine the legal way from `planned` to
/// `target` — the statuses a crash leaves behind are only ever ones the
/// store itself recorded.
async fn seed_status(store: &SqliteStateStore, id: &RunId, target: RunStatus) {
    if target == RunStatus::Planned {
        return;
    }
    store
        .set_run_status(id, RunStatus::Approved, None)
        .await
        .unwrap();
    if target != RunStatus::Approved {
        store
            .set_run_status(id, RunStatus::Executing, None)
            .await
            .unwrap();
    }
    if target == RunStatus::Paused {
        store
            .set_run_status(id, RunStatus::Paused, None)
            .await
            .unwrap();
    }
    if target == RunStatus::Completed {
        store
            .set_run_status(id, RunStatus::Completed, None)
            .await
            .unwrap();
    }
}

/// Builds a run row, a run directory with `events` appended to its journal,
/// and a store row at `status`.
async fn crashed_run(
    label: &str,
    events: &[RunEvent],
    status: RunStatus,
    plan: RunPlan,
) -> CrashedRun {
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
    seed_status(&store, &run_id, status).await;
    // The store's step rows mirror what the journal's step events say —
    // the journal is written before every mirror, so the store stands at
    // or behind the journal, never ahead of it.
    for event in events {
        match event {
            RunEvent::StepStarted { step } => store
                .upsert_step(&run_id, *step, RunStepStatus::Running)
                .await
                .unwrap(),
            RunEvent::StepCompleted { step } => store
                .upsert_step(&run_id, *step, RunStepStatus::Done)
                .await
                .unwrap(),
            _ => {}
        }
    }
    let run_dir = root.join("run");
    fs::create_dir_all(&run_dir).unwrap();
    let journal = Journal::open(&run_dir);
    for event in events {
        journal.append(event).unwrap();
    }
    let executor: Arc<dyn ToolExecutor> = Arc::new(NoTools);
    let toolsets = (0..plan.steps.len())
        .map(|_| StepToolset {
            executor: Arc::clone(&executor),
            definitions: Vec::new(),
        })
        .collect();
    CrashedRun {
        root,
        run_dir,
        store,
        run_id,
        stubs: Stubs::default(),
        plan,
        toolsets,
    }
}

/// Drives a two-step run end to end on a fresh store: `RunStarted` claimed,
/// the plan approved through the sink, both steps driven. Its journal is
/// the fixture the crash-prefix test truncates.
async fn driven_run(label: &str) -> CrashedRun {
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
    let journal = Journal::open(&run_dir);
    journal.append(&RunEvent::RunStarted).unwrap();
    let stubs = Stubs::default();
    let sink = EngineEventSink::new(
        run_id.clone(),
        RunState::Planned,
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
    sink.record(TransitionEvent::Approve { scopes: vec![] })
        .await
        .unwrap();
    let plan = two_step_plan();
    let executor: Arc<dyn ToolExecutor> = Arc::new(NoTools);
    let toolsets: Vec<StepToolset> = (0..plan.steps.len())
        .map(|_| StepToolset {
            executor: Arc::clone(&executor),
            definitions: Vec::new(),
        })
        .collect();
    let driver = EpisodeDriver::new(
        EpisodeCollaborators {
            provider: &stubs.provider,
            approval: &stubs.approval,
            toolsets: &toolsets,
            cancellation: CancellationToken::default(),
        },
        EpisodeRun {
            run_id: run_id.clone(),
            store: store.clone(),
            journal: journal.clone(),
        },
        request(),
        bounds(),
    );
    driver
        .run_step(&sink, &plan, 0, &workspace(&root))
        .await
        .unwrap();
    driver
        .run_step(&sink, &plan, 1, &workspace(&root))
        .await
        .unwrap();
    CrashedRun {
        root,
        run_dir,
        store,
        run_id,
        stubs,
        plan,
        toolsets,
    }
}

/// Every prefix of the driven two-step run's journal, resumed. The full
/// journal is seven events; for each prefix the table says where resume
/// must continue, how many provider requests that is, and what the journal
/// gains — completed steps are never re-run, never re-journalled.
#[tokio::test]
async fn a_crash_at_every_journal_position_resumes_at_the_first_incomplete_step() {
    let run = driven_run("prefixes").await;
    let raw = fs::read_to_string(run.run_dir.join("events.ndjson")).unwrap();
    let lines: Vec<&str> = raw.split_inclusive('\n').collect();
    assert_eq!(lines.len(), 7, "the driven fixture journal: {raw}");
    let full = run.journal();

    let ss = |step: usize| RunEvent::StepStarted { step };
    let sc = |step: usize| RunEvent::StepCompleted { step };
    let pd = RunEvent::Paused {
        reason: PauseReason::ProcessDeath,
    };
    let done = RunState::Completed;
    let cases: Vec<(usize, ResumeOutcome, usize, Vec<RunEvent>)> = vec![
        (0, ResumeOutcome::NoRun, 0, vec![]),
        (1, ResumeOutcome::Unapproved, 0, vec![]),
        (
            2,
            ResumeOutcome::Resumed {
                first_step: 0,
                state: done,
            },
            2,
            vec![ss(0), sc(0), ss(1), sc(1), RunEvent::Completed],
        ),
        (
            3,
            ResumeOutcome::Resumed {
                first_step: 0,
                state: done,
            },
            2,
            vec![pd.clone(), ss(0), sc(0), ss(1), sc(1), RunEvent::Completed],
        ),
        (
            4,
            ResumeOutcome::Resumed {
                first_step: 1,
                state: done,
            },
            1,
            vec![pd.clone(), ss(1), sc(1), RunEvent::Completed],
        ),
        (
            5,
            ResumeOutcome::Resumed {
                first_step: 1,
                state: done,
            },
            1,
            vec![pd.clone(), ss(1), sc(1), RunEvent::Completed],
        ),
        (
            6,
            ResumeOutcome::Settled { state: done },
            0,
            vec![pd, RunEvent::Completed],
        ),
        (7, ResumeOutcome::Settled { state: done }, 0, vec![]),
    ];

    for (prefix, outcome, requests, appended) in cases {
        // The store row a real crash leaves: at or behind the journal, never
        // ahead of it — the journal is written before every mirror.
        let status = match prefix {
            0 | 1 => RunStatus::Planned,
            2 => RunStatus::Approved,
            3..=6 => RunStatus::Executing,
            _ => RunStatus::Completed,
        };
        let crashed = crashed_run(
            &format!("prefix-{prefix}"),
            &full[..prefix],
            status,
            run.plan.clone(),
        )
        .await;

        let result = resume(&crashed.run_dir, crashed.inputs()).await;

        let continuing = match &outcome {
            ResumeOutcome::Resumed { first_step, .. } => format!("continue at step {first_step}"),
            ResumeOutcome::Settled { .. } => "settle the run".into(),
            other => format!("report {other:?}"),
        };
        assert_eq!(
            result.as_ref().unwrap(),
            &outcome,
            "prefix {prefix}: resume must {continuing}"
        );
        assert_eq!(
            crashed.stubs.provider.count(),
            requests,
            "prefix {prefix}: exactly the remaining steps' work, no more"
        );
        let expected = [full[..prefix].to_vec(), appended].concat();
        assert_eq!(
            crashed.journal(),
            expected,
            "prefix {prefix}: the journal must hold the prefix plus exactly the resumed events"
        );
        let recorded = crashed
            .store
            .get_run(&crashed.run_id)
            .await
            .unwrap()
            .unwrap();
        let expected_status = if prefix < 2 {
            RunStatus::Planned
        } else {
            RunStatus::Completed
        };
        assert_eq!(
            recorded.status, expected_status,
            "prefix {prefix}: the store must mirror the run's final state"
        );
        if (2..=6).contains(&prefix) {
            let steps = RunStore::list_steps(&*crashed.store, &crashed.run_id)
                .await
                .unwrap();
            assert_eq!(steps.len(), 2, "prefix {prefix}: both steps mirrored");
            assert!(
                steps.iter().all(|step| step.status == RunStepStatus::Done),
                "prefix {prefix}: every driven step is done in the store"
            );
        }
    }
}

/// A completed run resumes to "nothing to do": no step re-runs, no event is
/// re-journalled.
#[tokio::test]
async fn a_completed_run_resumes_to_nothing_to_do() {
    let run = driven_run("settled").await;
    let journal_before = run.journal();
    let requests_before = run.stubs.provider.count();

    let outcome = resume(&run.run_dir, run.inputs()).await.unwrap();

    assert_eq!(
        outcome,
        ResumeOutcome::Settled {
            state: RunState::Completed
        }
    );
    assert_eq!(
        run.stubs.provider.count(),
        requests_before,
        "a completed run must not re-run its last step"
    );
    assert_eq!(run.journal(), journal_before);
    let recorded = run.store.get_run(&run.run_id).await.unwrap().unwrap();
    assert_eq!(recorded.status, RunStatus::Completed);
}

/// A step executing at the crash restarts from its beginning — a fresh
/// attempt, not a mid-turn resume — and the restart is visible in the
/// journal as a second `StepStarted` for the same step.
#[tokio::test]
async fn a_step_executing_at_the_crash_restarts_from_its_start() {
    let plan = RunPlan::new(vec![step("the only step")]).unwrap();
    let run = crashed_run(
        "restart",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
        ],
        RunStatus::Executing,
        plan,
    )
    .await;

    let outcome = resume(&run.run_dir, run.inputs()).await.unwrap();

    assert_eq!(
        outcome,
        ResumeOutcome::Resumed {
            first_step: 0,
            state: RunState::Completed
        }
    );
    assert_eq!(
        run.stubs.provider.count(),
        1,
        "the restarted step ran one fresh attempt from its start"
    );
    let events = run.journal();
    assert_eq!(
        events,
        vec![
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
            RunEvent::Paused {
                reason: PauseReason::ProcessDeath
            },
            RunEvent::StepStarted { step: 0 },
            RunEvent::StepCompleted { step: 0 },
            RunEvent::Completed,
        ],
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, RunEvent::StepStarted { step: 0 }))
            .count(),
        2,
        "the restart must be visible in the journal"
    );
}

/// A second engine on the same run directory is refused while the first
/// holds the lock — and succeeds once the lock is released.
#[tokio::test]
async fn a_second_engine_on_the_same_run_directory_is_refused() {
    let run = crashed_run(
        "locked",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
        ],
        RunStatus::Executing,
        RunPlan::new(vec![step("the only step")]).unwrap(),
    )
    .await;
    let holder = RunLock::acquire(run.run_dir.join("lock")).unwrap();

    let error = resume(&run.run_dir, run.inputs()).await.unwrap_err();

    assert!(
        matches!(
            error,
            ResumeError::Lock {
                source: HarnessError::LockHeld { .. }
            }
        ),
        "the second engine must be refused by the run lock: {error:?}"
    );
    // The first engine releases the lock; the refusal was the lock alone.
    drop(holder);
    let outcome = resume(&run.run_dir, run.inputs()).await.unwrap();
    assert_eq!(
        outcome,
        ResumeOutcome::Resumed {
            first_step: 0,
            state: RunState::Completed
        },
        "once the lock is released the resume must proceed"
    );
}

/// One journaled usage event, shaped exactly as the engine journals a
/// provider call: the combined input-plus-output figure under the ceiling's
/// arithmetic, with every figure no call reported absent.
fn journaled_usage(tokens: u64) -> RunEvent {
    RunEvent::Usage {
        endpoint: "orchestrator".into(),
        tokens: Some(tokens),
        turns: None,
        tool_calls: None,
        cached_input_tokens: None,
        cache_creation_input_tokens: None,
    }
}

/// A torn final line — the crash that matters most — replays to the last
/// whole event: the half-written event is dropped, the run resumes past it.
#[tokio::test]
async fn a_torn_final_line_replays_to_the_last_whole_event() {
    let run = crashed_run(
        "torn",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
            RunEvent::StepCompleted { step: 0 },
        ],
        RunStatus::Executing,
        two_step_plan(),
    )
    .await;
    let torn = "{\"type\":\"step_started\",\"step\":1";
    fs::write(
        run.run_dir.join("events.ndjson"),
        format!(
            "{}\n{torn}",
            run.journal()
                .iter()
                .map(|event| serde_json::to_string(event).unwrap())
                .collect::<Vec<_>>()
                .join("\n")
        ),
    )
    .unwrap();

    let outcome = resume(&run.run_dir, run.inputs()).await.unwrap();

    assert_eq!(
        outcome,
        ResumeOutcome::Resumed {
            first_step: 1,
            state: RunState::Completed
        },
        "the torn line must not error the run nor hide step 1's incompleteness"
    );
    assert_eq!(
        run.journal(),
        vec![
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
            RunEvent::StepCompleted { step: 0 },
            RunEvent::Paused {
                reason: PauseReason::ProcessDeath
            },
            RunEvent::StepStarted { step: 1 },
            RunEvent::StepCompleted { step: 1 },
            RunEvent::Completed,
        ],
        "the torn line must be dropped, not replayed"
    );
}

/// The resume inputs for a run with a declared token ceiling and a provider
/// whose turn reports usage — the budget posture of the tests below; every
/// other input is the crashed run's own.
fn budgeted_inputs<'a>(
    run: &'a CrashedRun,
    provider: &'a UsageProvider,
    ceiling: Option<u64>,
) -> ResumeRun<'a> {
    ResumeRun {
        run_id: run.run_id.clone(),
        store: run.store.clone(),
        plan: run.plan.clone(),
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
        token_ceiling: ceiling,
        download_budget: None,
        journal_wire: None,
        agent_stream: None,
    }
}

/// The one that matters: the token ceiling binds the **run**, not each
/// invocation. A run pauses on its 150-token ceiling having spent 160; a
/// resume seeds the sink's totals from the journal's usage record instead of
/// re-arming the ceiling, so — resumed with no fresh budget — the run pauses
/// again at its cumulative spend. The pause lands on the first tick of the
/// resumed episode, before the turn's own usage folds in: the carried spend
/// alone is already past the ceiling, which no re-armed ceiling could trip.
/// Under the re-arm behaviour this replaces, the resumed invocation would
/// have started from zero and spent a whole fresh ceiling before pausing.
#[tokio::test]
async fn a_budget_paused_run_resumed_without_a_fresh_budget_pauses_at_its_cumulative_spend() {
    // The posture a `BudgetExhausted` pause leaves: the per-call usage the
    // engine journals, then the pause naming the budget.
    let plan = RunPlan::new(vec![step("the only step")]).unwrap();
    let run = crashed_run(
        "budget-resume",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
            journaled_usage(100),
            journaled_usage(60),
            RunEvent::Paused {
                reason: PauseReason::BudgetExhausted,
            },
        ],
        RunStatus::Paused,
        plan,
    )
    .await;
    let provider = UsageProvider::default();

    // Resumed against the same ceiling, granted nothing fresh.
    let outcome = resume(&run.run_dir, budgeted_inputs(&run, &provider, Some(150))).await;

    // The resumed episode tripped the ceiling on its first tick and then ran
    // to its natural end — a paused run's episode is not interrupted — so
    // the driver's completion meets the machine: only an executing run
    // completes. That refusal surfaces as the step's error; the pause itself
    // is in the journal, and it is the shape every mid-episode pause leaves.
    let error = outcome.unwrap_err();
    assert!(
        matches!(error, ResumeError::Episode { .. }),
        "the machine must refuse a paused run's completion: {error:?}"
    );

    // The journal: the first invocation's record, then the resume's — one
    // step started, the ceiling pause, the turn's usage (journaled after the
    // pause that tripped on the carried spend), and the step's completion.
    assert_eq!(
        run.journal(),
        vec![
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
            journaled_usage(100),
            journaled_usage(60),
            RunEvent::Paused {
                reason: PauseReason::BudgetExhausted
            },
            RunEvent::StepStarted { step: 0 },
            RunEvent::Paused {
                reason: PauseReason::BudgetExhausted
            },
            journaled_usage(10),
            RunEvent::StepCompleted { step: 0 },
        ],
        "the resume must pause again on the run's cumulative spend, not at a \
         re-armed ceiling"
    );
    let events = run.journal();
    // The run's whole spend across both invocations.
    let total: u64 = events
        .iter()
        .map(|event| match event {
            RunEvent::Usage {
                tokens: Some(tokens),
                ..
            } => *tokens,
            _ => 0,
        })
        .sum();
    assert_eq!(total, 170, "the run spent 160 before the pause, 10 after");
    // The store mirrors the pause the resume recorded.
    let recorded = run.store.get_run(&run.run_id).await.unwrap().unwrap();
    assert_eq!(
        recorded.status,
        RunStatus::Paused,
        "the run must stand paused, not running on a fresh budget"
    );
    // One call — the paused episode's own turn, not a fresh budget's worth.
    assert_eq!(provider.count(), 1);

    let _ = fs::remove_dir_all(&run.root);
}

/// Seeding rides the repaired record: a torn trailing usage line — the
/// half-written event a crash left — never counts toward the carried spend.
/// The journal shows 100 tokens spent and a torn line that would have added
/// 60; counting it would push the carried spend past the 150 ceiling and
/// pause the resumed run at its first tick. The repaired record carries only
/// the whole event, so the run resumes and completes.
#[tokio::test]
async fn a_torn_usage_line_never_counts_toward_the_carried_spend() {
    let plan = RunPlan::new(vec![step("the only step")]).unwrap();
    let run = crashed_run(
        "torn-usage",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
            journaled_usage(100),
        ],
        RunStatus::Executing,
        plan,
    )
    .await;
    let torn = "{\"type\":\"usage\",\"endpoint\":\"orchestrator\",\"tokens\":60";
    fs::write(
        run.run_dir.join("events.ndjson"),
        format!(
            "{}\n{torn}",
            run.journal()
                .iter()
                .map(|event| serde_json::to_string(event).unwrap())
                .collect::<Vec<_>>()
                .join("\n")
        ),
    )
    .unwrap();
    let provider = UsageProvider::default();

    let outcome = resume(&run.run_dir, budgeted_inputs(&run, &provider, Some(150))).await;

    // The carried spend is the repaired record's 100 — the torn 60 never
    // counted — so the resumed turn's 10 keep the run under the ceiling.
    assert_eq!(
        outcome.as_ref().unwrap(),
        &ResumeOutcome::Resumed {
            first_step: 0,
            state: RunState::Completed
        },
        "counting the torn line would have paused the run at 170 against 150"
    );
    assert_eq!(
        run.journal(),
        vec![
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
            journaled_usage(100),
            RunEvent::Paused {
                reason: PauseReason::ProcessDeath
            },
            RunEvent::StepStarted { step: 0 },
            journaled_usage(10),
            RunEvent::StepCompleted { step: 0 },
            RunEvent::Completed,
        ]
    );
    assert_eq!(provider.count(), 1);
}

// --- the download budget's carry (the wallet, not the latch) ------------------

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

/// The one that matters for the wallet: the download budget binds the
/// **run**, not each invocation. A run pauses on its 100-byte wallet having
/// claimed 97; a resume seeds the wallet the composition armed with the
/// spend the journal records, so the resumed download's 40-byte claim is
/// refused — a fresh wallet would have accepted it — and the refusal, the
/// same latch, pauses the run again. Under the re-arm behaviour this
/// replaces, the resumed invocation would have started from zero and spent
/// a whole fresh wallet.
#[tokio::test]
async fn a_download_paused_run_resumed_without_a_fresh_wallet_continues_against_its_recorded_spend()
{
    let plan = RunPlan::new(vec![step("download the corpus")]).unwrap();
    let run = crashed_run(
        "download-resume",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
            RunEvent::DownloadedBytes { bytes: 60 },
            RunEvent::DownloadedBytes { bytes: 97 },
            RunEvent::Paused {
                reason: PauseReason::BudgetExhausted,
            },
        ],
        RunStatus::Paused,
        plan,
    )
    .await;
    let wallet = DownloadBudget::new(100);
    let provider = ScriptProvider::new(vec![
        Turn::Tools(vec![tool_call("http_download")]),
        Turn::Answer("done"),
    ]);
    let executor: Arc<dyn ToolExecutor> = Arc::new(Claiming {
        wallet: wallet.clone(),
        bytes: 40,
    });
    let toolsets = vec![StepToolset {
        executor,
        definitions: vec![download_definition()],
    }];

    let outcome = resume(
        &run.run_dir,
        download_inputs(&run, &provider, &toolsets, wallet.clone()),
    )
    .await;

    // The machine refuses the paused run's completion — the same shape
    // every mid-episode pause leaves.
    let error = outcome.unwrap_err();
    assert!(
        matches!(error, ResumeError::Episode { .. }),
        "the machine must refuse a paused run's completion: {error:?}"
    );

    // The wallet binds the run: the claimed 40 were refused against the 97
    // the record holds, the refusal tripped the latch, and the run stands
    // paused — not running on a fresh wallet.
    assert_eq!(
        wallet.consumed(),
        97,
        "the run's budget is spent, not a fresh one"
    );
    assert!(wallet.tripped(), "the refusal is the recorded event");
    let recorded = run.store.get_run(&run.run_id).await.unwrap().unwrap();
    assert_eq!(recorded.status, RunStatus::Paused);

    // The journal: the first invocation's record, then the resume's — the
    // step restarted, the refusal paused the run again, the episode ran to
    // its natural end, and nothing was claimed (no level grew past 97).
    assert_eq!(
        run.journal(),
        vec![
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
            RunEvent::DownloadedBytes { bytes: 60 },
            RunEvent::DownloadedBytes { bytes: 97 },
            RunEvent::Paused {
                reason: PauseReason::BudgetExhausted
            },
            RunEvent::StepStarted { step: 0 },
            RunEvent::Paused {
                reason: PauseReason::BudgetExhausted
            },
            RunEvent::StepCompleted { step: 0 },
        ],
        "the resume must continue against the recorded spend, not a fresh wallet"
    );
}

/// The carried headroom is real, and the resumed claims record the level
/// they reached: the resumed download claims the three bytes the record
/// leaves (97 of 100), the sink journals the level — 100, the carried 97
/// plus this invocation's 3, never the claim's own 3 — and the next claim
/// past the limit is refused, tripping the latch and pausing the run. The
/// journaled level is what pins the whole chain: without the carry it would
/// read 3, and a fresh baseline would have re-journaled the record's 97.
#[tokio::test]
async fn a_resumed_download_claims_the_carried_headroom_and_records_the_level_it_reached() {
    let plan = RunPlan::new(vec![step("download the corpus")]).unwrap();
    let run = crashed_run(
        "download-headroom",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
            RunEvent::DownloadedBytes { bytes: 60 },
            RunEvent::DownloadedBytes { bytes: 97 },
            RunEvent::Paused {
                reason: PauseReason::BudgetExhausted,
            },
        ],
        RunStatus::Paused,
        plan,
    )
    .await;
    let wallet = DownloadBudget::new(100);
    let provider = ScriptProvider::new(vec![
        Turn::Tools(vec![tool_call("http_download")]),
        Turn::Tools(vec![tool_call("http_download")]),
        Turn::Answer("done"),
    ]);
    let executor: Arc<dyn ToolExecutor> = Arc::new(Claiming {
        wallet: wallet.clone(),
        bytes: 3,
    });
    let toolsets = vec![StepToolset {
        executor,
        definitions: vec![download_definition()],
    }];

    let outcome = resume(
        &run.run_dir,
        download_inputs(&run, &provider, &toolsets, wallet.clone()),
    )
    .await;

    let error = outcome.unwrap_err();
    assert!(
        matches!(error, ResumeError::Episode { .. }),
        "the machine must refuse a paused run's completion: {error:?}"
    );

    // The headroom claim fit and was spent; the claim past the limit was
    // refused and tripped the latch — the run's budget, not a fresh one.
    assert_eq!(wallet.consumed(), 100, "the headroom the record leaves");
    assert!(wallet.tripped(), "the claim past the limit is the refusal");
    let recorded = run.store.get_run(&run.run_id).await.unwrap().unwrap();
    assert_eq!(recorded.status, RunStatus::Paused);

    // The journal: the resume journaled the level the wallet reached —
    // 100, cumulative — and re-journaled nothing the record already held.
    assert_eq!(
        run.journal(),
        vec![
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
            RunEvent::DownloadedBytes { bytes: 60 },
            RunEvent::DownloadedBytes { bytes: 97 },
            RunEvent::Paused {
                reason: PauseReason::BudgetExhausted
            },
            RunEvent::StepStarted { step: 0 },
            RunEvent::DownloadedBytes { bytes: 100 },
            RunEvent::Paused {
                reason: PauseReason::BudgetExhausted
            },
            RunEvent::StepCompleted { step: 0 },
        ],
        "the resumed claims must record the level they reached, past the \
         carried spend, never the carried spend again"
    );
}
