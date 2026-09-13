//! The `/allow` and `/grants` commands: the session grant store's seeding
//! and listing, shared by the headless loop and the TUI dispatch — one
//! operation, two adapters.

use super::session_grants::{allow, listing};
use crate::grant_token::TurnPrimary;
use crate::grant_token_tests::registry_with_primary;
use saya_agent::{ApprovalPolicy, SessionGrants, SessionPolicy};
use saya_store::{GrantSource, JournalEvent, SessionJournal};
use std::path::PathBuf;

/// A store with one grant seeded through the engine's own record.
fn store_with(token: &str) -> SessionGrants {
    let grants = SessionGrants::default();
    grants.grant(token);
    grants
}

/// A fresh state directory per test, the way a session's is created.
fn state_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("saya-allow-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("state dir creates");
    dir
}

/// A journal whose file cannot be written: the journal path is a directory,
/// so every append fails.
fn broken_journal(label: &str) -> SessionJournal {
    let dir = state_dir(label);
    std::fs::create_dir_all(dir.join("journal.ndjson")).expect("the block is made");
    SessionJournal::open(&dir)
}

/// `/grants` on an empty store: the lifetime header with the zero count and
/// an explicit empty state — never a bare nothing.
#[test]
fn the_listing_states_the_empty_store_explicitly() {
    assert_eq!(
        listing(ApprovalPolicy::Ask, &SessionGrants::default()),
        "session grants (die with this session): 0\n  (none — nothing pre-answers \
         this session yet; /allow <scopes> or answer [s] at an ask)"
    );
}

/// `/grants` under bypass states the mode **first**: the count alone would
/// read "nothing runs", when the truth is everything does — every call runs
/// without asking and the store is not consulted. The listing follows.
#[test]
fn the_listing_states_the_mode_first_under_bypass() {
    let grants = store_with("runner:bench");
    let text = listing(ApprovalPolicy::Bypass, &grants);
    let (mode_line, rest) = text
        .split_once('\n')
        .expect("the bypass listing carries the mode line first");
    assert_eq!(
        mode_line, "mode bypass: every call runs without asking; grants are not consulted",
        "the mode is stated before the listing: {text}"
    );
    assert!(
        rest.contains("session grants (die with this session): 1") && rest.contains("runner:bench"),
        "the listing follows the mode line: {rest}"
    );
    // The mode line comes before the count, byte-ordered.
    let count = text
        .find("session grants (die with this session): 1")
        .expect("the count line is present");
    let mode = text.find("mode bypass:").expect("the mode line is present");
    assert!(mode < count, "the mode line precedes the listing: {text}");
}

/// The non-bypass modes render today's listing exactly: no mode line, the
/// lifetime header, the count, and the tokens.
#[test]
fn the_listing_without_bypass_keeps_its_exact_bytes() {
    let grants = store_with("sql:analytics");
    grants.grant("runner:bench");
    for mode in [
        ApprovalPolicy::Ask,
        ApprovalPolicy::ReadOnly,
        ApprovalPolicy::Never,
    ] {
        assert_eq!(
            listing(mode, &grants),
            "session grants (die with this session): 2\n  runner:bench\n  sql:analytics",
            "no mode line under {mode:?} — the listing is today's bytes"
        );
    }
}

/// `/grants` lists the store's tokens **verbatim**, one per line, sorted,
/// under a header stating the lifetime, with a count: the words are the
/// record, and they are the same words the prompt offered.
#[test]
fn the_listing_prints_the_tokens_verbatim_sorted() {
    let grants = store_with("sql:analytics");
    grants.grant("runner:bench");
    assert_eq!(
        listing(ApprovalPolicy::Ask, &grants),
        "session grants (die with this session): 2\n  runner:bench\n  sql:analytics",
        "sorted, one per line, under the lifetime header with the count"
    );
}

/// `/allow` seeds the store through the same parser and says what it
/// seeded: the message names the tokens, and the store answers the granted
/// word.
#[test]
fn allow_seeds_the_stated_scopes_into_the_store() {
    let dir = state_dir("seed");
    let grants = SessionGrants::default();
    let journal = SessionJournal::open(&dir);
    let message = allow(
        &["sql:analytics".to_owned(), "runner:bench".to_owned()],
        &grants,
        &journal,
    )
    .expect("the session surface accepts both scopes");
    assert!(
        message.contains("sql:analytics") && message.contains("runner:bench"),
        "the message names what was seeded: {message}"
    );
    assert!(grants.is_granted("sql:analytics"));
    assert!(grants.is_granted("runner:bench"));
}

