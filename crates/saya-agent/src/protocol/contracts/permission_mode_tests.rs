//! The session-facing projection's own tests: every mode decomposes into the
//! engine's two axes, and every pair the modes produce projects back to the
//! mode it came from.

use super::{AgentMode, PermissionMode};
use crate::protocol::approval::ApprovalPolicy;

/// The table, read in both directions: `accept-edits` and `bypass` set a
/// consent posture of their own, `plan` sets none — it is a task posture, and
/// the consent it found is the consent it leaves — and every mode names the
/// task posture it sets.
#[test]
fn every_mode_decomposes_into_the_engine_s_two_axes() {
    assert_eq!(
        PermissionMode::Ask.decompose(),
        (Some(ApprovalPolicy::Ask), AgentMode::Build)
    );
    assert_eq!(
        PermissionMode::AcceptEdits.decompose(),
        (Some(ApprovalPolicy::AcceptEdits), AgentMode::Build)
    );
    assert_eq!(
        PermissionMode::Plan.decompose(),
        (None, AgentMode::Plan),
        "plan leaves the consent posture exactly as it found it"
    );
    assert_eq!(
        PermissionMode::Bypass.decompose(),
        (Some(ApprovalPolicy::Bypass), AgentMode::Build)
    );
}

/// `of` reads the task axis first: a Plan session projects to `plan` whatever
/// its consent posture is, because the write surface refuses before the
/// approval match under every policy.
#[test]
fn of_reads_plan_first_and_the_consent_posture_otherwise() {
    for policy in [
        ApprovalPolicy::Ask,
        ApprovalPolicy::ReadOnly,
        ApprovalPolicy::Never,
        ApprovalPolicy::Bypass,
        ApprovalPolicy::AcceptEdits,
    ] {
        assert_eq!(
            PermissionMode::of(policy, AgentMode::Plan),
            Some(PermissionMode::Plan),
            "the task posture decides first, under {policy:?}"
        );
    }
    assert_eq!(
        PermissionMode::of(ApprovalPolicy::Ask, AgentMode::Build),
        Some(PermissionMode::Ask)
    );
    assert_eq!(
        PermissionMode::of(ApprovalPolicy::AcceptEdits, AgentMode::Build),
        Some(PermissionMode::AcceptEdits)
    );
    assert_eq!(
        PermissionMode::of(ApprovalPolicy::Bypass, AgentMode::Build),
        Some(PermissionMode::Bypass)
    );
}

/// `read-only` and `never` are consent postures with no mode of their own — a
/// pair carrying one projects to nothing, so a bar keeps showing the pair
/// rather than a mode the user never chose.
#[test]
fn read_only_and_never_project_to_no_mode() {
    assert_eq!(
        PermissionMode::of(ApprovalPolicy::ReadOnly, AgentMode::Build),
        None
    );
    assert_eq!(
        PermissionMode::of(ApprovalPolicy::Never, AgentMode::Build),
        None
    );
}

/// The round trip: a mode's own pair projects back to that mode. `plan` names
/// no consent posture, so it is read back from the one the session had.
#[test]
fn the_modes_round_trip_through_their_pair() {
    for mode in [
        PermissionMode::Ask,
        PermissionMode::AcceptEdits,
        PermissionMode::Plan,
        PermissionMode::Bypass,
    ] {
        let (policy, agent_mode) = mode.decompose();
        let policy = policy.unwrap_or(ApprovalPolicy::Ask);
        assert_eq!(
            PermissionMode::of(policy, agent_mode),
            Some(mode),
            "{mode:?} does not survive its own pair"
        );
    }
}
