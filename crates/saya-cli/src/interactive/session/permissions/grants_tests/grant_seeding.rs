/// `/allow` seeds the store through the same parser and says what it
/// seeded: the message names the tokens, and the store answers the granted
/// word.
#[test]
fn allow_seeds_the_stated_scopes_into_the_store() {
    let dir = state_dir("seed");
    let grants = SessionGrants::default();
    let journal = SessionJournal::open(&dir);
    let message = allow(
        &["sql:analytics".to_owned()],
        &composed_facts(),
        &grants,
        &journal,
    )
    .expect("the session surface accepts the scope");
    assert!(
        message.contains("sql:analytics"),
        "the message names what was seeded: {message}"
    );
    assert!(grants.is_granted("sql:analytics"));
}

/// The job shell's word typed by hand records the token answering [s] would
/// record: the grammar normalises the payload through the suggester's own
/// function, so a spaced spelling and the suggested one are one grant.
#[test]
fn allow_records_the_job_shells_prefix_as_the_suggester_spells_it() {
    let dir = state_dir("seed-shell");
    let grants = SessionGrants::default();
    let journal = SessionJournal::open(&dir);
    let message = allow(
        &["shell:cargo   test".to_owned()],
        &composed_facts(),
        &grants,
        &journal,
    )
    .expect("the session surface accepts the job shell's prefix");
    assert!(
        message.contains("shell:cargo test"),
        "the message names the normalised token: {message}"
    );
    assert!(
        grants.is_granted("shell:cargo test"),
        "the store holds the one spelling the suggester derives"
    );
    assert!(
        !grants.is_granted("shell:cargo   test"),
        "the typed spacing is not a second grant"
    );
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
        &["sql:analytics".to_owned()],
        &composed_facts(),
        &grants,
        &journal,
    )
    .expect("the session surface accepts the scope");
    assert_eq!(
        journal.read().expect("the journal reads"),
        vec![
            JournalEvent::Granted {
                token: "sql:analytics".to_owned(),
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
        1,
        "a second grant of the same token writes none"
    );
}
