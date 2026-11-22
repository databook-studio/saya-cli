//! The root's write-shaped calls on a job with an operator attached: the
//! job's mode decides them, and only `ask` puts one to the person — under a
//! subject line that names the job and a target that names the path.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use saya_agent::{ApprovalDecider, ApprovalPolicy, ToolDefinition};
use serde_json::{Value, json};

use super::direct_tests::working_checkout_root;
use super::plan_gate_tests::plan_root;
use crate::agent::tools::child::tests_root_compose_fixture as fixture;
use crate::agent::tools::consent::Operator;

/// An operator that records every subject line it is shown and answers with
/// the one choice the test states.
struct Recording {
    approve: bool,
    subjects: Mutex<Vec<String>>,
}

impl Recording {
    fn new(approve: bool) -> Arc<Self> {
        Arc::new(Self {
            approve,
            subjects: Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> usize {
        self.subjects.lock().unwrap().len()
    }

    fn subjects(&self) -> Vec<String> {
        self.subjects.lock().unwrap().clone()
    }
}

#[async_trait]
impl Operator for Recording {
    async fn ask(&self, subject: &str, _arguments: &Value) -> bool {
        self.subjects.lock().unwrap().push(subject.to_owned());
        self.approve
    }
}

/// The root's own `write`, as the workspace family composes it. Taken from the
/// family rather than from the advertised list: a `read-only` or `never` root
/// does not offer it at all — its calls are refused at the door — and this file
/// is about what the *execution-time* gate answers a call with, which the
/// advertisement's gate never replaces.
fn write() -> ToolDefinition {
    let mut definitions = Vec::new();
    crate::agent::tools::workspace::append_definitions(&mut definitions, true);
    definitions
        .into_iter()
        .find(|definition| definition.name == "write")
        .expect("the workspace family composes write")
}

/// One write inside the working checkout.
fn call() -> Value {
    json!({"path": "notes/new.txt", "content": "hello"})
}

/// The subject line the operator reads for the root of `job`.
fn line(job: &saya_types::JobId) -> String {
    format!("Change a workspace file for job {job}")
}

/// Under `ask`, a root write inside the working checkout reaches the operator
/// exactly once and the answer decides: an approval admits the call.
#[tokio::test]
async fn ask_puts_a_root_write_to_the_operator_once() {
    let operator = Recording::new(true);
    let fixture = fixture::Fixture::asking(
        "direct-write-ask",
        &["notes"],
        operator.clone(),
        ApprovalPolicy::Ask,
    )
    .await;
    let root = working_checkout_root(&fixture).await;
    let write = write();
    let job = fixture.delegate_job.job_id().clone();

    assert!(
        root.approve(&write, &call()).await,
        "an approving operator admits the write"
    );
    assert_eq!(operator.calls(), 1, "the write is put to the operator once");
    assert_eq!(
        operator.subjects(),
        vec![line(&job)],
        "the prompt names the job that asks"
    );
    assert_eq!(
        root.would_ask(&write, &call()),
        Some(true),
        "a write under ask is the one call that asks"
    );
}

/// A refusal is a normal denial: the write is refused, unasked no further.
#[tokio::test]
async fn the_operators_refusal_refuses_the_root_write() {
    let operator = Recording::new(false);
    let fixture = fixture::Fixture::asking(
        "direct-write-deny",
        &["notes"],
        operator.clone(),
        ApprovalPolicy::Ask,
    )
    .await;
    let root = working_checkout_root(&fixture).await;
    let write = write();

    assert!(
        !root.approve(&write, &call()).await,
        "a refusing operator refuses the write"
    );
    assert_eq!(operator.calls(), 1);
}

/// `read-only` and `never` refuse a write outright: nobody is asked, because
/// no answer could widen the mode.
#[tokio::test]
async fn read_only_and_never_refuse_a_root_write_without_a_question() {
    for mode in [ApprovalPolicy::ReadOnly, ApprovalPolicy::Never] {
        let operator = Recording::new(true);
        let fixture =
            fixture::Fixture::asking("direct-write-refused", &["notes"], operator.clone(), mode)
                .await;
        let root = working_checkout_root(&fixture).await;
        let write = write();

        assert!(
            !root.approve(&write, &call()).await,
            "{mode:?} refuses a write"
        );
        assert_eq!(operator.calls(), 0, "{mode:?} asks nobody");
    }
}

/// `accept-edits` and `bypass` admit a write the capability admits, without a
/// question: the mode is the consent.
#[tokio::test]
async fn accept_edits_and_bypass_admit_a_root_write_without_a_question() {
    for mode in [ApprovalPolicy::AcceptEdits, ApprovalPolicy::Bypass] {
        let operator = Recording::new(false);
        let fixture =
            fixture::Fixture::asking("direct-write-admitted", &["notes"], operator.clone(), mode)
                .await;
        let root = working_checkout_root(&fixture).await;
        let write = write();

        assert!(
            root.approve(&write, &call()).await,
            "{mode:?} admits a write"
        );
        assert_eq!(operator.calls(), 0, "{mode:?} asks nobody");
    }
}

/// The capability decides first: a path outside the working checkout is
/// refused without a question in every mode, `bypass` included.
#[tokio::test]
async fn a_write_outside_the_workspace_is_refused_without_a_question() {
    for mode in [
        ApprovalPolicy::Ask,
        ApprovalPolicy::AcceptEdits,
        ApprovalPolicy::ReadOnly,
        ApprovalPolicy::Never,
        ApprovalPolicy::Bypass,
    ] {
        let operator = Recording::new(true);
        let fixture =
            fixture::Fixture::asking("direct-write-escape", &["notes"], operator.clone(), mode)
                .await;
        let root = working_checkout_root(&fixture).await;
        let write = write();
        let escape = json!({"path": "../escape.txt", "content": "must not exist"});

        assert!(
            !root.approve(&write, &escape).await,
            "{mode:?} refuses a path outside the workspace"
        );
        assert_eq!(
            operator.calls(),
            0,
            "{mode:?} asks nobody about a path the capability already refuses"
        );
    }
}

/// A plan-first job's closed gate hides the write family, so a `write` it never
/// offered is refused before the question: the executor would refuse it too,
/// and a question that could only end in a refusal must not be asked.
#[tokio::test]
async fn a_write_the_plan_posture_hides_is_refused_without_a_question() {
    let operator = Recording::new(true);
    let fixture = fixture::Fixture::asking(
        "direct-write-plan-hidden",
        &["notes"],
        operator.clone(),
        ApprovalPolicy::Ask,
    )
    .await;
    let write = write();
    let closed = plan_root(&fixture, None).await;

    assert!(
        !closed.approve(&write, &call()).await,
        "a closed plan gate refuses the write it hides"
    );
    assert_eq!(operator.calls(), 0, "the operator is not asked");
    assert_eq!(
        closed.would_ask(&write, &call()),
        Some(false),
        "a refused-anyway call reports no ask"
    );
}

/// A job with no operator attached keeps the consent it composes: an
/// edit-capable mode admits its writes and never asks, because there is
/// nobody to ask.
#[tokio::test]
async fn a_detached_job_writes_with_no_question_at_all() {
    for mode in [ApprovalPolicy::AcceptEdits, ApprovalPolicy::Bypass] {
        let fixture = fixture::Fixture::stating("direct-write-detached", &["notes"], mode).await;
        let root = working_checkout_root(&fixture).await;
        let write = write();

        assert!(
            root.approve(&write, &call()).await,
            "{mode:?} admits a detached job's write as before"
        );
        assert_eq!(
            root.would_ask(&write, &call()),
            None,
            "{mode:?} has no ask to report"
        );
    }
}
