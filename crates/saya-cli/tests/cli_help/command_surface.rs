use super::*;

// ---------------------------------------------------------------------------
// `config show` advertised two flags it never read. Both `--resolved` and
// `--redacted` were accepted and discarded
// since the initial release: `config show` always printed the resolved,
// redacted view regardless. Keeping a flag that implies redaction is optional
// is worse than no flag (redaction is never optional), so the
// slice removes both. Verified against the unchanged binary beforehand:
// `config show`, `config show --resolved`, `config show --redacted`, and
// `config show --resolved --redacted` produced byte-identical output (run with
// `--config /dev/null --connections /dev/null` for a hermetic input), and
// `config show --help` advertised both flags. The two tests below pin the
// post-removal surface: the flags no longer parse and are no longer advertised.
// ---------------------------------------------------------------------------

/// `config show --resolved` and `config show --redacted` no longer parse: a
/// script passing the dead flags now gets a clap error instead of a silent
/// success that changed nothing. This is the breaking change recorded in the
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

/// `config show --help` no longer advertises the removed flags, so the help
/// surface and behaviour agree. A user reading `--help` should
/// not find an off switch for redaction that does not exist.
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
