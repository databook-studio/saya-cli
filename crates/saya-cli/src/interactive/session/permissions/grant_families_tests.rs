//! The remaining families' own tests: the per-family availability
//! decision table, read through [`crate::grant_token::grant_family`]'s
//! words — the held-token arm, the standing rule, and the postures that
//! never widen.

use saya_agent::{AgentMode, ApprovalPolicy, SessionGrants};

use super::{command_available, fetch_available};

/// A store with `tokens` granted.
fn granted(tokens: &[&str]) -> SessionGrants {
    let grants = SessionGrants::default();
    for token in tokens {
        grants.grant(token);
    }
    grants
}

/// Every family's own token under `ask`, on every posture — the shared
/// shape the four decisions pin: the standing surface rule, the held-token
/// arm headless, and nowhere else, whatever else is held.
fn the_family_table(
    family_tokens: &[&str],
    foreign_tokens: &[&str],
    available: fn(AgentMode, ApprovalPolicy, bool, Option<&SessionGrants>) -> bool,
) {
    let family = granted(family_tokens);
    let foreign = granted(foreign_tokens);
    for (agent_mode, mode, can_obtain_approval, grants, expected, label) in [
        (
            AgentMode::Build,
            ApprovalPolicy::Ask,
            true,
            None,
            true,
            "ask with a surface",
        ),
        (
            AgentMode::Build,
            ApprovalPolicy::Ask,
            false,
            Some(&family),
            true,
            "headless ask with the family's grant",
        ),
        (
            AgentMode::Build,
            ApprovalPolicy::Ask,
            false,
            None,
            false,
            "headless ask with no grant",
        ),
        (
            AgentMode::Build,
            ApprovalPolicy::Ask,
            false,
            Some(&foreign),
            false,
            "headless ask with only foreign families' grants",
        ),
        (
            AgentMode::Build,
            ApprovalPolicy::Ask,
            true,
            Some(&family),
            true,
            "ask with a surface and the grant",
        ),
        (
            AgentMode::Build,
            ApprovalPolicy::Bypass,
            false,
            None,
            true,
            "bypass, no surface, no grant",
        ),
        (
            AgentMode::Build,
            ApprovalPolicy::Bypass,
            false,
            Some(&family),
            true,
            "bypass with the grant",
        ),
        (
            AgentMode::Build,
            ApprovalPolicy::ReadOnly,
            true,
            Some(&family),
            false,
            "read-only never consults grants",
        ),
        (
            AgentMode::Build,
            ApprovalPolicy::Never,
            true,
            Some(&family),
            false,
            "never never consults grants",
        ),
        (
            AgentMode::Plan,
            ApprovalPolicy::Ask,
            true,
            Some(&family),
            false,
            "plan denies before the approval match",
        ),
        (
            AgentMode::Plan,
            ApprovalPolicy::Bypass,
            false,
            Some(&family),
            false,
            "plan hides even under bypass",
        ),
    ] {
        assert_eq!(
            available(agent_mode, mode, can_obtain_approval, grants),
            expected,
            "{label}"
        );
    }
}

/// The fetch family: the `fetch:` prefix group opens it; a command,
/// workspace, or SQL token does not.
#[test]
fn the_fetch_family_advertises_exactly_where_a_call_can_run() {
    the_family_table(
        &["fetch:https+docs.example"],
        &["command:cargo", "workspace-write", "sql:docs"],
        fetch_available,
    );
}

/// The command family: the `command:` prefix group opens it; no other
/// family's token does.
#[test]
fn the_command_family_advertises_exactly_where_a_call_can_run() {
    the_family_table(
        &["command:cargo"],
        &["fetch:https+docs.example", "workspace-write", "sql:docs"],
        command_available,
    );
}

/// The words come from the grammar's own classifier: a held token the
/// classifier places in a family is the family's key, so a family's
/// decision cannot drift from what `grant_token` records. Pinned one way
/// per family: the classifier's word for each token matches the decision's
/// key.
#[test]
fn the_family_keys_are_the_grammar_classifier_s_own_words() {
    use crate::grant_token::grant_family;
    assert_eq!(grant_family("fetch:https+docs.example"), Some("fetch"));
    assert_eq!(grant_family("runner:touch"), Some("runner"));
    assert_eq!(grant_family("interpreter:python3"), Some("runner"));
    assert_eq!(grant_family("command:cargo"), Some("command"));
    assert_eq!(grant_family("workspace-write"), Some("workspace-write"));
    assert_eq!(grant_family("sql:docs"), Some("SQL"));
    assert_eq!(grant_family("unknown-thing"), None);
}
