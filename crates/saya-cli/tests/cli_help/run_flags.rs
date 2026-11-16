use super::*;

// ---------------------------------------------------------------------------
// `saya run` advertised `--continue`, an interactive flag it ignored. The
// flag continues the interactive REPL session — the only surface that reads
// it — and a run is not a session: it resumes by explicit id. A flag that
// implies a capability is available is worse than no flag, so the flag comes
// off the run surface (it is not global) and clap refuses
// `saya run --continue`. The pre-subcommand spelling (`saya --continue run`)
// is refused by the dispatch guard in `app.rs` with run-shaped guidance.
// ---------------------------------------------------------------------------

/// `saya run --continue` does not parse: a script passing the flag gets a
/// clap usage error instead of a run that silently ignored it — the same
/// accepted-and-discarded defect `config show --resolved`'s removal fixed.
#[test]
fn run_no_longer_accepts_continue() {
    let parsed = Cli::try_parse_from(["saya", "run", "--continue", "goal"]);
    assert!(
        parsed.is_err(),
        "`saya run --continue` must not parse once the flag is off the run surface: {parsed:?}"
    );
}

/// `saya run --help` does not advertise `--continue`, so the help surface
/// and the run grammar agree. The bare REPL keeps the flag: its root `--help`
/// still lists it, which `mvp/cli.rs` pins. The tree is built before the
/// subcommand renders — the propagation step that puts a global arg into a
/// subcommand's help, i.e. what `saya run --help` actually prints.
#[test]
fn run_help_no_longer_advertises_continue() {
    let mut cmd = Cli::command();
    cmd.build();
    let run_help = cmd
        .find_subcommand_mut("run")
        .expect("`saya run` is declared")
        .render_help()
        .to_string();
    assert!(
        !run_help.contains("--continue"),
        "`saya run --help` must not advertise the REPL-only `--continue`: {run_help}"
    );
}

/// The bare REPL keeps the flag: `saya --continue` parses at the top level,
/// where its only consumer (the interactive session resume) reads it.
#[test]
fn continue_still_parses_for_the_bare_repl() {
    let parsed = Cli::try_parse_from(["saya", "--continue"]).expect("bare `--continue` parses");
    assert!(parsed.command.is_none(), "no subcommand: the REPL path");
    assert!(parsed.options.continue_session, "the flag reached options");
}

/// `saya --host-commands` is a clap usage error: the flag does not exist and
/// has no no-op alias, so a stale invocation gets exit 2.
#[test]
fn host_commands_flag_is_now_a_usage_error() {
    let error = Cli::try_parse_from(["saya", "--host-commands"])
        .expect_err("--host-commands must not parse after G2");
    assert_eq!(
        error.kind(),
        clap::error::ErrorKind::UnknownArgument,
        "a stale flag is a usage error"
    );
    let rendered = error.to_string();
    assert!(
        rendered.contains("host-commands"),
        "the usage error names the stale spelling: {rendered}"
    );
    let help = saya_cli::Cli::command().render_help().to_string();
    assert!(
        !help.contains("--host-commands"),
        "the root help no longer names the deleted flag"
    );
}

/// The trusted command rules are one parser for every launch: `saya start`,
/// `saya run` and the bare REPL all accept the repeatable flags.
#[test]
fn command_rule_flags_parse_on_every_launch() {
    let repl = Cli::try_parse_from([
        "saya",
        "--allow-command",
        "cargo test",
        "--deny-command",
        "rm",
    ])
    .expect("the bare REPL accepts the rule flags");
    assert!(repl.command.is_none());
    assert_eq!(repl.options.allow_command, vec!["cargo test".to_owned()]);
    assert_eq!(repl.options.deny_command, vec!["rm".to_owned()]);

    let run = Cli::try_parse_from(["saya", "run", "--allow-command", "cargo test", "goal"])
        .expect("`saya run` accepts the rule flags");
    assert_eq!(run.options.allow_command, vec!["cargo test".to_owned()]);

    let start = Cli::try_parse_from([
        "saya",
        "start",
        "--allow-command",
        "cargo test",
        "--deny-command",
        "rm",
        "job-id",
        "task",
    ])
    .expect("`saya start` accepts the rule flags");
    assert_eq!(start.options.allow_command, vec!["cargo test".to_owned()]);
    assert_eq!(start.options.deny_command, vec!["rm".to_owned()]);
}

/// The session-only `--allow command:<x>` launch seed parses on the
/// bare REPL.
#[test]
fn session_allow_launch_seed_parses_for_the_bare_repl() {
    let parsed =
        Cli::try_parse_from(["saya", "--allow", "command:npm"]).expect("the launch seed parses");
    assert!(parsed.command.is_none(), "no subcommand: the REPL path");
    assert_eq!(parsed.options.allow, vec!["command:npm".to_owned()]);
}
