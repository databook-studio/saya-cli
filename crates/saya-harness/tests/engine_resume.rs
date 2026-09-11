//! Resume — contract tests (M1-5, resume slice).
//!
//! Five guarantees, one per test:
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

use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use saya_agent::{
    ApprovalDecider, CancellationToken, ChatMessage, ChatProvider, ChatRequest, ChatResponse,
    ProviderError, ToolDefinition, ToolError, ToolExecutor,
};
use saya_harness::HarnessError;
use saya_harness::engine::{
    EngineEventSink, EpisodeCollaborators, EpisodeDriver, EpisodeRequest, EpisodeRun,
    ManifestBounds, ResumeError, ResumeOutcome, ResumeRun, RunState, TransitionEvent, resume,
};
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

/// The stub collaborators every resume under test carries.
#[derive(Default)]
struct Stubs {
    provider: AnswerProvider,
    tools: NoTools,
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
                tools: &self.stubs.tools,
                approval: &self.stubs.approval,
                universe: Vec::new(),
                cancellation: CancellationToken::default(),
            },
            request: request(),
            bounds: bounds(),
            wall_clock: None,
            token_ceiling: None,
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
    CrashedRun {
        root,
        run_dir,
        store,
        run_id,
        stubs: Stubs::default(),
        plan,
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
        None,
        None,
        std::time::Instant::now,
    );
    sink.record(TransitionEvent::Approve).await.unwrap();
    let plan = two_step_plan();
    let driver = EpisodeDriver::new(
        EpisodeCollaborators {
            provider: &stubs.provider,
            tools: &stubs.tools,
            approval: &stubs.approval,
            universe: Vec::new(),
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
            RunEvent::PlanApproved,
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
            RunEvent::PlanApproved,
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
            RunEvent::PlanApproved,
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

/// A torn final line — the crash that matters most — replays to the last
/// whole event: the half-written event is dropped, the run resumes past it.
#[tokio::test]
async fn a_torn_final_line_replays_to_the_last_whole_event() {
    let run = crashed_run(
        "torn",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved,
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
            RunEvent::PlanApproved,
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
