//! The episode driver — contract tests (M1-5, episode slice).
//!
//! Five guarantees, one per test:
//! 1. A tool outside the step's capabilities is **absent** from the
//!    definitions the loop receives — hidden, not advertised-and-refused.
//! 2. The step's budget becomes the loop's `AgentLimits`, and
//!    `SAYA_AGENT_MAX_TURNS` set in the environment changes nothing (a run
//!    is reproducible from its spec and config alone — plan G3).
//! 3. `permit_candidate_writes` is pinned false by construction, even when
//!    the caller carries the user's memory-mode permission to write
//!    candidates (DESIGN §5.8).
//! 4. A failing episode retries a bounded number of times, each with a
//!    fresh brief, then records a typed failure and pauses — never
//!    unbounded, never silent.
//! 5. The brief carries the workspace manifest: names, sizes, digests.
//!
//! The scope-wiring slices add their own gates on the same harness; S1's
//! (scratch) proves a scratch-only step runs DDL through `scratch_sql` in a
//! real episode.

use std::{
    collections::VecDeque,
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use saya_agent::{
    ApprovalDecider, CancellationToken, ChatMessage, ChatProvider, ChatRequest, ChatResponse,
    LocalStateEffect, ProviderError, ToolCall, ToolDefinition, ToolEffect, ToolError, ToolExecutor,
};
use saya_harness::engine::{
    EngineEventSink, EpisodeCollaborators, EpisodeDriver, EpisodeError, EpisodeRequest, EpisodeRun,
    ManifestBounds, RunState, SinkBudgets, StepToolset, UsageTotals,
};
use saya_harness::fetch::{FetchDestination, FetchTransportError};
use saya_harness::journal::Journal;
use saya_harness::workspace::{Workspace, manifest};
use saya_store::{
    NewRun, RunBudgets, RunCapabilityFlags, RunStatus, RunStepStatus, RunStore, SqliteStateStore,
};
use saya_types::{
    Budgets, Capabilities, PauseReason, RunEvent, RunFailureCode, RunId, RunPlan, StepSpec,
};

/// A per-test scratch root: the state database and the run directory both
/// live under it, and one cleanup covers both.
fn temp_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "saya-engine-episode-{label}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

/// A run already standing at `approved` — the state a run is in when its
/// first episode begins.
struct ApprovedRun {
    root: PathBuf,
    run_dir: PathBuf,
    store: Arc<SqliteStateStore>,
    run_id: RunId,
}

impl ApprovedRun {
    fn workspace(&self) -> Workspace {
        let dir = self.root.join("workspace");
        fs::create_dir_all(&dir).unwrap();
        Workspace::open(&dir).unwrap()
    }
}

async fn approved_run(label: &str) -> ApprovedRun {
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
    store
        .set_run_status(&run_id, RunStatus::Approved, None)
        .await
        .unwrap();
    let run_dir = root.join("run");
    fs::create_dir_all(&run_dir).unwrap();
    ApprovedRun {
        root,
        run_dir,
        store,
        run_id,
    }
}

/// One scripted provider turn.
enum Turn {
    /// An assistant turn carrying tool calls.
    Tools(Vec<ToolCall>),
    /// A terminal prose answer (no tool calls).
    Answer(&'static str),
    /// A provider failure.
    Fail,
}

/// A scripted `ChatProvider`: serves its turns in order and records every
/// request it received, so tests assert on the definitions and the brief
/// the loop actually sent.
struct ScriptProvider {
    script: Mutex<VecDeque<Turn>>,
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}

impl ScriptProvider {
    fn new(script: Vec<Turn>) -> Self {
        Self {
            script: Mutex::new(script.into()),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn requests(&self) -> Vec<ChatRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl ChatProvider for ScriptProvider {
    fn name(&self) -> &str {
        "script"
    }

    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        self.requests.lock().unwrap().push(request);
        match self.script.lock().unwrap().pop_front() {
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

/// A tool executor that records every call it received.
#[derive(Default)]
struct RecordingTools {
    calls: Arc<Mutex<Vec<String>>>,
}

impl RecordingTools {
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl ToolExecutor for RecordingTools {
    async fn execute(
        &self,
        name: &str,
        _: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        self.calls.lock().unwrap().push(name.into());
        Ok(serde_json::json!({"rows": 1}))
    }
}

/// A tool executor that seeds a numbered file into the workspace on every
/// call, so a fresh brief (rebuilt per attempt) can be told apart from a
/// carried-over one by the manifest it carries.
struct SeedingTools {
    workspace: PathBuf,
    calls: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl ToolExecutor for SeedingTools {
    async fn execute(
        &self,
        name: &str,
        _: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        let index = {
            let mut calls = self.calls.lock().unwrap();
            calls.push(name.into());
            calls.len() - 1
        };
        fs::write(
            self.workspace.join(format!("seeded-{index}.csv")),
            format!("row {index}"),
        )
        .unwrap();
        Ok(serde_json::json!({"rows": index}))
    }
}

/// One tool definition with the given local-state effect; auto-runnable
/// (no approval, no external side effect, no database data) so the loop's
/// own gates are the only thing that can refuse it.
fn def(name: &str, local_state: LocalStateEffect) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: name.into(),
        read_only: local_state == LocalStateEffect::None,
        parameters: serde_json::json!({"type": "object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state,
        },
        completion: None,
    }
}

/// One step's toolset: the executor behind it and the definitions the
/// step's episodes advertise — the composition root's per-step shape.
fn toolset(executor: Arc<dyn ToolExecutor>, definitions: Vec<ToolDefinition>) -> StepToolset {
    StepToolset {
        executor,
        definitions,
    }
}

fn call(name: &str) -> ToolCall {
    ToolCall {
        id: format!("c-{name}"),
        name: name.into(),
        arguments: serde_json::json!({}),
    }
}

fn step(goal: &str, capabilities: Capabilities) -> StepSpec {
    StepSpec::new(goal, capabilities, None, Vec::new(), None).unwrap()
}

fn step_with_budget(goal: &str, budget: Option<Budgets>) -> StepSpec {
    StepSpec::new(goal, Capabilities::default(), budget, Vec::new(), None).unwrap()
}

fn bounds() -> ManifestBounds {
    ManifestBounds {
        max_files: 8,
        max_file_bytes: 64 * 1024,
    }
}

/// Auto-approves everything: the stub tools declare no approval requirement,
/// so the episodes' runs need no interactive decision.
struct AllowApproval;

#[async_trait]
impl ApprovalDecider for AllowApproval {
    async fn approve(&self, _: &ToolDefinition, _: &serde_json::Value) -> bool {
        true
    }
}

fn driver_and_sink<'a>(
    run: &ApprovedRun,
    provider: &'a ScriptProvider,
    approval: &'a AllowApproval,
    toolsets: &'a [StepToolset],
    memory_allows_candidate_writes: bool,
) -> (EngineEventSink, EpisodeDriver<'a>) {
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
            provider,
            approval,
            toolsets,
            cancellation: CancellationToken::default(),
        },
        EpisodeRun {
            run_id: run.run_id.clone(),
            store: run.store.clone(),
            journal: Journal::open(&run.run_dir),
        },
        EpisodeRequest {
            model: "mock-model".into(),
            profile_names: Vec::new(),
            memory_allows_candidate_writes,
        },
        bounds(),
    );
    (sink, driver)
}

/// Every message's content of one captured request, joined — the brief is
/// in there and this is how tests read it.
fn texts(request: &ChatRequest) -> String {
    request
        .messages
        .iter()
        .map(|message| message.content.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

/// A step whose capabilities exclude a tool gets definitions without that
/// tool — the tool is hidden, never advertised-and-refused.
#[tokio::test]
async fn a_tool_outside_the_step_s_capabilities_is_absent_from_definitions() {
    let run = approved_run("narrow").await;
    let provider = ScriptProvider::new(vec![Turn::Answer("done")]);
    let approval = AllowApproval;
    let toolsets = vec![toolset(
        Arc::new(RecordingTools::default()),
        vec![
            def("probe", LocalStateEffect::None),
            def("workspace_writer", LocalStateEffect::WriteWorkspace),
            def("remember", LocalStateEffect::WriteCandidate),
        ],
    )];
    let (sink, driver) = driver_and_sink(&run, &provider, &approval, &toolsets, false);
    let workspace = run.workspace();
    let plan = RunPlan::new(vec![step("read the schema", Capabilities::default())]).unwrap();

    driver.run_step(&sink, &plan, 0, &workspace).await.unwrap();

    // The loop received the definitions, not a refusal: a step without the
    // workspace-write capability never sees the workspace-write tool.
    let received: Vec<String> = provider.requests()[0]
        .tools
        .iter()
        .map(|tool| tool.name.clone())
        .collect();
    assert_eq!(
        received,
        vec!["probe", "remember"],
        "the workspace-write tool must be absent from the step's definitions"
    );
    assert_eq!(sink.state(), RunState::Completed);

    let _ = fs::remove_dir_all(run.root);
}

/// The step's budget becomes the loop's `AgentLimits`, and
/// `SAYA_AGENT_MAX_TURNS` set in the environment changes nothing.
#[tokio::test]
async fn the_step_budget_becomes_the_limits_and_the_environment_changes_nothing() {
    unsafe { std::env::set_var("SAYA_AGENT_MAX_TURNS", "99") };

    // A two-turn ceiling with a model that always wants a tool: two tool
    // turns, then the salvage call with no tools — three provider calls in
    // total, not the 100 an environment-derived ceiling of 99 would allow.
    let run = approved_run("budget-turns").await;
    let provider = ScriptProvider::new(vec![
        Turn::Tools(vec![call("probe")]),
        Turn::Tools(vec![call("probe")]),
        Turn::Answer("salvaged"),
    ]);
    let approval = AllowApproval;
    let toolsets = vec![toolset(
        Arc::new(RecordingTools::default()),
        vec![def("probe", LocalStateEffect::None)],
    )];
    let (sink, driver) = driver_and_sink(&run, &provider, &approval, &toolsets, false);
    let workspace = run.workspace();
    let mut budget = Budgets::default();
    budget.turns = Some(2);
    budget.tool_calls = Some(99);
    let plan = RunPlan::new(vec![step_with_budget("probe twice", Some(budget))]).unwrap();

    driver.run_step(&sink, &plan, 0, &workspace).await.unwrap();

    assert_eq!(
        provider.requests().len(),
        3,
        "the step's turn ceiling must bind: two tool turns plus one salvage call"
    );
    assert!(
        provider.requests()[2].tools.is_empty(),
        "the third call must be the tool-less salvage"
    );
    assert_eq!(sink.state(), RunState::Completed);
    let _ = fs::remove_dir_all(run.root);

    // The tool-call ceiling maps the same way: one executed call, then the
    // second projected call breaches it and the run salvages.
    let run = approved_run("budget-tools").await;
    let provider = ScriptProvider::new(vec![
        Turn::Tools(vec![call("probe")]),
        Turn::Tools(vec![call("probe")]),
        Turn::Answer("salvaged"),
    ]);
    let approval = AllowApproval;
    let toolsets = vec![toolset(
        Arc::new(RecordingTools::default()),
        vec![def("probe", LocalStateEffect::None)],
    )];
    let (sink, driver) = driver_and_sink(&run, &provider, &approval, &toolsets, false);
    let workspace = run.workspace();
    let mut budget = Budgets::default();
    budget.tool_calls = Some(1);
    let plan = RunPlan::new(vec![step_with_budget("probe once", Some(budget))]).unwrap();

    driver.run_step(&sink, &plan, 0, &workspace).await.unwrap();

    assert_eq!(
        provider.requests().len(),
        3,
        "the step's tool-call ceiling must bind regardless of the environment"
    );
    assert_eq!(sink.state(), RunState::Completed);
    unsafe { std::env::remove_var("SAYA_AGENT_MAX_TURNS") };
    let _ = fs::remove_dir_all(run.root);
}

/// Candidate writes are pinned off even when the caller carries the user's
/// memory-mode permission to write candidates (DESIGN §5.8): the tool is
/// advertised (so the pin, not hiding, is what stops it) and the loop
/// refuses it.
#[tokio::test]
async fn candidate_writes_are_pinned_off_even_when_memory_would_permit_them() {
    let run = approved_run("pin").await;
    let provider = ScriptProvider::new(vec![
        Turn::Tools(vec![call("remember")]),
        Turn::Answer("noted"),
    ]);
    let tools = Arc::new(RecordingTools::default());
    let approval = AllowApproval;
    let toolsets = vec![toolset(
        tools.clone(),
        vec![
            def("probe", LocalStateEffect::None),
            def("remember", LocalStateEffect::WriteCandidate),
        ],
    )];
    let (sink, driver) = driver_and_sink(&run, &provider, &approval, &toolsets, true);
    let workspace = run.workspace();
    let plan = RunPlan::new(vec![step("record a claim", Capabilities::default())]).unwrap();

    driver.run_step(&sink, &plan, 0, &workspace).await.unwrap();

    // The tool was advertised, so the refusal came from the pinned limit —
    // not from the definitions hiding it.
    let received: Vec<String> = provider.requests()[0]
        .tools
        .iter()
        .map(|tool| tool.name.clone())
        .collect();
    assert!(
        received.contains(&"remember".to_string()),
        "a candidate-write tool is not capability-scoped; it must be advertised: {received:?}"
    );
    // The loop refused it: the executor never ran.
    assert_eq!(
        tools.calls(),
        Vec::<String>::new(),
        "a candidate-write tool must never execute in an episode"
    );
    // The model was told the call was denied, not left guessing.
    assert!(
        texts(&provider.requests()[1]).contains("denied by approval policy"),
        "the denial must reach the model as the tool result"
    );
    assert_eq!(sink.state(), RunState::Completed);

    let _ = fs::remove_dir_all(run.root);
}

/// A failing step retries a bounded number of times, each with a fresh
/// brief, then records a typed failure and pauses.
#[tokio::test]
async fn a_failing_step_retries_with_a_fresh_brief_then_pauses_with_a_typed_code() {
    let run = approved_run("retry").await;
    let provider = ScriptProvider::new(vec![
        Turn::Tools(vec![call("probe")]),
        Turn::Fail,
        Turn::Tools(vec![call("probe")]),
        Turn::Fail,
        Turn::Tools(vec![call("probe")]),
        Turn::Fail,
    ]);
    let approval = AllowApproval;
    let toolsets = vec![toolset(
        Arc::new(SeedingTools {
            workspace: run.root.join("workspace"),
            calls: Arc::new(Mutex::new(Vec::new())),
        }),
        vec![def("probe", LocalStateEffect::None)],
    )];
    let (sink, driver) = driver_and_sink(&run, &provider, &approval, &toolsets, false);
    let workspace = run.workspace();
    let plan = RunPlan::new(vec![step("keep failing", Capabilities::default())]).unwrap();

    let error = driver
        .run_step(&sink, &plan, 0, &workspace)
        .await
        .unwrap_err();

    assert!(
        matches!(
            &error,
            EpisodeError::StepExhausted {
                step: 0,
                attempts: 3,
                code: RunFailureCode::Provider,
            } | _
        ),
        "the bound spent must surface as a typed failure with its cause: {error:?}"
    );
    assert_eq!(
        sink.state(),
        RunState::Paused,
        "a spent bound pauses the run"
    );

    // Exactly the bound was spent: three attempts, two provider calls each.
    let requests = provider.requests();
    assert_eq!(requests.len(), 6, "three attempts, each of two calls");

    // Each attempt got a fresh brief: the workspace the previous attempt's
    // tool call seeded appears in the next attempt's brief.
    assert!(
        !texts(&requests[0]).contains("seeded-"),
        "the first attempt's brief must not know the files later attempts create"
    );
    assert!(
        texts(&requests[2]).contains("seeded-0.csv"),
        "the second attempt's brief must carry the manifest as of its start"
    );
    assert!(
        texts(&requests[4]).contains("seeded-0.csv")
            && texts(&requests[4]).contains("seeded-1.csv"),
        "the third attempt's brief must carry everything seeded so far"
    );

    // The journal tells the bounded-retry story and pauses with its reason.
    assert_eq!(
        Journal::open(&run.run_dir).read().unwrap(),
        vec![
            RunEvent::StepStarted { step: 0 },
            RunEvent::StepFailed { step: 0 },
            RunEvent::StepStarted { step: 0 },
            RunEvent::StepFailed { step: 0 },
            RunEvent::StepStarted { step: 0 },
            RunEvent::StepFailed { step: 0 },
            RunEvent::Paused {
                reason: PauseReason::StepFailedAfterRetry,
            },
        ],
    );
    // The store's step row ends Failed — the mirror resume will read.
    let steps = RunStore::list_steps(&*run.store, &run.run_id)
        .await
        .unwrap();
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].status, RunStepStatus::Failed);

    let _ = fs::remove_dir_all(run.root);
}

/// The brief carries the workspace manifest: names, sizes, digests.
#[tokio::test]
async fn the_brief_carries_the_workspace_manifest() {
    let run = approved_run("brief").await;
    let workspace = run.workspace();
    fs::create_dir_all(workspace.root().join("notes")).unwrap();
    fs::write(workspace.root().join("notes/a.md"), "hello notes").unwrap();
    fs::write(workspace.root().join("b.csv"), "col\n1\n").unwrap();
    let provider = ScriptProvider::new(vec![Turn::Answer("done")]);
    let approval = AllowApproval;
    let toolsets = vec![toolset(
        Arc::new(RecordingTools::default()),
        vec![def("probe", LocalStateEffect::None)],
    )];
    let (sink, driver) = driver_and_sink(&run, &provider, &approval, &toolsets, false);
    let plan = RunPlan::new(vec![step("read the workspace", Capabilities::default())]).unwrap();

    driver.run_step(&sink, &plan, 0, &workspace).await.unwrap();

    // The expected manifest, computed the same way the driver builds it.
    let entries = manifest::build(&workspace, 8, 64 * 1024).unwrap();
    assert!(!entries.is_empty(), "the test workspace must hold files");
    let brief = texts(&provider.requests()[0]);
    for entry in &entries {
        assert!(
            brief.contains(&entry.path),
            "the brief must name {} — brief: {brief}",
            entry.path
        );
        assert!(
            brief.contains(&entry.digest),
            "the brief must carry {}'s digest — brief: {brief}",
            entry.path
        );
    }

    let _ = fs::remove_dir_all(run.root);
}

/// The scratch scope's end-to-end gate: a step that asked for scratch — and
/// only scratch, no `workspace_write` anywhere in its capabilities — runs
/// DDL through `scratch_sql` in a real episode. The definition rides the
/// toolset's universe, the write permit arrives through the scope→permit
/// union rather than `workspace_write`, and the statement lands in the
/// run's scratch file. If this fails, the union or the write-shaped filter
/// is wrong, not the test.
#[tokio::test]
async fn a_scratch_only_step_runs_ddl_through_scratch_sql() {
    use saya_harness::scratch::ScratchSql;

    let run = approved_run("scratch").await;
    let mut scratch_caps = Capabilities::default();
    scratch_caps.scratch = true;
    let scratch = Arc::new(
        ScratchSql::admit(&run.run_dir, &scratch_caps)
            .unwrap()
            .expect("the run approved scratch, so admission admits"),
    );
    let provider = ScriptProvider::new(vec![
        Turn::Tools(vec![ToolCall {
            id: "c-create".into(),
            name: "scratch_sql".into(),
            arguments: serde_json::json!({"sql": "CREATE TABLE staged (a INTEGER)"}),
        }]),
        Turn::Tools(vec![ToolCall {
            id: "c-insert".into(),
            name: "scratch_sql".into(),
            arguments: serde_json::json!({"sql": "INSERT INTO staged VALUES (42)"}),
        }]),
        Turn::Answer("done"),
    ]);
    let approval = AllowApproval;
    let toolsets = vec![toolset(
        scratch.clone(),
        ScratchSql::definitions(&scratch_caps),
    )];
    let (sink, driver) = driver_and_sink(&run, &provider, &approval, &toolsets, false);
    let workspace = run.workspace();
    let plan = RunPlan::new(vec![step("stage results", scratch_caps.clone())]).unwrap();

    driver.run_step(&sink, &plan, 0, &workspace).await.unwrap();

    assert_eq!(sink.state(), RunState::Completed);

    // The DDL and the DML landed in the run's scratch file: the write was
    // auto-run — no workspace-write scope anywhere, so only the permit
    // union carried it — and the staged row reads back.
    let read_back = scratch
        .run("SELECT a FROM staged")
        .await
        .expect("the table the DDL created must exist in the run's scratch file");
    assert!(
        serde_json::to_string(&read_back).unwrap().contains("42"),
        "the inserted row must be staged in the scratch file, got: {read_back:?}"
    );

    let _ = fs::remove_dir_all(run.root);
}

// --- S2: the fetch step, end to end ------------------------------------------

/// A hermetic in-process transport: every policy-judged URL resolves to a
/// public TEST-NET address and is served the canned body in declared
/// chunks. No test touches the real network.
struct FetchNet {
    chunks: VecDeque<Vec<u8>>,
}

#[async_trait]
impl saya_harness::fetch::FetchTransport for FetchNet {
    async fn resolve(&self, _: &str) -> Result<Vec<std::net::IpAddr>, FetchTransportError> {
        Ok(vec![std::net::IpAddr::V4(std::net::Ipv4Addr::new(
            192, 0, 2, 7,
        ))])
    }
    async fn get(
        &self,
        _: saya_harness::fetch::FetchRequest,
    ) -> Result<saya_harness::fetch::WireResponse, FetchTransportError> {
        Ok(saya_harness::fetch::WireResponse {
            status: 200,
            location: None,
            content_range: None,
            body: Box::new(FetchChunks(self.chunks.clone())),
        })
    }
}

/// The canned body, chunk by chunk.
struct FetchChunks(VecDeque<Vec<u8>>);

#[async_trait]
impl saya_harness::fetch::FetchBody for FetchChunks {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, FetchTransportError> {
        Ok(self.0.pop_front())
    }
}

/// The bytes on disk under a directory tree, summed — sidecar `.json`
/// files excluded (they are the resume contract, not downloaded bytes).
fn walk_dir_bytes(root: &std::path::Path) -> u64 {
    std::fs::read_dir(root)
        .expect("tree")
        .filter_map(|entry| entry.ok())
        .map(|entry| {
            let path = entry.path();
            if path.is_dir() {
                walk_dir_bytes(&path)
            } else if path.to_string_lossy().ends_with(".json") {
                0
            } else {
                entry.metadata().expect("size").len()
            }
        })
        .sum()
}

/// The fetch scope's end-to-end gate (S2 decision 2, test 5): a step whose
/// capabilities asked for fetch — and only fetch — downloads through
/// `http_download` in a real episode; the tiny run budget refuses a chunk,
/// the tool's typed error reaches the model, the trip latch pauses the run
/// with the shared `BudgetExhausted` reason, and the next `run_step`
/// refuses on state. Paused, not overrun — end to end.
#[tokio::test]
async fn a_download_that_trips_the_run_budget_pauses_the_run_typed() {
    use saya_harness::fetch::{
        DownloadBudget, DownloadLimits, FetchLimits, FetchPolicy, FetchTools,
    };
    use saya_types::{Destination, FetchScope};

    let run = approved_run("fetch-budget").await;
    let workspace = Arc::new(run.workspace());
    let mut fetch_caps = Capabilities::default();
    fetch_caps.fetch = Some(
        FetchScope::new(vec![
            Destination::new("https", "files.example.org").unwrap(),
        ])
        .expect("shaped"),
    );
    let budget = DownloadBudget::new(8);
    let tools = FetchTools::new(
        FetchPolicy::new(vec![FetchDestination::new("https", "files.example.org")]),
        Arc::new(FetchNet {
            chunks: VecDeque::from(vec![vec![b'x'; 4], vec![b'x'; 4], vec![b'x'; 40]]),
        }),
        FetchLimits::for_tool_lane(),
        DownloadLimits::default(),
        Arc::clone(&workspace),
        budget.clone(),
    );
    let provider = ScriptProvider::new(vec![
        Turn::Tools(vec![ToolCall {
            id: "c-download".into(),
            name: "http_download".into(),
            arguments: serde_json::json!({
                "url": "https://files.example.org/corpus.bin",
                "destination": "downloads/corpus.bin"
            }),
        }]),
        Turn::Answer("done"),
    ]);
    let approval = AllowApproval;
    let toolsets = vec![toolset(
        Arc::new(tools),
        vec![saya_harness::fetch::http_download_definition()],
    )];
    // The sink arms the same wallet the step's member holds: a download
    // refusal the model has already seen as a typed error also pauses the
    // run here — clone-shares-state.
    let sink = EngineEventSink::new(
        run.run_id.clone(),
        RunState::Approved,
        Journal::open(&run.run_dir),
        run.store.clone(),
        SinkBudgets {
            wall_clock: None,
            token_ceiling: None,
            download_budget: Some(budget.clone()),
            carried_usage: UsageTotals::default(),
        },
        std::time::Instant::now,
    );
    let driver = EpisodeDriver::new(
        EpisodeCollaborators {
            provider: &provider,
            approval: &approval,
            toolsets: &toolsets,
            cancellation: CancellationToken::default(),
        },
        EpisodeRun {
            run_id: run.run_id.clone(),
            store: run.store.clone(),
            journal: Journal::open(&run.run_dir),
        },
        EpisodeRequest {
            model: "mock-model".into(),
            profile_names: Vec::new(),
            memory_allows_candidate_writes: false,
        },
        bounds(),
    );
    let plan = RunPlan::new(vec![
        step("pull the corpus", fetch_caps.clone()),
        step("read only", Capabilities::default()),
    ])
    .unwrap();

    // Step 0 runs: the download claims 8 bytes, the next chunk is refused
    // typed, the episode ends informed, and the tick after the trip pauses
    // the run.
    driver
        .run_step(&sink, &plan, 0, workspace.as_ref())
        .await
        .expect("the episode itself completes; the pause is level-triggered");
    let second_request = &provider.requests()[1];
    let tool_text = texts(second_request);
    assert!(
        tool_text.contains("the run's download budget of 8 bytes tripped after 8"),
        "the model must see the typed budget error, never a short success: {tool_text}"
    );
    assert_eq!(
        sink.state(),
        RunState::Paused,
        "the trip latch pauses the run on the first tick after the trip"
    );
    assert!(budget.tripped(), "the refusal is the recorded event");
    let on_disk = walk_dir_bytes(&run.root.join("workspace"));
    assert!(on_disk <= 8, "paused, not overrun: {on_disk} bytes on disk");

    // The next step refuses on state — a paused run is not runnable.
    let error = driver
        .run_step(&sink, &plan, 1, workspace.as_ref())
        .await
        .expect_err("a paused run cannot begin its next step");
    assert!(
        matches!(
            error,
            EpisodeError::NotRunnable {
                state: RunState::Paused,
                ..
            }
        ),
        "the step boundary stops the run: {error:?}"
    );

    // The durable record names the reason — the shared vocabulary, no new
    // pause reason.
    let events = Journal::open(&run.run_dir).read().unwrap();
    assert!(
        events.iter().any(|event| matches!(
            event,
            RunEvent::Paused {
                reason: PauseReason::BudgetExhausted
            }
        )),
        "the pause must name the budget: {events:?}"
    );

    let _ = fs::remove_dir_all(run.root);
}
