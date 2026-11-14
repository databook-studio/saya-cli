/// `/rotate` parses bare and refuses any argument: the rotation is one
/// command, and a trailing word is a mistake worth naming.
#[test]
fn rotate_parses_bare_and_refuses_arguments() {
    assert_eq!(
        parse_slash_command("/rotate"),
        Ok(Some(SlashCommand::Rotate))
    );
    let error = parse_slash_command("/rotate now").unwrap_err();
    assert_eq!(error.0, "rotate takes no argument: /rotate");
}

/// `/tasks` shows the list bare and clears it on `clear`; anything else
/// is a usage error naming the one subcommand, so a typo cannot read as
/// a silent show.
#[test]
fn test_parse_tasks_shows_clears_and_refuses() {
    assert_eq!(
        parse_slash_command("/tasks"),
        Ok(Some(SlashCommand::Tasks(None)))
    );
    assert_eq!(
        parse_slash_command("/tasks clear"),
        Ok(Some(SlashCommand::Tasks(Some("clear".into()))))
    );
    let error = parse_slash_command("/tasks bogus").unwrap_err();
    assert!(
        error.0.contains("/tasks clear"),
        "the error names the one subcommand, got: {error}"
    );
}

/// `/mode` is not bypass consent: it must not trip the journalling gate
/// the two executors consult (`session_loop.rs` and `tui/dispatch.rs`
/// match on `SlashCommand::Approvals(Some(_))`). A mode change — report
/// or switch — never matches that shape.
#[test]
fn mode_never_matches_the_bypass_consent_gate() {
    for command in [
        parse_slash_command("/mode").unwrap().unwrap(),
        parse_slash_command("/mode plan").unwrap().unwrap(),
        parse_slash_command("/mode build").unwrap().unwrap(),
    ] {
        assert!(
            !matches!(command, SlashCommand::Approvals(Some(_))),
            "a mode change is not bypass consent: {command:?}"
        );
    }
    assert!(
        matches!(
            parse_slash_command("/approvals bypass").unwrap().unwrap(),
            SlashCommand::Approvals(Some(_))
        ),
        "the gate still matches its own /approvals shape"
    );
}
