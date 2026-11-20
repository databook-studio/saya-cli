//! The job a TUI session started, moved by the same switch its own mode moves:
//! the mode lands on the job's one cell, a withheld `bypass` reaches neither,
//! and a frozen job's cell is told the mode it keeps.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use saya_agent::{ApprovalPolicy, PermissionMode};
use saya_config::CommandRules;
use saya_types::JobId;
use serde_json::{Value, json};

use super::super::{BYPASS_REFUSAL, FROZEN_REFUSAL, cycle_from, switch_with_job};
use super::support::Fixture;
use crate::agent::tools::consent::{Caller, Operator, Prompt};
use crate::agent::tools::shell_policy::{ModeCell, ShellPolicy};

/// An operator that counts what a job's `shell` puts to it and allows it: the
/// stand-in for the TUI's prompt when a test drives the root shell's own lane.
#[derive(Clone, Default)]
struct Counts(Arc<AtomicUsize>);

impl Counts {
    fn asks(&self) -> usize {
        self.0.load(Ordering::Relaxed)
    }
}

#[async_trait::async_trait]
impl Operator for Counts {
    async fn ask(&self, _subject: &str, _arguments: &Value) -> bool {
        self.0.fetch_add(1, Ordering::Relaxed);
        true
    }
}

/// The prompt a root job's `shell` asks through, naming the job as the panel
/// does.
fn prompt(operator: Counts) -> Prompt {
    Prompt::new(
        Arc::new(operator),
        Caller::Root {
            job_id: JobId::parse("job").expect("the fixture job id parses"),
        }
        .line("Run a shell command", "run a shell command"),
    )
}

/// A call that names no lane, so the schema default — the host — applies.
fn host_call() -> Value {
    json!({ "command": "true", "description": "a probe" })
}

/// The request's first case: a TUI-started job running under `ask`, switched
/// through the path Shift+Tab uses, reads the new mode on its next call — its
/// root `shell` no longer puts the call to the operator, because `read-only`
/// serves it from the sandbox.
#[tokio::test]
async fn a_cycle_switch_reaches_the_running_jobs_cell_and_its_next_call() {
    let mut fixture = Fixture::acquire("job-cycle", ApprovalPolicy::Ask, false);
    let cell = ModeCell::new(ApprovalPolicy::Ask);
    let operator = Counts::default();
    let shell = ShellPolicy::new(
        ApprovalPolicy::Ask,
        Some(prompt(operator.clone())),
        true,
        CommandRules::default(),
        None,
    )
    .following(cell.clone());

    assert!(shell.approve(&host_call()).await);
    assert_eq!(operator.asks(), 1, "the asking mode puts the call to us");

    // Shift+Tab's own path: `ask → accept-edits → plan`, and `plan` is the
    // mode whose own words are "read-only".
    let mut message = String::new();
    for _ in 0..2 {
        let next = cycle_from(&fixture.state);
        message = switch_with_job(&mut fixture.state, &mut fixture.session, next, Some(&cell))
            .expect("neither step of the cycle needs the gate");
    }

    assert_eq!(
        cell.get(),
        ApprovalPolicy::ReadOnly,
        "the job's cell reads read-only"
    );
    assert!(
        message.contains("The running job switched too"),
        "the mode line says the job got it: {message}"
    );
    assert!(shell.approve(&host_call()).await);
    assert_eq!(
        operator.asks(),
        1,
        "the job's next call is not put on the host"
    );
}

/// The request's second case: a withheld `bypass` is refused for the session
/// and is not applied to the job either — the switch's own check comes first.
#[test]
fn a_withheld_bypass_is_refused_for_the_session_and_the_job_alike() {
    let mut fixture = Fixture::acquire("job-bypass", ApprovalPolicy::Ask, false);
    let cell = ModeCell::new(ApprovalPolicy::Ask);
    let error = switch_with_job(
        &mut fixture.state,
        &mut fixture.session,
        PermissionMode::Bypass,
        Some(&cell),
    )
    .expect_err("bypass is withheld");

    assert_eq!(error, BYPASS_REFUSAL);
    assert_eq!(fixture.state.approval_mode, "ask");
    assert_eq!(fixture.session.policy().mode(), ApprovalPolicy::Ask);
    assert_eq!(
        cell.get(),
        ApprovalPolicy::Ask,
        "a mode the session refuses is not applied to the job either"
    );
}

/// The request's third case: a frozen job — the cell
/// `Launch::Detached(..).compose(..)` leaves behind — keeps its mode while the
/// session switches, and the answer says so in the switch's own frozen words.
#[test]
fn a_frozen_job_keeps_its_mode_and_the_answer_says_so() {
    let mut fixture = Fixture::acquire("job-frozen", ApprovalPolicy::Ask, false);
    let cell = ModeCell::new(ApprovalPolicy::Ask);
    cell.freeze_at(ApprovalPolicy::Ask);
    let message = switch_with_job(
        &mut fixture.state,
        &mut fixture.session,
        PermissionMode::AcceptEdits,
        Some(&cell),
    )
    .expect("the session's own policy is not frozen");

    assert_eq!(fixture.state.approval_mode, "accept-edits");
    assert_eq!(fixture.session.policy().mode(), ApprovalPolicy::AcceptEdits);
    assert_eq!(
        cell.get(),
        ApprovalPolicy::Ask,
        "the job keeps the mode it was frozen at"
    );
    assert!(message.contains("Mode: accept-edits"), "{message}");
    assert!(
        message.contains("The running job keeps its mode"),
        "the line says the job did not take it: {message}"
    );
    assert!(
        message.contains(FROZEN_REFUSAL),
        "in the existing frozen wording: {message}"
    );
}

/// A switch narrows or holds: `plan` states no consent posture of its own, so
/// it is stated on the job as `read-only` — never over a cell already at
/// `never`, which would widen it.
#[test]
fn a_plan_switch_never_widens_a_never_job() {
    let mut fixture = Fixture::acquire("job-never", ApprovalPolicy::Never, false);
    let cell = ModeCell::new(ApprovalPolicy::Never);
    let message = switch_with_job(
        &mut fixture.state,
        &mut fixture.session,
        PermissionMode::Plan,
        Some(&cell),
    )
    .expect("plan needs no gate");

    assert_eq!(
        cell.get(),
        ApprovalPolicy::Never,
        "a switch never widens the job it moves"
    );
    assert!(
        message.contains("The running job keeps its mode"),
        "{message}"
    );
}

/// The request's fourth case: with no job running the answer is the mode's
/// own line and nothing else — the wording a session without a job always had.
#[test]
fn a_switch_with_no_job_still_answers_only_the_mode() {
    let mut fixture = Fixture::acquire("job-none", ApprovalPolicy::Ask, false);
    let message = switch_with_job(
        &mut fixture.state,
        &mut fixture.session,
        PermissionMode::AcceptEdits,
        None,
    )
    .expect("accept-edits needs no gate");

    assert!(message.contains("Mode: accept-edits"), "{message}");
    assert!(
        !message.contains("running job"),
        "no job running, no job line: {message}"
    );
}
