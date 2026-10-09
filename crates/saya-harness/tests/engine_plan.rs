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
    PlanParseFailure, PlanRejection, PlanRequest, RunState, SinkBudgets, StepToolset, UsageTotals,
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

#[path = "engine_plan/capability_refusal.rs"]
mod capability_refusal;

#[path = "engine_plan/budget_limits.rs"]
mod budget_limits;

#[path = "engine_plan/bound_plan.rs"]
mod bound_plan;
#[path = "engine_plan/planner_refusals.rs"]
mod planner_refusals;
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
        concurrency: saya_agent::ToolConcurrency::Serial,
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
