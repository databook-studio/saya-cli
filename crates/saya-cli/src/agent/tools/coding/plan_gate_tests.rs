//! The coding root's plan gate: a plan-first job's advertisement is the
//! read-shaped surface plus `checklist` and `submit_plan`, and the operator's
//! answer to a submission is the only thing that opens the rest.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use saya_agent::{ApprovalDecider, ToolDefinition, ToolError, ToolExecutor};

use super::direct::RootTools;
use super::direct_tests::compose_root_data;
use super::direct_tools::DirectCodingTools;
use super::parent::CodingTools;
use super::plan_gate::{MAX_PLAN_CHARS, PlanGate, SUBMIT_PLAN};
use super::root_shell_background_tests::shell_over;
use super::root_shell_tests::fixture::TempCheckout;
use crate::agent::tools::child::tests_root_compose_fixture as fixture;
use crate::agent::tools::consent::{Caller, Operator};
use crate::agent::tools::workspace;

/// The tools a closed plan posture must hide: the write family, `delete`, the
/// shell and the delegation tool.
const HIDDEN: &[&str] = &[
    "write",
    "edit",
    "notebook_edit",
    "delete",
    "shell",
    "helpers",
];

/// The tools a closed plan posture still offers: the read family and the gate
/// itself. The lean default already withholds `list` and `helpers`.
const READ_SURFACE: &[&str] = &["read", "glob", "grep", "checklist", SUBMIT_PLAN];

/// An operator that answers with a fixed verdict and counts how often it was
/// asked, so a test can prove a second submission did not ask again.
struct CannedOperator {
    answer: bool,
    asks: AtomicUsize,
}