/// The journal-once rule (property 1): a first grant writes exactly one
/// line per token, and a second grant of the same token — a re-stated
/// `/allow`, the store answering "already" — writes none. The line is the
/// `seed` source: the token was stated before anything ran under it.
#[test]
fn allow_journals_each_newly_seeded_token_once_as_a_seed() {
    let dir = state_dir("seed-journal");
    let grants = SessionGrants::default();
    let journal = SessionJournal::open(&dir);
    allow(
        &["sql:analytics".to_owned(), "runner:bench".to_owned()],
        &grants,
        &journal,
    )
    .expect("the session surface accepts both scopes");
    assert_eq!(
        journal.read().expect("the journal reads"),
        vec![
            JournalEvent::Granted {
                token: "sql:analytics".to_owned(),
                source: GrantSource::Seed,
            },
            JournalEvent::Granted {
                token: "runner:bench".to_owned(),
                source: GrantSource::Seed,
            },
        ],
        "exactly one line per newly seeded token, in seed order"
    );
    // Re-stating the same token: the store answers "already", nothing
    // changes, and the journal records nothing.
    allow(&["sql:analytics".to_owned()], &grants, &journal)
        .expect("a re-stated scope is not a usage error");
    assert_eq!(
        journal.read().expect("the journal reads").len(),
        2,
        "a second grant of the same token writes none"
    );
}

/// `/allow none` seeds nothing, so it journals nothing; a refused scope is
/// a usage error that seeds nothing and journals nothing.
#[test]
fn allow_none_and_refused_scopes_journal_nothing() {
    let dir = state_dir("seed-none");
    let grants = store_with("runner:bench");
    let journal = SessionJournal::open(&dir);
    allow(&["none".to_owned()], &grants, &journal).expect("`none` parses on the session surface");
    allow(&["wat".to_owned()], &grants, &journal).expect_err("an unknown scope is a usage error");
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
    let message = allow(&["runner:bench".to_owned()], &grants, &journal)
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
    let message = allow(&["fetch:HTTPS+Example.com".to_owned()], &grants, &journal)
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

/// `/allow none` keeps the grammar's meaning — the empty approval, alone —
/// and seeds nothing, saying so. It is not a revoke: whatever the session
/// already holds stays held.
#[test]
fn allow_none_seeds_nothing_and_says_so() {
    let grants = store_with("runner:bench");
    let journal = SessionJournal::open(state_dir("none"));
    let message = allow(&["none".to_owned()], &grants, &journal)
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
    let error = allow(&["endpoint:analyst=fast".to_owned()], &grants, &journal)
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
        &SessionGrants::default(),
        &journal,
    )
    .expect_err("a non-name-shaped payload is a usage error");
    assert!(
        error.contains("sql:<connection>"),
        "the refusal names the grammar: {error}"
    );

    let error = allow(&["wat".to_owned()], &SessionGrants::default(), &journal)
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

/// `/run --seed-grants` (test: the child's --allow is seeded from this
/// session's grants **on request**): only tokens a run's own parser accepts
/// are forwarded — each forwarded token parses on the run surface — and the
/// rest are named, never silently dropped. The child's parser stays the
/// authority; the filter asks it, one token at a time.
#[test]
fn a_seed_request_forwards_only_what_a_run_accepts_and_names_the_rest() {
    use super::session_grants::{run_seed, seed_message};
    let tokens = [
        "runner:bench".to_owned(),
        "sql:analytics".to_owned(),
        "endpoint:analyst=fast".to_owned(),
        "workspace-write".to_owned(),
    ];
    let seed = run_seed(&tokens);
    assert_eq!(
        seed.forwarded,
        vec![
            "runner:bench".to_owned(),
            "sql:analytics".to_owned(),
            "workspace-write".to_owned(),
        ],
        "every forwarded token parses on the run surface"
    );
    assert_eq!(seed.dropped, vec!["endpoint:analyst=fast".to_owned()]);
    for token in &seed.forwarded {
        assert!(
            crate::commands::run::scopes::parse(
                std::slice::from_ref(token),
                crate::commands::run::scopes::Surface::Run
            )
            .is_ok(),
            "a forwarded token is one the run's parser accepts: {token}"
        );
    }
    let message = seed_message(&seed);
    assert!(
        message.contains("seeded the run's --allow from this session's grants:")
            && message.contains("runner:bench")
            && message.contains("sql:analytics")
            && message.contains("workspace-write"),
        "the message names what was forwarded: {message}"
    );
    assert!(
        message.contains("not forwarded") && message.contains("endpoint:analyst=fast"),
        "the message names what was not forwarded — never a silent drop: {message}"
    );
}

/// A seed request over an empty store says so — never silence.
#[test]
fn a_seed_request_over_an_empty_store_says_so() {
    use super::session_grants::{run_seed, seed_message};
    let seed = run_seed(&[]);
    assert!(seed.forwarded.is_empty() && seed.dropped.is_empty());
    let message = seed_message(&seed);
    assert!(
        message.contains("no session grants to seed"),
        "an empty store is said, not silent: {message}"
    );
}

/// A seed where the store holds only tokens a run refuses: nothing is
/// forwarded and every token is named — the child's `--allow` is untouched.
#[test]
fn a_seed_request_names_every_token_it_does_not_forward() {
    use super::session_grants::{run_seed, seed_message};
    let tokens = ["endpoint:analyst=fast".to_owned()];
    let seed = run_seed(&tokens);
    assert!(seed.forwarded.is_empty(), "a run accepts none of it");
    let message = seed_message(&seed);
    assert!(
        message.contains("not forwarded") && message.contains("endpoint:analyst=fast"),
        "the dropped tokens are named: {message}"
    );
    assert!(
        !message.contains("seeded"),
        "nothing was forwarded, so the message claims no seeding: {message}"
    );
}
