/// H1 red: the `command:<program>` family joins the grammar on the
/// session surface — payload a bare name, shells included, no family
/// mirror. Written before the family exists, so the token is refused as
/// unknown today (the failure is the grammar, not the composition seam;
/// the composed-vs-bare half is pinned at the `/allow` level by
/// `command_tokens_parse_on_a_composed_session_and_refuse_on_a_bare_one`).
#[test]
fn command_joins_the_grammar_on_the_session_surface() {
    let Ok(approved) = parse(&["command:npm".to_string()], Surface::Session) else {
        panic!("`command:npm` must parse on the session surface once H1 lands");
    };
    assert!(
        approved.tokens.iter().any(|token| token == "command:npm"),
        "the approval carries the token verbatim: {:?}",
        approved.tokens
    );
    // The family mirror deliberately does not apply here: shells ride
    // `command:`, never `interpreter:`-by-mirror.
    let Ok(shell) = parse(&["command:bash".to_string()], Surface::Session) else {
        panic!("`command:bash` parses — shells are included, no mirror");
    };
    assert!(
        shell.tokens.iter().any(|token| token == "command:bash"),
        "the shell token rides verbatim: {:?}",
        shell.tokens
    );
}

/// H1 red: the `command:` payload is a bare name — never a path or a
/// traversal. Written before the family exists, so the refusal today is
/// the unknown-scope error rather than the shape rule.
#[test]
fn the_command_payload_is_a_bare_name_never_a_path() {
    for token in ["command:", "command:bin/npm", "command:../npm"] {
        let Err(error) = parse(&[token.to_string()], Surface::Session) else {
            panic!("`{token}` must refuse: the payload is a bare name");
        };
        assert!(
            error.contains("bare name"),
            "the refusal states the shape rule: {error}"
        );
    }
}

/// H1 red: on the run surface `command:` is a permanent policy refusal in
/// its own class — never an absence claim, with its own pinned wording.
/// Written before the refusal exists, so the run parser's unknown-scope
/// error is what fires today.
#[test]
fn a_run_refuses_command_scopes_by_design() {
    for token in ["command:npm", "command:bash"] {
        let Err(error) = parse(&[token.to_string()], Surface::Run) else {
            panic!("a run never gets this lane");
        };
        assert!(
            error.contains("not available on runs, by design"),
            "the run refusal is policy, pinned: {error}"
        );
        assert!(
            error.contains(super::RUN_COMMAND_REFUSAL_PIN),
            "the pin rides the production constant: {error}"
        );
        assert!(
            error.contains("unconfined host execution"),
            "the refusal says what the scope names: {error}"
        );
        assert!(
            error.contains("runner:") && error.contains("interpreter:"),
            "the refusal steers to a sandboxed scope: {error}"
        );
        assert!(
            !error.contains("not available yet"),
            "never an absence claim — this refusal never leaves: {error}"
        );
        assert!(
            error.contains(token),
            "the refusal names the scope: {error}"
        );
        assert!(error.contains("Re-run without it."), "got: {error}");
    }
}

/// The permanent refusal is its own class: the wiring-shaped list never
/// carries the `command` family on any surface — the run refusal above is
/// policy, and the session surface parses the family rather than
/// refusing it.
#[test]
fn the_command_family_is_never_a_wiring_refusal() {
    for entry in NOT_YET_WIRED {
        assert!(
            entry.family != "command",
            "`command` must not join the wiring-shaped list on any surface"
        );
    }
}
