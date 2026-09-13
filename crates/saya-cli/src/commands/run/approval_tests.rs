//! The plan-approval gate's surface: the view the user sees (plan, scopes,
//! budgets, artifact digests) and the channel seam's deny-by-default
//! decisions — the TUI approve/deny via channel stub (the `tui/agent.rs`
//! approval pattern).

use super::approval;
use super::approval::PlanApproval;
use super::approval_view;
use super::approval_view::{PlanApprovalView, StepView};
use saya_harness::engine::ManifestBounds;
use saya_harness::workspace::Workspace;
use saya_types::{Budgets, Capabilities, RunPlan, StepSpec};
use std::fs;
use std::path::PathBuf;
use tokio::sync::mpsc::unbounded_channel;

fn bounds() -> ManifestBounds {
    ManifestBounds {
        max_files: 8,
        max_file_bytes: 64 * 1024,
    }
}

fn one_step_plan(goal: &str, capabilities: Capabilities) -> RunPlan {
    RunPlan::new(vec![
        StepSpec::new(goal, capabilities, None, Vec::new(), None).unwrap(),
    ])
    .unwrap()
}

/// A scratch workspace holding one artifact, so the view has a digest to
/// show.
fn workspace_with_artifact(label: &str) -> (PathBuf, Workspace) {
    let root = std::env::temp_dir().join(format!(
        "saya-approval-tests-{label}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let dir = root.join("workspace");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("driver.py"), "print('survey')").unwrap();
    let workspace = Workspace::open(&dir).unwrap();
    (root, workspace)
}

fn empty_workspace(label: &str) -> (PathBuf, Workspace) {
    let root = std::env::temp_dir().join(format!(
        "saya-approval-tests-{label}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let dir = root.join("workspace");
    fs::create_dir_all(&dir).unwrap();
    let workspace = Workspace::open(&dir).unwrap();
    (root, workspace)
}

/// The view shows the approved scopes, the plan, the scopes each step
/// requests (the `--allow` grammar's words), the budgets, and the
/// artifacts' digests.
#[test]
fn the_view_shows_plan_scopes_budgets_and_digests() {
    let (root, workspace) = workspace_with_artifact("view");
    let mut approved = Capabilities::default();
    approved.workspace_write = true;
    let mut requested = Capabilities::default();
    requested.scratch = true;
    let plan = one_step_plan("survey the data quality", requested);
    let mut budgets = Budgets::default();
    budgets.turns = Some(40);
    let view = approval_view::view_of(
        "audit the database",
        &approved,
        &plan,
        &budgets,
        &workspace,
        bounds(),
        &[],
    )
    .unwrap();
    let text = approval_view::render(&view);

    assert!(text.contains("audit the database"), "the goal: {text}");
    assert!(
        text.contains("approved scopes: workspace-write"),
        "the run's granted scopes: {text}"
    );
    assert!(text.contains("survey the data quality"), "the step: {text}");
    assert!(
        text.contains("scopes: scratch"),
        "the step's requested scopes in the --allow grammar: {text}"
    );
    assert!(text.contains("turns=40"), "the declared budget: {text}");
    let line = text
        .lines()
        .find(|line| line.contains("driver.py"))
        .expect("the artifact is listed");
    let digest = line
        .split("sha256: ")
        .nth(1)
        .expect("the digest is labeled")
        .trim_end_matches(')');
    assert!(
        digest.len() == 64 && digest.chars().all(|c| c.is_ascii_hexdigit()),
        "the artifact's sha256 digest: {line}"
    );
    let _ = fs::remove_dir_all(root);
}

/// The carried grant words the run stated (`sql:<connection>`) ride the
/// approved scopes the view shows — so the journal's `PlanApproved` payload
/// records every word the user approved, and a resume re-derives them from
/// the journal. A run without carried tokens renders today's words exactly.
#[test]
fn the_view_s_approved_scopes_carry_the_stated_grant_words() {
    let (root, workspace) = empty_workspace("carried");
    let plan = one_step_plan("read only", Capabilities::default());
    let view = approval_view::view_of(
        "g",
        &Capabilities::default(),
        &plan,
        &Budgets::default(),
        &workspace,
        bounds(),
        &["sql:analytics".to_owned(), "sql:staging".to_owned()],
    )
    .unwrap();
    assert_eq!(
        view.approved,
        vec!["sql:analytics".to_owned(), "sql:staging".to_owned()],
        "the carried words are the approval the view (and the journal) states"
    );
    // Capability words first, carried words after: one deterministic order.
    let mut approved = Capabilities::default();
    approved.workspace_write = true;
    let view = approval_view::view_of(
        "g",
        &approved,
        &plan,
        &Budgets::default(),
        &workspace,
        bounds(),
        &["sql:analytics".to_owned()],
    )
    .unwrap();
    assert_eq!(
        view.approved,
        vec!["workspace-write".to_owned(), "sql:analytics".to_owned()],
        "the payload is the capability words plus the carried grant words"
    );
    let _ = fs::remove_dir_all(root);
}

/// An empty workspace is shown as having no artifacts yet — never as a
/// fabricated entry.
#[test]
fn a_fresh_workspace_shows_no_artifacts_yet() {
    let (root, workspace) = empty_workspace("empty");
    let plan = one_step_plan("read only", Capabilities::default());
    let view = approval_view::view_of(
        "g",
        &Capabilities::default(),
        &plan,
        &Budgets::default(),
        &workspace,
        bounds(),
        &[],
    )
    .unwrap();
    let text = approval_view::render(&view);
    assert!(
        text.contains("(none yet)"),
        "an empty manifest must be said out loud: {text}"
    );
    let _ = fs::remove_dir_all(root);
}

/// The pre-authorized surface (headless) approves: the RunSpec declared the
/// scopes and the engine refused any plan outside them.
#[tokio::test]
async fn the_pre_authorized_surface_approves() {
    let surface = PlanApproval::PreAuthorized;
    let view = view_with_no_steps();
    assert!(approval::decide(&surface, &view).await);
}

/// The channel seam's deny-by-default decisions: an approval and a refusal
/// travel the oneshot; a closed channel (the UI went away) refuses without
/// hanging; every request carries the full rendered view.
#[tokio::test]
async fn the_channel_denies_by_default_and_carries_the_view() {
    let view = view_with_no_steps();

    // The approve path: the stub replies through the oneshot.
    let (sender, mut receiver) = unbounded_channel();
    let surface = PlanApproval::ViaChannel(sender);
    let responder = tokio::spawn(async move {
        if let Some(request) = receiver.recv().await {
            assert!(
                request.view_text.contains("run approval — goal:"),
                "the modal receives the rendered view: {}",
                request.view_text
            );
            let _ = request.respond.send(true);
        }
    });
    assert!(approval::decide(&surface, &view).await);
    responder.await.unwrap();

    // A deny reply is a refusal.
    let (sender, mut receiver) = unbounded_channel();
    let surface = PlanApproval::ViaChannel(sender);
    let responder = tokio::spawn(async move {
        if let Some(request) = receiver.recv().await {
            let _ = request.respond.send(false);
        }
    });
    assert!(!approval::decide(&surface, &view).await);
    responder.await.unwrap();

    // A closed channel denies without hanging.
    let (sender, receiver) = unbounded_channel();
    drop(receiver);
    let surface = PlanApproval::ViaChannel(sender);
    assert!(!approval::decide(&surface, &view).await);
}

fn view_with_no_steps() -> PlanApprovalView {
    let plan = one_step_plan("one step", Capabilities::default());
    PlanApprovalView {
        goal: "the goal".to_string(),
        approved: Vec::new(),
        steps: plan
            .steps
            .iter()
            .map(|step| StepView {
                goal: step.goal.clone(),
                scopes: Vec::new(),
                endpoint: None,
            })
            .collect(),
        budgets: Budgets::default(),
        artifacts: Vec::new(),
    }
}
