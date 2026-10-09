use std::fs;

use saya_types::Capabilities;

use super::{journal_grants, journal_with, spec_scopes, temp_dir};

/// The resume's seeds are the journal payload's own words — exactly what the
/// original approval stated, carried grant tokens (`sql:<connection>`)
/// included — so a resumed run's frozen decider consults what the journal
/// approved, never the spec. A spec.json edited between invocations cannot
/// add a seed: only the journal's payload states grant words.
#[test]
fn the_resume_s_seeds_are_the_journal_s_payload_words() {
    let dir = temp_dir("seed-words");
    journal_with(
        vec![
            "workspace-write".to_string(),
            "sql:analytics".to_string(),
            "sql:staging".to_string(),
        ],
        &dir,
    );
    // The spec carries neither sql token — as a tampered file might carry
    // anything — but the journal is the seed authority.
    let spec = spec_scopes(&[]);
    let grant = journal_grants(&dir, &spec).expect("the journal's scopes parse");
    assert_eq!(
        grant.tokens,
        vec![
            "workspace-write".to_owned(),
            "sql:analytics".to_owned(),
            "sql:staging".to_owned(),
        ],
        "the seeds are the journal's stated words, verbatim and in order"
    );
    assert!(
        grant.capabilities.workspace_write,
        "the capability words re-grant alongside the seeds"
    );

    let _ = fs::remove_dir_all(dir);
}

/// A carried token the journal states re-parses on the run surface (U4
/// wired `sql:` there), so the payload the fresh run recorded is exactly
/// the payload a resume replays — the words a run seeds with are the words
/// it journaled, round trip.
#[test]
fn a_carried_sql_token_in_a_journal_re_grants_as_a_seed() {
    let dir = temp_dir("carried-seed");
    journal_with(vec!["sql:analytics".to_string()], &dir);
    let spec = Capabilities::default();
    let grant = journal_grants(&dir, &spec).expect("the journal's carried token parses");
    assert_eq!(
        grant.tokens,
        vec!["sql:analytics".to_owned()],
        "the resumed decider is seeded with the connection's grant"
    );
    assert!(
        !grant.capabilities.workspace_write
            && !grant.capabilities.scratch
            && grant.capabilities.fetch.is_none()
            && grant.capabilities.runner.is_none()
            && grant.capabilities.interpreter.is_none()
            && grant.capabilities.endpoints.as_map().is_empty(),
        "a carried token is a grant word, not a plan capability"
    );

    let _ = fs::remove_dir_all(dir);
}
