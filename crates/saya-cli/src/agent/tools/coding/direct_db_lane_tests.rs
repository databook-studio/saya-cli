//! The root's database lane: a chat-surface run decides its database calls in
//! the plugin's table, and a job surface keeps the decider it always had.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use saya_agent::{
    ApprovalDecider, ApprovalPolicy, RunContext, RunSurface, SessionGrants, ToolDefinition,
};
use serde_json::{Value, json};

use super::direct_tests::working_checkout_root;
use crate::agent::tools::DatabaseTools;
use crate::agent::tools::child::tests_root_compose_fixture as fixture;
use crate::agent::tools::consent::{Answer, Caller, Operator, Prompt};
use crate::agent::tools::shell_policy::ModeCell;
use crate::plugins::database::approval::{DbApprovalLane, Verdict, line};

/// An operator that answers "for this run", keeps the run's store, and records
/// every line it was shown.
pub(super) struct Recording {
    subjects: Mutex<Vec<String>>,
    grants: SessionGrants,
}

impl Recording {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            subjects: Mutex::new(Vec::new()),
            grants: SessionGrants::default(),
        })
    }

    pub(super) fn calls(&self) -> usize {
        self.subjects.lock().unwrap().len()
    }

    pub(super) fn subjects(&self) -> Vec<String> {
        self.subjects.lock().unwrap().clone()
    }
}

#[async_trait]
impl Operator for Recording {
    async fn ask(&self, subject: &str, _arguments: &Value) -> bool {
        self.subjects.lock().unwrap().push(subject.to_owned());
        true
    }

    async fn answer(&self, subject: &str, _arguments: &Value) -> Answer {
        self.subjects.lock().unwrap().push(subject.to_owned());
        // The widest answer, so a recorded token is what the test then reads.
        Answer::Session
    }

    fn grants(&self) -> Option<SessionGrants> {
        Some(self.grants.clone())
    }
}

pub(super) fn db_tool(name: &str) -> ToolDefinition {
    DatabaseTools::definitions(true, true, false, false)
        .into_iter()
        .find(|definition| definition.name == name)
        .unwrap_or_else(|| panic!("{name} is a database tool"))
}

pub(super) fn query(connection: &str) -> Value {
    json!({ "connection": connection, "sql": "SELECT 1" })
}

/// A chat-surface run's root asks once per connection: the session answer
/// records `sql:demo`, the second query on it runs unasked through the root,
/// and a query on another connection asks again.
#[tokio::test]
async fn a_chat_surface_root_asks_once_per_connection() {
    let operator = Recording::new();
    let fixture = fixture::Fixture::asking(
        "direct-db-lane",
        &["tracked.txt"],
        operator.clone(),
        ApprovalPolicy::Ask,
    )
    .await;
    let mut root = working_checkout_root(&fixture).await;
    root.attach_db_lane(Arc::new(DbApprovalLane::for_root(
        &fixture.delegate_job,
        Some("demo".to_owned()),
    )));
    let tool = db_tool("bounded_sql_query");

    assert!(
        root.approve(&tool, &query("demo")).await,
        "the operator's session answer admits the query"
    );
    assert_eq!(operator.calls(), 1, "the query is put to the operator once");
    assert_eq!(
        operator.subjects(),
        vec![line(&Caller::Root {
            job_id: fixture.delegate_job.job_id().clone(),
        })],
        "the prompt names the run that asks, in the lane's own words"
    );
    assert!(
        operator.grants.is_granted("sql:demo"),
        "the session answer reaches the run's grant store"
    );

    assert!(root.approve(&tool, &query("demo")).await);
    assert_eq!(
        operator.calls(),
        1,
        "the granted connection runs unasked through the root"
    );
    assert!(root.approve(&tool, &query("other")).await);
    assert_eq!(operator.calls(), 2, "another connection asks again");
}

/// The composition's one entry point: a chat run with a database takes the
/// lane, and every job surface composes none — the decider it has is the one it
/// keeps. This is the arm `parent_setup` calls, so applying the lane where the
/// task says it does not belong fails here.
#[tokio::test]
async fn only_a_chat_run_composes_the_lane() {
    let fixture =
        fixture::Fixture::stating("direct-db-compose", &["tracked.txt"], ApprovalPolicy::Ask).await;
    let chat = RunContext {
        surface: RunSurface::Chat,
        database_profiles: 1,
    };
    assert!(
        DbApprovalLane::for_run(&fixture.delegate_job, &chat, Some("demo".to_owned())).is_some(),
        "a chat run with a profile takes the lane"
    );
    let profileless_chat = RunContext {
        surface: RunSurface::Chat,
        database_profiles: 0,
    };
    assert!(
        DbApprovalLane::for_run(
            &fixture.delegate_job,
            &profileless_chat,
            Some("demo".to_owned())
        )
        .is_none(),
        "a chat with no database has nothing to decide"
    );
    for surface in [
        RunSurface::Headless,
        RunSurface::Background,
        RunSurface::Helper,
    ] {
        let context = RunContext {
            surface,
            database_profiles: 2,
        };
        assert!(
            DbApprovalLane::for_run(&fixture.delegate_job, &context, Some("demo".to_owned()))
                .is_none(),
            "{surface:?} keeps today's decider"
        );
    }
}

/// The lane reads the run's mode cell per call, so a mode stated after the lane
/// was composed decides the next call: the cell is shared, never copied.
#[test]
fn the_lane_reads_the_runs_mode_cell_per_call() {
    let mode = ModeCell::new(ApprovalPolicy::Ask);
    let operator = Recording::new();
    let lane = DbApprovalLane::new(
        mode.clone(),
        SessionGrants::default(),
        None,
        saya_config::ResolvedTools::default(),
        Some(Prompt::new(operator, line(&Caller::Session))),
    );
    let tool = db_tool("bounded_sql_query");
    assert!(lane.would_ask(&tool, &query("demo")));
    mode.set(ApprovalPolicy::Bypass).unwrap();
    assert!(!lane.would_ask(&tool, &query("demo")));
    assert_eq!(lane.decide(&tool, &query("demo")), Verdict::Allow);
}
