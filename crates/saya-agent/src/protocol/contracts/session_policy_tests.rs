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
        ApprovalDecision::Deny { reason: None },
        "read-only must not allow a tool with an external side effect"
    );
}

#[test]
fn never_denies_everything() {
    let policy = SessionPolicy::new(ApprovalPolicy::Never);
    assert_eq!(
        policy.resolve(&read_shaped(), None),
        ApprovalDecision::Deny { reason: None }
    );
    assert_eq!(
        policy.resolve(&side_effecting(), None),
        ApprovalDecision::Deny { reason: None }
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
        ApprovalDecision::Deny { reason: None },
        "read-only does not ask, so no grant can answer for it"
    );
    let never = SessionPolicy::new(ApprovalPolicy::Never);
    assert_eq!(
        never.resolve(&read_shaped(), Some("runner:bench")),
        ApprovalDecision::Deny { reason: None },
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

/// Under `bypass` every effect resolves `Allow` — read-shaped, write-shaped,
/// and side-effecting alike — and no grant is consulted: the per-call consent
/// was given once, in the launch flag. The store stays untouched because no
/// ask ever occurs, so nothing `record` could carry exists.
#[test]
fn bypass_allows_every_effect_without_asking_or_grants() {
    let policy = SessionPolicy::new(ApprovalPolicy::Bypass);
    for (effect, label) in [
        (read_shaped(), "read-shaped"),
        (write_shaped(), "write-shaped"),
        (side_effecting(), "side-effecting"),
    ] {
        assert_eq!(
            policy.resolve(&effect, None),
            ApprovalDecision::Allow,
            "bypass allows every effect without asking: {label}"
        );
    }
    // Grants are inert under bypass — never consulted, and nothing to consult
    // with: no ask fires, so `record` never fires either.
    policy.grants().grant("workspace-write");
    assert_eq!(
        policy.resolve(&side_effecting(), Some("workspace-write")),
        ApprovalDecision::Allow,
        "bypass allows the same either way: grants are not the judge, the mode is"
    );
    assert!(
        !policy.grants().is_empty(),
        "the pre-existing grant stays held — bypass consults nothing, it revokes nothing"
    );
    assert_eq!(
        policy.resolve(&side_effecting(), None),
        ApprovalDecision::Allow,
        "an ungranted call allows identically: the grant never moved anything"
    );
}

fn write_shaped() -> ToolEffect {
    // The session's workspace_write shape: requires approval, writes the
    // workspace, no external side effect.
    ToolEffect {
        database_data: false,
        external_side_effect: false,
        requires_approval: true,
        local_state: LocalStateEffect::WriteWorkspace,
    }
}

/// The headless policy (U4: the run's decider is this engine, frozen): an
/// `Ask` the seeds do not cover cannot be answered — a headless surface has
/// no reader — so it resolves to a structured deny that names why, in the
/// words the task pins: the run's approval is its `--allow` scopes. The
/// reason is on the decision, never a bare refusal.
#[test]
fn a_frozen_ask_denies_naming_the_run_s_approval() {
    let policy = SessionPolicy::frozen(ApprovalPolicy::Ask, &[]);
    let decision = policy.resolve(&read_shaped(), None);
    let ApprovalDecision::Deny { reason } = decision else {
        panic!("a headless ask denies, got {decision:?}");
    };
    let reason = reason.expect("the headless denial names why");
    assert!(
        reason.contains("cannot prompt"),
        "the denial says why it cannot ask: {reason}"
    );
    assert!(
        reason.contains("--allow"),
        "the denial names what a run's approval is: {reason}"
    );
}

/// A seed pre-answers exactly what `--allow` stated: `--allow sql:analytics`
/// seeds the frozen policy, and a call naming that connection resolves
/// `Allow` where the unseeded ask denied. Another connection's call is
/// outside the seed and denies — grants are narrow, frozen or not.
#[test]
fn a_run_s_seeds_pre_answer_what_allow_stated() {
    let policy = SessionPolicy::frozen(ApprovalPolicy::Ask, &["sql:analytics".to_owned()]);
    assert_eq!(
        policy.resolve(&read_shaped(), Some("sql:analytics")),
        ApprovalDecision::Allow,
        "the seeded token pre-answers the call it names"
    );
    assert!(
        matches!(
            policy.resolve(&read_shaped(), Some("sql:staging")),
            ApprovalDecision::Deny { .. }
        ),
        "another connection is outside the seed: the headless ask denies"
    );
}

/// A run's policy never accumulates: no decision is an `Ask` — there is
/// nothing any frontend could answer — and `record` cannot move the store
/// even if a caller tried. The store holds exactly the seeds, forever.
#[test]
fn a_frozen_policy_cannot_accumulate_a_grant() {
    let policy = SessionPolicy::frozen(ApprovalPolicy::Ask, &["runner:bench".to_owned()]);
    for (effect, label) in [
        (read_shaped(), "read-shaped"),
        (side_effecting(), "side-effecting"),
        (write_shaped(), "write-shaped"),
    ] {
        assert!(
            !matches!(policy.resolve(&effect, None), ApprovalDecision::Ask),
            "a headless ask is unanswerable, so none is produced: {label}"
        );
    }
    assert!(
        !policy.record(ApprovalChoice::AllowSession {
            token: "interpreter:python3".into(),
        }),
        "record cannot move the store on the run surface"
    );
    assert!(
        !policy.record(ApprovalChoice::AllowOnce),
        "allow once never records, frozen or not"
    );
    assert_eq!(
        policy.grants().tokens(),
        vec!["runner:bench".to_owned()],
        "the store holds exactly the seeds — no answer could have added one"
    );
}

/// The modes that never ask are unchanged by the freeze, exactly as in a
/// session: read-only still allows exactly the read-shaped tools and denies
/// the rest, `never` denies everything — the seeds ride along inert.
#[test]
fn a_frozen_read_only_or_never_run_is_unchanged_by_its_seeds() {
    let seeds = ["runner:bench".to_owned(), "sql:analytics".to_owned()];
    let read_only = SessionPolicy::frozen(ApprovalPolicy::ReadOnly, &seeds);
    assert_eq!(
        read_only.resolve(&read_shaped(), Some("sql:analytics")),
        ApprovalDecision::Allow,
        "read-only auto-approves the read-shaped tools as before"
    );
    assert!(
        matches!(
            read_only.resolve(&side_effecting(), Some("runner:bench")),
            ApprovalDecision::Deny { .. }
        ),
        "read-only denies the rest; no seed can move it"
    );
    let never = SessionPolicy::frozen(ApprovalPolicy::Never, &seeds);
    assert!(
        matches!(
            never.resolve(&read_shaped(), Some("sql:analytics")),
            ApprovalDecision::Deny { .. }
        ),
        "never is a standing refusal under the freeze too"
    );
}
