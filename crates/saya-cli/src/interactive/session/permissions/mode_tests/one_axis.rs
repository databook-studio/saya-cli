//! The consent axis as the switch sees it: `/mode read-only` and `/mode never`
//! set their postures, the `/approvals` alias runs the same switch and adds its
//! deprecation note once, and the bypass gate answers both spellings in the
//! same words.

use saya_agent::{ApprovalPolicy, PermissionMode};

use super::super::alias::APPROVALS_ALIAS_NOTE;
use super::super::{BYPASS_REFUSAL, switch_requested};
use super::support::Fixture;
use crate::slash::ModeRequest;

/// The request `/mode <value>` builds.
fn mode(value: PermissionMode) -> ModeRequest {
    ModeRequest {
        value: Some(value),
        ..Default::default()
    }
}

/// The request `/approvals <value>` builds.
fn alias(value: PermissionMode) -> ModeRequest {
    ModeRequest {
        value: Some(value),
        via_approvals_alias: true,
    }
}

/// `/mode read-only` sets the read-only consent posture: the session's own
/// cell, its task posture back to `build`, and an answer that names the mode.
#[test]
fn mode_read_only_sets_the_read_only_posture() {
    let mut fixture = Fixture::acquire("axis-read-only", ApprovalPolicy::Ask, true);
    let message = switch_requested(
        &mut fixture.state,
        &mut fixture.session,
        mode(PermissionMode::ReadOnly),
        None,
    )
    .expect("read-only needs no bypass gate");

    assert_eq!(fixture.state.approval_mode, "read-only");
    assert_eq!(fixture.state.agent_mode, "build");
    assert_eq!(fixture.session.policy().mode(), ApprovalPolicy::ReadOnly);
    assert!(
        message.starts_with("Mode: read-only"),
        "the answer names the mode it set: {message}"
    );
    assert!(
        !message.contains(APPROVALS_ALIAS_NOTE),
        "the canonical spelling owes no deprecation note: {message}"
    );
}

/// `/mode never` likewise: the posture is `never`, and nothing prompts.
#[test]
fn mode_never_sets_the_never_posture() {
    let mut fixture = Fixture::acquire("axis-never", ApprovalPolicy::Ask, true);
    let message = switch_requested(
        &mut fixture.state,
        &mut fixture.session,
        mode(PermissionMode::Never),
        None,
    )
    .expect("never needs no bypass gate");

    assert_eq!(fixture.state.approval_mode, "never");
    assert_eq!(fixture.state.agent_mode, "build");
    assert_eq!(fixture.session.policy().mode(), ApprovalPolicy::Never);
    assert!(
        message.starts_with("Mode: never"),
        "the answer names the mode it set: {message}"
    );
}

/// `/approvals never` still works, and says once that it is an alias: the
/// posture moves through the same switch, and the note appears exactly once.
#[test]
fn approvals_never_works_and_notes_the_alias_once() {
    let mut fixture = Fixture::acquire("axis-alias", ApprovalPolicy::Ask, true);
    let message = switch_requested(
        &mut fixture.state,
        &mut fixture.session,
        alias(PermissionMode::Never),
        None,
    )
    .expect("the alias runs the same switch");

    assert_eq!(
        fixture.state.approval_mode, "never",
        "the alias sets the same posture the canonical spelling does"
    );
    assert_eq!(
        message.matches(APPROVALS_ALIAS_NOTE).count(),
        1,
        "the deprecation note is said once: {message}"
    );
    assert!(
        message.starts_with("Mode: never"),
        "the answer is the one switch's answer: {message}"
    );
}

/// `/mode bypass` without `allow_bypass` is refused in today's exact wording,
/// through both spellings: the gate is one check, ahead of the switch.
#[test]
fn mode_bypass_without_allow_bypass_is_refused_in_todays_words() {
    const PINNED: &str = "bypass is off: `[permissions] allow_bypass = true` in your user \
                          configuration is the only way to allow it (a project's \
                          `.saya/config.toml` cannot).";

    let mut fixture = Fixture::acquire("axis-gate", ApprovalPolicy::Ask, false);
    let refusal = switch_requested(
        &mut fixture.state,
        &mut fixture.session,
        mode(PermissionMode::Bypass),
        None,
    )
    .expect_err("the configuration withholds bypass");
    assert_eq!(refusal, PINNED, "one refusal, worded once");
    assert_eq!(refusal, BYPASS_REFUSAL);
    assert_eq!(
        fixture.state.approval_mode, "ask",
        "a refused switch leaves the posture as it found it"
    );

    // The alias is gated by the same check, and its note rides the refusal.
    let refusal = switch_requested(
        &mut fixture.state,
        &mut fixture.session,
        alias(PermissionMode::Bypass),
        None,
    )
    .expect_err("the alias is no way around the gate");
    assert!(
        refusal.starts_with(PINNED),
        "the alias answers the gate's own words first: {refusal}"
    );
    assert_eq!(refusal.matches(APPROVALS_ALIAS_NOTE).count(), 1);
    assert_eq!(fixture.state.approval_mode, "ask");
}
