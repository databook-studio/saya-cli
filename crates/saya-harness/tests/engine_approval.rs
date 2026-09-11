//! The approval gate at `planned → approved` — contract tests (M1-10).
//!
//! Four guarantees:
//! 1. A planned run refuses to begin a step until the plan is approved, the
//!    approval goes through the engine's `TransitionEvent` path once, and a
//!    second approval is refused — approval is per plan, granted once.
//! 2. A plan asking for a scope the approved set does not hold is refused
//!    by the propose gate rather than silently binding it — the same
//!    approved set refuses it, naming the missing scope; only an explicit
//!    widening binds. The engine proposes once, before approval; there is
//!    no separate mid-run revision flow, so this gate is the only one.
//! 3. Approving the plan does not produce a per-tool-call prompt afterwards:
//!    the steps complete with a tool that requires approval, and the
//!    channel-mirroring decider never prompts.
//! 4. The needs-approval refusal names the missing scope — the composition
//!    root's message can say what was missing, not refuse generically.

use std::{
    collections::VecDeque,
    fs,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use saya_agent::{
    ApprovalDecider, CancellationToken, ChatMessage, ChatProvider, ChatRequest, ChatResponse,
    LocalStateEffect, ProviderError, ToolCall, ToolEffect, ToolExecutor, read_only_permits,
};
use saya_harness::engine::{
    EngineEventSink, EpisodeCollaborators, EpisodeDriver, EpisodeError, EpisodeRun, ManifestBounds,
    PlanDriver, PlanError, PlanRejection, PlanRequest, RunState, StepToolset, TransitionEvent,
};
use saya_harness::journal::Journal;
use saya_harness::workspace::Workspace;
use saya_store::{NewRun, RunBudgets, RunCapabilityFlags, RunStatus, RunStore, SqliteStateStore};
use saya_types::{Budgets, Capabilities, RunId, StepSpec};
use tokio::sync::oneshot;

/// A per-test scratch root: the state database and the run directory both
/// live under it, and one cleanup covers both.
fn temp_root(label: &str) -> PathBuf {
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
struct PlannedRun {
    root: PathBuf,
    run_dir: PathBuf,
    store: Arc<SqliteStateStore>,
    run_id: RunId,
}

impl PlannedRun {
    fn workspace(&self) -> Workspace {
        let dir = self.root.join("workspace");
        fs::create_dir_all(&dir).unwrap();
        Workspace::open(&dir).unwrap()
    }
}

async fn planned_run(label: &str) -> PlannedRun {
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
struct ScriptedPlanner {
    script: Mutex<VecDeque<String>>,
    requests: Mutex<Vec<ChatRequest>>,
}

impl ScriptedPlanner {
    fn new(script: Vec<String>) -> Self {
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
fn plan_json(steps: &str) -> String {
    format!(r#"{{"steps": [{steps}]}}"#)
}

fn step_json(goal: &str, capabilities: &str) -> String {
    format!(
        r#"{{"goal": "{goal}", "capabilities": {capabilities}, "budget": null, "expects": [], "endpoint": null}}"#
    )
}

fn planner_request(run_goal: &str) -> PlanRequest {
    PlanRequest {
        model: "mock-model".into(),
        run_goal: run_goal.into(),
    }
}

fn workspace_write_scopes() -> Capabilities {
    let mut scopes = Capabilities::default();
    scopes.workspace_write = true;
    scopes
}

/// One tool definition the plan approval covers: it requires approval (a
/// read-shaped SQL probe — `database_data`, no external side effect, a
/// contained read) so the per-call decider is consulted, and it is visible
/// in every step because a contained read is not capability-scoped.
fn approved_read_tool() -> saya_agent::ToolDefinition {
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
        completion: None,
    }
}

/// One scripted episode turn: a tool call or a terminal prose answer.
enum Turn {
    Tools(Vec<ToolCall>),
    Answer(&'static str),
}

/// A scripted episode provider, the `engine_episode.rs` pattern.
struct ScriptedEpisode {
    script: Mutex<VecDeque<Turn>>,
}

impl ScriptedEpisode {
    fn new(script: Vec<Turn>) -> Self {
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
struct RecordingTools {
    calls: Mutex<Vec<String>>,
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

fn call(name: &str) -> ToolCall {
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
struct CountingApproval {
    prompts: AtomicUsize,
}

impl CountingApproval {
    fn prompted(&self) -> usize {
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

/// A planned run refuses to begin a step until the plan is approved; the
/// approval goes through the sink's `TransitionEvent` path exactly once,
/// and a second approval is refused by the machine.
#[tokio::test]
async fn a_planned_run_refuses_to_begin_until_the_plan_is_approved_once() {
    let run = planned_run("gate").await;
    let plan = saya_types::RunPlan::new(vec![
        StepSpec::new(
            "survey the data",
            Capabilities::default(),
            None,
            Vec::new(),
            None,
        )
        .unwrap(),
    ])
    .unwrap();
    let journal = Journal::open(&run.run_dir);
    let sink = EngineEventSink::new(
        run.run_id.clone(),
        RunState::Planned,
        journal.clone(),
        run.store.clone(),
        None,
        None,
        std::time::Instant::now,
    );
    let episode = ScriptedEpisode::new(vec![Turn::Answer("done")]);
    let tools: Arc<dyn ToolExecutor> = Arc::new(RecordingTools::default());
    let approval = CountingApproval {
        prompts: AtomicUsize::new(0),
    };
    let toolsets = vec![StepToolset {
        executor: tools,
        definitions: vec![],
    }];
    let driver = EpisodeDriver::new(
        EpisodeCollaborators {
            provider: &episode,
            approval: &approval,
            toolsets: &toolsets,
            cancellation: CancellationToken::default(),
        },
        EpisodeRun {
            run_id: run.run_id.clone(),
            store: run.store.clone(),
            journal: journal.clone(),
        },
        saya_harness::engine::EpisodeRequest {
            model: "mock-model".into(),
            profile_names: Vec::new(),
            memory_allows_candidate_writes: false,
        },
        ManifestBounds {
            max_files: 8,
            max_file_bytes: 64 * 1024,
        },
    );

    // Before approval there is nothing to run under.
    let error = driver
        .run_step(&sink, &plan, 0, &run.workspace())
        .await
        .unwrap_err();
    assert!(
        matches!(
            &error,
            EpisodeError::NotRunnable {
                step: 0,
                state: RunState::Planned
            }
        ),
        "a planned run must refuse to begin: {error:?}"
    );

    // The one approval, through the transition path — never a direct write.
    sink.record(TransitionEvent::Approve).await.unwrap();
    assert_eq!(sink.state(), RunState::Approved);
    let recorded = RunStore::get_run(run.store.as_ref(), &run.run_id)
        .await
        .unwrap()
        .expect("the run is recorded");
    assert_eq!(recorded.status, RunStatus::Approved);

    // Approval is per plan, granted once: the machine refuses a second one.
    assert!(
        sink.record(TransitionEvent::Approve).await.is_err(),
        "approving an already-approved plan must be refused"
    );

    // Now the step runs and the plan completes.
    driver
        .run_step(&sink, &plan, 0, &run.workspace())
        .await
        .unwrap();
    assert_eq!(sink.state(), RunState::Completed);

    let journal_text = fs::read_to_string(run.run_dir.join("events.ndjson")).unwrap();
    assert_eq!(
        journal_text.matches("plan_approved").count(),
        1,
        "exactly one approval is on the durable record: {journal_text}"
    );
    let _ = fs::remove_dir_all(run.root);
}

/// A plan asking for a scope the approved set does not hold is refused
/// rather than inheriting anything: the same approved set refuses it,
/// naming the missing scope; only an explicit widening binds. The engine
/// proposes once, before approval — there is no mid-run revision flow —
/// so this test drives the propose gate itself, twice.
#[tokio::test]
async fn a_reproposed_plan_asking_a_new_scope_is_refused_rather_than_inheriting() {
    let first = plan_json(&step_json(
        "write the report",
        r#"{"workspace_write": true}"#,
    ));
    let widening = plan_json(&step_json(
        "seed the scratch database",
        r#"{"scratch": true}"#,
    ));
    // The first proposal binds on one answer; the re-proposal is re-prompted
    // to the bound; the widened proposal binds on one answer.
    let planner = ScriptedPlanner::new(vec![
        first,
        widening.clone(),
        widening.clone(),
        widening,
        plan_json(&step_json(
            "seed the scratch database",
            r#"{"scratch": true}"#,
        )),
    ]);
    let driver = PlanDriver::new(&planner, planner_request("write, then seed"));

    let approved = workspace_write_scopes();
    let first_plan = driver
        .propose(&approved, &Budgets::default())
        .await
        .unwrap();
    assert_eq!(first_plan.steps.len(), 1, "the approved plan binds");

    // A re-proposal against the same approved set: the new scope does not
    // inherit.
    let error = driver
        .propose(&approved, &Budgets::default())
        .await
        .unwrap_err();
    assert!(
        matches!(
            &error,
            PlanError::Exhausted {
                attempts: 3,
                last: PlanRejection::NeedsApproval { step: 0, scopes },
            } if scopes == &["scratch".to_string()]
        ),
        "the refusal must name the new scope: {error:?}"
    );

    // The approval widens explicitly; the re-proposed plan binds.
    let mut granted = workspace_write_scopes();
    granted.scratch = true;
    let reproposed = driver.propose(&granted, &Budgets::default()).await.unwrap();
    assert!(reproposed.steps[0].capabilities.scratch);
}

/// Approving the plan once does not produce a per-tool-call prompt
/// afterwards: the steps complete with a tool that requires approval, and
/// the prompting decider is never asked.
#[tokio::test]
async fn approving_the_plan_once_does_not_prompt_per_tool_call() {
    let run = planned_run("no-per-call").await;
    let plan = saya_types::RunPlan::new(vec![
        StepSpec::new(
            "survey the schema",
            workspace_write_scopes(),
            None,
            Vec::new(),
            None,
        )
        .unwrap(),
        StepSpec::new(
            "survey the rows",
            workspace_write_scopes(),
            None,
            Vec::new(),
            None,
        )
        .unwrap(),
    ])
    .unwrap();
    let journal = Journal::open(&run.run_dir);
    let sink = EngineEventSink::new(
        run.run_id.clone(),
        RunState::Planned,
        journal.clone(),
        run.store.clone(),
        None,
        None,
        std::time::Instant::now,
    );
    // The one approval interaction: the plan, decided once.
    sink.record(TransitionEvent::Approve).await.unwrap();

    let episode = ScriptedEpisode::new(vec![
        Turn::Tools(vec![call("sql_probe")]),
        Turn::Answer("first step done"),
        Turn::Tools(vec![call("sql_probe")]),
        Turn::Answer("second step done"),
    ]);
    // One executor shared by both steps' toolsets: the call log the test
    // asserts on is the one the loop wrote across both episodes.
    let tools = Arc::new(RecordingTools::default());
    let definitions = vec![approved_read_tool()];
    let toolsets: Vec<StepToolset> = (0..2)
        .map(|_| StepToolset {
            executor: tools.clone(),
            definitions: definitions.clone(),
        })
        .collect();
    let approval = CountingApproval {
        prompts: AtomicUsize::new(0),
    };
    let driver = EpisodeDriver::new(
        EpisodeCollaborators {
            provider: &episode,
            approval: &approval,
            toolsets: &toolsets,
            cancellation: CancellationToken::default(),
        },
        EpisodeRun {
            run_id: run.run_id.clone(),
            store: run.store.clone(),
            journal: journal.clone(),
        },
        saya_harness::engine::EpisodeRequest {
            model: "mock-model".into(),
            profile_names: Vec::new(),
            memory_allows_candidate_writes: false,
        },
        ManifestBounds {
            max_files: 8,
            max_file_bytes: 64 * 1024,
        },
    );
    let workspace = run.workspace();

    driver.run_step(&sink, &plan, 0, &workspace).await.unwrap();
    driver.run_step(&sink, &plan, 1, &workspace).await.unwrap();

    // The tool calls really ran — the zero below is meaningful.
    assert_eq!(
        tools.calls.lock().unwrap().clone(),
        vec!["sql_probe".to_string(), "sql_probe".to_string()],
        "both steps' tool calls executed under the plan approval"
    );
    assert_eq!(
        approval.prompted(),
        0,
        "an approved plan must not produce per-tool-call prompts"
    );
    assert_eq!(sink.state(), RunState::Completed);

    let journal_text = fs::read_to_string(run.run_dir.join("events.ndjson")).unwrap();
    assert_eq!(
        journal_text.matches("plan_approved").count(),
        1,
        "exactly one approval interaction is on the durable record: {journal_text}"
    );
    let _ = fs::remove_dir_all(run.root);
}

/// The needs-approval refusal names the missing scope, so the composition
/// root's message can say what was missing rather than refuse generically.
#[tokio::test]
async fn the_needs_approval_refusal_names_the_missing_scope() {
    let asking = plan_json(&step_json(
        "fetch the report",
        r#"{"fetch": {"destinations": [{"scheme": "https", "host": "example.com"}]}}"#,
    ));
    let planner = ScriptedPlanner::new(vec![asking; 3]);
    let driver = PlanDriver::new(&planner, planner_request("fetch"));

    let error = driver
        .propose(&workspace_write_scopes(), &Budgets::default())
        .await
        .unwrap_err();

    let last = match &error {
        PlanError::Exhausted { last, .. } => last,
        other => panic!("expected exhaustion, got {other:?}"),
    };
    assert!(
        last.to_string().contains("fetch:https+example.com"),
        "the refusal must name the missing scope: {last}"
    );
}
