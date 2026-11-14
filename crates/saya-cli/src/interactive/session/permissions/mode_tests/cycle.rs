//! The keyboard cycle from the session's own pair: what Shift+Tab moves to,
//! what it skips, and where a pair with no mode of its own enters.

use saya_agent::{ApprovalPolicy, PermissionMode};

use super::super::{cycle_from, switch};
use super::support::Fixture;

/// Shift+Tab walks `ask → accept-edits → plan → ask` where the configuration
/// withholds `bypass`: three presses return the session to the pair it
/// started on, and the withheld mode is never one of them. Each step is a
/// real switch, so the state the next step reads is the one the previous
/// step left.
#[test]
fn the_cycle_walks_the_reachable_modes_and_closes() {
    let mut fixture = Fixture::acquire("cycle-withheld", ApprovalPolicy::Ask, false);
    for expected in [
        PermissionMode::AcceptEdits,
        PermissionMode::Plan,
        PermissionMode::Ask,
    ] {
        let next = cycle_from(&fixture.state);
        assert_eq!(next, expected, "the cycle's next member");
        switch(&mut fixture.state, &mut fixture.session, next)
            .expect("every reachable member switches");
    }
    assert_eq!(
        (
            fixture.state.approval_mode.as_str(),
            fixture.state.agent_mode.as_str()
        ),
        ("ask", "build"),
        "the walk closed on the pair it started from"
    );
}

/// Where the configuration allows `bypass`, the cycle closes through it —
/// and the press that reaches it switches exactly like `/mode bypass`, the
/// same one operation.
#[test]
fn the_cycle_reaches_bypass_only_when_the_configuration_allows_it() {
    let mut fixture = Fixture::acquire("cycle-allowed", ApprovalPolicy::Ask, true);
    for expected in [
        PermissionMode::AcceptEdits,
        PermissionMode::Plan,
        PermissionMode::Bypass,
        PermissionMode::Ask,
    ] {
        let next = cycle_from(&fixture.state);
        assert_eq!(next, expected, "the cycle's next member");
        switch(&mut fixture.state, &mut fixture.session, next)
            .expect("the configuration allows every member");
    }
    assert_eq!(fixture.state.approval_mode, "ask");
}

/// A pair with no mode of its own — `read-only`, set by `/approvals` — is
/// not a cycle member: the press enters at `ask`, which asks for every call,
/// rather than landing on a mode the user never walked through.
#[test]
fn a_pair_with_no_mode_of_its_own_enters_the_cycle_at_ask() {
    let fixture = Fixture::acquire("cycle-read-only", ApprovalPolicy::ReadOnly, false);
    assert_eq!(cycle_from(&fixture.state), PermissionMode::Ask);
}

/// The projection the cycle reads is the one the bar reads: a session under
/// `never` has no mode either, and `plan` is the mode whatever consent sits
/// beside it.
#[test]
fn the_projection_reads_the_pair() {
    let mut fixture = Fixture::acquire("cycle-projection", ApprovalPolicy::Ask, false);
    assert_eq!(
        super::super::current(&fixture.state),
        Some(PermissionMode::Ask)
    );

    fixture.state.approval_mode = "never".into();
    assert_eq!(super::super::current(&fixture.state), None);

    fixture.state.approval_mode = "bypass".into();
    fixture.state.agent_mode = "plan".into();
    assert_eq!(
        super::super::current(&fixture.state),
        Some(PermissionMode::Plan),
        "the task posture is read first"
    );
}
