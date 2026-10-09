/// The `/allow` runner-family refusal for a program this session composed
/// no runner for: still a usage error (U8 stands — a grant the composition
/// cannot honour seeds nothing), and it now names the fallback: host
/// commands still run, they just ask for approval. `bench` — not
/// `python3` — is the program here: a name the grammar itself recognizes
/// as a shell or interpreter is refused before composition is ever
/// consulted (`scopes::parse`'s own family mirror), which is a different,
/// already-said reason than the one this test pins.
#[test]
fn slash_allow_runner_without_a_runner_says_so() {
    let dir = state_dir("allow-runner-no-runner");
    let grants = SessionGrants::default();
    let journal = SessionJournal::open(&dir);
    let error = allow(
        &["runner:bench".to_owned()],
        &facts_without_runner(),
        &grants,
        &journal,
    )
    .expect_err("a runner grant with no composed runner is still a usage error");
    assert!(
        error.contains("composed no runner"),
        "the refusal names the gap: {error}"
    );
    assert!(
        error.contains("run_command") && error.contains("ask for approval"),
        "the refusal names the fallback: {error}"
    );
    assert!(
        !grants.is_granted("runner:bench"),
        "U8 stands: the composition still refuses to seed a grant it cannot honour"
    );
}

/// `/allow none` seeds nothing, so it journals nothing; a refused scope is
/// a usage error that seeds nothing and journals nothing.
#[test]
fn allow_none_and_refused_scopes_journal_nothing() {
    let dir = state_dir("seed-none");
    let grants = store_with("runner:bench");
    let journal = SessionJournal::open(&dir);
    allow(&["none".to_owned()], &composed_facts(), &grants, &journal)
        .expect("`none` parses on the session surface");
    allow(&["wat".to_owned()], &composed_facts(), &grants, &journal)
        .expect_err("an unknown scope is a usage error");
    assert_eq!(
        journal.read().expect("the journal reads"),
        Vec::new(),
        "nothing seeded, nothing journalled"
    );
}

/// A journal write that fails must not take the seeding down — the user's
/// explicit grant stands — and must not fail silently: the message still
/// names what was seeded and says the audit line could not be written.
#[test]
fn a_failed_journal_write_warns_and_does_not_take_the_seed_down() {
    let grants = SessionGrants::default();
    let message = allow(
        &["sql:analytics".to_owned()],
        &composed_facts(),
        &grants,
        &broken_journal("seed-fail"),
    )
    .expect("the seeding itself stands");
    assert!(
        grants.is_granted("sql:analytics"),
        "the grant lands in the store: a failed audit write does not revoke consent"
    );
    assert!(
        message.contains("sql:analytics"),
        "the message still names what was seeded: {message}"
    );
    assert!(
        message.to_lowercase().contains("journal"),
        "the failed write is said, never silent: {message}"
    );
}

/// Seeding a scope the store already holds changes nothing and says so:
/// the grant is the same explicit fact, additive only.
#[test]
fn allow_over_an_existing_grant_says_so_and_changes_nothing() {
    let grants = store_with("runner:bench");
    let journal = SessionJournal::open(state_dir("already"));
    let message = allow(
        &["runner:bench".to_owned()],
        &composed_facts(),
        &grants,
        &journal,
    )
    .expect("a re-stated scope is not a usage error");
    assert!(
        message.contains("runner:bench"),
        "the message still names the word: {message}"
    );
    assert!(grants.is_granted("runner:bench"));
    assert_eq!(grants.tokens(), vec!["runner:bench".to_owned()]);
}

/// A fetch grant seeded in any casing is the URL parser's word — the same
/// word the suggester offers at an ask on that destination — so it
/// pre-answers the call it names (U6 defect 2: the verbatim seed never
/// matched the lowercased suggestion, and `/grants` listed a grant that
/// pre-answered nothing while the user was told they granted it).
#[test]
fn a_fetch_grant_seeded_in_any_casing_pre_answers_the_call_it_names() {
    let grants = SessionGrants::default();
    let journal = SessionJournal::open(state_dir("fetch"));
    let message = allow(
        &["fetch:HTTPS+Example.com".to_owned()],
        &composed_facts(),
        &grants,
        &journal,
    )
    .expect("the session surface accepts the fetch scope");
    assert!(
        message.contains("fetch:https+example.com"),
        "the echo names the grammar's spelling of the token: {message}"
    );
    assert_eq!(
        grants.tokens(),
        vec!["fetch:https+example.com".to_owned()],
        "the store holds the token the suggester produces for that destination"
    );
    // The seeded grant is in force: the session policy pre-answers the
    // fetch call the grant names — the call's suggested token and the
    // seeded token are one string. The effect is `http_fetch`'s own
    // (`session_definitions.rs`): an external side effect consented per
    // call.
    let policy = SessionPolicy::new(ApprovalPolicy::Ask);
    allow(
        &["fetch:HTTPS+Example.com".to_owned()],
        &composed_facts(),
        policy.grants(),
        &SessionJournal::open(state_dir("fetch2")),
    )
    .expect("the session surface accepts the fetch scope");
    let effect = saya_agent::ToolEffect {
        database_data: false,
        external_side_effect: true,
        requires_approval: true,
        local_state: saya_agent::LocalStateEffect::None,
    };
    assert!(
        policy.resolve(&effect, Some("fetch:https+example.com"))
            == saya_agent::ApprovalDecision::Allow,
        "the seeded grant pre-answers the fetch call it names"
    );
    assert!(
        policy.resolve(&effect, Some("fetch:https+other.example"))
            == saya_agent::ApprovalDecision::Ask,
        "and nothing on a different destination"
    );
}
