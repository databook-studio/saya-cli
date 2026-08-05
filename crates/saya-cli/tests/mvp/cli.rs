use clap::{CommandFactory, Parser};
use saya_cli::{
    Cli, Command, FormatArg, RenderFormat, SessionAction, SessionState, SlashCommand,
    TerminalEvent, parse_slash_command, render_event,
};

#[test]
fn help_exposes_interactive_and_automation_flags() {
    let help = Cli::command().render_help().to_string();
    for flag in [
        "--continue",
        "--resume",
        "--include-profile",
        "--approval-mode",
        "--env-file",
        "--allow-data-sharing",
        "--non-interactive",
    ] {
        assert!(help.contains(flag), "missing {flag}");
    }
}

#[test]
fn parser_is_public_and_testable() {
    let cli = Cli::try_parse_from([
        "saya",
        "--profile",
        "analytics",
        "--format",
        "json",
        "ask",
        "revenue",
    ])
    .unwrap();
    assert_eq!(cli.options.profile.as_deref(), Some("analytics"));
    assert_eq!(cli.options.format, FormatArg::Json);
    assert!(
        matches!(cli.command, Some(Command::Ask { prompt: Some(value), .. }) if value == "revenue")
    );
}

#[test]
fn slash_commands_parse_and_state_transitions_are_deterministic() {
    let mut state = SessionState::new("one", None, "model-a");
    assert_eq!(
        parse_slash_command("/connect analytics").unwrap(),
        Some(SlashCommand::Connect("analytics".into()))
    );
    let selected = state.apply(
        SlashCommand::Connect("analytics".into()),
        &["analytics".into()],
    );
    assert!(
        matches!(selected, SessionAction::Message(ref message) if message == "Selected profile: analytics")
    );
    assert!(matches!(
        state.apply(
            SlashCommand::Connect("unknown".into()),
            &["analytics".into()]
        ),
        SessionAction::Error(_)
    ));
    assert!(matches!(
        state.apply(
            SlashCommand::Include("unknown".into()),
            &["analytics".into()]
        ),
        SessionAction::Error(_)
    ));
    state.apply(
        SlashCommand::Include("staging".into()),
        &["analytics".into(), "staging".into()],
    );
    state.apply(SlashCommand::Privacy(Some(true)), &[]);
    state.apply(
        parse_slash_command("/approvals never").unwrap().unwrap(),
        &[],
    );
    assert_eq!(state.profile.as_deref(), Some("analytics"));
    assert_eq!(state.included_profiles, vec!["staging"]);
    assert!(state.allow_data_sharing);
    assert_eq!(state.approval_mode, "never");
    assert!(matches!(
        state.apply(SlashCommand::History, &[]),
        SessionAction::History
    ));
    assert!(matches!(
        state.apply(SlashCommand::Schema(false), &[]),
        SessionAction::Schema(false)
    ));
    assert_eq!(state.apply(SlashCommand::Exit, &[]), SessionAction::Exit);
}

#[test]
fn renderer_separates_diagnostics_and_emits_stable_envelopes() {
    let result = render_event(
        &TerminalEvent::Result {
            message: "ok".into(),
        },
        RenderFormat::Json,
    );
    assert_eq!(result.stderr, "");
    assert_eq!(result.stdout, "{\"event\":\"result\",\"message\":\"ok\"}\n");
    let diagnostic = render_event(
        &TerminalEvent::Diagnostic {
            message: "safe".into(),
        },
        RenderFormat::Ndjson,
    );
    assert_eq!(diagnostic.stdout, "");
    assert_eq!(
        diagnostic.stderr,
        "{\"event\":\"diagnostic\",\"message\":\"safe\"}\n"
    );
    let complete = render_event(&TerminalEvent::Complete, RenderFormat::Ndjson);
    assert_eq!(complete.stdout, "{\"event\":\"complete\"}\n");
    assert_eq!(complete.stderr, "");
}
