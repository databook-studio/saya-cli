/// H1 red: the `shell:<command prefix>` family joins the grammar on the
/// session surface. Its payload is the one the suggester derives, so
/// `/allow shell:cargo test` and answering a prompt with [s] on `cargo test`
/// record one identical token — the grammar validates with the suggester's
/// own function rather than a second parser of its own.
#[test]
fn shell_joins_the_grammar_on_the_session_surface() {
    let Ok(approved) = parse(&["shell:cargo test".to_string()], Surface::Session) else {
        panic!("`shell:cargo test` must parse on the session surface");
    };
    assert_eq!(
        approved.tokens,
        vec!["shell:cargo test".to_string()],
        "the shell token rides the approval's tokens"
    );
    assert_eq!(
        approved.capabilities,
        saya_types::Capabilities::default(),
        "a `shell:` token binds no run capability"
    );
}

/// The spacing is the suggester's: whitespace separates words and never
/// joins them, so the typed spelling and the prompted spelling are one
/// token.
#[test]
fn the_shell_payload_is_normalised_exactly_as_the_suggester_spells_it() {
    let Ok(typed) = parse(&["shell:cargo   test".to_string()], Surface::Session) else {
        panic!("the spaced payload names the same prefix and must parse");
    };
    let Ok(plain) = parse(&["shell:cargo test".to_string()], Surface::Session) else {
        panic!("the plain payload must parse");
    };
    assert_eq!(
        typed.tokens, plain.tokens,
        "one command prefix has one spelling, however the words were spaced"
    );
    let suggested = crate::grant_token::shell_token_for_host_route(
        &serde_json::json!({ "command": "cargo   test" }),
    )
    .expect("a plain command suggests the job shell's key");
    let Ok(parsed) = parse(std::slice::from_ref(&suggested), Surface::Session) else {
        panic!("the suggester's own key must parse: `{suggested}`");
    };
    assert_eq!(
        parsed.tokens,
        vec![suggested],
        "the suggested token and the typed token are the same word"
    );
}

/// A payload the suggester would never produce is a usage error, never a
/// seeded grant: the same validation the suggester applies decides it.
#[test]
fn a_shell_payload_the_suggester_never_produces_is_refused() {
    let long = format!("shell:{}", "a".repeat(257));
    let cases = [
        ("shell:", "an empty payload"),
        ("shell:a | b", "a pipeline"),
        ("shell:cd x", "a leading cd"),
        ("shell:FOO=1 x", "a leading assignment"),
        ("shell:a > f", "an output redirect"),
        ("shell:cargo\ttest", "a control character"),
        (long.as_str(), "a 257-byte payload"),
    ];
    for (token, label) in cases {
        let result = parse(&[token.to_string()], Surface::Session);
        assert!(
            result.is_err(),
            "`{token}` ({label}) must refuse: the suggester names no such key"
        );
        let error = result.err().unwrap_or_default();
        assert!(
            error.contains("shell:<command prefix>"),
            "the refusal states the payload's shape rule: {error}"
        );
    }
}

/// H1 red: on the run surface `shell:` is a permanent policy refusal in its
/// own class — a run is unattended and composes no session grant store.
#[test]
fn a_run_refuses_shell_scopes_by_design() {
    for token in ["shell:cargo test", "shell:ls"] {
        let Err(error) = parse(&[token.to_string()], Surface::Run) else {
            panic!("a run never gets a session grant word");
        };
        assert!(
            error.contains("not available on runs, by design"),
            "the run refusal is policy, pinned: {error}"
        );
        assert!(
            error.contains(token),
            "the refusal names the scope: {error}"
        );
        assert!(
            !error.contains("not available yet"),
            "never an absence claim — this refusal never leaves: {error}"
        );
        assert!(error.contains("Re-run without it."), "got: {error}");
    }
}
