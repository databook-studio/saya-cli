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

/// `/mode` and `/approvals` are one command: the same variant, the same value,
/// and only the spelling differs. Nothing downstream can gate one spelling and
/// not the other, because there is nothing left to gate separately — the one
/// switch owns the bypass journal both spellings reach.
#[test]
fn mode_and_approvals_are_one_command_with_two_spellings() {
    use saya_agent::PermissionMode;
    for spelling in [
        Some(PermissionMode::Ask),
        Some(PermissionMode::Plan),
        Some(PermissionMode::Bypass),
        Some(PermissionMode::ReadOnly),
        Some(PermissionMode::Never),
        None,
    ] {
        let tail = match spelling {
            Some(mode) => format!(" {}", crate::interactive::session::permissions::mode::name(mode)),
            None => String::new(),
        };
        let canonical = parse_slash_command(&format!("/mode{tail}")).unwrap().unwrap();
        let aliased = parse_slash_command(&format!("/approvals{tail}"))
            .unwrap()
            .unwrap();
        let SlashCommand::Mode(canonical) = canonical else {
            panic!("/mode{tail} is a mode statement");
        };
        let SlashCommand::Mode(aliased) = aliased else {
            panic!("/approvals{tail} is the same mode statement");
        };
        assert_eq!(canonical.value, aliased.value);
        assert_eq!(canonical.value, spelling);
        assert!(
            !canonical.via_approvals_alias,
            "/mode is the canonical spelling"
        );
        assert!(aliased.via_approvals_alias, "/approvals is the alias");
    }
}
