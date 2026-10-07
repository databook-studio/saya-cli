//! The grant surface's own tests: the per-family availability decision
//! table, and the per-turn grant view's bind/share/close semantics.

use saya_agent::{AgentMode, ApprovalPolicy, SessionGrants};

use super::{GrantView, workspace_files_available};

/// A store with `token` granted.
fn granted(token: &str) -> SessionGrants {
    let grants = SessionGrants::default();
    grants.grant(token);
    grants
}

/// The decision table: the family advertises exactly where a call can run
/// — an approval surface under ask, or bypass, or an explicit
/// `workspace-write` grant under ask — and nowhere else, whatever is held.
#[test]
fn the_workspace_file_family_advertises_exactly_where_a_call_can_run() {
    let workspace_write = granted("workspace-write");
    let scratch = granted("scratch");
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
            Some(&workspace_write),
            true,
            "headless ask with the grant",
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
            Some(&scratch),
            false,
            "headless ask with a foreign family's grant",
        ),
        (
            AgentMode::Build,
            ApprovalPolicy::Ask,
            true,
            Some(&workspace_write),
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
            Some(&workspace_write),
            true,
            "bypass with the grant",
        ),
        (
            AgentMode::Build,
            ApprovalPolicy::ReadOnly,
            true,
            Some(&workspace_write),
            false,
            "read-only never consults grants",
        ),
        (
            AgentMode::Build,
            ApprovalPolicy::Never,
            true,
            Some(&workspace_write),
            false,
            "never never consults grants",
        ),
        (
            AgentMode::Plan,
            ApprovalPolicy::Ask,
            true,
            Some(&workspace_write),
            false,
            "plan denies before the approval match",
        ),
        (
            AgentMode::Plan,
            ApprovalPolicy::Bypass,
            false,
            Some(&workspace_write),
            false,
            "plan hides even under bypass",
        ),
    ] {
        assert_eq!(
            workspace_files_available(agent_mode, mode, can_obtain_approval, grants),
            expected,
            "{label}"
        );
    }
}

/// The view starts closed and binds per turn: unbound is `None`, a bind
/// hands back the store, and — the property the whole design rests on —
/// the bound clone shares the one `Arc`-backed set, so a grant recorded
/// through the policy after the bind is visible through the view.
#[test]
fn the_grant_view_shares_the_live_store_and_starts_closed() {
    let view = GrantView::default();
    assert!(view.get().is_none(), "unbound: the closed shape");
    let policy = saya_agent::SessionPolicy::new(ApprovalPolicy::Ask);
    policy.grants().grant("workspace-write");
    view.bind(policy.grants());
    let bound = view.get().expect("bound after a bind");
    assert!(bound.is_granted("workspace-write"));
    // A later grant through the same policy lands in the same set.
    policy.grants().grant("scratch");
    let rebound = view.get().expect("still bound");
    assert!(
        rebound.is_granted("scratch"),
        "the bound clone shares the live store, never a stale copy"
    );
    // A rebind swaps the store wholesale.
    let fresh = SessionGrants::default();
    view.bind(&fresh);
    let swapped = view.get().expect("bound after the rebind");
    assert!(
        !swapped.is_granted("workspace-write"),
        "a rebind replaces the store"
    );
}
