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
        &composed_facts(),
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
        &composed_facts(),
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
    allow(
        &["sql:analytics".to_owned()],
        &composed_facts(),
        &grants,
        &journal,
    )
    .expect("a re-stated scope is not a usage error");
    assert_eq!(
        journal.read().expect("the journal reads").len(),
        2,
        "a second grant of the same token writes none"
    );
}
