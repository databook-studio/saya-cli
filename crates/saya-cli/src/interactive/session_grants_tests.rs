//! The `/allow` and `/grants` commands: the session grant store's seeding
//! and listing, shared by the headless loop and the TUI dispatch — one
//! operation, two adapters.

use super::session_grants::{allow, listing};
use crate::grant_token::TurnPrimary;
use crate::grant_token_tests::registry_with_primary;
use saya_agent::{ApprovalPolicy, SessionGrants, SessionPolicy};

/// A store with one grant seeded through the engine's own record.
fn store_with(token: &str) -> SessionGrants {
    let grants = SessionGrants::default();
    grants.grant(token);
    grants
}

/// `/grants` on an empty store: the lifetime header with the zero count and
/// an explicit empty state — never a bare nothing.
#[test]
fn the_listing_states_the_empty_store_explicitly() {
    assert_eq!(
        listing(&SessionGrants::default()),
        "session grants (die with this session): 0\n  (none — nothing pre-answers \
         this session yet; /allow <scopes> or answer [s] at an ask)"
    );
}

/// `/grants` lists the store's tokens **verbatim**, one per line, sorted,
/// under a header stating the lifetime, with a count: the words are the
/// record, and they are the same words the prompt offered.
#[test]
fn the_listing_prints_the_tokens_verbatim_sorted() {
    let grants = store_with("sql:analytics");
    grants.grant("runner:bench");
    assert_eq!(
        listing(&grants),
        "session grants (die with this session): 2\n  runner:bench\n  sql:analytics",
        "sorted, one per line, under the lifetime header with the count"
    );
}

/// `/allow` seeds the store through the same parser and says what it
/// seeded: the message names the tokens, and the store answers the granted
/// word.
#[test]
fn allow_seeds_the_stated_scopes_into_the_store() {
    let grants = SessionGrants::default();
    let message = allow(
        &["sql:analytics".to_owned(), "runner:bench".to_owned()],
        &grants,
    )
    .expect("the session surface accepts both scopes");
    assert!(
        message.contains("sql:analytics") && message.contains("runner:bench"),
        "the message names what was seeded: {message}"
    );
    assert!(grants.is_granted("sql:analytics"));
    assert!(grants.is_granted("runner:bench"));
}

/// Seeding a scope the store already holds changes nothing and says so:
/// the grant is the same explicit fact, additive only.
#[test]
fn allow_over_an_existing_grant_says_so_and_changes_nothing() {
    let grants = store_with("runner:bench");
    let message = allow(&["runner:bench".to_owned()], &grants)
        .expect("a re-stated scope is not a usage error");
    assert!(
        message.contains("runner:bench"),
        "the message still names the word: {message}"
    );
    assert!(grants.is_granted("runner:bench"));
    assert_eq!(grants.tokens(), vec!["runner:bench".to_owned()]);
}

/// `/allow none` keeps the grammar's meaning — the empty approval, alone —
/// and seeds nothing, saying so. It is not a revoke: whatever the session
/// already holds stays held.
#[test]
fn allow_none_seeds_nothing_and_says_so() {
    let grants = store_with("runner:bench");
    let message =
        allow(&["none".to_owned()], &grants).expect("`none` parses on the session surface");
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
    let error = allow(&["endpoint:analyst=fast".to_owned()], &grants)
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

    let error = allow(&["sql:bad name".to_owned()], &SessionGrants::default())
        .expect_err("a non-name-shaped payload is a usage error");
    assert!(
        error.contains("sql:<connection>"),
        "the refusal names the grammar: {error}"
    );

    let error = allow(&["wat".to_owned()], &SessionGrants::default())
        .expect_err("an unknown scope is a usage error");
    assert!(error.contains("unknown scope `wat`"), "got: {error}");
}

/// The seeded grant is the session's grant in force: a policy over the
/// seeded store pre-answers the call shape the grant names — the seed is
/// the same fact the [s] answer records, from the other side.
#[test]
fn a_seeded_grant_pre_answers_like_a_prompted_one() {
    let policy = SessionPolicy::new(ApprovalPolicy::Ask);
    allow(&["sql:analytics".to_owned()], policy.grants())
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
