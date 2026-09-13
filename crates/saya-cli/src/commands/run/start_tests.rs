//! The run boundary's start refusals: the fresh-run entry's mode guard.
//! The missing-`--allow` refusal is pinned through the drive (`approval_tests`,
//! `grants_tests`); the bypass refusal sits beside it and is pinned here.

use super::start::refuse_bypass_mode;
use saya_agent::ApprovalPolicy;

/// A run refuses the bypass mode at start, naming what a run's approval
/// actually is (DESIGN §6, test 16): a run's approval is its `--allow`
/// scopes — typed, per-capability, journaled — and bypass is a session
/// mode's blanket per-call consent, which a run has no per-call ask to
/// replace. Every other mode starts.
#[test]
fn a_run_refuses_the_bypass_mode_naming_allow() {
    for refusal in [
        ApprovalPolicy::ReadOnly,
        ApprovalPolicy::Never,
        ApprovalPolicy::Ask,
    ] {
        assert!(
            refuse_bypass_mode(refusal).is_ok(),
            "{refusal:?} starts: a run's mode is its own decision surface"
        );
    }
    let error = refuse_bypass_mode(ApprovalPolicy::Bypass).expect_err("bypass never starts a run");
    assert!(
        error.to_string().contains("--allow"),
        "the refusal names the run's real approval: {error}"
    );
    assert!(
        error.to_string().contains("session mode"),
        "the refusal names where bypass does live: {error}"
    );
}
