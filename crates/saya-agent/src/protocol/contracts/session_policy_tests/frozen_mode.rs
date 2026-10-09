/// The default is Build: a policy built the old way resolves every effect
/// exactly as it does on `release/0.4.1` — `Ask` hands every call to the
/// frontend, `ReadOnly` allows exactly reads, `Never` denies, and `Bypass`
/// allows.
#[test]
fn the_default_mode_is_build_and_resolves_as_before() {
    assert_eq!(AgentMode::default(), AgentMode::Build);
    for policy in [
        ApprovalPolicy::Ask,
        ApprovalPolicy::ReadOnly,
        ApprovalPolicy::Never,
        ApprovalPolicy::Bypass,
    ] {
        let built = SessionPolicy::new(policy);
        assert_eq!(
            built.agent_mode(),
            AgentMode::Build,
            "the old constructor defaults to Build under {policy:?}"
        );
        for (effect, label) in [
            (read_shaped(), "read-shaped"),
            (write_shaped(), "workspace-write"),
            (external_effect(), "external side effect"),
        ] {
            let expected = match policy {
                ApprovalPolicy::Ask => ApprovalDecision::Ask,
                ApprovalPolicy::ReadOnly if label == "read-shaped" => ApprovalDecision::Allow,
                ApprovalPolicy::ReadOnly | ApprovalPolicy::Never => {
                    ApprovalDecision::Deny { reason: None }
                }
                ApprovalPolicy::Bypass => ApprovalDecision::Allow,
            };
            assert_eq!(
                built.resolve(&effect, None),
                expected,
                "Build resolves as before under {policy:?}: {label}"
            );
        }
    }
}

/// The mode vocabulary parses and names itself: `plan` and `build` round-trip
/// through the same spelling every surface renders.
#[test]
fn agent_mode_parses_and_names_itself() {
    assert_eq!(AgentMode::from_str("plan"), Ok(AgentMode::Plan));
    assert_eq!(AgentMode::from_str("build"), Ok(AgentMode::Build));
    assert_eq!(AgentMode::Build.as_str(), "build");
    assert_eq!(AgentMode::Plan.as_str(), "plan");
    assert!(
        AgentMode::from_str("readonly").is_err(),
        "consent spellings are not task spellings"
    );
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