impl CannedOperator {
    fn new(answer: bool) -> Arc<Self> {
        Arc::new(Self {
            answer,
            asks: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl Operator for CannedOperator {
    async fn ask(&self, _subject: &str, _arguments: &serde_json::Value) -> bool {
        self.asks.fetch_add(1, Ordering::AcqRel);
        self.answer
    }
}

/// The root as a plan-first job composes it: the working-checkout toolset every
/// other test builds, carrying the two members this composition leaves out — a
/// real shell over a temp checkout, and `delete`, which the root's own coding
/// operations attach — with a closed plan gate over `operator`.
///
/// The shell is `None` on a host that proves no sandbox, and the test then
/// asserts only the members this host can compose.
async fn plan_root(fixture: &fixture::Fixture, operator: Option<Arc<dyn Operator>>) -> RootTools {
    let gate = PlanGate::new(
        operator,
        Caller::Root {
            job_id: fixture.job.clone(),
        },
    );
    compose(fixture, Some(Arc::new(gate))).await
}

/// The same root with no gate at all: what a job that never stated `--plan`
/// composes.
async fn ungated_root(fixture: &fixture::Fixture) -> RootTools {
    compose(fixture, None).await
}

async fn compose(fixture: &fixture::Fixture, plan: Option<Arc<PlanGate>>) -> RootTools {
    let mut workspace =
        compose_root_data(fixture, saya_config::ResolvedTools::default(), None).await;
    let checkout = TempCheckout::new("plan-gate");
    let shell = shell_over(&checkout, saya_agent::ApprovalPolicy::Bypass, None);
    // `delete` is attached by the root's own coding operations, which this
    // fixture composes without; adding the definition the composition would
    // carry is what lets the plan filter be read against a real member.
    workspace.definitions.push(workspace::delete_definition());
    let coordinator = CodingTools {
        job: Arc::clone(&fixture.delegate_job),
        databases: None,
        denials: crate::agent::tools::coding::owner_denial::DenialTally::default(),
    };
    let decider = workspace.approval;
    RootTools::WorkingCheckout(Box::new(DirectCodingTools::new(
        coordinator,
        workspace.tools,
        workspace.definitions,
        decider,
        shell,
        &saya_config::HelperAgents::default(),
        None,
        None,
        plan,
        String::new(),
        saya_config::ResolvedTools::default(),
    )))
}

fn names(definitions: &[ToolDefinition]) -> Vec<String> {
    definitions
        .iter()
        .map(|definition| definition.name.clone())
        .collect()
}

fn has(definitions: &[ToolDefinition], name: &str) -> bool {
    definitions.iter().any(|definition| definition.name == name)
}

fn submission(plan: &str) -> serde_json::Value {
    serde_json::json!({"plan": plan})
}

/// A closed plan posture advertises the read surface and nothing else: the
/// write family, `delete`, the shell family, the delegation tool and the fetch
/// tool are hidden, so a call to one is refused as unknown rather than denied
/// after it was offered.
#[tokio::test]
async fn a_plan_first_root_hides_the_write_surface_until_the_operator_answers() {
    let fixture = fixture::Fixture::new("plan-closed-advertisement", &["tracked.txt"]).await;
    let root = plan_root(&fixture, Some(CannedOperator::new(true))).await;
    let advertised = root.definitions();
    for name in READ_SURFACE {
        assert!(has(&advertised, name), "{name} is offered under a plan");
    }
    for name in HIDDEN {
        assert!(
            !has(&advertised, name),
            "{name} is not advertised under a plan: {:?}",
            names(&advertised)
        );
    }
    // Hidden, not merely denied: the dispatch gate refuses the name outright.
    for name in ["write", "edit", "notebook_edit", "delete", "shell"] {
        assert!(
            matches!(
                root.execute(name, serde_json::json!({"path": "tracked.txt"}))
                    .await,
                Err(ToolError::UnsupportedTool)
            ),
            "{name} must refuse as an unknown tool under a plan"
        );
    }
}

/// The operator's approval opens the write surface for the same job: the very
/// next advertisement is the composition's whole list, and a write runs.
#[tokio::test]
async fn an_approved_plan_opens_the_write_surface_for_the_same_job() {
    let fixture = fixture::Fixture::new("plan-approved-opens", &["tracked.txt"]).await;
    let operator = CannedOperator::new(true);
    let root = plan_root(&fixture, Some(operator.clone())).await;
    assert!(!has(&root.definitions(), "write"), "closed to begin with");
    let answered = root
        .execute(SUBMIT_PLAN, submission("1. change tracked.txt\n2. verify"))
        .await
        .expect("the submission runs");
    assert_eq!(answered["approved"], true, "{answered}");
    assert_eq!(operator.asks.load(Ordering::Acquire), 1);
    let advertised = root.definitions();
    for name in ["read", "write", "edit", "checklist"] {
        assert!(
            has(&advertised, name),
            "{name} is advertised once the plan is approved: {:?}",
            names(&advertised)
        );
    }
    // The gate opens the write surface; the lean default still withholds
    // `delete` and `helpers` from a request that names neither.
    for name in ["delete", "helpers"] {
        assert!(
            !has(&advertised, name),
            "{name} stays off the lean default: {:?}",
            names(&advertised)
        );
    }
    let write = advertised
        .iter()
        .find(|definition| definition.name == "write")
        .expect("write is advertised")
        .clone();
    let call = serde_json::json!({"path": "planned.txt", "content": "planned"});
    assert!(root.approve(&write, &call).await);
    root.execute("write", call).await.expect("the write runs");
    assert_eq!(
        std::fs::read_to_string(fixture.checkout_path.join("planned.txt")).unwrap(),
        "planned"
    );
}

/// A refusal leaves the surface closed, and the model is free to submit a
/// revised plan: the gate is a question, not a one-shot.
#[tokio::test]
async fn a_refused_plan_leaves_the_write_surface_closed() {
    let fixture = fixture::Fixture::new("plan-refused-stays-closed", &["tracked.txt"]).await;
    let operator = CannedOperator::new(false);
    let root = plan_root(&fixture, Some(operator.clone())).await;
    let answered = root
        .execute(SUBMIT_PLAN, submission("1. change tracked.txt"))
        .await
        .expect("the submission runs");
    assert_eq!(answered["approved"], false, "{answered}");
    assert_eq!(operator.asks.load(Ordering::Acquire), 1);
    for name in HIDDEN {
        assert!(
            !has(&root.definitions(), name),
            "{name} stays hidden after a refusal"
        );
    }
    assert!(
        matches!(
            root.execute("write", serde_json::json!({"path": "x", "content": "y"}))
                .await,
            Err(ToolError::UnsupportedTool)
        ),
        "a refusal is not a grant"
    );
}

/// A submission that is not one bounded plan never reaches the operator: the
/// arguments are refused, and the surface stays closed.
#[tokio::test]
async fn a_malformed_submission_never_reaches_the_operator() {
    let fixture = fixture::Fixture::new("plan-malformed", &["tracked.txt"]).await;
    let operator = CannedOperator::new(true);
    let root = plan_root(&fixture, Some(operator.clone())).await;
    let too_long = "x".repeat(MAX_PLAN_CHARS + 1);
    for arguments in [
        serde_json::json!({}),
        serde_json::json!({"plan": ""}),
        serde_json::json!({"plan": 7}),
        serde_json::json!({"plan": "ok", "extra": "no"}),
        serde_json::json!({"plan": too_long}),
    ] {
        assert!(
            matches!(
                root.execute(SUBMIT_PLAN, arguments.clone()).await,
                Err(ToolError::UnsupportedProperty)
            ),
            "{arguments} must be refused before anyone is asked"
        );
    }
    assert_eq!(operator.asks.load(Ordering::Acquire), 0);
    assert!(!has(&root.definitions(), "write"));
}

/// A job that never entered plan mode is unchanged: no gate, the whole
/// composition advertised from the first request, and `submit_plan` nowhere in
/// it.
#[tokio::test]
async fn a_job_without_plan_mode_is_unchanged() {
    let fixture = fixture::Fixture::new("plan-absent", &["tracked.txt"]).await;
    let root = ungated_root(&fixture).await;
    let advertised = root.definitions();
    for name in ["read", "glob", "grep", "write", "edit", "checklist"] {
        assert!(
            has(&advertised, name),
            "{name} is advertised: {:?}",
            names(&advertised)
        );
    }
    assert!(!has(&advertised, SUBMIT_PLAN), "no plan mode, no gate");
    assert!(
        matches!(
            root.execute(SUBMIT_PLAN, submission("a plan")).await,
            Err(ToolError::UnsupportedTool)
        ),
        "a job with no gate cannot submit a plan"
    );
}
