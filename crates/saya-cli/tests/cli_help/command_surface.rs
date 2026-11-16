use super::*;

// ---------------------------------------------------------------------------
// `config show` reads neither `--resolved` nor `--redacted`: it always prints
// the resolved, redacted view regardless. Keeping a flag that implies
// redaction is optional is worse than no flag (redaction is never optional),
// so both are usage errors. The two tests below pin the current surface: the
// flags do not parse and are not advertised.
// ---------------------------------------------------------------------------

/// `config show --resolved` and `config show --redacted` do not parse: a
/// script passing the dead flags gets a clap error instead of a silent
/// success that changed nothing. This is a breaking change recorded in the
/// changelog — the flags never had an effect, so the output is identical to
/// plain `config show` for anyone who drops them.
#[test]
fn config_show_no_longer_accepts_resolved_or_redacted() {
    for flag in ["--resolved", "--redacted"] {
        let parsed = Cli::try_parse_from(["saya", "config", "show", flag]);
        assert!(
            parsed.is_err(),
            "`config show {flag}` must not parse after S15: {parsed:?}"
        );
    }
}

/// `config show --help` does not advertise flags it does not accept, so the
/// help surface and behaviour agree. A user reading `--help` should not find an
/// off switch for redaction that does not exist.
#[test]
fn config_show_help_no_longer_advertises_resolved_or_redacted() {
    let mut cmd = Cli::command();
    let show_help = cmd
        .find_subcommand_mut("config")
        .expect("`config` subcommand exists")
        .find_subcommand_mut("show")
        .expect("`config show` subcommand exists")
        .render_help()
        .to_string();
    for flag in ["--resolved", "--redacted"] {
        assert!(
            !show_help.contains(flag),
            "`config show --help` must not advertise {flag} after S15"
        );
    }
}

/// `contracts approve-all --help` states what the batch
/// approves, that every item still gets the per-item validation (so some are
/// refused and every refusal is reported), and that consent is explicit via
/// `--yes`. A user deciding whether to run it must be able to learn from the
/// help alone that approving is preview-then-consent, never silent.
#[test]
fn approve_all_help_states_scope_consent_and_per_item_reporting() {
    let mut cmd = Cli::command();
    let help = cmd
        .find_subcommand_mut("contracts")
        .expect("`contracts` subcommand exists")
        .find_subcommand_mut("approve-all")
        .expect("`contracts approve-all` subcommand exists")
        .render_help()
        .to_string();
    // The scope: the bounded queue the user was shown, not the archive.
    assert!(
        help.contains("review queue"),
        "the help names the queue scope: {help}"
    );
    // The failure mode this command exists to prevent: refusals are reported,
    // not aggregated away.
    assert!(
        help.contains("refused"),
        "the help says refusals are reported: {help}"
    );
    // Consent is explicit and visible before anything happens.
    assert!(
        help.contains("--yes"),
        "the help documents the --yes consent flag: {help}"
    );

    // And the surface parses: scope is the optional --profile, the queue
    // bound is the optional --limit, consent is --yes.
    let parsed = Cli::try_parse_from([
        "saya",
        "contracts",
        "approve-all",
        "--yes",
        "--limit",
        "5",
        "--profile",
        "local",
    ])
    .expect("approve-all parses profile/limit/yes");
    match parsed.command {
        Some(saya_cli::Command::Contracts {
            command:
                saya_cli::ContractsCommand::ApproveAll {
                    profile,
                    limit,
                    yes,
                },
        }) => {
            assert_eq!(profile.as_deref(), Some("local"));
            assert_eq!(limit, Some(5));
            assert!(yes);
        }
        other => panic!("expected Contracts ApproveAll, got {other:?}"),
    }
}

/// `saya --help` does not list `coding`: the spelling is `saya start` and
/// `saya job`, and showing a retired name in the command list would advertise
/// a spelling that only answers with a refusal. The name still parses (so the
/// refusal is specific), but it is hidden.
#[test]
fn saya_help_no_longer_lists_coding() {
    let help = Cli::command().render_help().to_string();
    let listed = listed_commands(&help);
    assert!(
        !listed.contains(&"coding".to_string()),
        "`saya --help` must not list the retired `coding` command: {listed:?}"
    );
    assert!(
        listed.contains(&"job".to_string()) && listed.contains(&"start".to_string()),
        "the replacement spellings are listed: {listed:?}"
    );
}

