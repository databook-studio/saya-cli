use super::*;

/// `--plain` is a launch surface a user has to be able to find: `saya --help`
/// states the flag and the environment spelling beside it, so nobody has to
/// read the source to learn the line REPL is still reachable.
#[test]
fn plain_flag_is_advertised_in_help() {
    let mut cmd = Cli::command();
    cmd.build();
    let help = cmd.render_help().to_string();
    // clap wraps to the terminal width, so the claims are matched on the
    // unwrapped text rather than across a line break.
    let flat = help.split_whitespace().collect::<Vec<_>>().join(" ");

    assert!(
        flat.contains("--plain"),
        "`saya --help` must list the plain-REPL flag: {flat}"
    );
    assert!(
        flat.contains("SAYA_PLAIN=1"),
        "the one help text names the environment spelling: {flat}"
    );
    assert!(
        flat.contains("instead of the full-screen TUI"),
        "the help says what the flag chooses between: {flat}"
    );
}

/// The flag parses on the bare REPL and defaults off: `saya` with no flag is
/// still the full-screen TUI, so an unstated surface must read as unchanged.
#[test]
fn plain_flag_parses_and_defaults_off() {
    let plain = Cli::try_parse_from(["saya", "--plain"]).expect("bare `--plain` parses");
    assert!(plain.command.is_none(), "no subcommand: the REPL path");
    assert!(plain.options.plain, "the flag reached options");

    let bare = Cli::try_parse_from(["saya"]).expect("bare `saya` parses");
    assert!(
        !bare.options.plain,
        "an unstated surface is the full-screen TUI"
    );
}
