//! The plan proposal and validation driver — contract tests (M1-5b, plan
//! slice).
//!
//! Six guarantees:
//! 1. A plan requesting an unapproved capability is rejected, re-prompted
//!    with what was wrong, and refused with a typed code after the third
//!    attempt — the attempt count is asserted, not just the final error.
//! 2. A step budget exceeding the run budget is rejected on every dimension
//!    in turn — a step cannot widen the run anywhere.
//! 3. A valid plan binds as the model proposed it, and the episode driving
//!    step *n* sees only step *n*'s capabilities.
//! 4. A model returning prose, truncated JSON, or the wrong shape is an
//!    ordinary typed failure, never a panic.
//! 5. A step naming an endpoint role the run does not bind is refused.
//! 6. A plan proposing a new capability mid-run yields the needs-approval
//!    outcome rather than running.

use std::{
    collections::VecDeque,
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use saya_agent::{
    ApprovalDecider, CancellationToken, ChatMessage, ChatProvider, ChatRequest, ChatResponse,
    LocalStateEffect, ProviderError, ReasoningEffort, ResponseFormat, ToolDefinition, ToolEffect,
    ToolExecutor,
};
use saya_harness::engine::{
    EngineEventSink, EpisodeCollaborators, EpisodeDriver, ManifestBounds, PlanDriver, PlanError,
    PlanParseFailure, PlanRejection, PlanRequest, RunState,
};
use saya_harness::journal::Journal;
use saya_harness::workspace::Workspace;
use saya_store::{NewRun, RunBudgets, RunCapabilityFlags, RunStatus, RunStore, SqliteStateStore};
use saya_types::{Budgets, Capabilities, EndpointBindings, RunId};

/// A per-test scratch root: the state database and the run directory both
/// live under it, and one cleanup covers both.
fn temp_root(label: &str) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("saya-engine-plan-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

/// A run already standing at `approved` — the state a run is in when a plan
/// binds and its first episode begins.
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

/// A scripted planner provider: serves its answers in order and records
/// every request it received, so tests assert on the attempt count and on
/// what the engine actually sent.
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

    fn requests(&self) -> Vec<ChatRequest> {
        self.requests.lock().unwrap().clone()
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

/// One step of a proposed plan: an optional budget (`null` inherits the
/// run's) and an optional endpoint role (`null` uses the default role).
fn step_json(
    goal: &str,
    capabilities: &str,
    budget: Option<&str>,
    endpoint: Option<&str>,
) -> String {
    let endpoint = endpoint.map_or_else(|| "null".to_string(), |role| format!(r#""{role}""#));
    format!(
        r#"{{"goal": "{goal}", "capabilities": {capabilities}, "budget": {}, "expects": [], "endpoint": {endpoint}}}"#,
        budget.unwrap_or("null")
    )
}

fn planner_request(run_goal: &str) -> PlanRequest {
    PlanRequest {
        model: "mock-model".into(),
        run_goal: run_goal.into(),
    }
}

/// A plan asking for the workspace-write capability the run was never
/// approved for is rejected, re-prompted with what was wrong, and refused
/// with a typed code after the third attempt.
#[tokio::test]
async fn an_unapproved_capability_is_reprompted_then_refused_with_a_typed_code() {
    let plan_text = plan_json(&step_json(
        "write the report",
        r#"{"workspace_write": true}"#,
        None,
        None,
    ));
    let planner = ScriptedPlanner::new(vec![plan_text; 3]);
    let driver = PlanDriver::new(&planner, planner_request("produce the report"));

    let error = driver
        .propose(&Capabilities::default(), &Budgets::default())
        .await
        .unwrap_err();

    assert!(
        matches!(
            &error,
            PlanError::Exhausted {
                attempts: 3,
                last: PlanRejection::NeedsApproval { step: 0 },
            }
        ),
        "the third refusal must be the typed needs-approval outcome: {error:?}"
    );

    // Exactly the bound was spent: three proposals, none more — the stub
    // panics on a fourth, so this also pins the cap.
    let requests = planner.requests();
    assert_eq!(requests.len(), 3, "three attempts, then the refusal");

    // The first ask carries no refusal; every re-prompt names what was wrong.
    assert!(
        !requests[0].messages[1].content.contains("REFUSAL"),
        "the first proposal must not carry a refusal"
    );
    for request in &requests[1..] {
        let user = &request.messages[1].content;
        assert!(
            user.contains("### REFUSAL OF THE PREVIOUS PLAN"),
            "a re-prompt must carry the previous refusal: {user}"
        );
        assert!(
            user.contains("outside the run's approved scopes"),
            "a re-prompt must name what was wrong: {user}"
        );
    }
}

/// One budget dimension, widened in turn.
#[derive(Debug, Clone, Copy)]
enum Dimension {
    WallClock,
    DownloadedBytes,
    WorkspaceBytes,
    WorkspaceFiles,
    ProcessCount,
    ProcessTime,
    Turns,
    ToolCalls,
    TokensPerEndpoint,
}

const DIMENSIONS: &[Dimension] = &[
    Dimension::WallClock,
    Dimension::DownloadedBytes,
    Dimension::WorkspaceBytes,
    Dimension::WorkspaceFiles,
    Dimension::ProcessCount,
    Dimension::ProcessTime,
    Dimension::Turns,
    Dimension::ToolCalls,
    Dimension::TokensPerEndpoint,
];

/// A budgets object carrying exactly one declared ceiling.
fn ceilings(dimension: Dimension, value: u64) -> Budgets {
    let mut ceilings = Budgets::default();
    match dimension {
        Dimension::WallClock => ceilings.wall_clock = Some(Duration::from_secs(value)),
        Dimension::DownloadedBytes => ceilings.downloaded_bytes = Some(value),
        Dimension::WorkspaceBytes => ceilings.workspace_bytes = Some(value),
        Dimension::WorkspaceFiles => ceilings.workspace_files = Some(value),
        Dimension::ProcessCount => ceilings.process_count = Some(value),
        Dimension::ProcessTime => ceilings.process_time = Some(Duration::from_secs(value)),
        Dimension::Turns => ceilings.turns = Some(value),
        Dimension::ToolCalls => ceilings.tool_calls = Some(value),
        Dimension::TokensPerEndpoint => {
            ceilings.tokens_per_endpoint.insert("primary".into(), value);
        }
    }
    ceilings
}

/// A step budget exceeding the run budget is rejected on each dimension in
/// turn — the run budget is the remaining budget at binding time, so a step
/// that widens any one of them cannot bind.
#[tokio::test]
async fn a_step_budget_exceeding_the_run_budget_is_rejected_on_every_dimension() {
    for dimension in DIMENSIONS {
        let run_ceilings = ceilings(*dimension, 10);
        let step_ceilings = ceilings(*dimension, 11);
        let step = format!(
            r#"{{"goal": "spend past the run", "capabilities": {{}}, "budget": {}, "expects": [], "endpoint": null}}"#,
            serde_json::to_string(&step_ceilings).unwrap()
        );
        let planner = ScriptedPlanner::new(vec![plan_json(&step); 3]);
        let driver = PlanDriver::new(&planner, planner_request("g"));

        let error = driver
            .propose(&Capabilities::default(), &run_ceilings)
            .await
            .unwrap_err();

        assert!(
            matches!(
                &error,
                PlanError::Exhausted {
                    attempts: 3,
                    last: PlanRejection::BudgetTooWide { step: 0 },
                }
            ),
            "dimension {dimension:?} must refuse a widening step budget: {error:?}"
        );
    }
}

/// A valid plan binds as the model proposed it, and the episode driving step
/// *n* sees only step *n*'s capabilities.
#[tokio::test]
async fn a_valid_plan_binds_and_step_n_sees_only_step_n_s_capabilities() {
    // Step 0 may not write the workspace, step 1 may; the run's approval
    // grants workspace-write, and each step narrows it.
    let plan_text = plan_json(&format!(
        "{},{}",
        step_json("read the manifest", "{}", None, None),
        step_json(
            "write the report",
            r#"{"workspace_write": true}"#,
            None,
            None
        )
    ));
    let planner = ScriptedPlanner::new(vec![plan_text]);
    let plan_driver = PlanDriver::new(&planner, planner_request("produce the report"));
    let mut approval = Capabilities::default();
    approval.workspace_write = true;

    let plan = plan_driver
        .propose(&approval, &Budgets::default())
        .await
        .unwrap();

    assert_eq!(plan.steps.len(), 2);
    assert_eq!(plan.steps[0].goal, "read the manifest");
    assert!(!plan.steps[0].capabilities.workspace_write);
    assert!(plan.steps[1].capabilities.workspace_write);

    // Driving the bound plan through the merged episode driver: each step's
    // episode is narrowed to that step's capabilities.
    let run = approved_run("bind").await;
    let episode = ScriptedEpisode::new(vec!["observed", "written"]);
    let tools = NoTools;
    let allow = AllowApproval;
    let sink = EngineEventSink::new(
        run.run_id.clone(),
        RunState::Approved,
        Journal::open(&run.run_dir),
        run.store.clone(),
        None,
        std::time::Instant::now,
    );
    let driver = EpisodeDriver::new(
        EpisodeCollaborators {
            provider: &episode,
            tools: &tools,
            approval: &allow,
            universe: vec![
                def("probe", LocalStateEffect::None),
                def("workspace_writer", LocalStateEffect::WriteWorkspace),
            ],
            cancellation: CancellationToken::default(),
        },
        saya_harness::engine::EpisodeRun {
            run_id: run.run_id.clone(),
            store: run.store.clone(),
            journal: Journal::open(&run.run_dir),
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

    let definitions: Vec<Vec<String>> = episode
        .requests()
        .iter()
        .map(|request| {
            request
                .tools
                .iter()
                .map(|tool| tool.name.clone())
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(
        definitions,
        vec![
            vec!["probe".to_string()],
            vec!["probe".to_string(), "workspace_writer".to_string()],
        ],
        "step n must see only step n's capabilities"
    );
    assert_eq!(sink.state(), RunState::Completed);

    let _ = fs::remove_dir_all(run.root);
}

/// A model returning prose, truncated JSON, or the wrong shape is an
/// ordinary typed failure — never a panic.
#[tokio::test]
async fn prose_truncated_and_wrong_shaped_answers_are_typed_failures() {
    let cases = [
        ("I will draft the plan shortly.", PlanParseFailure::Prose),
        (r#"{"steps": [{"goal": "read"#, PlanParseFailure::Truncated),
        (
            r#"{"steps": "two, trust me"}"#,
            PlanParseFailure::WrongShape,
        ),
    ];
    for (answer, expected) in cases {
        let planner = ScriptedPlanner::new(vec![answer.to_string(); 3]);
        let driver = PlanDriver::new(&planner, planner_request("g"));

        let error = driver
            .propose(&Capabilities::default(), &Budgets::default())
            .await
            .unwrap_err();

        assert!(
            matches!(
                &error,
                PlanError::Exhausted {
                    attempts: 3,
                    last: PlanRejection::Malformed { kind },
                } if *kind == expected
            ),
            "`{answer}` must be an ordinary typed failure: {error:?}"
        );

        // The plan call asks for JSON mode; the effort lever stays with the
        // endpoint — a provider that cannot honour an effort variant drops
        // it rather than erroring, so the call must not depend on it.
        let sent = &planner.requests()[0];
        assert_eq!(sent.response_format, ResponseFormat::JsonObject);
        assert_eq!(sent.reasoning_effort, ReasoningEffort::Default);
    }
}

/// A step naming an endpoint role the run has no binding for is refused.
#[tokio::test]
async fn an_endpoint_role_the_run_does_not_bind_is_refused() {
    let mut approval = Capabilities::default();
    approval.endpoints = EndpointBindings::new([("orchestrator", "primary")]).unwrap();
    let plan_text = plan_json(&step_json(
        "ask the shadow endpoint",
        r#"{"endpoints": {"orchestrator": "primary"}}"#,
        None,
        Some("shadow"),
    ));
    let planner = ScriptedPlanner::new(vec![plan_text; 3]);
    let driver = PlanDriver::new(&planner, planner_request("g"));

    let error = driver
        .propose(&approval, &Budgets::default())
        .await
        .unwrap_err();

    assert!(
        matches!(
            &error,
            PlanError::Exhausted {
                attempts: 3,
                last: PlanRejection::EndpointUnbound { step: 0, role },
            } if role == "shadow"
        ),
        "the unbound role must be refused by name: {error:?}"
    );
}

/// A plan proposing a new capability mid-run — at a re-plan boundary, against
/// the budget that is left — yields the needs-approval outcome rather than
/// running.
#[tokio::test]
async fn a_new_capability_mid_run_yields_the_needs_approval_outcome_rather_than_running() {
    let plan_text = plan_json(&format!(
        "{},{}",
        step_json("read the manifest", "{}", None, None),
        step_json(
            "seed the scratch database",
            r#"{"scratch": true}"#,
            None,
            None
        )
    ));
    let planner = ScriptedPlanner::new(vec![plan_text; 3]);
    let driver = PlanDriver::new(&planner, planner_request("seed and report"));
    let mut remaining = Budgets::default();
    remaining.turns = Some(3);

    let error = driver
        .propose(&Capabilities::default(), &remaining)
        .await
        .unwrap_err();

    assert!(
        matches!(
            &error,
            PlanError::Exhausted {
                attempts: 3,
                last: PlanRejection::NeedsApproval { step: 1 },
            }
        ),
        "the new capability must be the needs-approval outcome, pointed at its step: {error:?}"
    );
    // The refusal is the outcome: no plan bound, so nothing ran.
    assert_eq!(planner.requests().len(), 3);
}

// ---------------------------------------------------------------------------
// The episode rig for the binding test — the same stubs engine_episode.rs
// uses, trimmed to what driving two steps needs.
// ---------------------------------------------------------------------------

/// One scripted episode-provider turn: a terminal prose answer.
struct ScriptedEpisode {
    script: Mutex<VecDeque<&'static str>>,
    requests: Mutex<Vec<ChatRequest>>,
}

impl ScriptedEpisode {
    fn new(script: Vec<&'static str>) -> Self {
        Self {
            script: Mutex::new(script.into()),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<ChatRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl ChatProvider for ScriptedEpisode {
    fn name(&self) -> &str {
        "scripted-episode"
    }

    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        self.requests.lock().unwrap().push(request);
        match self.script.lock().unwrap().pop_front() {
            Some(text) => Ok(ChatResponse::new(ChatMessage::text("assistant", text))),
            None => panic!("episode script exhausted"),
        }
    }
}

/// A tool executor that is never reached: the scripted episodes answer in
/// prose, so no tool executes.
struct NoTools;

#[async_trait]
impl ToolExecutor for NoTools {
    async fn execute(
        &self,
        _: &str,
        _: serde_json::Value,
    ) -> Result<serde_json::Value, saya_agent::ToolError> {
        Ok(serde_json::json!({}))
    }
}

/// One tool definition with the given local-state effect; auto-runnable so
/// the capability narrowing is the only thing that can hide it.
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

/// Auto-approves everything: the stub tools declare no approval requirement.
struct AllowApproval;

#[async_trait]
impl ApprovalDecider for AllowApproval {
    async fn approve(&self, _: &ToolDefinition, _: &serde_json::Value) -> bool {
        true
    }
}
