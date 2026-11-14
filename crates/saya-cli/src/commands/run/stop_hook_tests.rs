//! `Stop` on the `saya run` surface: each step's agent loop ends inside the
//! engine, so the same hook runner the root uses must fire when it does.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use saya_agent::{
    AllowReadOnlyApproval, CancellationToken, ChatMessage, ChatProvider, ChatRequest, ChatResponse,
    ProviderError, ToolError, ToolExecutor,
};
use saya_harness::engine::{
    EngineEventSink, EpisodeCollaborators, EpisodeDriver, EpisodeRequest, EpisodeRun,
    ManifestBounds, RunState, SinkBudgets, StepToolset, UsageTotals,
};
use saya_harness::journal::Journal;
use saya_harness::workspace::Workspace;
use saya_store::{NewRun, RunBudgets, RunCapabilityFlags, RunStatus, RunStore, SqliteStateStore};
use saya_types::{Capabilities, RunId, RunPlan, StepSpec};
use serde_json::Value;

use crate::agent::hooks::test_support::stop_hooks;
use crate::agent::hooks::{HookCaller, HookGate};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

fn temp_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "saya-run-stop-{label}-{}-{}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("run")).expect("the run dir must create");
    std::fs::create_dir_all(root.join("workspace")).expect("the workspace must create");
    root
}

struct AnsweringProvider;

#[async_trait]
impl ChatProvider for AnsweringProvider {
    fn name(&self) -> &str {
        "stop-script"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        Ok(ChatResponse::new(ChatMessage::text("assistant", "done")))
    }
}

struct NoTools;

#[async_trait]
impl ToolExecutor for NoTools {
    async fn execute(&self, _: &str, _: Value) -> Result<Value, ToolError> {
        Ok(serde_json::json!({}))
    }
}

/// A step's loop end runs the run's `Stop` hook exactly once, with the payload
/// shape the root's `Stop` gets: the event and the run's id.
#[tokio::test]
async fn a_stop_hook_runs_once_when_a_run_steps_loop_ends() {
    let root = temp_root("step");
    let run_dir = root.join("run");
    let workspace = Workspace::open(&root.join("workspace")).expect("the workspace must open");
    let store = Arc::new(SqliteStateStore::new(root.join("state.sqlite3")));
    let run_id = RunId::parse("run-stop").expect("the run id is valid");
    store
        .create_run(NewRun {
            id: run_id.clone(),
            capabilities: RunCapabilityFlags::default(),
            budgets: RunBudgets::default(),
        })
        .await
        .expect("the run row must create");
    store
        .set_run_status(&run_id, RunStatus::Approved, None)
        .await
        .expect("the run must stand approved");

    let seen = root.join("stop.json");
    let hooks = stop_hooks(
        &format!("cat >> '{}'", seen.display()),
        workspace.root(),
        HookCaller::Run("run-stop".to_owned()),
    );
    let toolset = StepToolset {
        executor: Arc::new(HookGate::new(hooks, Arc::new(NoTools))),
        definitions: Vec::new(),
    };
    let sink = EngineEventSink::new(
        run_id.clone(),
        RunState::Approved,
        Journal::open(&run_dir),
        store.clone(),
        SinkBudgets {
            wall_clock: None,
            token_ceiling: None,
            download_budget: None,
            carried_usage: UsageTotals::default(),
        },
        std::time::Instant::now,
    );
    let provider = AnsweringProvider;
    let approval = AllowReadOnlyApproval;
    let driver = EpisodeDriver::new(
        EpisodeCollaborators {
            provider: &provider,
            approval: &approval,
            toolsets: std::slice::from_ref(&toolset),
            cancellation: CancellationToken::new(),
        },
        EpisodeRun {
            run_id: run_id.clone(),
            store: store.clone(),
            journal: Journal::open(&run_dir),
        },
        EpisodeRequest {
            model: "test-model".to_owned(),
            profile_names: Vec::new(),
            memory_allows_candidate_writes: false,
        },
        ManifestBounds {
            max_files: 8,
            max_file_bytes: 64 * 1024,
        },
    );
    let plan = RunPlan::new(vec![
        StepSpec::new(
            "finish one step",
            Capabilities::default(),
            None,
            Vec::new(),
            None,
        )
        .expect("the step is valid"),
    ])
    .expect("the plan is valid");

    driver
        .run_step(&sink, &plan, 0, &workspace)
        .await
        .expect("the step's loop must finish");

    let recorded = std::fs::read_to_string(&seen).expect("the stop hook must have run");
    assert_eq!(
        recorded.matches("\"hook_event_name\":\"Stop\"").count(),
        1,
        "Stop must run exactly once when the step's loop ends: {recorded}"
    );
    assert!(
        recorded.contains("\"run_id\":\"run-stop\""),
        "the payload must name the run: {recorded}"
    );
    let _ = std::fs::remove_dir_all(&root);
}
