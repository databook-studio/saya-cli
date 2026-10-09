#[test]
fn test_parse_chart_command() {
    assert_eq!(
        parse_slash_command("/chart"),
        Ok(Some(SlashCommand::Chart("".into())))
    );
    assert_eq!(
        parse_slash_command("/chart foo"),
        Ok(Some(SlashCommand::Chart("foo".into())))
    );
}

#[test]
fn test_parse_explain_command() {
    assert_eq!(
        parse_slash_command("/explain"),
        Ok(Some(SlashCommand::Explain("".into())))
    );
    assert_eq!(
        parse_slash_command("/explain SELECT 1"),
        Ok(Some(SlashCommand::Explain("SELECT 1".into())))
    );
}

#[test]
fn test_parse_columns_command() {
    assert_eq!(
        parse_slash_command("/columns"),
        Ok(Some(SlashCommand::Columns(None)))
    );
    assert_eq!(
        parse_slash_command("/columns id, total"),
        Ok(Some(SlashCommand::Columns(Some("id, total".into()))))
    );
    assert_eq!(
        parse_slash_command("/columns all"),
        Ok(Some(SlashCommand::Columns(Some("all".into()))))
    );
}

#[test]
fn test_unknown_command_suggestion() {
    let err = parse_slash_command("/conect prod").unwrap_err();
    assert!(
        err.0.contains("did you mean /connect"),
        "expected suggestion in error message, got: {}",
        err.0
    );

    let err = parse_slash_command("/zzzzzzzz").unwrap_err();
    assert!(
        !err.0.contains("did you mean"),
        "unexpected suggestion in error message, got: {}",
        err.0
    );

    assert_eq!(
        parse_slash_command("/connect prod"),
        Ok(Some(SlashCommand::Connect("prod".into())))
    );
}

/// `/history` and `/sessions` are both still known commands, and
/// `exit`/`quit` are a
/// deliberate conventional alias pair. The typo suggester
/// must still resolve anything it resolved before for names that still
/// exist — so a near-miss on each lands on the kept name, never on a
/// removed one.
#[test]
fn kept_alias_pairs_still_parse_and_suggest() {
    // Both names still parse.
    assert_eq!(
        parse_slash_command("/history"),
        Ok(Some(SlashCommand::History))
    );
    assert_eq!(
        parse_slash_command("/sessions"),
        Ok(Some(SlashCommand::Sessions))
    );
    assert_eq!(parse_slash_command("/exit"), Ok(Some(SlashCommand::Exit)));
    assert_eq!(parse_slash_command("/quit"), Ok(Some(SlashCommand::Exit)));

    // A one-char typo on a kept name suggests that name, not something else.
    assert_eq!(registry::closest_command("histor"), Some("history"));
    assert_eq!(registry::closest_command("session"), Some("sessions"));
    // `quit` is a deliberate alias of `exit`; a near-miss still lands on a
    // known name (the suggester picks the closest, never a removed one).
    assert_eq!(registry::closest_command("exi"), Some("exit"));
}

/// `/run` keeps its tail verbatim — the nested child's parser is the
/// authority on it — and `/run cancel <id>` is the subcommand form, the
/// same disambiguation the headless `saya run` grammar gives.
#[test]
fn test_parse_run_and_run_cancel() {
    assert_eq!(
        parse_slash_command("/run survey the data --allow workspace-write"),
        Ok(Some(SlashCommand::Run(
            "survey the data --allow workspace-write".into()
        )))
    );
    assert_eq!(
        parse_slash_command("/run resume r-1"),
        Ok(Some(SlashCommand::Run("resume r-1".into())))
    );
    assert_eq!(
        parse_slash_command("/run cancel r-1"),
        Ok(Some(SlashCommand::RunCancel("r-1".into())))
    );
    assert_eq!(
        parse_slash_command("/run  cancel  r-1 "),
        Ok(Some(SlashCommand::RunCancel("r-1".into())))
    );
    // A bare /run and a cancelless id are usage errors with guidance.
    assert!(parse_slash_command("/run").is_err());
    assert!(parse_slash_command("/run cancel").is_err());
}

/// `/runs` lists without an id and shows one with it.
#[test]
fn test_parse_runs() {
    assert_eq!(
        parse_slash_command("/runs"),
        Ok(Some(SlashCommand::Runs(None)))
    );
    assert_eq!(
        parse_slash_command("/runs r-1"),
        Ok(Some(SlashCommand::Runs(Some("r-1".into()))))
    );
}

/// `/allow` carries the scopes as stated, whitespace-separated, and
/// `/allow` alone is not a listing — it is a usage error pointing at
/// the listing (`/grants`), which parses bare.
#[test]
fn test_parse_allow_and_grants() {
    assert_eq!(
        parse_slash_command("/allow sql:analytics"),
        Ok(Some(SlashCommand::Allow(vec!["sql:analytics".into()])))
    );
    assert_eq!(
        parse_slash_command("/allow sql:analytics runner:bench"),
        Ok(Some(SlashCommand::Allow(vec![
            "sql:analytics".into(),
            "runner:bench".into()
        ])))
    );
    assert_eq!(
        parse_slash_command("/grants"),
        Ok(Some(SlashCommand::Grants))
    );
    // Trailing arguments are ignored, like /doctor's.
    assert_eq!(
        parse_slash_command("/grants now"),
        Ok(Some(SlashCommand::Grants))
    );
    let error = parse_slash_command("/allow").unwrap_err();
    assert!(
        error.0.contains("not a listing") && error.0.contains("/grants"),
        "the error points at the listing, got: {error}"
    );
}

/// `/context` parses bare and ignores trailing text, like `/grants`.
#[test]
fn test_parse_context() {
    assert_eq!(
        parse_slash_command("/context"),
        Ok(Some(SlashCommand::Context))
    );
    assert_eq!(
        parse_slash_command("/context extra"),
        Ok(Some(SlashCommand::Context))
    );
}

/// `/mode` reports bare, switches on `plan`/`build` through the
/// `AgentMode::FromStr` grammar, and refuses anything else naming both
/// valid values. Matching `/approvals`'s case handling: the parse is
/// case-sensitive, so `Plan` is an error, not a mode.
#[test]
fn test_parse_mode_reports_switches_and_refuses() {
    use saya_agent::AgentMode;
    assert_eq!(
        parse_slash_command("/mode"),
        Ok(Some(SlashCommand::Mode(None)))
    );
    assert_eq!(
        parse_slash_command("/mode plan"),
        Ok(Some(SlashCommand::Mode(Some(AgentMode::Plan))))
    );
    assert_eq!(
        parse_slash_command("/mode build"),
        Ok(Some(SlashCommand::Mode(Some(AgentMode::Build))))
    );
    let error = parse_slash_command("/mode nonsense").unwrap_err();
    assert!(
        error.0.contains("plan") && error.0.contains("build"),
        "the error names both valid values, got: {error}"
    );
    assert!(
        parse_slash_command("/mode Plan").is_err(),
        "the parse is case-sensitive like /approvals"
    );
}
