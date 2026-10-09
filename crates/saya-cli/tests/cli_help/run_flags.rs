use super::*;

// ---------------------------------------------------------------------------
// `saya run` advertised `--continue`, an interactive flag it ignored. The
// flag continues the interactive REPL session — the only surface that reads
// it — and a run is not a session: it resumes by explicit id. A flag that
// implies a capability is available is worse than no flag, so the flag comes
// off the run surface (it is no longer global) and clap refuses
// `saya run --continue`. The pre-subcommand spelling (`saya --continue run`)
// is refused by the dispatch guard in `app.rs` with run-shaped guidance.
// ---------------------------------------------------------------------------

/// `saya run --continue` no longer parses: a script passing the flag gets a
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

/// `saya run --help` no longer advertises `--continue`, so the help surface
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

/// G2 slice 5 — `saya --host-commands` is a clap usage error: the flag was
/// deleted with no no-op alias, so a stale invocation gets exit 2. Moved
/// from `host_commands_flag_parses_for_the_bare_repl` (reason: the flag the
/// old test pinned no longer exists).
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

/// H1 red: the session-only `--allow command:<x>` launch seed parses on the
/// bare REPL. Written before the flag exists, so clap errors today.
#[test]
fn session_allow_launch_seed_parses_for_the_bare_repl() {
    let parsed =
        Cli::try_parse_from(["saya", "--allow", "command:npm"]).expect("the launch seed parses");
    assert!(parsed.command.is_none(), "no subcommand: the REPL path");
    assert_eq!(parsed.options.allow, vec!["command:npm".to_owned()]);
}
