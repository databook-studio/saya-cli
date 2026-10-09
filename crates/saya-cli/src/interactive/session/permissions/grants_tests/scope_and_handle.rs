/// `/allow none` keeps the grammar's meaning — the empty approval, alone —
/// and seeds nothing, saying so. It is not a revoke: whatever the session
/// already holds stays held.
#[test]
fn allow_none_seeds_nothing_and_says_so() {
    let grants = store_with("runner:bench");
    let journal = SessionJournal::open(state_dir("none"));
    let message = allow(&["none".to_owned()], &composed_facts(), &grants, &journal)
        .expect("`none` parses on the session surface");
    assert!(
        message.contains("nothing"),
        "the message says nothing was seeded: {message}"
    );
    assert!(
        grants.is_granted("runner:bench"),
        "`/allow none` is not a revoke — the store keeps what it holds"
    );
    assert_eq!(
        grants.tokens(),
        vec!["runner:bench".to_owned()],
        "exactly the pre-existing grant, nothing more"
    );
}

/// A scope the session surface refuses is a usage error, and the store
/// keeps exactly what it had: the refusal never half-seeds.
#[test]
fn allow_refuses_a_scope_the_session_surface_refuses() {
    let grants = store_with("runner:bench");
    let journal = SessionJournal::open(state_dir("refuse"));
    let error = allow(
        &["endpoint:analyst=fast".to_owned()],
        &composed_facts(),
        &grants,
        &journal,
    )
    .expect_err("a session binds no per-step endpoint roles");
    assert!(
        error.contains("binds no per-step endpoint roles"),
        "the refusal is the session surface's own reason: {error}"
    );
    assert_eq!(
        grants.tokens(),
        vec!["runner:bench".to_owned()],
        "a refused /allow seeds nothing"
    );

    let error = allow(
        &["sql:bad name".to_owned()],
        &composed_facts(),
        &SessionGrants::default(),
        &journal,
    )
    .expect_err("a non-name-shaped payload is a usage error");
    assert!(
        error.contains("sql:<connection>"),
        "the refusal names the grammar: {error}"
    );

    let error = allow(
        &["wat".to_owned()],
        &composed_facts(),
        &SessionGrants::default(),
        &journal,
    )
    .expect_err("an unknown scope is a usage error");
    assert!(error.contains("unknown scope `wat`"), "got: {error}");
}

/// The seeded grant is the session's grant in force: a policy over the
/// seeded store pre-answers the call shape the grant names — the seed is
/// the same fact the [s] answer records, from the other side.
#[test]
fn a_seeded_grant_pre_answers_like_a_prompted_one() {
    let policy = SessionPolicy::new(ApprovalPolicy::Ask);
    allow(
        &["sql:analytics".to_owned()],
        &composed_facts(),
        policy.grants(),
        &SessionJournal::open(state_dir("pre-answer")),
    )
    .expect("the session surface accepts the scope");
    let effect = saya_agent::ToolEffect {
        database_data: true,
        external_side_effect: false,
        requires_approval: true,
        local_state: saya_agent::LocalStateEffect::None,
    };
    assert!(
        policy.resolve(&effect, Some("sql:analytics")) == saya_agent::ApprovalDecision::Allow,
        "the seeded grant pre-answers the sql call it names"
    );
    assert!(
        policy.resolve(&effect, Some("sql:staging")) == saya_agent::ApprovalDecision::Ask,
        "and nothing on a different connection"
    );
}

/// The primary handle is the suggester's fact about the turn: bound from
/// the registry, it names the primary's real registry name — never a magic
/// word — and an empty registry binds nothing.
#[test]
fn the_primary_handle_names_the_registry_s_primary() {
    let primary = TurnPrimary::default();
    assert_eq!(primary.get(), None, "unbound, nothing to suggest");
    primary.bind(&registry_with_primary("analytics"));
    assert_eq!(primary.get().as_deref(), Some("analytics"));
    primary.bind(&crate::connection::ConnectionRegistry::new(""));
    assert_eq!(
        primary.get(),
        None,
        "an empty registry resolves no primary, so the handle binds nothing"
    );
}
