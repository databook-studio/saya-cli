use super::*;

#[test]
fn every_subcommand_and_argument_has_help_text() {
    let root = Cli::command();
    let mut offenders: Vec<String> = Vec::new();
    walk(&root, &mut offenders);

    if !offenders.is_empty() {
        panic!(
            "blank --help entries (no description found):\n  - {}\n\
             Add a `///` doc comment (subcommands) or `///` on the arg field \
             (arguments) so `--help` is never silent.",
            offenders.join("\n  - ")
        );
    }
}

/// `--tools` is declared once, on the global options. A local re-declaration
/// on `start` would make clap's propagation skip the global copy into that
/// subcommand: the flag would read as a job-scoped narrowing in one help text
/// and as the saya-wide list in the other. The subcommand's help must be the
/// one global text.
#[test]
fn start_shows_the_one_global_tools_help() {
    // The help a user actually sees, so the global args clap propagates at
    // build time are the ones under test.
    let help = Cli::command()
        .try_get_matches_from(["saya", "start", "--help"])
        .expect_err("`--help` renders and exits")
        .to_string();

    assert_eq!(
        help.matches("--tools").count(),
        1,
        "the flag is declared once for `saya start`: {help}"
    );
    assert!(
        help.contains("Advertise only these model tools"),
        "`saya start` shows the global flag's one help text: {help}"
    );
}

/// A headless driver — `--non-interactive`, or no terminal — defaults to
/// `never`, which advertises no `shell`. The one-line `--help` keeps that
/// fact; the full explanation, including the bypass way out, is in
/// `docs/commands.md`.
#[test]
fn start_help_states_the_headless_approval_default() {
    let help = Cli::command()
        .try_get_matches_from(["saya", "start", "--help"])
        .expect_err("`--help` renders and exits")
        .to_string();
    // clap wraps to the terminal width, so the claims are matched on the
    // unwrapped text rather than across a line break.
    let flat = help.split_whitespace().collect::<Vec<_>>().join(" ");

    assert!(
        flat.contains("headless default: never, no `shell`"),
        "the headless default is stated: {flat}"
    );
    assert!(
        flat.contains("--approval-mode"),
        "the flag that changes the default is stated: {flat}"
    );
}

/// `saya run`'s summary names the run-management grammar. The words are
/// subcommands, so a stray space inside `resume|cancel|list|show|log` reads as
/// a command named ` list` — a name the binary does not have.
#[test]
fn run_help_spells_the_run_management_grammar_without_gaps() {
    let mut cmd = Cli::command();
    let help = cmd
        .find_subcommand_mut("run")
        .expect("`run` subcommand exists")
        .render_help()
        .to_string();
    let flat = help.split_whitespace().collect::<Vec<_>>().join(" ");

    assert!(
        flat.contains("resume|cancel|list|show|log"),
        "the run-management grammar is stated: {flat}"
    );
    assert!(
        !flat.contains("list| show"),
        "no space splits a subcommand name: {flat}"
    );
}

#[test]
fn config_doctor_help_describes_configuration_without_reachability_claims() {
    let mut cmd = Cli::command();
    let help = cmd
        .find_subcommand_mut("config")
        .expect("`config` subcommand exists")
        .find_subcommand_mut("doctor")
        .expect("`config doctor` subcommand exists")
        .render_help()
        .to_string();

    assert!(
        help.contains("configured"),
        "help must describe configuration: {help}"
    );
    assert!(
        !help.contains("reachable"),
        "help must not claim provider reachability: {help}"
    );
}
