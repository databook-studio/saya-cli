/// H1 red: a session's `/run` child never inherits a `command:` token, and
/// the parent names what it dropped. Written before the family exists: the
/// run surface's unknown-scope refusal already drops it, but the test pins
/// the forward-compat shape H1 must keep.
#[test]
fn run_seed_names_command_tokens_as_dropped() {
    use crate::interactive::session::permissions::grants::{run_seed, seed_message};
    let seed = run_seed(&["command:npm".to_owned(), "workspace-write".to_owned()]);
    assert!(
        seed.forwarded.contains(&"workspace-write".to_owned()),
        "a run-accepted token still forwards: {:?}",
        seed.forwarded
    );
    assert!(
        seed.dropped.contains(&"command:npm".to_owned()),
        "a session's /run child never inherits a command: token: {:?}",
        seed.dropped
    );
    let message = seed_message(&seed);
    assert!(
        message.contains("not forwarded") && message.contains("command:npm"),
        "the parent names what it dropped: {message}"
    );
}

/// `/run --seed-grants` (test: the child's --allow is seeded from this
/// session's grants **on request**): only tokens a run's own parser accepts
/// are forwarded — each forwarded token parses on the run surface — and the
/// rest are named, never silently dropped. The child's parser stays the
/// authority; the filter asks it, one token at a time.
#[test]
fn a_seed_request_forwards_only_what_a_run_accepts_and_names_the_rest() {
    use crate::interactive::session::permissions::grants::{run_seed, seed_message};
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
    use crate::interactive::session::permissions::grants::{run_seed, seed_message};
    let seed = run_seed(&[]);
    assert!(seed.forwarded.is_empty() && seed.dropped.is_empty());
    let message = seed_message(&seed);
    assert!(
        message.contains("no session grants to seed"),
        "an empty store is said, not silent: {message}"
    );
}
