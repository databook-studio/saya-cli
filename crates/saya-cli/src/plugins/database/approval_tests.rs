//! The database lane's table: which database call is run, asked about, or
//! refused, in which mode, and in what order the run's narrowing, the mode, the
//! remembered grants and the operator are consulted.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use saya_agent::{ApprovalPolicy, RunContext, RunSurface, SessionGrants, ToolDefinition};
use serde_json::{Value, json};

use super::approval::{DbApprovalLane, Verdict, line, runs_on_chat};
use crate::agent::tools::DatabaseTools;
use crate::agent::tools::consent::{Answer, Caller, Operator, Prompt};
use crate::agent::tools::shell_policy::ModeCell;

/// Every definition the chat surface can carry, so the lane is exercised over
/// the real effects rather than a test's idea of them.
fn definitions() -> Vec<ToolDefinition> {
    DatabaseTools::definitions(true, true, false, false)
}

pub(super) fn tool(name: &str) -> ToolDefinition {
    definitions()
        .into_iter()
        .find(|definition| definition.name == name)
        .unwrap_or_else(|| panic!("{name} is a database tool"))
}

/// An operator that records every line it is shown and answers with the one
/// choice the test states.
pub(super) struct Recording {
    answer: Answer,
    subjects: Mutex<Vec<String>>,
}

impl Recording {
    pub(super) fn answering(answer: Answer) -> Arc<Self> {
        Arc::new(Self {
            answer,
            subjects: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn calls(&self) -> usize {
        self.subjects.lock().unwrap().len()
    }
}

#[async_trait]
impl Operator for Recording {
    async fn ask(&self, subject: &str, _arguments: &Value) -> bool {
        self.subjects.lock().unwrap().push(subject.to_owned());
        self.answer != Answer::Deny
    }

    async fn answer(&self, subject: &str, _arguments: &Value) -> Answer {
        self.subjects.lock().unwrap().push(subject.to_owned());
        self.answer
    }
}

/// The lane one run holds, over the operator the test states.
fn for_run(mode: ApprovalPolicy, operator: Arc<Recording>) -> DbApprovalLane {
    with_grants_and_tools(
        mode,
        operator,
        saya_config::ResolvedTools::default(),
        SessionGrants::default(),
    )
}

pub(super) fn with_grants_and_tools(
    mode: ApprovalPolicy,
    operator: Arc<Recording>,
    tools: saya_config::ResolvedTools,
    grants: SessionGrants,
) -> DbApprovalLane {
    let prompt = Prompt::new(operator, line(&Caller::Session));
    DbApprovalLane::new(ModeCell::new(mode), grants, None, tools, Some(prompt))
}

/// One query on `connection`.
pub(super) fn query(connection: &str) -> Value {
    json!({ "connection": connection, "sql": "SELECT 1" })
}

/// Under `ask` a query is put to the operator once, with the connection's own
/// grant token; a session answer then runs that connection unasked while
/// another connection asks again.
#[tokio::test]
async fn a_session_answer_pre_answers_its_connection_and_no_other() {
    let operator = Recording::answering(Answer::Session);
    let lane = for_run(ApprovalPolicy::Ask, operator.clone());
    let query_tool = tool("bounded_sql_query");

    assert!(
        lane.approve(&query_tool, &query("demo")).await,
        "an approving operator admits the query"
    );
    assert_eq!(operator.calls(), 1, "the query is put to the operator once");
    assert_eq!(
        lane.decide(&query_tool, &query("demo")),
        Verdict::Allow,
        "the session answer recorded `sql:demo`"
    );

    assert!(
        lane.approve(&query_tool, &query("demo")).await,
        "the granted connection runs unasked"
    );
    assert_eq!(
        operator.calls(),
        1,
        "the second query on the granted connection asks nobody"
    );
    assert!(
        lane.approve(&query_tool, &query("other")).await,
        "another connection still runs on the operator's answer"
    );
    assert_eq!(
        operator.calls(),
        2,
        "a query on another connection asks again"
    );
}

/// The fan-out takes no token — its referent can grow after an approval — so
/// even a session answer leaves nothing behind and every call asks.
#[tokio::test]
async fn the_fan_out_asks_every_call() {
    let operator = Recording::answering(Answer::Session);
    let lane = for_run(ApprovalPolicy::Ask, operator.clone());
    let fan_out = tool("bounded_sql_query_all");

    for expected in 1..=2 {
        assert!(
            lane.approve(&fan_out, &json!({ "sql": "SELECT 1" })).await,
            "the operator's answer admits the call"
        );
        assert_eq!(operator.calls(), expected, "every fan-out call is asked");
    }
    assert_eq!(
        lane.decide(&fan_out, &json!({ "sql": "SELECT 1" })),
        Verdict::Ask { token: None },
        "nothing the answer could record would pre-answer it"
    );
}

/// `accept-edits` still asks about SQL: its one pre-answer is the workspace file
/// family. `bypass` and `read-only` run the read-shaped call unasked, and
/// `never` refuses it.
#[tokio::test]
async fn accept_edits_still_asks_and_the_other_modes_do_not() {
    let operator = Recording::answering(Answer::Once);
    let lane = for_run(ApprovalPolicy::AcceptEdits, operator.clone());
    let query_tool = tool("bounded_sql_query");
    assert!(
        lane.would_ask(&query_tool, &query("demo")),
        "accept-edits pre-answers no SQL call"
    );
    assert!(lane.approve(&query_tool, &query("demo")).await);
    assert_eq!(operator.calls(), 1, "accept-edits asks about SQL");

    for mode in [ApprovalPolicy::Bypass, ApprovalPolicy::ReadOnly] {
        let operator = Recording::answering(Answer::Once);
        let lane = for_run(mode, operator.clone());
        assert!(
            lane.approve(&query_tool, &query("demo")).await,
            "{mode:?} runs a read-shaped query"
        );
        assert_eq!(operator.calls(), 0, "{mode:?} asks nobody");
    }

    let operator = Recording::answering(Answer::Once);
    let lane = for_run(ApprovalPolicy::Never, operator.clone());
    assert!(
        !lane.approve(&query_tool, &query("demo")).await,
        "never refuses a database call"
    );
    assert_eq!(operator.calls(), 0, "never asks nobody");
}

/// The lane is the chat surface's alone: every job surface keeps the decider it
/// composes, whatever database profiles it holds. The ordering — narrowing,
/// mode, then grants — is pinned in `approval_order_tests`.
#[test]
fn only_a_chat_run_takes_the_lane() {
    let chat = RunContext {
        surface: RunSurface::Chat,
        database_profiles: 1,
    };
    assert!(runs_on_chat(&chat));
    assert!(
        !runs_on_chat(&RunContext {
            surface: RunSurface::Chat,
            database_profiles: 0,
        }),
        "a chat with no profile has no database to decide"
    );
    for surface in [
        RunSurface::Headless,
        RunSurface::Background,
        RunSurface::Helper,
    ] {
        assert!(
            !runs_on_chat(&RunContext {
                surface,
                database_profiles: 3,
            }),
            "{surface:?} keeps today's decider"
        );
    }
}

/// No operator is a refusal: an ask nobody can answer must never read as
/// consent.
#[tokio::test]
async fn an_ask_with_nobody_to_answer_is_refused() {
    let lane = DbApprovalLane::new(
        ModeCell::new(ApprovalPolicy::Ask),
        SessionGrants::default(),
        None,
        saya_config::ResolvedTools::default(),
        None,
    );
    assert!(
        !lane
            .approve(&tool("bounded_sql_query"), &query("demo"))
            .await
    );
}
