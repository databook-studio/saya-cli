//! The engine's decisions, cell by cell: the mode match every approval
//! frontend used to carry, and the grant seam that pre-answers an ask.
//! Nothing grants a token yet, so the grant tests drive the store directly —
//! the seam must work before any frontend asks its question.

use super::session_policy::{ApprovalChoice, ApprovalDecision, SessionPolicy};
use super::{LocalStateEffect, ToolEffect};
use crate::protocol::approval::ApprovalPolicy;

fn read_shaped() -> ToolEffect {
    // The SQL tools' shape: requires approval, but no side effect and no
    // local-state write — the cell read-only must keep auto-approving.
    ToolEffect {
        database_data: true,
        external_side_effect: false,
        requires_approval: true,
        local_state: LocalStateEffect::None,
    }
}

fn side_effecting() -> ToolEffect {
    ToolEffect {
        database_data: false,
        external_side_effect: true,
        requires_approval: true,
        local_state: LocalStateEffect::None,
    }
}

#[test]
fn read_only_allows_read_shaped_and_denies_the_rest() {
    let policy = SessionPolicy::new(ApprovalPolicy::ReadOnly);
    assert_eq!(
        policy.resolve(&read_shaped(), None),
        ApprovalDecision::Allow,
        "the SQL tools stay auto-approved under read-only"
    );
    assert_eq!(
        policy.resolve(&side_effecting(), None),
        ApprovalDecision::Deny,
        "read-only must not allow a tool with an external side effect"
    );
}

#[test]
fn never_denies_everything() {
    let policy = SessionPolicy::new(ApprovalPolicy::Never);
    assert_eq!(policy.resolve(&read_shaped(), None), ApprovalDecision::Deny);
    assert_eq!(
        policy.resolve(&side_effecting(), None),
        ApprovalDecision::Deny
    );
}

/// Under `ask` every call goes to the frontend to render — including the
/// read-shaped SQL tools, which both interactive frontends have always
/// asked about.
#[test]
fn ask_hands_every_call_to_the_frontend() {
    let policy = SessionPolicy::new(ApprovalPolicy::Ask);
    assert_eq!(policy.resolve(&read_shaped(), None), ApprovalDecision::Ask);
    assert_eq!(
        policy.resolve(&side_effecting(), None),
        ApprovalDecision::Ask
    );
}

/// A session grant pre-answers the ask: once recorded, the same call
/// resolves to allow with no further prompting, and the store is shared by
/// clones of the policy, as one store must be across a session's deciders.
#[test]
fn a_session_grant_pre_answers_the_ask() {
    let policy = SessionPolicy::new(ApprovalPolicy::Ask);
    let granted = policy.record(ApprovalChoice::AllowSession {
        token: "runner:bench".into(),
    });
    assert!(granted, "the first grant for a token is new");
    assert_eq!(
        policy.resolve(&side_effecting(), Some("runner:bench")),
        ApprovalDecision::Allow,
        "the granted token allows the call it was granted for"
    );
    assert_eq!(
        policy.resolve(&side_effecting(), None),
        ApprovalDecision::Ask,
        "without the token named, the call still asks"
    );
    assert_eq!(
        policy.resolve(&side_effecting(), Some("runner:other")),
        ApprovalDecision::Ask,
        "a different token is not covered: grants are narrow"
    );
    let shared = policy.clone();
    assert!(
        shared.grants().is_granted("runner:bench"),
        "clones share the one store"
    );
    assert_eq!(
        shared.grants().tokens(),
        vec!["runner:bench".to_string()],
        "the listing reads the same store"
    );
}

/// The modes that never ask are untouched by grants: `read-only` denies a
/// side-effecting call even when a token is named, and `never` denies
/// everything, granted or not.
#[test]
fn grants_cannot_move_read_only_or_never() {
    let granted = SessionPolicy::new(ApprovalPolicy::Ask);
    granted.record(ApprovalChoice::AllowSession {
        token: "runner:bench".into(),
    });
    let read_only = SessionPolicy::new(ApprovalPolicy::ReadOnly);
    assert_eq!(
        read_only.resolve(&side_effecting(), Some("runner:bench")),
        ApprovalDecision::Deny,
        "read-only does not ask, so no grant can answer for it"
    );
    let never = SessionPolicy::new(ApprovalPolicy::Never);
    assert_eq!(
        never.resolve(&read_shaped(), Some("runner:bench")),
        ApprovalDecision::Deny,
        "never is a standing refusal"
    );
}

/// "Allow once" and "deny" leave no state behind: the store stays empty, so
/// the next call of the same shape asks again.
#[test]
fn allow_once_and_deny_leave_no_state() {
    let policy = SessionPolicy::new(ApprovalPolicy::Ask);
    assert!(!policy.record(ApprovalChoice::AllowOnce));
    assert!(!policy.record(ApprovalChoice::Deny));
    assert!(policy.grants().is_empty(), "nothing was granted");
    assert_eq!(
        policy.resolve(&side_effecting(), None),
        ApprovalDecision::Ask,
        "an answered ask does not pre-answer the next one"
    );
}
