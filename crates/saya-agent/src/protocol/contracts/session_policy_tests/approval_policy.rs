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

/// The session-history fact: every call a grant pre-answers increments the
/// token's allowed-call count — the number a prompt's session-history line
/// reads ("3 calls so far"). Allow-once, denies, and unanswered asks count
/// nothing: only grant-answered calls do.
#[test]
fn each_call_a_grant_answers_counts_under_its_token() {
    let policy = SessionPolicy::new(ApprovalPolicy::Ask);
    policy.record(ApprovalChoice::AllowSession {
        token: "sql:analytics".into(),
    });
    assert_eq!(
        policy.grants().calls("sql:analytics"),
        0,
        "a grant records no calls until one runs"
    );
    assert_eq!(
        policy.resolve(&read_shaped(), Some("sql:analytics")),
        ApprovalDecision::Allow
    );
    assert_eq!(
        policy.resolve(&read_shaped(), Some("sql:analytics")),
        ApprovalDecision::Allow
    );
    assert_eq!(
        policy.grants().calls("sql:analytics"),
        2,
        "two grant-answered calls, counted twice"
    );
    assert_eq!(
        policy.grants().calls("sql:staging"),
        0,
        "tokens count separately"
    );
    // An unanswered ask counts nothing, and neither does allow-once.
    assert_eq!(
        policy.resolve(&read_shaped(), Some("sql:staging")),
        ApprovalDecision::Ask
    );
    assert_eq!(
        policy.grants().calls("sql:staging"),
        0,
        "a token nothing ran under stays at zero"
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
