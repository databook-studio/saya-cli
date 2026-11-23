//! Twin of `mode_tests/switch.rs::leaving_plan_re_arms_the_gate`, deleted with
//! the old universe: the session's one plan gate is closed when the mode walks
//! out of `plan`, so re-entering the posture asks the operator again.
//!
//! The gate engine's own contract is pinned in
//! `saya-agent/protocol/contracts/session_policy_tests/plan_gate.rs` ("closing
//! the gate re-arms the denial"). This file pins the other half — that the
//! switch performs the close on the session's own gate, the cell the operator's
//! approval opened.

use saya_agent::{ApprovalPolicy, PermissionMode};

use super::super::switch_with_job;
use super::support::Fixture;

#[test]
fn leaving_plan_closes_the_sessions_plan_gate() {
    let mut fixture = Fixture::acquire("plan-rearm", ApprovalPolicy::Ask, false);
    let gate = fixture.session.universe().plan_gate();
    gate.cell().open();
    assert!(
        gate.is_open(),
        "the operator's approval opened the session's gate"
    );

    let message = switch_with_job(
        &mut fixture.state,
        &mut fixture.session,
        PermissionMode::Ask,
        None,
    )
    .expect("ask needs no gate");
    assert!(!gate.is_open(), "leaving plan re-arms the gate: {message}");

    // Re-entering the posture does not carry the old approval back in: the next
    // submission is a fresh question.
    switch_with_job(
        &mut fixture.state,
        &mut fixture.session,
        PermissionMode::Plan,
        None,
    )
    .expect("plan needs no gate");
    assert!(!gate.is_open(), "re-entering plan asks the operator again");
}
