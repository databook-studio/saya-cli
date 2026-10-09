#[test]
fn test_help_command() {
    assert_eq!(
        parse_slash_command("/help"),
        Ok(Some(SlashCommand::Help(None)))
    );
    assert_eq!(
        parse_slash_command("/help connect"),
        Ok(Some(SlashCommand::Help(Some("connect".into()))))
    );

    let help_connect = help_for(Some("connect"));
    assert!(help_connect.contains("connect"));
    assert!(help_connect.contains("Example"));

    let help_unknown = help_for(Some("nope"));
    assert!(help_unknown.contains("No help"));

    assert_eq!(help_for(None), help_text().to_string());
}

#[test]
fn test_parse_doctor_takes_no_arguments() {
    assert!(matches!(
        parse_slash_command("/doctor").unwrap(),
        Some(SlashCommand::Doctor)
    ));
    // Like /clear and /history, a trailing argument is ignored.
    assert!(matches!(
        parse_slash_command("/doctor now").unwrap(),
        Some(SlashCommand::Doctor)
    ));
}

/// `/setup` parses bare; a trailing word is a typo, never an argument —
/// the pinned parse error says so instead of reading it as one.
#[test]
fn test_parse_setup_takes_no_argument() {
    assert_eq!(parse_slash_command("/setup"), Ok(Some(SlashCommand::Setup)));
    assert_eq!(
        parse_slash_command("/setup now"),
        Err(SlashParseError("setup takes no argument: /setup".into()))
    );
}

#[test]
fn test_parse_sessions_and_resume() {
    assert_eq!(
        parse_slash_command("/sessions"),
        Ok(Some(SlashCommand::Sessions))
    );
    assert_eq!(
        parse_slash_command("/resume 12345"),
        Ok(Some(SlashCommand::Resume("12345".into())))
    );
    assert_eq!(
        parse_slash_command("/resume"),
        Err(SlashParseError("command requires an argument".into()))
    );
}

/// `/sessions` keeps today's meaning (REPL history list, TUI picker);
/// `list` and `delete <id>` are the shared saved-session operations.
#[test]
fn test_parse_sessions_list_and_delete() {
    assert_eq!(
        parse_slash_command("/sessions list"),
        Ok(Some(SlashCommand::SessionsList))
    );
    assert_eq!(
        parse_slash_command("/sessions delete abc"),
        Ok(Some(SlashCommand::SessionsDelete("abc".into())))
    );
}

/// `/sessions delete` without an id names the usage, so a missing
/// argument can never read as a list or a picker.
#[test]
fn sessions_delete_requires_an_id() {
    let error = parse_slash_command("/sessions delete").unwrap_err();
    assert!(
        error
            .0
            .contains("sessions delete requires a session id: /sessions delete <id>"),
        "the missing-id error names the usage: {error}"
    );
}

/// `/sessions delete a b` carries two tokens where exactly one id is
/// required — refused, never silently read as one id.
#[test]
fn sessions_delete_takes_exactly_one_id() {
    let error = parse_slash_command("/sessions delete a b").unwrap_err();
    assert!(
        error.0.contains("exactly one session id"),
        "the multi-token error says one id only: {error}"
    );
}

/// Anything after `/sessions` that is not `list` or `delete <id>` is a
/// usage error naming every valid form, so a typo cannot read as a
/// silent show.
#[test]
fn sessions_unknown_subcommand_is_refused() {
    let error = parse_slash_command("/sessions nope").unwrap_err();
    assert!(
        error
            .0
            .contains("unknown /sessions subcommand: nope (usage: /sessions, /sessions list, /sessions delete <id>)"),
        "the unknown-subcommand error names every form: {error}"
    );
}

#[test]
fn test_parse_sql_command() {
    assert_eq!(
        parse_slash_command("/sql SELECT * FROM users;"),
        Ok(Some(SlashCommand::Sql("SELECT * FROM users;".into())))
    );
    assert_eq!(
        parse_slash_command("/sql   SELECT  a,  b  FROM  table  "),
        Ok(Some(SlashCommand::Sql("SELECT  a,  b  FROM  table".into())))
    );
    assert_eq!(
        parse_slash_command("/sql"),
        Err(SlashParseError("sql requires a query".into()))
    );
    assert_eq!(
        parse_slash_command("/sql   "),
        Err(SlashParseError("sql requires a query".into()))
    );
}

fn export_request(mode: Option<ExportMode>, overwrite: bool, path: &str) -> ExportRequest {
    ExportRequest {
        mode,
        overwrite,
        path: path.to_string(),
    }
}
