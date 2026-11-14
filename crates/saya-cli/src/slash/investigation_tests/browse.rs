#[test]
fn list_parses_bare_and_with_bounds() {
    assert_eq!(
        parsed("investigation", "list").unwrap(),
        InvestigationCommand::List {
            limit: None,
            offset: None
        }
    );
    assert_eq!(
        parsed("investigation", "list --limit 5 --offset 3").unwrap(),
        InvestigationCommand::List {
            limit: Some(5),
            offset: Some(3)
        }
    );
    let not_a_number = parsed("investigation", "list --limit abc").unwrap_err();
    assert!(
        not_a_number.0.contains("--limit needs a number"),
        "a non-numeric --limit must be a usage error: {not_a_number}"
    );
    // list takes no positional; a stray token is a usage error.
    assert!(parsed("investigation", "list all").is_err());
}

/// `/investigations` is `list`: bare and with the same bounds.
#[test]
fn investigations_alias_parses_as_list() {
    assert_eq!(
        parsed("investigations", "").unwrap(),
        InvestigationCommand::List {
            limit: None,
            offset: None
        }
    );
    assert_eq!(
        parsed("investigations", "--limit 7").unwrap(),
        InvestigationCommand::List {
            limit: Some(7),
            offset: None
        }
    );
    // `--list` is the explicit escape hatch back to the text listing (the
    // TUI opens its picker on the bare alias before this parser runs).
    assert_eq!(
        parsed("investigations", "--list").unwrap(),
        InvestigationCommand::List {
            limit: None,
            offset: None
        }
    );
    // And through the full slash parser, the alias lands on the same command.
    assert_eq!(
        parse_slash_command("/investigations").unwrap(),
        Some(crate::slash::SlashCommand::Investigation(
            InvestigationCommand::List {
                limit: None,
                offset: None
            }
        ))
    );
}
