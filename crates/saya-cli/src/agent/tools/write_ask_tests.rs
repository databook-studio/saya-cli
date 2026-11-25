//! The write ask's `ask` arm — the one mode that puts a write-shaped call to
//! the job's operator, and what each kind of answer leaves behind. The fixtures
//! here are shared with `write_ask_mode_tests`, which covers the modes that
//! answer without asking.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use saya_agent::{
    ApprovalDecider, ApprovalPolicy, LocalStateEffect, SessionGrants, ToolConcurrency,
    ToolDefinition, ToolEffect,
};
use saya_types::{AgentId, JobId};
use serde_json::{Value, json};

use super::consent::{Answer, Caller, Operator};
use super::shell_policy::ModeCell;
use super::write_ask::{WriteAsk, line};

/// A capability that admits exactly the calls its flag names, and counts the
/// ones it was asked about.
pub(super) struct Capability {
    admit: bool,
    seen: Mutex<usize>,
}

impl Capability {
    pub(super) fn new(admit: bool) -> Self {
        Self {
            admit,
            seen: Mutex::new(0),
        }
    }

    pub(super) fn seen(&self) -> usize {
        *self.seen.lock().unwrap()
    }
}

#[async_trait]
impl ApprovalDecider for Capability {
    async fn approve(&self, _tool: &ToolDefinition, _arguments: &Value) -> bool {
        *self.seen.lock().unwrap() += 1;
        self.admit
    }
}

/// An operator that answers with the choice the test states, records the
/// subject lines it is shown, and keeps the run's grants the way the job's own
/// operator does.
pub(super) struct Recording {
    answer: Answer,
    grants: SessionGrants,
    subjects: Mutex<Vec<String>>,
}

impl Recording {
    pub(super) fn new(answer: Answer) -> Arc<Self> {
        Self::granting(answer, &[])
    }

    /// An operator whose store already holds `tokens`, so a call they
    /// pre-answer never reaches the question.
    pub(super) fn granting(answer: Answer, tokens: &[&str]) -> Arc<Self> {
        let grants = SessionGrants::default();
        for token in tokens {
            grants.grant(token);
        }
        Arc::new(Self {
            answer,
            grants,
            subjects: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn calls(&self) -> usize {
        self.subjects.lock().unwrap().len()
    }

    pub(super) fn is_granted(&self, token: &str) -> bool {
        self.grants.is_granted(token)
    }

    pub(super) fn subjects(&self) -> Vec<String> {
        self.subjects.lock().unwrap().clone()
    }
}

#[async_trait]
impl Operator for Recording {
    fn grants(&self) -> Option<SessionGrants> {
        Some(self.grants.clone())
    }

    async fn ask(&self, subject: &str, _arguments: &Value) -> bool {
        self.subjects.lock().unwrap().push(subject.to_owned());
        !matches!(self.answer, Answer::Deny)
    }

    async fn answer(&self, subject: &str, _arguments: &Value) -> Answer {
        self.subjects.lock().unwrap().push(subject.to_owned());
        self.answer
    }
}

pub(super) fn caller() -> Caller {
    Caller::Helper {
        agent_id: AgentId::parse("helper-1").unwrap(),
        job_id: JobId::parse("job-1").unwrap(),
    }
}

/// One caller's ask over a fresh mode cell.
pub(super) fn ask(mode: ApprovalPolicy, operator: Arc<Recording>) -> WriteAsk {
    WriteAsk::new(ModeCell::new(mode), operator, &caller())
}

pub(super) fn definition(name: &str) -> ToolDefinition {
    ToolDefinition {
        name: name.to_owned(),
        description: "a tool the write ask resolves".into(),
        read_only: false,
        parameters: json!({}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::WriteWorkspace,
        },
        concurrency: ToolConcurrency::Serial,
        completion: None,
    }
}

pub(super) fn call() -> Value {
    json!({"path": "src/lib.rs", "content": "changed"})
}

/// The operator is shown the caller's line, and an approval admits the call.
#[tokio::test]
async fn ask_puts_a_write_to_the_operator_under_the_callers_line() {
    let operator = Recording::new(Answer::Once);
    let ask = ask(ApprovalPolicy::Ask, operator.clone());
    let capability = Capability::new(true);

    assert!(
        ask.approve(&capability, &definition("write"), &call())
            .await
    );
    assert_eq!(operator.calls(), 1, "one write, one question");
    assert_eq!(
        operator.subjects(),
        vec![line(&caller())],
        "the prompt names the helper and its job"
    );
    assert_eq!(
        ask.would_ask(&capability, &definition("write"), &call()),
        Some(true)
    );
}

/// A refusal refuses the call, and the next one asks again.
#[tokio::test]
async fn a_refusal_refuses_the_write_and_is_not_remembered() {
    let operator = Recording::new(Answer::Deny);
    let ask = ask(ApprovalPolicy::Ask, operator.clone());
    let capability = Capability::new(true);

    assert!(
        !ask.approve(&capability, &definition("write"), &call())
            .await
    );
    assert!(!ask.approve(&capability, &definition("edit"), &call()).await);
    assert_eq!(operator.calls(), 2, "every write asks again");
    assert!(operator.grants.is_empty(), "a refusal grants nothing");
}

/// "Allow once" admits this call and remembers nothing: the next write asks.
#[tokio::test]
async fn allow_once_admits_one_write_and_the_next_asks_again() {
    let operator = Recording::new(Answer::Once);
    let ask = ask(ApprovalPolicy::Ask, operator.clone());
    let capability = Capability::new(true);

    assert!(
        ask.approve(&capability, &definition("write"), &call())
            .await
    );
    assert!(
        ask.approve(&capability, &definition("write"), &call())
            .await
    );
    assert_eq!(operator.calls(), 2);
    assert!(operator.grants.is_empty());
}

/// "Allow for this run" admits the call and records `workspace-write`, so the
/// next write of the same job is not asked.
#[tokio::test]
async fn allow_session_records_the_write_grant_and_stops_asking() {
    let operator = Recording::new(Answer::Session);
    let ask = ask(ApprovalPolicy::Ask, operator.clone());
    let capability = Capability::new(true);

    assert!(
        ask.approve(&capability, &definition("write"), &call())
            .await
    );
    assert!(operator.grants.is_granted("workspace-write"));
    assert!(
        ask.approve(&capability, &definition("edit"), &call()).await,
        "a held grant pre-answers the call"
    );
    assert_eq!(operator.calls(), 1, "the second write is not asked");
    assert_eq!(
        ask.would_ask(&capability, &definition("edit"), &call()),
        Some(false)
    );
}