/// The command names clap renders in the `Commands:` block. Wrapped
/// descriptions are indented further, so they are not names; the block ends
/// at the next section header or a blank line.
fn listed_commands(help: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut in_commands = false;
    for line in help.lines() {
        if line.trim_end() == "Commands:" {
            in_commands = true;
            continue;
        }
        if !in_commands {
            continue;
        }
        if line.is_empty() || !line.starts_with(' ') {
            break;
        }
        let rest = line.strip_prefix("  ").unwrap_or_default();
        if rest.is_empty() || rest.starts_with(' ') {
            continue;
        }
        if let Some(name) = rest.split_whitespace().next()
            && name != "help"
        {
            names.push(name.to_string());
        }
    }
    names
}

/// `saya job review --help` and `saya job apply --help` must not promise
/// checks. There are no check profiles: the operator's review and apply IS the
/// check, and a receipt written before the removal is tolerated but never
/// produced. Help that says "run configured checks" sends a user looking for
/// configuration that does nothing.
#[test]
fn job_review_and_apply_help_promise_no_checks() {
    let mut cmd = Cli::command();
    let job = cmd
        .find_subcommand_mut("job")
        .expect("`job` subcommand exists");
    for name in ["review", "review-existing", "apply"] {
        let help = job
            .find_subcommand_mut(name)
            .unwrap_or_else(|| panic!("`job {name}` subcommand exists"))
            .render_help()
            .to_string();
        assert!(
            !help.contains("checks"),
            "`job {name} --help` must not promise checks: {help}"
        );
        assert!(
            help.contains("--approve"),
            "`job {name} --help` still documents its consent flag: {help}"
        );
    }
}

/// `--candidates` parses into `GlobalOptions.candidates`, and its `--help`
/// text warns that each candidate is a full agent run — the one-line cost
/// note that stops a user from quietly multiplying their bill. Pins both the
/// surface and the wiring so the flag is not reintroduced as "advertised but
/// unread", the regression `config show`'s `--resolved`/`--redacted` once had.
#[test]
fn candidates_flag_parses_and_help_warns_about_cost() {
    let parsed = Cli::try_parse_from(["saya", "--candidates", "3"]).expect("--candidates parses");
    assert_eq!(parsed.options.candidates, Some(3));

    let help = Cli::command().render_help().to_string();
    assert!(help.contains("--candidates"), "help lists the flag: {help}");
    assert!(
        help.contains("full agent run"),
        "help must warn that each candidate is a full agent run: {help}"
    );
}

/// `-p/--print` is a top-level launch: it parses with no subcommand, and its
/// value is the task.
#[test]
fn print_flag_parses_without_a_subcommand() {
    let parsed = Cli::try_parse_from(["saya", "-p", "say hi"]).expect("`-p` parses");
    assert!(parsed.command.is_none(), "`-p` is the whole surface");
    assert_eq!(parsed.print.task.as_deref(), Some("say hi"));

    let long = Cli::try_parse_from(["saya", "--print", "say hi"]).expect("`--print` parses");
    assert_eq!(long.print.task.as_deref(), Some("say hi"));
}

/// The two CC spellings resolve to the same NDJSON format the stream uses.
#[test]
fn output_format_stream_json_is_ndjson() {
    for argv in [
        vec!["saya", "--output-format", "stream-json", "-p", "a task"],
        vec!["saya", "--format", "stream-json", "-p", "a task"],
        vec!["saya", "--format", "ndjson", "-p", "a task"],
    ] {
        let parsed = Cli::try_parse_from(argv).expect("the format spelling parses");
        assert_eq!(
            parsed.options.format,
            saya_cli::FormatArg::Ndjson,
            "stream-json is ndjson"
        );
    }
}

/// `--job-id` shapes the `-p` launch, so it cannot be stated without it.
#[test]
fn job_id_requires_print() {
    let error = Cli::try_parse_from(["saya", "--job-id", "job-1"])
        .expect_err("`--job-id` alone is not a launch");
    assert_eq!(
        error.kind(),
        clap::error::ErrorKind::MissingRequiredArgument,
        "{error}"
    );

    let parsed =
        Cli::try_parse_from(["saya", "-p", "a task", "--job-id", "job-1"]).expect("parses");
    assert_eq!(parsed.print.job_id.as_deref(), Some("job-1"));
}

/// `-` is a task value meaning stdin, not a flag.
#[test]
fn print_dash_reads_stdin() {
    let parsed = Cli::try_parse_from(["saya", "-p", "-"]).expect("`-` is a task value");
    assert_eq!(parsed.print.task.as_deref(), Some("-"));
}

/// The root help advertises the one-shot surface.
#[test]
fn root_help_lists_the_print_flag() {
    let help = Cli::command().render_help().to_string();
    assert!(
        help.contains("--print"),
        "`saya --help` lists `--print`: {help}"
    );
}
