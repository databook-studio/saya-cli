//! The `bypass` gate and the frozen cell, on the running job the switch moves:
//! a mode reaches the job's cell only where the configuration allows it, and a
//! frozen job answers every mode in the one frozen sentence.

use saya_agent::{ApprovalPolicy, PermissionMode};

use super::super::{BYPASS_REFUSAL, FROZEN_REFUSAL, switch_with_job};
use super::support::Fixture;
use crate::agent::tools::shell_policy::ModeCell;

/// Where the configuration allows `bypass`, the switch states it on the job's
/// cell along with the session's: the job runs under the mode the user chose,
/// not one step behind it.
#[test]
fn an_allowed_bypass_switch_reaches_the_job() {
    let mut fixture = Fixture::acquire("gate-allowed", ApprovalPolicy::Ask, true);
    let cell = ModeCell::new(ApprovalPolicy::Ask);
    let message = switch_with_job(
        &mut fixture.state,
        &mut fixture.session,
        PermissionMode::Bypass,
        Some(&cell),
    )
    .expect("the configuration allows bypass");

    assert_eq!(
        cell.get(),
        ApprovalPolicy::Bypass,
        "the job's next call reads the mode the session took"
    );
    assert!(
        message.contains("The running job switched too"),
        "the answer says the job got it: {message}"
    );
}

/// Where the configuration withholds it, the same switch refuses in the
/// session's own words and leaves the job's cell exactly as it found it: the
/// gate is one check, ahead of both surfaces.
#[test]
fn a_withheld_bypass_leaves_the_job_cell_alone() {
    let mut fixture = Fixture::acquire("gate-withheld", ApprovalPolicy::Ask, false);
    let cell = ModeCell::new(ApprovalPolicy::Ask);
    let error = switch_with_job(
        &mut fixture.state,
        &mut fixture.session,
        PermissionMode::Bypass,
        Some(&cell),
    )
    .expect_err("bypass is withheld");

    assert_eq!(error, BYPASS_REFUSAL, "one refusal, worded once");
    assert_eq!(
        cell.get(),
        ApprovalPolicy::Ask,
        "a mode the session refuses never reaches the job"
    );
    assert_eq!(fixture.state.approval_mode, "ask");
}

/// A frozen job — the cell a detached `-p`, `start`, `follow-up` or
/// `job resume` composes — refuses every mode, not only the one the other
/// tests happen to try, and says so in the session's frozen sentence.
#[test]
fn every_mode_is_refused_on_a_frozen_job_in_the_one_frozen_wording() {
    let mut fixture = Fixture::acquire("gate-frozen", ApprovalPolicy::Ask, true);
    let cell = ModeCell::new(ApprovalPolicy::Ask);
    cell.freeze_at(ApprovalPolicy::Ask);

    for mode in [
        PermissionMode::Ask,
        PermissionMode::AcceptEdits,
        PermissionMode::Plan,
        PermissionMode::Bypass,
    ] {
        let message = switch_with_job(&mut fixture.state, &mut fixture.session, mode, Some(&cell))
            .expect("the session's own policy is not frozen");
        assert!(
            message.contains(FROZEN_REFUSAL),
            "{mode:?} must be refused in the frozen wording: {message}"
        );
        assert_eq!(
            cell.get(),
            ApprovalPolicy::Ask,
            "{mode:?} must leave the frozen job where it was"
        );
    }
}
