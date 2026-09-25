//! The run boundary's entry-point property: every entry point into a run
//! refuses the bypass mode (DESIGN §6, test 16's twin). A run's approval is
//! its `--allow` scopes — typed, per-capability, journaled — and bypass is a
//! session mode's blanket per-call consent, which a run has no per-call ask
//! to replace. The start guard taught the rule; the resume entry never
//! consulted it (U6 defect 1: a resumed run composed a frozen policy in
//! bypass mode and auto-allowed every ask-shaped call on it). The property
//! is stated once over
//! the enumerated entry set — the headless fresh run, the host panel's fresh
//! run, and the resume — and every entry must return the run boundary's one
//! refusal, byte-identical, because the refusal is one guard, not three
//! wordings.

use super::cancel::tests::{runtime_at, temp_root};
use super::host::{HostRun, RunRequest};
use super::mode::RunApproval;
use super::resume::resume;
use super::start::{StartInputs, start, start_for_panel};
use crate::render::RenderFormat;
use saya_agent::{ApprovalPolicy, CancellationToken};
use saya_store::SqliteStateStore;
use std::fs;

/// The refusal every entry point must produce: the run boundary's own words,
/// naming what a run's approval actually is and where bypass does live.
fn assert_run_boundary_refusal(entry: &str, error: &dyn std::error::Error) {
    let message = error.to_string();
    assert!(
        message.contains("--allow"),
        "{entry} must refuse bypass naming the run's real approval: {message}"
    );
    assert!(
        message.contains("session mode"),
        "{entry} must refuse bypass naming where bypass does live: {message}"
    );
}

/// Every entry point into a run refuses the bypass mode, with one shared
/// refusal: a run's approval is its `--allow` scopes; bypass is a session
/// mode. Three entries drive the property through their real signatures —
/// the headless fresh run, the host panel's fresh run (the TUI run panel's
/// path), and the resume — and none may reach a run directory, a store row,
/// or a decider with bypass composed in. A future entry point into a run is
/// caught by construction: the composition (`assembly::assemble`, the drive,
/// the engine resume) admits only the boundary's own admitted type, which
/// bypass cannot construct — see [`super::mode::RunApproval`].
#[tokio::test]
async fn every_entry_point_into_a_run_refuses_bypass() {
    let root = temp_root("entry-bypass");
    let runtime = runtime_at(&root);
    let store = SqliteStateStore::new(root.join("state.sqlite3"));

    // Entry 1: the headless fresh run (`saya run "<goal>"`).
    let error = start(
        StartInputs {
            prompt: Some("survey the data".to_owned()),
            allow: &[],
            budget_tokens: &[],
            can_prompt: false,
        },
        &runtime,
        RenderFormat::Text,
        ApprovalPolicy::Bypass,
        &store,
    )
    .await
    .expect_err("the headless fresh run refuses bypass");
    assert_run_boundary_refusal("the headless fresh run", error.as_ref());

    // Entry 2: the host panel's fresh run (the TUI run panel's path).
    let host = HostRun {
        journal_wire: None,
        agent_stream: None,
        profile: None,
        plan_approval: &super::approval::PlanApproval::PreAuthorized,
        cancellation: CancellationToken::default(),
    };
    let error = start_for_panel(
        RunRequest {
            run_id: super::new_run_id(),
            goal: Some("survey the data".to_owned()),
            allow: Vec::new(),
            budget: Vec::new(),
        },
        &runtime,
        RenderFormat::Text,
        ApprovalPolicy::Bypass,
        &store,
        host,
    )
    .await
    .expect_err("the panel's fresh run refuses bypass");
    assert_run_boundary_refusal("the panel's fresh run", error.as_ref());

    // Entry 3: the resume (`saya run resume <id>`).
    let error = resume(
        "r-entry-bypass",
        &runtime,
        RenderFormat::Text,
        ApprovalPolicy::Bypass,
        &store,
    )
    .await
    .expect_err("the resume refuses bypass");
    assert_run_boundary_refusal("the resume", error.as_ref());

    let _ = fs::remove_dir_all(root);
}

/// A run's admission refuses the bypass mode, naming what a run's approval
/// actually is (DESIGN §6, test 16): a run's approval is its `--allow`
/// scopes — typed, per-capability, journaled — and bypass is a session
/// mode's blanket per-call consent, which a run has no per-call ask to
/// replace. Every other mode admits.
///
/// Moved from `start_tests` with its test: the guard's home changed — it was
/// a free function the fresh-run entries shared, and the resume entry never
/// consulted it (U6 defect 1), so the guard became the admission
/// constructor every entry must pass. The property over the entry points is
/// the test above; this pins the admission type's own behavior.
#[test]
fn a_run_refuses_the_bypass_mode_naming_allow() {
    for admitted in [
        ApprovalPolicy::ReadOnly,
        ApprovalPolicy::Never,
        ApprovalPolicy::Ask,
    ] {
        assert!(
            RunApproval::admit(admitted).is_ok(),
            "{admitted:?} admits: a run's mode is its own decision surface"
        );
    }
    let error = RunApproval::admit(ApprovalPolicy::Bypass).expect_err("bypass never starts a run");
    assert!(
        error.to_string().contains("--allow"),
        "the refusal names the run's real approval: {error}"
    );
    assert!(
        error.to_string().contains("session mode"),
        "the refusal names where bypass does live: {error}"
    );
}
