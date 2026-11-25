/// `/usage` bare reads the session's panel; `--limit <usd>` states the
/// session's ceiling and carries it as micro-USD, the unit the spend is kept
/// in.
#[test]
fn usage_parses_the_panel_and_its_limit() {
    for (text, expected) in [
        ("/usage", UsageRequest::show()),
        ("/usage --limit 2.5", UsageRequest::limit(2_500_000)),
        ("/usage --limit 3", UsageRequest::limit(3_000_000)),
        ("/usage --limit 0.000001", UsageRequest::limit(1)),
    ] {
        assert_eq!(
            parse_slash_command(text).unwrap(),
            Some(SlashCommand::Usage(expected)),
            "{text}"
        );
    }
}

/// Anything else after `/usage` is a usage error that names the one form, so a
/// typo cannot read as a silent show or a silently ignored ceiling.
#[test]
fn usage_refuses_an_argument_it_does_not_understand() {
    for text in [
        "/usage 2",
        "/usage --limit",
        "/usage --limit cheap",
        "/usage --limit 0",
        "/usage --limit -1",
        "/usage --limit 1000001",
    ] {
        let error = parse_slash_command(text).expect_err(text);
        assert!(
            error.0.contains("--limit <usd>"),
            "{text} must name the valid form: {error}"
        );
    }
}
