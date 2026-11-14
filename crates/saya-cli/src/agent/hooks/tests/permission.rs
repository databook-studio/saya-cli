//! `PermissionRequest`: a hook runs only where the operator was about to be
//! asked, can refuse that ask, and can never approve one.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use saya_agent::{ApprovalDecider, LocalStateEffect, ToolConcurrency, ToolDefinition, ToolEffect};
use serde_json::{Value, json};

use super::super::gate::HookApprovalGate;
use super::{TempRoot, engine};

/// The operator behind the gate: whether it would ask, what it decides, and
/// whether it was reached at all.
struct Operator {
    ask: Option<bool>,
    verdict: bool,
    asked: Arc<AtomicU64>,
}

impl Operator {
    fn new(ask: Option<bool>, verdict: bool) -> (Arc<Self>, Arc<AtomicU64>) {
        let asked = Arc::new(AtomicU64::new(0));
        (
            Arc::new(Self {
                ask,
                verdict,
                asked: Arc::clone(&asked),
            }),
            asked,
        )
    }
}

#[async_trait]
impl ApprovalDecider for Operator {
    async fn approve(&self, _tool: &ToolDefinition, _arguments: &Value) -> bool {
        self.asked.fetch_add(1, Ordering::Relaxed);
        self.verdict
    }

    fn would_ask(&self, _tool: &ToolDefinition, _arguments: &Value) -> Option<bool> {
        self.ask
    }
}

fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "shell".into(),
        description: String::new(),
        read_only: false,
        parameters: json!({"type": "object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: true,
            requires_approval: true,
            local_state: LocalStateEffect::None,
        },
        concurrency: ToolConcurrency::Serial,
        completion: None,
    }
}

/// Exit `2` denies the ask with the hook's own bounded stderr as the reason.
#[tokio::test]
async fn a_permission_hook_can_refuse_the_ask() {
    let root = TempRoot::new("permission-deny");
    let engine = engine(
        "[[hooks.permission_request]]\ncommand = \"printf 'blocked: no writes here' >&2; exit 2\"\n",
        &root,
    );
    let (operator, asked) = Operator::new(Some(true), true);
    let gate = HookApprovalGate::new(engine, operator);

    assert!(
        !gate.approve(&definition(), &json!({})).await,
        "the hook's refusal must deny the call"
    );
    assert_eq!(
        asked.load(Ordering::Relaxed),
        0,
        "the operator is not asked"
    );
    assert_eq!(
        gate.refusal_detail(&definition(), &json!({})).as_deref(),
        Some("blocked: no writes here"),
        "the model reads the hook's own reason"
    );
}

/// Exit `0` is not an approval: the operator is still asked, and its verdict
/// is the one that decides.
#[tokio::test]
async fn exit_zero_still_asks_the_operator() {
    let root = TempRoot::new("permission-allow");
    let engine = engine(
        "[[hooks.permission_request]]\ncommand = \"exit 0\"\n",
        &root,
    );
    let (operator, asked) = Operator::new(Some(true), false);
    let gate = HookApprovalGate::new(engine, operator);

    assert!(
        !gate.approve(&definition(), &json!({})).await,
        "the operator's refusal stands: the hook did not approve"
    );
    assert_eq!(
        asked.load(Ordering::Relaxed),
        1,
        "the operator was asked after the hook exited 0"
    );
}

/// No hook: the decider decides exactly as it did before.
#[tokio::test]
async fn no_hook_leaves_the_decision_unchanged() {
    let root = TempRoot::new("permission-none");
    let engine = engine("", &root);
    let (operator, asked) = Operator::new(Some(true), true);
    let gate = HookApprovalGate::new(engine, operator);

    assert!(gate.approve(&definition(), &json!({})).await);
    assert_eq!(asked.load(Ordering::Relaxed), 1);
}

/// A call the operator was never going to be asked about runs no hook: the
/// hook cannot pre-empt an automatic decision.
#[tokio::test]
async fn a_call_that_does_not_ask_runs_no_hook() {
    let root = TempRoot::new("permission-skip");
    let seen = root.path().join("seen");
    let engine = engine(
        &format!(
            "[[hooks.permission_request]]\ncommand = \"cat >> '{}'; exit 2\"\n",
            seen.display()
        ),
        &root,
    );
    let (operator, asked) = Operator::new(Some(false), true);
    let gate = HookApprovalGate::new(engine, operator);

    assert!(gate.approve(&definition(), &json!({})).await);
    assert_eq!(asked.load(Ordering::Relaxed), 1);
    assert!(
        !seen.exists(),
        "a hook must not run for a call that was not going to ask"
    );
}

/// A decider that cannot say whether it would ask is never pre-empted.
#[tokio::test]
async fn an_unknown_ask_shape_runs_no_hook() {
    let root = TempRoot::new("permission-unknown");
    let engine = engine(
        "[[hooks.permission_request]]\ncommand = \"exit 2\"\n",
        &root,
    );
    let (operator, asked) = Operator::new(None, true);
    let gate = HookApprovalGate::new(engine, operator);

    assert!(gate.approve(&definition(), &json!({})).await);
    assert_eq!(asked.load(Ordering::Relaxed), 1);
}

/// The payload names the event and the call, so one script can tell a
/// permission request from the pre-tool hook that follows it.
#[tokio::test]
async fn the_permission_payload_names_the_event_and_the_call() {
    let root = TempRoot::new("permission-payload");
    let seen = root.path().join("seen");
    let engine = engine(
        &format!(
            "[[hooks.permission_request]]\ncommand = \"cat >> '{}'\"\n",
            seen.display()
        ),
        &root,
    );
    let (operator, _) = Operator::new(Some(true), true);
    let gate = HookApprovalGate::new(engine, operator);
    gate.approve(&definition(), &json!({"command": "ls"})).await;
    let recorded = std::fs::read_to_string(&seen).expect("the hook must have run");
    assert!(
        recorded.contains("\"hook_event_name\":\"PermissionRequest\"")
            && recorded.contains("\"tool_name\":\"shell\"")
            && recorded.contains("\"job_id\":\"job-1\""),
        "the hook must read the event, the tool, and the caller: {recorded}"
    );
}
